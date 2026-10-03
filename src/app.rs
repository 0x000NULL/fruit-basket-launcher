//! The launcher's state and its frame: gather input, draw, apply commands.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use basket_app::pads::{Gamepads, PadPoll, Ports};
use basket_ui::input::{Action, ButtonSet, KeyMap, MenuRoles, PadMap, UiInput};
use basket_ui::{Canvas, Fonts};
use gilrs::Button;
use minifb::Key;

use crate::art::Art;
use crate::basket::{Basket, Current};
use crate::feed::{self, Channel, Feed, FeedError, Fetched, Fruit, Status};
use crate::history::{self, Entry};
use crate::jobs::{Job, Worker};
use crate::key;
use crate::platform;
use crate::queue::{self, Finished, Queue};
use crate::settings::{Settings, ThemePref};
use crate::shelf::Shelf;
use crate::ui::basket::{BasketView, Card, CardState, Detail, Primary};
use crate::ui::downloads::DownloadsView;
use crate::ui::library::Row;
use crate::ui::frame::{self, FrameView};
use crate::ui::settings::{FruitStorage, SettingsView};
use crate::ui::{self, capitalise, Cmd, Flag, Size, Tab, Ui};
use crate::window::Video;

pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

// Menu buttons: the launcher has no game, so the set is just the roles.
const UP: u16 = 1;
const DOWN: u16 = 2;
const LEFT: u16 = 4;
const RIGHT: u16 = 8;
const A: u16 = 16;
const B: u16 = 32;
const X: u16 = 64;
const Y: u16 = 128;
const LB: u16 = 256;
const RB: u16 = 512;
const START: u16 = 1024;

