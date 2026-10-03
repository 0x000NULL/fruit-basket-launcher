//! The launcher's state and its frame: gather input, draw, apply commands.

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use basket_app::pads::{Gamepads, PadPoll, Ports};
use basket_ui::input::{Action, ButtonSet, KeyMap, MenuRoles, PadMap, UiInput};
use basket_ui::text::Style;
use basket_ui::{Canvas, Fonts};
use gilrs::Button;
use minifb::Key;

use crate::art::Art;
use crate::basket::Basket;
use crate::feed::{self, Feed, FeedError, Fetched};
use crate::jobs::Worker;
use crate::key;
use crate::platform;
use crate::settings::{Settings, ThemePref};
use crate::ui::frame::{self, FrameView};
use crate::ui::settings::{FruitStorage, SettingsView};
use crate::ui::{self, Cmd, Flag, Tab, Ui};
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
        App {
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
            settings,
            basket,
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
                    }
                    Err(e) => self.feed_error = Some(e.to_string()),
                }
            }
        }
        for event in self.worker.poll() {
            let _ = event; // the Downloads tab (M2) consumes these
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
        let sv = match (&self.storage, self.tab) {
            (Some(st), Tab::Settings) => Some(settings_view(&self.settings, &self.basket, self.feed.as_ref(), st, self.scroll, self.os_dark)),
            _ => None,
        };
        let mut cmds = {
            let mut ui = Ui::new(&mut self.canvas, input, &self.art, night, self.pad_used);
            let fv = FrameView {
                tab: self.tab,
                find: &self.find,
                find_focused: self.find_focused,
                updates: 0,
                downloads: 0,
                launcher_update: self.launcher_update.as_deref(),
                hints: &hints,
                status: &status,
                controller: self.pad_used,
            };
            let top = frame::header(&mut ui, &fv);
            let bottom = ui.h() - 52.0;
            match &sv {
                Some(sv) => ui::settings::draw(&mut ui, sv, top, bottom),
                None => placeholder(&mut ui, self.tab, self.feed.as_ref(), self.feed_error.as_deref(), top),
            }
            frame::footer(&mut ui, &fv);
            ui.cmds
        };
        drop(sv);
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
        if input.pressed(Key::Slash) {
            out.push(Cmd::FocusFind(true));
        }
        if input.pressed(Key::Escape) {
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
                    if tab == Tab::Settings {
                        self.storage = None;
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
                }
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
            // Wired up in later milestones.
            Cmd::UpdateAll | Cmd::Couch | Cmd::AddFolder | Cmd::MoveBasket | Cmd::MapButtons | Cmd::RestartForUpdate => {}
        }
    }

    fn status(&self) -> String {
        let sep = std::path::MAIN_SEPARATOR;
        format!("{}{sep} · launcher {VERSION}", platform::tilde(&self.basket.root))
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.pad_used {
            let first = if self.tab == Tab::Library { ("A", "Play") } else { ("A", "Select") };
            return vec![first, ("B", "Back"), ("Y", "Details"), ("LB RB", "Tabs")];
        }
        match self.tab {
            Tab::Library => vec![("Z", "Play"), ("/", "Find")],
            Tab::Basket => vec![("Z", "Open"), ("/", "Find")],
            Tab::Downloads | Tab::Settings => vec![("/", "Find")],
        }
    }

    pub fn shutdown(&mut self, size: (usize, usize)) {
        self.settings.window = Some((size.0 as u32, size.1 as u32));
        self.settings.save();
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

/// Until each tab lands: what the feed says, so a run shows it loaded.
fn placeholder(ui: &mut Ui, tab: Tab, feed: Option<&Feed>, error: Option<&str>, top: f32) {
    let x0 = ui.pad_x();
    let w = ui.w() - 2.0 * x0;
    let y = ui.section(x0, top + 24.0, w, tab.label(), Some("in progress"));
    let body = ui.body();
    let mono = ui.mono();
    match feed {
        Some(f) => {
            ui.cv.text(x0, y + 16.0, &format!("Feed from {} · {} fruits · signature verified", f.generated, f.fruits.len()), &body);
            let mut fy = y + 52.0;
            for fruit in &f.fruits {
                let alpha = if fruit.status == feed::Status::Growing { 0.45 } else { 1.0 };
                ui.art.icon(ui.cv, &fruit.id, ui.night, crate::art::ICON_S, x0, fy, alpha);
                let name = Style::interface_bold(13.0).upper().tracking(1.6).color(ui.pal.fg);
                ui.cv.text(x0 + 44.0, fy, &format!("No. {}  {}", fruit.no, fruit.name), &name);
                let what = match (&fruit.stable, &fruit.nightly) {
                    (Some(s), Some(n)) => format!("{} · stable {} · nightly {}", fruit.system, s.build, n.build),
                    (Some(s), None) => format!("{} · stable {}", fruit.system, s.build),
                    (None, Some(n)) => format!("{} · nightly {}", fruit.system, n.build),
                    (None, None) => format!("{} · no build yet", fruit.system),
                };
                ui.cv.text(x0 + 44.0, fy + 17.0, &what, &mono);
                fy += 44.0;
            }
        }
        None => {
            ui.cv.text(x0, y + 16.0, "No feed yet.", &body);
        }
    }
    if let Some(e) = error {
        let st = Style::data(12.0).color(ui.pal.spot);
        ui.cv.text(x0, ui.h() - 80.0, e, &st);
    }
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