pub const BUTTONS: ButtonSet = ButtonSet {
    names: &["UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "LB", "RB", "START"],
    bits: &[UP, DOWN, LEFT, RIGHT, A, B, X, Y, LB, RB, START],
    roles: MenuRoles { up: UP, down: DOWN, left: LEFT, right: RIGHT, confirm: A, back: B, prev_page: LB, next_page: RB, start: START },
};

fn keymap() -> KeyMap {
    KeyMap::new(
        BUTTONS,
        &[(Key::Up, UP), (Key::Down, DOWN), (Key::Left, LEFT), (Key::Right, RIGHT), (Key::Z, A), (Key::X, B), (Key::PageUp, LB), (Key::PageDown, RB), (Key::Enter, START)],
    )
}

fn padmap() -> PadMap {
    PadMap::new(
        BUTTONS,
        &[
            (Button::DPadUp, UP),
            (Button::DPadDown, DOWN),
            (Button::DPadLeft, LEFT),
            (Button::DPadRight, RIGHT),
            (Button::South, A),
            (Button::East, B),
            (Button::West, X),
            (Button::North, Y),
            (Button::LeftTrigger, LB),
            (Button::RightTrigger, RB),
            (Button::Start, START),
        ],
    )
}

pub struct App {
    pub settings: Settings,
    pub basket: Basket,
    art: Art,
    canvas: Canvas,
    pub feed: Option<Feed>,
    feed_rx: Option<Receiver<Result<Fetched, FeedError>>>,
    pub feed_error: Option<String>,
    pub worker: Worker,
    pub tab: Tab,
    pub find: String,
    pub find_focused: bool,
    scroll: f32,
    scroll_max: f32,
    os_dark: bool,
    os_dark_at: Instant,
    pad_used: bool,
    pub quit: bool,
    launcher_update: Option<String>,
    storage: Option<(String, Vec<FruitStorage>)>,
    keymap: KeyMap,
    padmap: PadMap,
    queue: Queue,
    history: Vec<Entry>,
    /// What each fruit runs now, read from disk.
    installed: HashMap<String, Current>,
    /// Sizes and game counts, computed when the Basket tab needs them.
    disk: Option<HashMap<String, Disk>>,
    /// The Basket tab's selected fruit; `None` picks a sensible one.
    selected: Option<String>,
    /// Narrow: the selected fruit is open as a sheet.
    sheet: bool,
    aside_scroll: f32,
    aside_scroll_max: f32,
    /// Channel picked in the aside for a fruit not installed yet.
    chosen: HashMap<String, Channel>,
    /// The Library tab's games and everything known about them.
    pub shelf: Shelf,
    /// Why the last Play didn't start, shown in the footer.
    play_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Disk {
    pub program: u64,
    pub games_size: u64,
    pub games: usize,
}

impl App {
    pub fn new() -> App {
        let settings = Settings::load();
        let basket = settings.basket();
        let cached = feed::load_cached(&basket.launcher_dir(), &key::public_key());
        let mut app = App::with(settings, basket, cached);
        if app.settings.check_on_open || app.feed.is_none() {
            app.check_feed();
        }
        app
    }

    /// An app with no window and no network: `new` without the side effects.
    pub fn with(settings: Settings, basket: Basket, cached: Option<Feed>) -> App {
        let history = history::read(&basket.launcher_dir());
        let shelf = Shelf::load(&basket.launcher_dir());
        let mut app = App {
            worker: Worker::start(basket.clone()),
            feed: cached,
            feed_rx: None,
            feed_error: None,
            art: Art::load(),
            canvas: Canvas::new(basket_ui::tokens::layout::DESIGN_W, basket_ui::tokens::layout::DESIGN_H, Fonts::new()),
            tab: Tab::Library,
            find: String::new(),
            find_focused: false,
            scroll: 0.0,
            scroll_max: 0.0,
            os_dark: platform::os_dark(),
            os_dark_at: Instant::now(),
            pad_used: false,
            quit: false,
            launcher_update: None,
            storage: None,
            keymap: keymap(),
            padmap: padmap(),
            queue: Queue::default(),
            history,
            installed: HashMap::new(),
            disk: None,
            selected: None,
            sheet: false,
            aside_scroll: 0.0,
            aside_scroll_max: 0.0,
            chosen: HashMap::new(),
            shelf,
            play_error: None,
            settings,
            basket,
        };
        app.refresh_installed();
        if let Some(feed) = &app.feed {
            app.shelf.lists(&app.basket.launcher_dir(), feed);
        }
        app
    }

    /// Re-read every fruit's `current` file.
    fn refresh_installed(&mut self) {
        self.installed.clear();
        for f in self.feed.iter().flat_map(|f| f.fruits.iter()) {
            if let Some(c) = self.basket.current(&f.id) {
                self.installed.insert(f.id.clone(), c);
            }
        }
        self.disk = None;
        self.storage = None;
        self.rescan();
    }

    fn rescan(&mut self) {
        self.shelf.rescan(&self.basket, self.feed.as_ref(), &self.installed, &self.settings);
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            feed: self.feed.as_ref(),
            queue: &self.queue,
            installed: &self.installed,
            settings: &self.settings,
            chosen: &self.chosen,
            find: &self.find,
            selected: self.selected.as_deref(),
        }
    }

    /// Queue a job; start it if the worker is idle.
    fn enqueue(&mut self, job: Job) {
        if let Some(job) = self.queue.enqueue(job) {
            self.worker.send(job);
        }
    }

    pub fn window_size(&self) -> (usize, usize) {
        match self.settings.window {
            Some((w, h)) if w >= 480 && h >= 400 => (w as usize, h as usize),
            _ => (1280, 900),
        }
    }

    /// Fetch the feed on a background thread; `poll` picks the result up.
    pub fn check_feed(&mut self) {
        if self.feed_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let newest = self.feed.as_ref().map(|f| f.generated.clone());
        thread::spawn(move || {
            let _ = tx.send(feed::fetch(&key::feed_url(), &key::public_key(), newest.as_deref()));
        });
        self.feed_rx = Some(rx);
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.feed_rx {
            if let Ok(result) = rx.try_recv() {
                self.feed_rx = None;
                match result {
                    Ok(fetched) => {
                        if let Err(e) = feed::save_cached(&self.basket.launcher_dir(), &fetched) {
                            eprintln!("fruitbasket: caching feed: {e}");
                        }
                        self.feed = Some(fetched.feed);
                        self.feed_error = None;
                        self.refresh_installed();
                        if let Some(feed) = &self.feed {
                            self.shelf.lists(&self.basket.launcher_dir(), feed);
                        }
                        if self.settings.install_without_asking {
                            self.apply(Cmd::UpdateAll);
                        }
                    }
                    Err(e) => self.feed_error = Some(e.to_string()),
                }
            }
        }
        for event in self.worker.poll() {
            let (finished, next) = self.queue.on_event(&event);
            if let Some(job) = next {
                self.worker.send(job);
            }
            let Some(finished) = finished else { continue };
            let (job, failed, message) = match finished {
                Finished::Installed(job) => (job, None, String::new()),
                Finished::Failed(f) => (f.job, Some(f.kind), f.message),
            };
            let entry = Entry { when: std::time::SystemTime::now(), fruit: job.fruit, build: job.build, channel: job.channel, failed, message };
            if let Err(e) = history::append(&self.basket.launcher_dir(), &entry) {
                eprintln!("fruitbasket: writing history: {e}");
            }
            self.history.insert(0, entry);
            self.history.truncate(history::SHOWN);
            self.refresh_installed();
        }
        if self.shelf.poll(&self.basket.launcher_dir(), self.feed.as_ref()) && self.shelf.running().is_none() {
            self.play_error = None;
        }
        if self.settings.theme == ThemePref::System && self.os_dark_at.elapsed() > Duration::from_secs(5) {
            self.os_dark = platform::os_dark();
            self.os_dark_at = Instant::now();
        }
    }

    pub fn night(&self) -> bool {
        match self.settings.theme {
            ThemePref::System => self.os_dark,
            ThemePref::Paper => false,
            ThemePref::Night => true,
        }
    }

    pub fn gather(&mut self, video: &mut Video, pads: &mut Gamepads) -> UiInput {
        let pad: PadPoll = pads.poll(&self.padmap);
        let input = video.gather_input(&self.keymap, pad, Instant::now());
        if !input.pad_buttons.is_empty() {
            self.pad_used = true;
        } else if !input.pressed.is_empty() || input.clicked {
            self.pad_used = false;
        }
        input
    }

    /// One frame: draw from state, apply what the frame asked for, present.
    pub fn frame(&mut self, video: &mut Video, input: &UiInput) {
        self.poll();
        let (w, h) = video.size();
        self.draw(input, w as u32, h as u32);
        video.present(&self.canvas);
    }

    /// Draw at `w`×`h` and apply the frame's commands. No window needed.
    pub fn draw(&mut self, input: &UiInput, w: u32, h: u32) {
        self.canvas.resize(w, h);

        let status = self.status();
        let hints: Vec<(&str, &str)> = self.hints();
        let night = self.night();
        if self.tab == Tab::Settings && self.storage.is_none() {
            self.storage = Some(storage(&self.basket, self.feed.as_ref()));
        }
        if self.tab == Tab::Basket && self.disk.is_none() {
            self.disk = Some(disk(&self.basket, self.feed.as_ref()));
        }
        let sv = match (&self.storage, self.tab) {
            (Some(st), Tab::Settings) => Some(settings_view(&self.settings, &self.basket, self.feed.as_ref(), st, self.scroll, self.os_dark)),
            _ => None,
        };
        let ctx = Ctx {
            feed: self.feed.as_ref(),
            queue: &self.queue,
            installed: &self.installed,
            settings: &self.settings,
            chosen: &self.chosen,
            find: &self.find,
            selected: self.selected.as_deref(),
        };
        let root = format!("{}{}", platform::tilde(&self.basket.root), std::path::MAIN_SEPARATOR);
        let no_disk = HashMap::new();
        let bv = (self.tab == Tab::Basket).then(|| {
            let note = self.feed_error.as_deref().filter(|_| ctx.feed.is_none());
            ctx.basket_view(self.disk.as_ref().unwrap_or(&no_disk), root, self.scroll, self.aside_scroll, self.sheet, note)
        });
        let name = |id: &str| ctx.name(id);
        let has_build = |id: &str| ctx.installed.contains_key(id);
        let dv = (self.tab == Tab::Downloads).then(|| DownloadsView {
            active: self.queue.active(),
            failures: self.queue.failures().collect(),
            waiting: self.queue.waiting().collect(),
            earlier: &self.history,
            name: &name,
            has_build: &has_build,
            key_id: key::KEY_ID,
            scroll: self.scroll,
        });
        let lv = (self.tab == Tab::Library).then(|| self.shelf.view(&self.basket, self.feed.as_ref(), &self.find, self.sheet, self.scroll, self.aside_scroll));
        let mut cmds = {
            let mut ui = Ui::new(&mut self.canvas, input, &self.art, night, self.pad_used);
            let fv = FrameView {
                tab: self.tab,
                find: &self.find,
                find_focused: self.find_focused,
                updates: ctx.update_count(),
                downloads: self.queue.count(),
                launcher_update: self.launcher_update.as_deref(),
                hints: &hints,
                status: &status,
                controller: self.pad_used,
            };
            let top = frame::header(&mut ui, &fv);
            let bottom = ui.h() - 52.0;
            if let Some(sv) = &sv {
                ui::settings::draw(&mut ui, sv, top, bottom);
            } else if let Some(bv) = &bv {
                ui::basket::draw(&mut ui, bv, top, bottom);
            } else if let Some(dv) = &dv {
                ui::downloads::draw(&mut ui, dv, top, bottom);
            } else if let Some(lv) = &lv {
                ui::library::draw(&mut ui, lv, top, bottom);
            }
            frame::footer(&mut ui, &fv);
            ui.cmds
        };
        drop((sv, bv, dv, lv));
        cmds.extend(self.keys(input));
        for cmd in cmds {
            self.apply(cmd);
        }
    }

    /// Keyboard and pad shortcuts that aren't tied to something drawn.
    fn keys(&self, input: &UiInput) -> Vec<Cmd> {
        let mut out = Vec::new();
        if self.find_focused {
            return out;
        }
        if self.tab == Tab::Library {
            let rows = self.shelf.rows(self.feed.as_ref(), &self.find);
            let at = self.shelf.selected_in(&rows).and_then(|p| rows.iter().position(|r| r.game.path == p));
            let cols = if self.shelf.list { 1 } else if Size::of(self.canvas.width()) == Size::Narrow { 4 } else { 5 };
            let step = if input.action(Action::Left) {
                -1
            } else if input.action(Action::Right) {
                1
            } else if input.action(Action::Up) {
                -cols
            } else if input.action(Action::Down) {
                cols
            } else {
                0
            };
            if step != 0 && !rows.is_empty() {
                let i = at.map_or(0, |i| (i as i32 + step).clamp(0, rows.len() as i32 - 1) as usize);
                out.push(Cmd::SelectGame(rows[i].game.path.clone()));
            }
            if input.action(Action::Confirm) {
                if let Some(p) = self.shelf.selected_in(&rows) {
                    out.push(Cmd::Play(p.to_path_buf()));
                }
            }
        }
        if self.tab == Tab::Basket {
            let ctx = self.ctx();
            let order = ctx.order();
            let at = ctx.selected_id().and_then(|id| order.iter().position(|f| f.id == id));
            let step = if input.action(Action::Left) || input.action(Action::Up) {
                -1
            } else if input.action(Action::Right) || input.action(Action::Down) {
                1
            } else {
                0
            };
            if step != 0 && !order.is_empty() {
                let i = at.map_or(0, |i| (i as i32 + step).clamp(0, order.len() as i32 - 1) as usize);
                out.push(Cmd::Select(order[i].id.clone()));
            }
            if input.action(Action::Confirm) {
                if let Some(cmd) = ctx.selected_fruit().and_then(|f| ctx.primary_cmd(f)) {
                    out.push(cmd);
                }
            }
        }
        if input.pressed(Key::Slash) {
            out.push(Cmd::FocusFind(true));
        }
        // Esc closes the narrow sheet first (the sheet emits that itself).
        let sheet_open = self.sheet && matches!(self.tab, Tab::Basket | Tab::Library) && Size::of(self.canvas.width()) == Size::Narrow;
        if input.pressed(Key::Escape) && !sheet_open {
            out.push(Cmd::Quit);
        }
        if input.action(Action::PrevPage) {
            out.push(Cmd::Tab(self.tab.step(-1)));
        }
        if input.action(Action::NextPage) {
            out.push(Cmd::Tab(self.tab.step(1)));
        }
        out
    }

    fn apply(&mut self, cmd: Cmd) {
        let s = &mut self.settings;
        match cmd {
            Cmd::Tab(tab) => {
                if tab != self.tab {
                    self.tab = tab;
                    self.scroll = 0.0;
                    self.aside_scroll = 0.0;
                    self.sheet = false;
                    if tab == Tab::Settings {
                        self.storage = None;
                    }
                    if tab == Tab::Basket {
                        self.disk = None;
                    }
                    if tab == Tab::Library && self.settings.rescan_on_open {
                        self.rescan();
                    }
                }
            }
            Cmd::FocusFind(on) => self.find_focused = on,
            // Typing '/' to focus FIND must not also type the slash.
            Cmd::Find(text) => self.find = text.trim_start_matches('/').to_string(),
            Cmd::Quit => self.quit = true,
            Cmd::Scroll(dy) => self.scroll = (self.scroll + dy).clamp(0.0, self.scroll_max),
            Cmd::ScrollMax(max) => {
                self.scroll_max = max;
                self.scroll = self.scroll.min(max);
            }
            Cmd::AsideScroll(dy) => self.aside_scroll = (self.aside_scroll + dy).clamp(0.0, self.aside_scroll_max),
            Cmd::AsideScrollMax(max) => {
                self.aside_scroll_max = max;
                self.aside_scroll = self.aside_scroll.min(max);
            }
            Cmd::Theme(t) => {
                s.theme = t;
                s.save();
            }
            Cmd::NewChannel(c) => {
                s.new_channel = c;
                s.save();
            }
            Cmd::Keep(k) => {
                s.keep = k;
                s.save();
            }
            Cmd::Toggle(flag) => {
                let f = match flag {
                    Flag::RescanOnOpen => &mut s.rescan_on_open,
                    Flag::CheckOnOpen => &mut s.check_on_open,
                    Flag::InstallWithoutAsking => &mut s.install_without_asking,
                    Flag::CouchOnController => &mut s.couch_on_controller,
                };
                *f = !*f;
                s.save();
            }
            Cmd::RemoveFolder(i) => {
                if i < s.folders.len() {
                    s.folders.remove(i);
                    s.save();
                    self.rescan();
                }
            }
            Cmd::AddFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Add a game folder").pick_folder() {
                    if !s.folders.contains(&dir) {
                        s.folders.push(dir);
                        s.save();
                    }
                    self.rescan();
                }
            }
            Cmd::LibFilter(f) => {
                self.shelf.filter = f;
                self.scroll = 0.0;
            }
            Cmd::LibSort(az) => self.shelf.az = az,
            Cmd::LibView(list) => self.shelf.list = list,
            Cmd::SelectGame(p) => {
                if self.shelf.selected.as_ref() != Some(&p) {
                    self.aside_scroll = 0.0;
                }
                self.shelf.selected = Some(p);
                self.sheet = true;
            }
            Cmd::Play(p) => {
                self.play_error = self.shelf.play(&p, &self.basket, self.feed.as_ref()).err();
            }
            Cmd::ShowFile(p) => platform::reveal(&p),
            Cmd::ShowSaves(p) => {
                let fruit = self.shelf.games.iter().find(|g| g.path == p).map(|g| g.fruit.clone());
                let build = fruit.and_then(|f| self.basket.current(&f).map(|c| self.basket.build_dir(&f, &c.build)));
                if let Some(save) = crate::library::saves(&p, build.as_deref()).first() {
                    platform::reveal(save);
                }
            }
            Cmd::RemoveGame(p) => {
                if !s.hidden.contains(&p) {
                    s.hidden.push(p.clone());
                    s.save();
                }
                self.shelf.remove(&p);
            }
            Cmd::ClearOldBuilds(id) => {
                if let Err(e) = self.basket.prune(&id, 0) {
                    eprintln!("fruitbasket: clearing old {id} builds: {e}");
                }
                self.storage = None;
            }
            Cmd::CheckNow => self.check_feed(),
            Cmd::OpenUrl(url) => platform::open(&url),
            Cmd::DismissLauncherUpdate => self.launcher_update = None,
            Cmd::Select(id) => {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.aside_scroll = 0.0;
                }
                self.selected = Some(id);
                self.sheet = true;
            }
            Cmd::CloseSheet => self.sheet = false,
            Cmd::Install(id) => {
                let ctx = self.ctx();
                let job = ctx.fruit(&id).and_then(|f| queue::job_for(f, ctx.new_channel(f), self.settings.keep as usize));
                if let Some(job) = job {
                    self.enqueue(job);
                }
            }
            Cmd::Update(id) => {
                let ctx = self.ctx();
                let job = ctx
                    .fruit(&id)
                    .zip(self.installed.get(&id))
                    .and_then(|(f, c)| queue::job_for(f, c.channel, self.settings.keep as usize));
                if let Some(job) = job {
                    self.enqueue(job);
                }
            }
            Cmd::Retry(id) => {
                if let Some(f) = self.queue.failure(&id) {
                    let job = f.job.clone();
                    self.enqueue(job);
                }
            }
            Cmd::UpdateAll => {
                let jobs = match &self.feed {
                    Some(feed) => queue::updates(feed, |id| self.installed.get(id), self.settings.keep as usize),
                    None => Vec::new(),
                };
                for job in jobs {
                    self.enqueue(job);
                }
            }
            Cmd::SetChannel(id, ch) => self.set_channel(&id, ch),
            Cmd::Watch(id) => {
                if let Some(i) = s.watch.iter().position(|w| *w == id) {
                    s.watch.remove(i);
                } else {
                    s.watch.push(id);
                }
                s.save();
            }
            Cmd::OpenGames(id) => {
                let dir = self.basket.games_dir(&id);
                let _ = std::fs::create_dir_all(&dir);
                platform::open(&dir.to_string_lossy());
            }
            Cmd::Open(id) => self.open(&id),
            // Wired up in later milestones.
            Cmd::Couch | Cmd::MoveBasket | Cmd::MapButtons | Cmd::RestartForUpdate => {}
        }
    }

    /// Switch an installed fruit's channel: to a build already on disk if
    /// there is one, else download it. A fruit not installed just remembers
    /// the pick for its Install button.
    fn set_channel(&mut self, id: &str, ch: Channel) {
        let Some(fruit) = self.feed.as_ref().and_then(|f| f.fruit(id)).cloned() else { return };
        let Some(current) = self.installed.get(id).cloned() else {
            self.chosen.insert(id.to_string(), ch);
            return;
        };
        if current.channel == ch {
            return;
        }
        let Some(build) = fruit.channel(ch) else { return };
        if self.basket.build_dir(id, &build.build).join(".installed").exists() {
            let to = Current { build: build.build.clone(), channel: ch };
            if let Err(e) = self.basket.switch(id, to, &fruit.carry) {
                eprintln!("fruitbasket: switching {id} to {}: {e}", ch.name());
            }
            self.refresh_installed();
        } else if let Some(job) = queue::job_for(&fruit, ch, self.settings.keep as usize) {
            self.enqueue(job);
        }
    }

    /// Start the fruit's emulator with no game: it opens its own library.
    fn open(&self, id: &str) {
        let Some(bin) = self.feed.as_ref().and_then(|f| f.fruit(id)).and_then(|f| f.bin.clone()) else { return };
        let Some(exe) = self.basket.exe(id, &bin) else {
            eprintln!("fruitbasket: no {bin} in {id}'s current build");
            return;
        };
        let mut cmd = Command::new(&exe);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        if let Err(e) = cmd.spawn() {
            eprintln!("fruitbasket: starting {}: {e}", exe.display());
        }
    }

    fn status(&self) -> String {
        if let Some(a) = self.queue.active() {
            return format!("{} {} · {}%", a.step.name(), self.ctx().name(&a.job.fruit), a.pct);
        }
        if let Some(e) = &self.play_error {
            return format!("couldn't start: {e}");
        }
        if let Some(name) = self.shelf.running() {
            return format!("running in {name}");
        }
        let sep = std::path::MAIN_SEPARATOR;
        format!("{}{sep} · launcher {VERSION}", platform::tilde(&self.basket.root))
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.pad_used {
            let first = if self.tab == Tab::Library { ("A", "Play") } else { ("A", "Select") };
            return vec![first, ("B", "Back"), ("Y", "Details"), ("LB RB", "Tabs")];
        }
        match self.tab {
            Tab::Library => {
                let rows: Vec<Row> = self.shelf.rows(self.feed.as_ref(), &self.find);
                let sel = self.shelf.selected_in(&rows).and_then(|p| rows.iter().find(|r| r.game.path == p));
                let verb = if sel.is_some_and(|r| r.last.is_some()) { "Continue" } else { "Play" };
                vec![("Z", verb), ("/", "Find")]
            }
            Tab::Basket => {
                let ctx = self.ctx();
                let verb = match ctx.selected_fruit().map(|f| ctx.primary(f)) {
                    Some(Primary::Install) => "Install",
                    Some(Primary::Update) => "Update",
                    Some(Primary::TryAgain) => "Try again",
                    _ => "Open",
                };
                vec![("Z", verb), ("/", "Find")]
            }
            Tab::Downloads | Tab::Settings => vec![("/", "Find")],
        }
    }

    pub fn shutdown(&mut self, size: (usize, usize)) {
        self.settings.window = Some((size.0 as u32, size.1 as u32));
        self.settings.save();
    }
}

/// Read-only state the views and commands share, borrowed field by field
/// so it can live beside the canvas borrow while a frame draws.
struct Ctx<'a> {
    feed: Option<&'a Feed>,
    queue: &'a Queue,
    installed: &'a HashMap<String, Current>,
    settings: &'a Settings,
    chosen: &'a HashMap<String, Channel>,
    find: &'a str,
    selected: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    fn fruit(&self, id: &str) -> Option<&'a Fruit> {
        self.feed?.fruit(id)
    }

    fn name(&self, id: &str) -> String {
        self.fruit(id).map(|f| f.name.clone()).unwrap_or_else(|| capitalise(id))
    }

    /// The channel a first install of `f` uses: the aside's pick, else the
    /// setting, else whichever channel has a build.
    fn new_channel(&self, f: &Fruit) -> Channel {
        let want = self.chosen.get(&f.id).copied().unwrap_or(self.settings.new_channel);
        if f.channel(want).is_some() {
            return want;
        }
        [Channel::Stable, Channel::Nightly].into_iter().find(|&c| f.channel(c).is_some()).unwrap_or(want)
    }

    fn primary(&self, f: &Fruit) -> Primary {
        if let Some(a) = self.queue.busy(&f.id) {
            return Primary::Busy(a.step, a.pct);
        }
        if self.queue.is_waiting(&f.id) {
            return Primary::Queued;
        }
        if self.queue.failure(&f.id).is_some() {
            return Primary::TryAgain;
        }
        if f.status == Status::Growing {
            return Primary::Unavailable;
        }
        match self.installed.get(&f.id) {
            Some(c) if queue::update_for(f, c).is_some() => Primary::Update,
            Some(_) => Primary::Open,
            None if queue::job_for(f, self.new_channel(f), 0).is_some() => Primary::Install,
            None => Primary::Unavailable,
        }
    }

    fn primary_cmd(&self, f: &Fruit) -> Option<Cmd> {
        let id = f.id.clone();
        match self.primary(f) {
            Primary::Open => Some(Cmd::Open(id)),
            Primary::Update => Some(Cmd::Update(id)),
            Primary::Install => Some(Cmd::Install(id)),
            Primary::TryAgain => Some(Cmd::Retry(id)),
            Primary::Busy(..) | Primary::Queued | Primary::Unavailable => None,
        }
    }

    fn card_state(&self, f: &Fruit) -> CardState {
        match self.primary(f) {
            Primary::Busy(step, pct) => CardState::Busy(step, pct),
            Primary::Queued => CardState::Queued,
            Primary::TryAgain => CardState::Failed(self.queue.failure(&f.id).map_or(crate::jobs::FailKind::Network, |x| x.kind)),
            Primary::Update => CardState::UpdateReady,
            Primary::Open => CardState::UpToDate(self.installed.get(&f.id).map_or(Channel::Stable, |c| c.channel)),
            Primary::Install => CardState::NotInstalled,
            Primary::Unavailable => CardState::NotForThisPc,
        }
    }

    /// FIND on the Basket tab: fruit name, console, or extension.
    fn matches(&self, f: &Fruit) -> bool {
        let q = self.find.trim().to_lowercase();
        q.is_empty()
            || f.name.to_lowercase().contains(&q)
            || f.system.to_lowercase().contains(&q)
            || f.ext.iter().any(|e| e.to_lowercase().contains(q.trim_start_matches('.')))
    }

    /// (in the basket, ready to install, still growing), each in feed order.
    fn sections(&self) -> (Vec<&'a Fruit>, Vec<&'a Fruit>, Vec<&'a Fruit>) {
        let (mut inst, mut ready, mut growing) = (Vec::new(), Vec::new(), Vec::new());
        for f in self.feed.iter().flat_map(|f| f.fruits.iter()).filter(|f| self.matches(f)) {
            if self.installed.contains_key(&f.id) {
                inst.push(f);
            } else if f.status == Status::Growing {
                growing.push(f);
            } else {
                ready.push(f);
            }
        }
        (inst, ready, growing)
    }

    /// Every listed fruit, top to bottom: the arrow-key order.
    fn order(&self) -> Vec<&'a Fruit> {
        let (a, b, c) = self.sections();
        a.into_iter().chain(b).chain(c).collect()
    }

    /// The selection if it is listed, else the first installed fruit, else
    /// Strawberry (the first-run pick), else the first fruit.
    fn selected_id(&self) -> Option<&'a str> {
        let (inst, ready, growing) = self.sections();
        let all = || inst.iter().chain(&ready).chain(&growing);
        if let Some(f) = self.selected.and_then(|id| all().find(|f| f.id == id)) {
            return Some(&f.id);
        }
        inst.first()
            .or_else(|| ready.iter().find(|f| f.id == "strawberry"))
            .or_else(|| all().next())
            .map(|f| f.id.as_str())
    }

    fn selected_fruit(&self) -> Option<&'a Fruit> {
        self.selected_id().and_then(|id| self.fruit(id))
    }

    /// Fruits Update all would update: an update ready and nothing running.
    fn update_count(&self) -> usize {
        self.installed.keys().filter_map(|id| self.fruit(id)).filter(|f| self.primary(f) == Primary::Update).count()
    }

    fn basket_view(
        &self,
        disk: &HashMap<String, Disk>,
        root: String,
        scroll: f32,
        aside_scroll: f32,
        sheet: bool,
        feed_note: Option<&'a str>,
    ) -> BasketView<'a> {
        let (inst, ready, growing) = self.sections();
        let card = |f: &&'a Fruit| Card { fruit: f, state: self.card_state(f) };
        let selected = self.selected_id();
        let detail = self.selected_fruit().map(|f| {
            let current = self.installed.get(&f.id);
            let channel = current.map_or_else(|| self.new_channel(f), |c| c.channel);
            let target = match self.queue.failure(&f.id) {
                Some(fail) => f.build(&fail.job.build).map(|(_, b)| b),
                None => f.channel(channel),
            };
            let d = disk.get(&f.id).copied().unwrap_or_default();
            Detail {
                fruit: f,
                current,
                channel,
                primary: self.primary(f),
                target,
                games: d.games,
                program: d.program,
                games_size: d.games_size,
                watching: self.settings.watch.iter().any(|w| *w == f.id),
                key_id: key::KEY_ID,
            }
        });
        let feed_note = match (self.feed, feed_note) {
            (Some(_), _) => None,
            (None, Some(e)) => Some(e),
            (None, None) => Some("Fetching the list of fruits from the site…"),
        };
        BasketView {
            installed: inst.iter().map(card).collect(),
            ready: ready.iter().map(card).collect(),
            growing: growing.iter().map(|f| (*f, self.settings.watch.iter().any(|w| *w == f.id))).collect(),
            selected,
            detail,
            sheet,
            root,
            scroll,
            aside_scroll,
            feed_note,
        }
    }
}

fn settings_view<'a>(
    settings: &'a Settings,
    basket: &Basket,
    feed: Option<&Feed>,
    storage: &(String, Vec<FruitStorage>),
    scroll: f32,
    os_dark: bool,
) -> SettingsView<'a> {
    let sep = std::path::MAIN_SEPARATOR;
    let fruit_folders = feed
        .iter()
        .flat_map(|f| f.fruits.iter())
        .filter(|f| basket.current(&f.id).is_some())
        .map(|f| format!("{}{sep}", platform::tilde(&basket.games_dir(&f.id))))
        .collect();
    SettingsView {
        settings,
        fruit_folders,
        root: format!("{}{sep}", platform::tilde(&basket.root)),
        root_detail: storage.0.clone(),
        storage: storage.1.clone(),
        controller: None,
        os_dark,
        version: VERSION,
        key_id: key::KEY_ID,
        site: feed.map(|f| f.base.clone()).unwrap_or_else(|| key::feed_url().trim_end_matches("feed.json").to_string()),
        scroll,
    }
}

/// Sizes on disk, computed when the Settings tab opens.
fn storage(basket: &Basket, feed: Option<&Feed>) -> (String, Vec<FruitStorage>) {
    let used = dir_size(&basket.root);
    let rows = feed
        .iter()
        .flat_map(|f| f.fruits.iter())
        .filter_map(|f| {
            let current = basket.current(&f.id)?;
            let builds = basket.builds(&f.id);
            let program = dir_size(&basket.build_dir(&f.id, &current.build));
            let games = dir_size(&basket.games_dir(&f.id));
            let old = builds.len().saturating_sub(1);
            let kept = match old {
                0 => "no old builds".to_string(),
                1 => "1 old build kept".to_string(),
                n => format!("{n} old builds kept"),
            };
            Some(FruitStorage {
                id: f.id.clone(),
                name: f.name.clone(),
                detail: format!("program {} · games {} · {kept}", basket_ui::fmt::fmt_size(program), basket_ui::fmt::fmt_size(games)),
            })
        })
        .collect();
    (format!("{} used", basket_ui::fmt::fmt_size(used)), rows)
}

/// Program and games sizes and game counts, for the Basket aside.
fn disk(basket: &Basket, feed: Option<&Feed>) -> HashMap<String, Disk> {
    feed.iter()
        .flat_map(|f| f.fruits.iter())
        .filter_map(|f| {
            let current = basket.current(&f.id)?;
            let games_dir = basket.games_dir(&f.id);
            Some((
                f.id.clone(),
                Disk {
                    program: dir_size(&basket.build_dir(&f.id, &current.build)),
                    games_size: dir_size(&games_dir),
                    games: count_games(&games_dir, f, 3),
                },
            ))
        })
        .collect()
}

/// Files under `dir` (to `depth` folders down) that the fruit reads.
fn count_games(dir: &Path, fruit: &Fruit, depth: u32) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() && depth > 0 => count_games(&e.path(), fruit, depth - 1),
            Ok(t) if t.is_file() && fruit.reads(&e.path()) => 1,
            _ => 0,
        })
        .sum()
}

fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render every tab at the three mock sizes, paper and night, to
    /// target/shots/ for comparing with docs/mocks/screens/ by eye.
    #[test]
    fn shots() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        for (theme, tname) in [(ThemePref::Paper, "paper"), (ThemePref::Night, "night")] {
            for (w, h, size) in [(1280, 900, "regular"), (1024, 680, "compact"), (640, 880, "narrow")] {
                for tab in Tab::ALL {
                    let settings = Settings { theme, ..Settings::default() };
                    let mut app = App::with(settings, Basket::new(tmp.path()), Some(feed.clone()));
                    app.tab = tab;
                    app.draw(&UiInput::default(), w, h);
                    let name = format!("{}-{size}-{tname}.png", tab.label().to_lowercase());
                    app.canvas.save_png(&dir.join(name)).unwrap();
                }
            }
        }
    }

    /// End to end through the real worker: fetch and verify the live feed
    /// (or `FRUITBASKET_FEED`), install Strawberry with the Install command,
    /// match a game against its compat list, switch it to nightly, then
    /// feed a job a wrong hash. Downloads real builds. Run with
    /// `FRUITBASKET_E2E=1 cargo test e2e -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn e2e_install_switch_and_refuse() {
        use crate::jobs::FailKind;
        if std::env::var_os("FRUITBASKET_E2E").is_none() {
            return;
        }
        let fetched = feed::fetch(&key::feed_url(), &key::public_key(), None).expect("feed verifies");
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::with(Settings::default(), Basket::new(tmp.path()), Some(fetched.feed));
        let wait = |app: &mut App| {
            let start = Instant::now();
            while app.queue.active().is_some() {
                assert!(start.elapsed() < Duration::from_secs(180), "timed out");
                app.poll();
                thread::sleep(Duration::from_millis(50));
            }
        };

        app.apply(Cmd::Install("strawberry".into()));
        assert!(app.queue.busy("strawberry").is_some());
        wait(&mut app);
        let cur = app.installed.get("strawberry").cloned().expect("installed");
        assert_eq!(cur.channel, Channel::Stable);
        let exe = app.basket.exe("strawberry", "strawberry").expect("exe in the build");
        println!("installed {} at {}", cur.build, exe.display());
        assert_eq!(app.history[0].failed, None);

        // The compat list comes over HTTP, checked against the feed, and
        // the library matches a game in games/ against it.
        std::fs::write(app.basket.games_dir("strawberry").join("Final Fantasy IV Advance (USA).gba"), "x").unwrap();
        app.rescan();
        let start = Instant::now();
        loop {
            app.poll();
            let rows = app.shelf.rows(app.feed.as_ref(), "");
            if let Some(level) = rows.first().and_then(|r| r.level) {
                println!("compat: {}", level.label());
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "compat list never arrived");
            thread::sleep(Duration::from_millis(50));
        }

        app.apply(Cmd::SetChannel("strawberry".into(), Channel::Nightly));
        wait(&mut app);
        assert_eq!(app.installed["strawberry"].channel, Channel::Nightly);
        assert_eq!(app.basket.builds("strawberry").len(), 2, "the stable build is kept for rolling back");

        let mut bad = job(&app, "pomegranate");
        bad.asset.sha256 = "0".repeat(64);
        app.enqueue(bad);
        wait(&mut app);
        assert_eq!(app.queue.failure("pomegranate").map(|f| f.kind), Some(FailKind::Signature));
        assert!(!app.installed.contains_key("pomegranate"));
        assert!(!app.basket.builds_dir("pomegranate").exists(), "nothing written");
        assert_eq!(history::read(&app.basket.launcher_dir()).len(), 3);
    }

    /// A basket with Strawberry installed one build behind the feed.
    fn stocked(root: &Path) -> Basket {
        let b = Basket::new(root);
        std::fs::create_dir_all(b.build_dir("strawberry", "v1.3.0")).unwrap();
        std::fs::write(b.build_dir("strawberry", "v1.3.0").join(".installed"), "").unwrap();
        std::fs::write(b.fruit_dir("strawberry").join("current"), "v1.3.0\tstable\n").unwrap();
        std::fs::create_dir_all(b.games_dir("strawberry")).unwrap();
        std::fs::write(b.games_dir("strawberry").join("homebrew.gba"), "x").unwrap();
        b
    }

    fn job(app: &App, id: &str) -> Job {
        let f = app.feed.as_ref().unwrap().fruit(id).unwrap();
        queue::job_for(f, app.ctx().new_channel(f), 2).expect("a build for this PC")
    }

    /// The Library states from the mocks, every size and theme: empty,
    /// covers, list, one fruit's chip, a dump that didn't match.
    #[test]
    fn library_shots() {
        use crate::compat::Compat;
        use crate::dumps::DumpDb;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let titles = [
            "Final Fantasy IV Advance (USA)",
            "Mario Kart - Super Circuit (USA)",
            "Legend of Zelda, The (USA)",
            "Max Payne (USA)",
            "Cars (USA)",
            "Golden Sun (USA)",
            "Metroid Fusion (USA)",
        ];
        let compat = "title\tserial\tstatus\tnotes\nFinal Fantasy IV Advance\tBZ4E\tplayable\t\nMario Kart: Super Circuit\tAMKE\tin-game\t\nClassic NES Series: Legend of Zelda\tFZLE\tin-game\t\nMax Payne\tBMEE\tmenus\t\nCars\tBCAE\tboots\t\n";
        type Setup = fn(&mut App);
        let states: [(&str, bool, Setup); 5] = [
            ("library-first-run", false, |_| {}),
            ("library-covers", true, |_| {}),
            ("library-list", true, |app| app.shelf.list = true),
            ("library-strawberry", true, |app| app.shelf.filter = Some("strawberry".into())),
            ("library-unverified", true, |app| {
                app.shelf.list = true;
                let g = app.shelf.games.iter().find(|g| g.title == "Golden Sun").unwrap().clone();
                app.shelf.set_hash(&g, &"ab".repeat(20));
                app.shelf.selected = Some(g.path);
            }),
        ];
        for (theme, tname) in [(ThemePref::Paper, "paper"), (ThemePref::Night, "night")] {
            for (w, h, size) in [(1280, 900, "regular"), (1024, 680, "compact"), (640, 880, "narrow")] {
                for (name, stock, setup) in states {
                    let tmp = tempfile::tempdir().unwrap();
                    let basket = if stock { stocked(tmp.path()) } else { Basket::new(tmp.path()) };
                    if stock {
                        for t in titles {
                            std::fs::write(basket.games_dir("strawberry").join(format!("{t}.gba")), "x").unwrap();
                        }
                    }
                    let settings = Settings { theme, ..Settings::default() };
                    let mut app = App::with(settings, basket, Some(feed.clone()));
                    app.shelf.set_lists("strawberry", Compat::parse(compat), Some(DumpDb::parse("")));
                    let now = std::time::SystemTime::now();
                    for (i, t) in titles.iter().take(3).enumerate() {
                        let p = app.basket.games_dir("strawberry").join(format!("{t}.gba"));
                        app.shelf.played_mut().record(&p, now - Duration::from_secs(3_600 * 30 * i as u64), 4_000 * (i as u64 + 1));
                    }
                    app.tab = Tab::Library;
                    app.sheet = size == "narrow" && name == "library-unverified";
                    setup(&mut app);
                    app.draw(&UiInput::default(), w, h);
                    app.canvas.save_png(&dir.join(format!("{name}-{size}-{tname}.png"))).unwrap();
                }
            }
        }
    }

    /// The Basket and Downloads states from the mocks, every size and theme.
    #[test]
    fn basket_and_downloads_shots() {
        use crate::jobs::{Event, FailKind, Step};
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        if feed::this_platform() != "windows-x64" {
            return; // the test feed only has Windows builds
        }
        type Setup = fn(&mut App);
        let states: [(&str, Tab, bool, Setup); 7] = [
            ("basket-first-run", Tab::Basket, false, |_| {}),
            ("basket-update", Tab::Basket, true, |_| {}),
            ("basket-busy", Tab::Basket, true, |app| {
                let j = job(app, "pomegranate");
                app.queue.enqueue(j);
                app.queue.on_event(&Event::Progress { fruit: "pomegranate".into(), step: Step::Verify, pct: 76 });
                app.selected = Some("pomegranate".into());
            }),
            ("basket-failed", Tab::Basket, true, |app| {
                let j = job(app, "strawberry");
                app.queue.enqueue(j);
                app.queue.on_event(&Event::Failed { fruit: "strawberry".into(), kind: FailKind::Network, message: "couldn't reach projects.ethanaldrich.net".into() });
            }),
            ("basket-growing", Tab::Basket, true, |app| app.selected = Some("pear".into())),
            ("downloads-busy", Tab::Downloads, true, |app| {
                let j = job(app, "pomegranate");
                app.queue.enqueue(j);
                app.queue.on_event(&Event::Progress { fruit: "pomegranate".into(), step: Step::Download, pct: 46 });
                let j = job(app, "strawberry");
                app.queue.enqueue(j);
            }),
            ("downloads-signature", Tab::Downloads, true, |app| {
                let j = job(app, "pomegranate");
                app.queue.enqueue(j);
                app.queue.on_event(&Event::Failed { fruit: "pomegranate".into(), kind: FailKind::Signature, message: "hash".into() });
            }),
        ];
        for (theme, tname) in [(ThemePref::Paper, "paper"), (ThemePref::Night, "night")] {
            for (w, h, size) in [(1280, 900, "regular"), (1024, 680, "compact"), (640, 880, "narrow")] {
                for (name, tab, stock, setup) in states {
                    let tmp = tempfile::tempdir().unwrap();
                    let basket = if stock { stocked(tmp.path()) } else { Basket::new(tmp.path()) };
                    for (i, (fruit, failed)) in [("strawberry", None), ("pomegranate", Some(FailKind::Network))].into_iter().enumerate() {
                        let e = Entry {
                            when: std::time::SystemTime::now() - Duration::from_secs(86_400 * (i as u64 + 2)),
                            fruit: fruit.into(),
                            build: "v1.3.1".into(),
                            channel: Channel::Stable,
                            failed,
                            message: String::new(),
                        };
                        history::append(&basket.launcher_dir(), &e).unwrap();
                    }
                    let settings = Settings { theme, ..Settings::default() };
                    let mut app = App::with(settings, basket, Some(feed.clone()));
                    app.tab = tab;
                    app.sheet = size == "narrow" && name.starts_with("basket") && stock;
                    setup(&mut app);
                    app.draw(&UiInput::default(), w, h);
                    app.canvas.save_png(&dir.join(format!("{name}-{size}-{tname}.png"))).unwrap();
                }
            }
        }
    }
}

pub fn run() -> Result<(), String> {
    let mut app = App::new();
    let (w, h) = app.window_size();
    let mut video = Video::new("Fruit Basket", w, h, &app.keymap)?;
    let mut pads = Gamepads::new(Ports::Shared);
    while video.is_open() && !app.quit {
        let input = app.gather(&mut video, &mut pads);
        app.frame(&mut video, &input);
        thread::sleep(Duration::from_millis(12));
    }
    let size = video.size();
    app.shutdown(size);
    Ok(())
}
