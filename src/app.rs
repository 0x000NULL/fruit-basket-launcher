//! The launcher's state and its frame: gather input, draw, apply commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use basket_app::pads::{Gamepads, PadPoll, Ports};
use basket_app::prefs::PadBindings;
use basket_ui::input::{pad_button_name, Action, ButtonSet, KeyMap, MenuRoles, PadMap, UiInput};
use basket_ui::{Canvas, Fonts};
use gilrs::Button;
use minifb::Key;

use crate::art::Art;
use crate::basket::{Basket, Current, Games};
use crate::feed::{self, Channel, Feed, FeedError, Fetched, Fruit, Status};
use crate::focus::{self, Area, Dir, Spot};
use crate::folders;
use crate::history::{self, Entry};
use crate::jobs::{Job, Worker};
use crate::key;
use crate::library::{self, Slot};
use crate::mover;
use crate::pics::Pics;
use crate::platform;
use crate::queue::{self, Finished, Queue, RollOpt};
use crate::settings::{LibView, Settings, ThemePref};
use crate::shelf::Shelf;
use crate::ui::basket::{BasketView, Card, CardState, Detail, Primary};
use crate::ui::couch::{self, CouchView, Panel};
use crate::ui::downloads::DownloadsView;
use crate::ui::library::Row;
use crate::ui::frame::{self, Banner, FrameView};
use crate::ui::modal::{self, ModalView};
use crate::ui::settings::{FruitStorage, SettingsView};
use crate::ui::{self, capitalise, Cmd, Flag, Size, Tab, Ui};
use crate::update::{self, Upd};
use crate::window::Video;

pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
/// How often a launcher left open fetches the feed again.
const RECHECK: Duration = Duration::from_secs(4 * 60 * 60);
/// How often the game folders are checked for new or removed files.
const FOLDERS_EVERY: Duration = Duration::from_secs(15);

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
        &[
            (Key::Up, UP),
            (Key::Down, DOWN),
            (Key::Left, LEFT),
            (Key::Right, RIGHT),
            (Key::Z, A),
            (Key::X, B),
            (Key::C, Y),
            (Key::PageUp, LB),
            (Key::PageDown, RB),
            (Key::Enter, START),
        ],
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
    /// A newer launcher this folder can't take: (build, download URL).
    update_manual: Option<(String, String)>,
    update_rx: Option<Receiver<Upd>>,
    /// The launcher build the player said Later to, this run.
    update_dismissed: Option<String>,
    /// When the feed was last fetched, for the recheck while open; `None`
    /// in an app made by `with`, which never touches the network.
    feed_at: Option<Instant>,
    /// The game folders' last signature, the check on its way, and when
    /// it was asked for (a live app only, like `feed_at`).
    folders_sig: Option<u64>,
    folders_rx: Option<Receiver<u64>>,
    folders_at: Instant,
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
    /// The open dialog, if any; it takes all input.
    modal: Option<Modal>,
    /// A basket move under way: from, to, percent, and its thread.
    moving: Option<(PathBuf, PathBuf, u8, Receiver<mover::Progress>)>,
    /// The last thing that went wrong outside a download, for the footer.
    notice: Option<String>,
    /// The focused control (a `Ui::hot` key). `None` on the Library and
    /// Basket is the grid: the D-pad moves the selection instead.
    focus: Option<String>,
    /// The controls the last frame drew, for the D-pad.
    spots: Vec<Spot>,
    /// Scroll the focused control into view on the next frame.
    reveal: bool,
    /// The open dialog's columns at the last frame (Left and Right jump).
    modal_cols: usize,
    /// Where the page scrolls: below the header, above the footer.
    band: (f32, f32),
    /// Couch mode, when it is on.
    pub couch: Option<Couch>,
    /// The connected controllers' names.
    pub controllers: Vec<String>,
}

/// Couch mode's place: the system tab, the game, and what the hero shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Couch {
    /// 0 is All; then the installed fruits in feed order.
    pub system: usize,
    pub pick: usize,
    pub panel: CouchPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CouchPanel {
    Buttons,
    /// The saves list, with the picked row (one past the saves is Start fresh).
    Saves(usize),
    Details,
}

/// A dialog, with what it was opened on.
#[derive(Debug, Clone)]
pub enum Modal {
    /// The options are read when the dialog opens.
    Rollback { fruit: String, options: Vec<RollOpt>, pick: usize },
    Uninstall { fruit: String, delete: bool },
    Move { to: PathBuf },
    /// A game's saves (desktop: the aside's Saves button).
    Saves { game: PathBuf, title: String, slots: Vec<Slot>, pick: usize },
    /// `back`: reopen the saves dialog afterwards.
    DeleteSave { game: PathBuf, title: String, slot: Slot, back: bool },
    /// Map buttons: a copy of the map being changed; `listening` waits for
    /// a pad button for the picked row.
    Map { map: PadMap, pick: usize, listening: bool },
}

/// Map buttons' rows: the launcher's buttons as people name them.
const MAP_ROWS: [(&str, u16); 11] =
    [("A", A), ("B", B), ("X", X), ("Y", Y), ("LB", LB), ("RB", RB), ("Start", START), ("Up", UP), ("Down", DOWN), ("Left", LEFT), ("Right", RIGHT)];

#[derive(Debug, Clone, Default)]
pub struct Disk {
    pub program: u64,
    pub games_size: u64,
    pub games: usize,
    /// Builds on disk, newest first: what Roll back can switch to.
    pub builds: Vec<String>,
}

impl App {
    pub fn new() -> App {
        let settings = Settings::load();
        let basket = settings.basket();
        let cached = feed::load_cached(&basket.launcher_dir(), &key::public_key());
        let mut app = App::with(settings, basket, cached);
        app.feed_at = Some(Instant::now());
        app.check_launcher_update();
        if app.settings.check_on_open || app.feed.is_none() {
            app.check_feed();
        }
        app
    }

    /// An app with no window and no network: `new` without the side effects.
    pub fn with(settings: Settings, basket: Basket, cached: Option<Feed>) -> App {
        let history = history::read(&basket.launcher_dir());
        let mut shelf = Shelf::load(&basket.launcher_dir());
        shelf.az = settings.library_az;
        shelf.view = settings.library_view;
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
            update_manual: None,
            update_rx: None,
            update_dismissed: None,
            feed_at: None,
            folders_sig: None,
            folders_rx: None,
            folders_at: Instant::now(),
            storage: None,
            keymap: keymap(),
            padmap: settings.gamepad.as_ref().map_or_else(padmap, |b| b.to_padmap(padmap())),
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
            modal: None,
            moving: None,
            notice: None,
            focus: None,
            spots: Vec::new(),
            reveal: false,
            modal_cols: 1,
            band: (0.0, 0.0),
            couch: None,
            controllers: Vec::new(),
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
            running: self.shelf.running(),
            controller: self.controllers.first().map(String::as_str),
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

    /// A newer launcher in the feed: stage it on a thread, or if this
    /// folder can't be written, offer the download. Never in tests, where
    /// the exe is the test binary.
    fn check_launcher_update(&mut self) {
        if cfg!(test) || self.update_rx.is_some() || self.launcher_update.is_some() || self.update_manual.is_some() {
            return;
        }
        let Some(b) = self.feed.as_ref().and_then(|f| f.launcher.as_ref()) else { return };
        if !update::offer(&b.build, VERSION, self.update_dismissed.as_deref()) {
            return;
        }
        let Some(asset) = b.assets.get(feed::this_platform()).cloned() else { return };
        let dir = update::dir(&self.basket.launcher_dir());
        if update::staged(&dir).is_some_and(|(s, _)| s == b.build) {
            self.launcher_update = Some(b.build.clone());
        } else if std::env::current_exe().is_ok_and(|e| update::can_replace(&e)) {
            self.update_rx = Some(update::start(b.build.clone(), asset, dir));
        } else {
            self.update_manual = Some((b.build.clone(), asset.url));
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
                        self.feed_error = None;
                        // The same feed again (a recheck with nothing new):
                        // no rescan, no lists, no updates to start.
                        if self.feed.as_ref().is_some_and(|f| f.generated == fetched.feed.generated) {
                            self.check_launcher_update();
                            return self.poll_rest();
                        }
                        if let Err(e) = feed::save_cached(&self.basket.launcher_dir(), &fetched) {
                            eprintln!("fruitbasket: caching feed: {e}");
                        }
                        self.feed = Some(fetched.feed);
                        self.refresh_installed();
                        if let Some(feed) = &self.feed {
                            self.shelf.lists(&self.basket.launcher_dir(), feed);
                        }
                        if self.settings.install_without_asking {
                            self.apply(Cmd::UpdateAll);
                        }
                        self.check_launcher_update();
                    }
                    Err(e) => self.feed_error = Some(e.to_string()),
                }
            }
        }
        self.poll_rest();
    }

    fn poll_rest(&mut self) {
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
        self.poll_move();
        if let Some(rx) = &self.update_rx {
            if let Ok(r) = rx.try_recv() {
                self.update_rx = None;
                match r {
                    Upd::Staged(build) => self.launcher_update = Some(build),
                    Upd::Failed(e) => self.notice = Some(format!("the launcher update didn't download: {e}")),
                }
            }
        }
        if self.shelf.poll(&self.basket.launcher_dir(), self.feed.as_ref()) && self.shelf.running().is_none() {
            self.play_error = None;
        }
        if self.shelf.stale_covers {
            self.shelf.index_covers(&self.basket, self.feed.as_ref());
        }
        if self.settings.theme == ThemePref::System && self.os_dark_at.elapsed() > Duration::from_secs(5) {
            self.os_dark = platform::os_dark();
            self.os_dark_at = Instant::now();
        }
        self.watch_folders();
        // A launcher left open still hears about new builds, but never
        // while a game runs or a job is going.
        if self.feed_at.is_some_and(|at| at.elapsed() > RECHECK)
            && self.settings.check_on_open
            && self.shelf.running().is_none()
            && self.queue.count() == 0
        {
            self.feed_at = Some(Instant::now());
            self.check_feed();
        }
    }

    /// Every few seconds, hash the game folders on a thread; rescan when
    /// they changed. The first answer is only the baseline.
    fn watch_folders(&mut self) {
        if let Some(rx) = &self.folders_rx {
            if let Ok(sig) = rx.try_recv() {
                self.folders_rx = None;
                if self.folders_sig.is_some_and(|old| old != sig) {
                    self.rescan();
                }
                self.folders_sig = Some(sig);
            }
            return;
        }
        if self.feed_at.is_none() || self.folders_at.elapsed() < FOLDERS_EVERY {
            return;
        }
        self.folders_at = Instant::now();
        let mut dirs: Vec<PathBuf> = self.installed.keys().map(|id| self.basket.games_dir(id)).collect();
        dirs.extend(self.settings.folders.iter().cloned());
        self.folders_rx = Some(folders::start(dirs));
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
        if !pad.connected.is_empty() || !pad.disconnected.is_empty() {
            self.controllers = pads.connected();
            if !pad.connected.is_empty() && self.settings.couch_on_controller && self.couch.is_none() {
                self.apply(Cmd::Couch);
            }
        }
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
        if input.clicked {
            self.focus = None;
        }

        self.want_slot_pictures();
        let status = self.status();
        let hints: Vec<(&str, &str)> = self.hints();
        let night = self.night() || self.couch.is_some();
        if self.tab == Tab::Settings && self.storage.is_none() {
            self.storage = Some(storage(&self.basket, self.feed.as_ref()));
        }
        if self.tab == Tab::Basket && self.disk.is_none() {
            self.disk = Some(disk(&self.basket, self.feed.as_ref()));
        }
        let sv = match (&self.storage, self.tab) {
            (Some(st), Tab::Settings) => Some(settings_view(&self.settings, &self.basket, self.feed.as_ref(), st, self.scroll, self.os_dark, self.controllers.first().cloned())),
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
            running: self.shelf.running(),
            controller: self.controllers.first().map(String::as_str),
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
        let banner = banner(self.launcher_update.as_deref(), self.update_manual.as_ref(), &ctx);
        let mv = self.modal.as_ref().and_then(|m| modal_view(m, self.feed.as_ref(), &self.basket, &self.installed, &self.shelf.pics));
        // Under a dialog the page draws, but nothing on it can be clicked.
        let cview = self.couch.as_ref().map(|c| couch_view(c, &self.shelf, self.feed.as_ref(), &self.basket, &self.installed, &self.controllers));
        let blind = UiInput { mouse: (-1.0e6, -1.0e6), ..UiInput::default() };
        let page_input = if mv.is_some() { &blind } else { input };
        let activate = mv.is_none() && !self.find_focused && self.focus.is_some() && input.action(Action::Confirm);
        let (mut cmds, spots) = {
            let mut ui = Ui::new(&mut self.canvas, page_input, &self.art, night, self.pad_used);
            ui.focus = if mv.is_none() { self.focus.as_deref() } else { None };
            ui.activate = activate;
            let fv = FrameView {
                tab: self.tab,
                find: &self.find,
                find_focused: self.find_focused,
                updates: ctx.update_count(),
                downloads: self.queue.count(),
                banner: banner.as_ref(),
                hints: &hints,
                status: &status,
                controller: self.pad_used,
            };
            if let Some(cv) = &cview {
                couch::draw(&mut ui, cv);
            } else {
                let top = frame::header(&mut ui, &fv);
                let bottom = ui.h() - 52.0;
                self.band = (top, bottom);
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
            }
            (ui.cmds, ui.spots)
        };
        if mv.is_none() {
            self.spots = spots;
        }
        if std::mem::take(&mut self.reveal) {
            cmds.extend(self.reveal_focus());
        }
        if let Some(mv) = &mv {
            let mut ui = Ui::new(&mut self.canvas, input, &self.art, night, self.pad_used);
            self.modal_cols = modal::draw(&mut ui, mv);
            cmds.extend(ui.cmds);
        }
        let modal_open = mv.is_some();
        drop((sv, bv, dv, lv, mv, cview));
        cmds.extend(if modal_open {
            self.modal_keys(input)
        } else if self.couch.is_some() {
            self.couch_keys(input)
        } else {
            self.keys(input)
        });
        for cmd in cmds {
            self.apply(cmd);
        }
    }

    /// Keys while a dialog is open: Z confirms, X or Esc cancels, the
    /// arrows pick a row, Space ticks the box.
    fn modal_keys(&self, input: &UiInput) -> Vec<Cmd> {
        let mut out = Vec::new();
        if let Some(Modal::Map { pick, listening, .. }) = &self.modal {
            // Waiting for a button: the first pad button pressed is the answer.
            if *listening {
                if let Some(b) = input.pad_buttons.first() {
                    out.push(Cmd::ModalBind(*b));
                } else if input.pressed(Key::Escape) {
                    out.push(Cmd::ModalToggle);
                }
                return out;
            }
            if input.action(Action::Up) && *pick > 0 {
                out.push(Cmd::ModalPick(pick - 1));
            } else if input.action(Action::Down) && pick + 1 < MAP_ROWS.len() {
                out.push(Cmd::ModalPick(pick + 1));
            } else if self.modal_cols == 2 {
                let half = MAP_ROWS.len().div_ceil(2);
                if input.action(Action::Right) && pick + half < MAP_ROWS.len() {
                    out.push(Cmd::ModalPick(pick + half));
                } else if input.action(Action::Left) && *pick >= half {
                    out.push(Cmd::ModalPick(pick - half));
                }
            }
            // A listens for the picked row; Start (or Enter) is Done.
            if input.action(Action::Confirm) {
                out.push(Cmd::ModalToggle);
            } else if input.action(Action::Start) {
                out.push(Cmd::ModalConfirm);
            } else if input.action(Action::Back) || input.pressed(Key::Escape) {
                out.push(Cmd::ModalCancel);
            }
            if input.pad_buttons.contains(&self.padmap.button_for(X)) || input.pressed(Key::Delete) {
                out.push(Cmd::ModalExtra);
            }
            return out;
        }
        if input.action(Action::Confirm) {
            out.push(Cmd::ModalConfirm);
        } else if input.action(Action::Back) || input.pressed(Key::Escape) {
            out.push(Cmd::ModalCancel);
        }
        let rows = match &self.modal {
            Some(Modal::Rollback { options, pick, .. }) => Some((*pick, options.len())),
            Some(Modal::Saves { slots, pick, .. }) => Some((*pick, slots.len())),
            _ => None,
        };
        if let Some((pick, n)) = rows {
            if input.action(Action::Up) && pick > 0 {
                out.push(Cmd::ModalPick(pick - 1));
            } else if input.action(Action::Down) && pick + 1 < n {
                out.push(Cmd::ModalPick(pick + 1));
            }
        }
        if input.pad_buttons.contains(&self.padmap.button_for(X)) || input.pressed(Key::Delete) {
            out.push(Cmd::ModalExtra);
        }
        if input.pressed(Key::Space) {
            out.push(Cmd::ModalToggle);
        }
        out
    }

    /// Keyboard and pad shortcuts that aren't tied to something drawn.
    fn keys(&self, input: &UiInput) -> Vec<Cmd> {
        let mut out = Vec::new();
        if self.find_focused {
            return out;
        }
        let grid = matches!(self.tab, Tab::Library | Tab::Basket);
        let dir = [(Action::Up, Dir::Up), (Action::Down, Dir::Down), (Action::Left, Dir::Left), (Action::Right, Dir::Right)]
            .into_iter()
            .find(|(a, _)| input.action(*a))
            .map(|(_, d)| d);
        if input.pressed(Key::Tab) {
            let back = input.is_down(Key::LeftShift) || input.is_down(Key::RightShift);
            if let Some(s) = focus::cycle(&self.spots, self.focus.as_deref(), back) {
                out.push(Cmd::Focus(Some(s.key.clone())));
            }
        }
        if self.focus.is_some() || !grid {
            // On the controls: the D-pad walks them, B goes back to the grid.
            if let Some(s) = dir.and_then(|d| focus::next(&self.spots, self.focus.as_deref(), d)) {
                out.push(Cmd::Focus(Some(s.key.clone())));
            }
            if grid && (input.action(Action::Back) || input.pressed(Key::Escape)) {
                out.push(Cmd::Focus(None));
                return out;
            }
        } else if input.action(Action::Back) && self.sheet && Size::of(self.canvas.width()) == Size::Narrow {
            out.push(Cmd::CloseSheet);
        } else if self.y_pressed(input) {
            // Y: Details, into the aside (or the narrow sheet).
            let first = self.spots.iter().find(|s| s.area == Area::Aside).or_else(|| self.spots.iter().find(|s| s.area == Area::Main));
            if let Some(s) = first {
                out.push(Cmd::Focus(Some(s.key.clone())));
            }
        }
        if input.action(Action::Start) {
            out.push(Cmd::Couch);
        }
        if self.tab == Tab::Library && self.focus.is_none() {
            let rows = self.shelf.rows(self.feed.as_ref(), &self.find);
            let at = self.shelf.selected_in(&rows).and_then(|p| rows.iter().position(|r| r.game.path == p));
            let cols = if self.shelf.view == LibView::List { 1 } else if Size::of(self.canvas.width()) == Size::Narrow { 4 } else { 5 };
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
                    out.push(Cmd::Continue(p.to_path_buf()));
                }
            }
        }
        if self.tab == Tab::Basket && self.focus.is_none() {
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
        if input.pressed(Key::Escape) && !sheet_open && (self.focus.is_none() || !grid) {
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

    /// Keys in couch mode. While a game runs the emulator has the pad, so
    /// nothing here reacts.
    fn couch_keys(&self, input: &UiInput) -> Vec<Cmd> {
        let mut out = Vec::new();
        let Some(c) = &self.couch else { return out };
        if self.shelf.running().is_some() {
            return out;
        }
        let systems = self.couch_systems().len() + 1;
        if input.action(Action::PrevPage) {
            out.push(Cmd::CouchSystem((c.system + systems - 1) % systems));
        }
        if input.action(Action::NextPage) {
            out.push(Cmd::CouchSystem((c.system + 1) % systems));
        }
        match c.panel {
            CouchPanel::Saves(i) => {
                let n = self.couch_slots().len();
                if input.action(Action::Up) && i > 0 {
                    out.push(Cmd::CouchSavePick(i - 1));
                } else if input.action(Action::Down) && i < n {
                    out.push(Cmd::CouchSavePick(i + 1));
                }
            }
            _ => {
                let n = self.shelf.couch_rows(self.feed.as_ref(), self.couch_fruit()).len();
                if input.action(Action::Left) && c.pick > 0 {
                    out.push(Cmd::CouchPick(c.pick - 1));
                } else if input.action(Action::Right) && c.pick + 1 < n {
                    out.push(Cmd::CouchPick(c.pick + 1));
                }
            }
        }
        if input.action(Action::Confirm) {
            out.push(Cmd::CouchContinue);
        }
        // X: the pad's X, or S (and Delete) on the keyboard, where X is B.
        if input.pad_buttons.contains(&self.padmap.button_for(X)) || input.pressed(Key::S) || input.pressed(Key::Delete) {
            out.push(Cmd::CouchSaves);
        }
        if self.y_pressed(input) {
            out.push(Cmd::CouchDetails);
        }
        if input.action(Action::Back) {
            out.push(Cmd::CouchBack);
        }
        if input.action(Action::Start) || input.pressed(Key::Escape) {
            out.push(Cmd::Couch);
        }
        out
    }

    /// Couch mode's system tabs after All: the installed fruits.
    fn couch_systems(&self) -> Vec<&Fruit> {
        self.feed.iter().flat_map(|f| f.fruits.iter()).filter(|f| self.installed.contains_key(&f.id)).collect()
    }

    /// The fruit couch mode's tab shows; `None` is All.
    fn couch_fruit(&self) -> Option<&str> {
        let c = self.couch.as_ref()?;
        let i = c.system.checked_sub(1)?;
        self.couch_systems().get(i).map(|f| f.id.as_str())
    }

    /// Couch mode's picked game.
    fn couch_game(&self) -> Option<PathBuf> {
        let c = self.couch.as_ref()?;
        let rows = self.shelf.couch_rows(self.feed.as_ref(), self.couch_fruit());
        rows.get(c.pick.min(rows.len().saturating_sub(1))).map(|r| r.game.path.clone())
    }

    fn game_fruit(&self, game: &Path) -> Option<&Fruit> {
        let g = self.shelf.games.iter().find(|g| g.path == game)?;
        self.feed.as_ref()?.fruit(&g.fruit)
    }

    fn game_slots(&self, game: &Path) -> Vec<Slot> {
        match self.game_fruit(game) {
            Some(f) => self.shelf.slots(game, &self.basket, f),
            None => Vec::new(),
        }
    }

    /// Ask for the pictures of the saves on screen: couch mode's list or
    /// the Saves dialog.
    fn want_slot_pictures(&mut self) {
        let slots = match (&self.modal, &self.couch) {
            (Some(Modal::Saves { slots, .. }), _) => slots.clone(),
            (None, Some(c)) if matches!(c.panel, CouchPanel::Saves(_)) => self.couch_slots(),
            _ => return,
        };
        for p in slots.iter().filter_map(Slot::picture) {
            self.shelf.pics.want(p);
        }
    }

    /// The save Continue would load for `game`.
    fn resume(&self, game: &Path) -> Option<Slot> {
        self.shelf.resume(game, &self.basket, self.game_fruit(game)?)
    }

    fn couch_slots(&self) -> Vec<Slot> {
        self.couch_game().map(|g| self.game_slots(&g)).unwrap_or_default()
    }

    fn game_title(&self, game: &Path) -> String {
        self.shelf.games.iter().find(|g| g.path == game).map(|g| g.title.clone()).unwrap_or_default()
    }

    /// Start `game` from `slot` (or fresh); the reason it can't, if not.
    fn play_from(&mut self, game: &Path, slot: Option<u8>) -> bool {
        if slot.is_some() && !self.game_fruit(game).is_some_and(Fruit::loads_slots) {
            let name = self.game_fruit(game).map(|f| f.name.clone()).unwrap_or_default();
            self.play_error = Some(format!("{name} can't start from a save yet"));
            return false;
        }
        let couch = self.couch.is_some();
        self.play_error = self.shelf.play(game, &self.basket, self.feed.as_ref(), slot, couch).err();
        self.play_error.is_none()
    }

    /// Y went down: the pad's Y, or C on the keyboard.
    fn y_pressed(&self, input: &UiInput) -> bool {
        input.pad_buttons.contains(&self.padmap.button_for(Y)) || input.pressed(Key::C)
    }

    /// Scroll so the focused control is in view.
    fn reveal_focus(&self) -> Vec<Cmd> {
        let Some(s) = self.focus.as_deref().and_then(|k| self.spots.iter().find(|s| s.key == k)) else { return Vec::new() };
        let (top, bottom) = (self.band.0 + 16.0, self.band.1 - 16.0);
        let dy = if s.y < top {
            s.y - top
        } else if s.y + s.h > bottom {
            (s.y + s.h - bottom).min(s.y - top)
        } else {
            return Vec::new();
        };
        match s.area {
            Area::Main => vec![Cmd::Scroll(dy)],
            Area::Aside => vec![Cmd::AsideScroll(dy)],
            Area::Fixed => Vec::new(),
        }
    }

    fn apply(&mut self, cmd: Cmd) {
        let s = &mut self.settings;
        match cmd {
            Cmd::Focus(key) => {
                // Into the narrow sheet's controls: open it.
                if key.as_deref().is_some_and(|k| self.spots.iter().any(|s| s.key == k && s.area == Area::Aside)) {
                    self.sheet = true;
                }
                self.focus = key;
                self.reveal = true;
            }
            Cmd::Tab(tab) => {
                if tab != self.tab {
                    self.tab = tab;
                    self.focus = None;
                    self.notice = None;
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
            Cmd::Rescan => self.rescan(),
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
            Cmd::LibSort(az) => {
                self.shelf.az = az;
                self.settings.library_az = az;
                self.settings.save();
            }
            Cmd::LibView(view) => {
                self.shelf.view = view;
                self.settings.library_view = view;
                self.settings.save();
            }
            Cmd::SelectGame(p) => {
                if self.shelf.selected.as_ref() != Some(&p) {
                    self.aside_scroll = 0.0;
                }
                self.shelf.selected = Some(p);
                self.sheet = true;
            }
            Cmd::Play(p) => {
                self.play_error = self.shelf.play(&p, &self.basket, self.feed.as_ref(), None, false).err();
            }
            Cmd::Continue(p) => match self.resume(&p) {
                Some(slot) => {
                    self.play_from(&p, Some(slot.n));
                }
                None => self.apply(Cmd::Play(p)),
            },
            Cmd::ShowFile(p) => platform::reveal(&p),
            Cmd::ShowSaves(p) => {
                let slots = self.game_slots(&p);
                if !slots.is_empty() {
                    let title = self.game_title(&p);
                    self.modal = Some(Modal::Saves { game: p, title, slots, pick: 0 });
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
                if self.queue.busy(&id).is_some() {
                    return;
                }
                if let Err(e) = self.basket.prune(&id, 0) {
                    eprintln!("fruitbasket: clearing old {id} builds: {e}");
                }
                self.storage = None;
            }
            Cmd::CheckNow => self.check_feed(),
            Cmd::OpenUrl(url) => platform::open(&url),
            Cmd::DismissLauncherUpdate => {
                let build = self.launcher_update.take().or(self.update_manual.take().map(|(b, _)| b));
                self.update_dismissed = build.or(self.update_dismissed.take());
                self.update_manual = None;
            }
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
            Cmd::AskRollback(id) => {
                let options = match (self.feed.as_ref().and_then(|f| f.fruit(&id)), self.installed.get(&id)) {
                    (Some(f), Some(c)) => queue::rollback_options(f, c, &self.basket.builds(&id)),
                    _ => Vec::new(),
                };
                let running = self.shelf.running().is_some_and(|name| Some(name) == self.feed.as_ref().and_then(|f| f.fruit(&id)).map(|f| f.name.as_str()));
                if !options.is_empty() && self.queue.busy(&id).is_none() && !self.queue.is_waiting(&id) && !running {
                    self.modal = Some(Modal::Rollback { fruit: id, options, pick: 0 });
                }
            }
            Cmd::AskUninstall(id) => {
                if self.ctx().can_uninstall(&id) {
                    self.modal = Some(Modal::Uninstall { fruit: id, delete: false });
                }
            }
            Cmd::MoveBasket => {
                if let Some(why) = self.cant_move() {
                    self.notice = Some(why);
                } else if let Some(picked) = rfd::FileDialog::new().set_title("Move the basket to").pick_folder() {
                    match mover::destination(&self.basket.root, &picked) {
                        Ok(to) => self.modal = Some(Modal::Move { to }),
                        Err(e) => self.notice = Some(format!("can't move the basket there: {e}")),
                    }
                }
            }
            Cmd::ModalPick(i) => match &mut self.modal {
                Some(Modal::Rollback { options, pick, .. }) => *pick = i.min(options.len().saturating_sub(1)),
                Some(Modal::Saves { slots, pick, .. }) => *pick = i.min(slots.len().saturating_sub(1)),
                Some(Modal::Map { pick, .. }) => *pick = i.min(MAP_ROWS.len() - 1),
                _ => {}
            },
            Cmd::ModalBind(button) => {
                if let Some(Modal::Map { map, pick, listening }) = &mut self.modal {
                    map.rebind(MAP_ROWS[*pick].1, button);
                    *listening = false;
                }
            }
            Cmd::ModalExtra => {
                if let Some(Modal::Map { map, listening, .. }) = &mut self.modal {
                    *map = padmap();
                    *listening = false;
                }
                if let Some(Modal::Saves { game, title, slots, pick }) = &self.modal {
                    if let Some(slot) = slots.get(*pick) {
                        if self.shelf.running().is_some() {
                            self.notice = Some("close the game before deleting a save".into());
                        } else {
                            self.modal = Some(Modal::DeleteSave { game: game.clone(), title: title.clone(), slot: slot.clone(), back: true });
                        }
                    }
                }
            }
            Cmd::ModalToggle => match &mut self.modal {
                Some(Modal::Uninstall { delete, .. }) => *delete = !*delete,
                Some(Modal::Map { listening, .. }) => *listening = !*listening,
                _ => {}
            },
            Cmd::ModalCancel => self.modal = None,
            Cmd::ModalConfirm => {
                if let Some(m) = self.modal.take() {
                    self.confirm(m);
                }
            }
            Cmd::RipeInstall(id) => {
                self.unwatch(&id);
                self.selected = Some(id.clone());
                self.apply(Cmd::Tab(Tab::Basket));
                self.apply(Cmd::Install(id));
            }
            Cmd::RipeDismiss(id) => self.unwatch(&id),
            Cmd::Couch => {
                self.couch = match self.couch {
                    Some(_) => None,
                    None => Some(Couch { system: 0, pick: 0, panel: CouchPanel::Buttons }),
                };
                self.focus = None;
            }
            Cmd::CouchSystem(i) => {
                if let Some(c) = &mut self.couch {
                    *c = Couch { system: i, pick: 0, panel: CouchPanel::Buttons };
                }
            }
            Cmd::CouchPick(i) => {
                if let Some(c) = &mut self.couch {
                    c.pick = i;
                    if matches!(c.panel, CouchPanel::Saves(_)) {
                        c.panel = CouchPanel::Buttons;
                    }
                }
            }
            Cmd::CouchSavePick(i) => {
                if let Some(c) = &mut self.couch {
                    c.panel = CouchPanel::Saves(i);
                }
            }
            Cmd::CouchDetails => {
                if let Some(c) = &mut self.couch {
                    c.panel = if c.panel == CouchPanel::Details { CouchPanel::Buttons } else { CouchPanel::Details };
                }
            }
            Cmd::CouchBack => match &mut self.couch {
                Some(c) if c.panel != CouchPanel::Buttons => c.panel = CouchPanel::Buttons,
                _ => self.apply(Cmd::Couch),
            },
            Cmd::CouchContinue => {
                let Some(game) = self.couch_game() else { return };
                let slot = match self.couch.as_ref().map(|c| c.panel) {
                    Some(CouchPanel::Saves(i)) => self.couch_slots().get(i).map(|s| s.n),
                    // The buttons' Continue: the newest save.
                    _ => self.resume(&game).map(|s| s.n),
                };
                if self.play_from(&game, slot) {
                    if let Some(c) = &mut self.couch {
                        c.panel = CouchPanel::Buttons;
                    }
                }
            }
            Cmd::CouchSaves => {
                let Some(game) = self.couch_game() else { return };
                match self.couch.as_ref().map(|c| c.panel) {
                    Some(CouchPanel::Saves(i)) => {
                        if let Some(slot) = self.couch_slots().get(i).cloned() {
                            if self.shelf.running().is_none() {
                                let title = self.game_title(&game);
                                self.modal = Some(Modal::DeleteSave { game, title, slot, back: false });
                            }
                        }
                    }
                    Some(_) => {
                        if let Some(c) = &mut self.couch {
                            c.panel = CouchPanel::Saves(0);
                        }
                    }
                    None => {}
                }
            }
            Cmd::MapButtons => self.modal = Some(Modal::Map { map: self.padmap.clone(), pick: 0, listening: false }),
            // The new process swaps the staged build in, then starts it.
            Cmd::RestartForUpdate => {
                self.settings.save();
                match std::env::current_exe().and_then(|exe| Command::new(exe).spawn()) {
                    Ok(_) => self.quit = true,
                    Err(e) => self.notice = Some(format!("couldn't restart: {e}")),
                }
            }
        }
    }

    fn unwatch(&mut self, id: &str) {
        self.settings.watch.retain(|w| w != id);
        self.settings.save();
    }

    /// Do what a dialog asked, once it is confirmed.
    fn confirm(&mut self, m: Modal) {
        match m {
            Modal::Rollback { fruit, options, pick } => {
                let Some(opt) = options.get(pick) else { return };
                if opt.download.is_some() {
                    let f = self.feed.as_ref().and_then(|f| f.fruit(&fruit));
                    if let Some(job) = f.and_then(|f| queue::job_for_build(f, &opt.build, self.settings.keep as usize)) {
                        self.enqueue(job);
                    }
                } else {
                    self.switch_to(&fruit, Current { build: opt.build.clone(), channel: opt.channel });
                }
            }
            Modal::Uninstall { fruit, delete } => {
                if !self.ctx().can_uninstall(&fruit) {
                    return;
                }
                let games = if delete { Games::Delete } else { Games::Keep };
                if let Err(e) = self.basket.uninstall(&fruit, games) {
                    self.notice = Some(format!("uninstalling {}: {e}", self.ctx().name(&fruit)));
                }
                self.selected = None;
                self.refresh_installed();
            }
            Modal::Saves { game, slots, pick, .. } => {
                let Some(slot) = slots.get(pick) else { return };
                if self.game_fruit(&game).is_some_and(Fruit::loads_slots) {
                    self.play_from(&game, Some(slot.n));
                } else {
                    platform::reveal(&slot.path);
                }
            }
            Modal::Map { map, .. } => {
                let (a, b) = (map.button_for(A), map.button_for(B));
                if a == Button::Unknown || b == Button::Unknown || a == b {
                    self.notice = Some("A and B must each have a button".into());
                    self.modal = Some(Modal::Map { map, pick: 0, listening: false });
                    return;
                }
                self.settings.gamepad = Some(PadBindings::from_padmap(&map));
                self.settings.save();
                self.padmap = map;
            }
            Modal::DeleteSave { game, title, slot, back } => {
                if self.shelf.running().is_some() {
                    self.notice = Some("close the game before deleting a save".into());
                    return;
                }
                if let Err(e) = library::delete_slot(&slot) {
                    self.notice = Some(format!("deleting the save: {e}"));
                }
                self.rescan();
                let slots = self.game_slots(&game);
                if back && !slots.is_empty() {
                    self.modal = Some(Modal::Saves { game, title, slots, pick: 0 });
                }
            }
            Modal::Move { to } => {
                if let Some(why) = self.cant_move() {
                    self.notice = Some(why);
                    return;
                }
                let from = self.basket.root.clone();
                let rx = mover::start(from.clone(), to.clone());
                self.moving = Some((from, to, 0, rx));
            }
        }
    }

    /// Why the basket can't move now, if it can't.
    fn cant_move(&self) -> Option<String> {
        if self.moving.is_some() {
            Some("the basket is already moving".into())
        } else if self.queue.active().is_some() || self.queue.waiting().next().is_some() {
            Some("wait for the downloads to finish before moving the basket".into())
        } else if self.shelf.running().is_some() {
            Some("close the game before moving the basket".into())
        } else {
            None
        }
    }

    /// Pick up the mover's progress; on success, the basket lives at the new
    /// place from now on.
    fn poll_move(&mut self) {
        let Some((from, to, pct, rx)) = &mut self.moving else { return };
        let mut done = None;
        while let Ok(p) = rx.try_recv() {
            match p {
                mover::Progress::Pct(n) => *pct = n,
                mover::Progress::Done(r) => done = Some(r),
            }
        }
        let Some(result) = done else { return };
        let (from, to) = (from.clone(), to.clone());
        self.moving = None;
        if let Err(e) = result {
            self.notice = Some(format!("the basket didn't move: {e}. Nothing changed"));
            return;
        }
        let s = &mut self.settings;
        s.root = Some(to.clone());
        for p in s.folders.iter_mut().chain(s.hidden.iter_mut()) {
            *p = crate::library::rebase(p, &from, &to);
        }
        s.save();
        self.basket = self.settings.basket();
        self.worker = Worker::start(self.basket.clone());
        self.shelf.rebase(&self.basket.launcher_dir(), &from, &to);
        self.refresh_installed();
        self.notice = Some(format!("the basket is now in {}", platform::tilde(&to)));
    }

    /// Make a build on disk current, then re-read what is installed.
    fn switch_to(&mut self, id: &str, to: Current) {
        let carry = self.feed.as_ref().and_then(|f| f.fruit(id)).map(|f| f.build_carry().to_vec()).unwrap_or_default();
        if let Err(e) = self.basket.switch(id, to, &carry) {
            self.notice = Some(format!("switching {}: {e}", self.ctx().name(id)));
        }
        self.refresh_installed();
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
            self.switch_to(id, Current { build: build.build.clone(), channel: ch });
        } else if let Some(job) = queue::job_for(&fruit, ch, self.settings.keep as usize) {
            self.enqueue(job);
        }
    }

    /// Start the fruit's emulator with no game: it opens its own library.
    fn open(&mut self, id: &str) {
        let Some(fruit) = self.feed.as_ref().and_then(|f| f.fruit(id)) else { return };
        let Some(bin) = fruit.bin.as_deref() else { return };
        let Some(exe) = self.basket.exe(id, bin) else {
            self.notice = Some(format!("no {bin} in {}'s current build", fruit.name));
            return;
        };
        let data = fruit.uses_data().then(|| self.basket.data_dir(id));
        let started = match &data {
            Some(_) => self.basket.migrate_data(id, fruit.data_carry()).map(|_| ()).map_err(|e| format!("moving saves to data/: {e}")),
            None => Ok(()),
        }
        .and_then(|_| crate::launch::args(&fruit.open, None, None, data.as_deref()))
        .and_then(|args| {
            let mut cmd = Command::new(&exe);
            cmd.args(args);
            if let Some(dir) = exe.parent() {
                cmd.current_dir(dir);
            }
            cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
        });
        if let Err(e) = started {
            self.notice = Some(format!("couldn't open {}: {e}", fruit.name));
        }
    }

    fn status(&self) -> String {
        if let Some(a) = self.queue.active() {
            return format!("{} {} · {}%", a.step.name(), self.ctx().name(&a.job.fruit), a.pct);
        }
        if let Some((_, to, pct, _)) = &self.moving {
            return format!("moving the basket to {} · {pct}%", platform::tilde(to));
        }
        if let Some(e) = &self.play_error {
            return format!("couldn't start: {e}");
        }
        if let Some(n) = &self.notice {
            return n.clone();
        }
        if let Some(name) = self.shelf.running() {
            return format!("running in {name}");
        }
        let sep = std::path::MAIN_SEPARATOR;
        format!("{}{sep} · launcher {VERSION}", platform::tilde(&self.basket.root))
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.pad_used {
            let grid = matches!(self.tab, Tab::Library | Tab::Basket) && self.focus.is_none();
            return match (grid, self.tab) {
                (true, Tab::Library) => vec![("A", "Play"), ("Y", "Details"), ("LB RB", "Tabs")],
                (true, _) => vec![("A", "Select"), ("Y", "Details"), ("LB RB", "Tabs")],
                (false, Tab::Library | Tab::Basket) => vec![("A", "Select"), ("B", "Back"), ("LB RB", "Tabs")],
                (false, _) => vec![("A", "Select"), ("LB RB", "Tabs")],
            };
        }
        match self.tab {
            Tab::Library => {
                let rows: Vec<Row> = self.shelf.rows(self.feed.as_ref(), &self.find);
                let sel = self.shelf.selected_in(&rows).and_then(|p| rows.iter().find(|r| r.game.path == p));
                let verb = if sel.is_some_and(|r| self.resume(&r.game.path).is_some()) { "Continue" } else { "Play" };
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
    /// The fruit name a game is running in.
    running: Option<&'a str>,
    controller: Option<&'a str>,
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

    /// Nothing is installing the fruit and none of its games is running.
    fn can_uninstall(&self, id: &str) -> bool {
        let Some(f) = self.fruit(id) else { return false };
        self.installed.contains_key(id) && self.queue.busy(id).is_none() && !self.queue.is_waiting(id) && self.running != Some(f.name.as_str())
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
            let d = disk.get(&f.id).cloned().unwrap_or_default();
            let idle = self.queue.busy(&f.id).is_none() && !self.queue.is_waiting(&f.id) && self.running != Some(f.name.as_str());
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
                can_roll_back: idle && current.is_some_and(|c| !queue::rollback_options(f, c, &d.builds).is_empty()),
                can_uninstall: self.can_uninstall(&f.id),
                controller: self.controller,
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

/// The banner: a launcher update staged, else the first watched fruit
/// that has ripened (released, with a build for this PC, not installed).
fn banner(launcher_update: Option<&str>, manual: Option<&(String, String)>, ctx: &Ctx) -> Option<Banner> {
    if let Some((v, url)) = manual {
        return Some(Banner {
            lead: format!("Launcher {v}"),
            text: "is out. This folder can't be written, so get it from the site.".into(),
            action: ("Download".into(), Cmd::OpenUrl(url.clone())),
            later: Cmd::DismissLauncherUpdate,
        });
    }
    if let Some(v) = launcher_update {
        return Some(Banner {
            lead: format!("Launcher {v}"),
            text: "is downloaded and verified. It installs the next time the launcher opens.".into(),
            action: ("Restart now".into(), Cmd::RestartForUpdate),
            later: Cmd::DismissLauncherUpdate,
        });
    }
    let f = ctx.settings.watch.iter().filter_map(|id| ctx.fruit(id)).find(|f| {
        f.status == Status::Released && !ctx.installed.contains_key(&f.id) && queue::job_for(f, ctx.new_channel(f), 0).is_some()
    })?;
    let build = f.channel(ctx.new_channel(f)).map_or(String::new(), |b| b.build.clone());
    Some(Banner {
        lead: format!("{} is ripe", f.name),
        text: format!("{build} for {} is ready to install.", f.system),
        action: ("Install".into(), Cmd::RipeInstall(f.id.clone())),
        later: Cmd::RipeDismiss(f.id.clone()),
    })
}

/// What the open dialog shows, from the mocks' Rollback and Uninstall.
fn modal_view<'a>(m: &'a Modal, feed: Option<&'a Feed>, basket: &Basket, installed: &HashMap<String, Current>, pics: &'a Pics) -> Option<ModalView<'a>> {
    let sep = std::path::MAIN_SEPARATOR;
    match m {
        Modal::Rollback { fruit, options, pick } => {
            let f = feed?.fruit(fruit)?;
            let current = installed.get(fruit)?;
            Some(ModalView {
                fruit: Some(f),
                title: format!("Roll back {}?", f.name),
                body: "Saves, settings and controller maps stay as they are. You can update again any time.".into(),
                current: Some((format!("{} {}", current.channel.name(), current.build), "installed now".into())),
                rows: options
                    .iter()
                    .map(|o| {
                        let note = match o.download {
                            None => "kept on disk".to_string(),
                            Some(size) => format!("download · {}", crate::ui::fmt_size(size)),
                        };
                        (format!("{} {}", o.channel.name(), o.build), note)
                    })
                    .collect(),
                pics: Vec::new(),
                pick: *pick,
                tick: None,
                cancel: "Cancel",
                confirm: "Roll back",
                extra: None,
            })
        }
        Modal::Uninstall { fruit, delete } => {
            let f = feed?.fruit(fruit)?;
            Some(ModalView {
                fruit: Some(f),
                title: format!("Uninstall {}?", f.name),
                body: format!(
                    "The program is removed. Your games and saves stay in {}{sep} unless you tick the box.",
                    platform::tilde(&basket.games_dir(fruit))
                ),
                current: None,
                rows: Vec::new(),
                pics: Vec::new(),
                pick: 0,
                tick: Some(("Also delete games and saves", *delete)),
                cancel: "Keep it",
                confirm: "Uninstall",
                extra: None,
            })
        }
        Modal::Move { to } => Some(ModalView {
            fruit: None,
            title: "Move the basket?".into(),
            body: format!(
                "Everything in {}{sep} moves to {}{sep}: the programs, games and saves. The launcher keeps working from the new place.",
                platform::tilde(&basket.root),
                platform::tilde(to)
            ),
            current: None,
            rows: Vec::new(),
            pics: Vec::new(),
            pick: 0,
            tick: None,
            cancel: "Cancel",
            confirm: "Move",
            extra: None,
        }),
        Modal::Saves { game, title, slots, pick } => {
            let f = feed?.fruit(&game_fruit_id(basket, game)?)?;
            let loads = f.loads_slots();
            Some(ModalView {
                fruit: Some(f),
                title: title.clone(),
                body: if loads {
                    "Load starts the game from the save you pick.".into()
                } else {
                    format!("{} can't start from a save yet: start the game, then load it there.", f.name)
                },
                current: None,
                rows: slots.iter().map(|s| (format!("Slot {}", s.n), basket_ui::fmt::fmt_when(s.saved))).collect(),
                pics: if slots.iter().any(|s| s.picture().is_some()) {
                    slots.iter().map(|s| s.picture().and_then(|p| pics.get(p))).collect()
                } else {
                    Vec::new()
                },
                pick: *pick,
                tick: None,
                cancel: "Close",
                confirm: if loads { "Load" } else { "Show file" },
                extra: Some("Delete"),
            })
        }
        Modal::Map { map, pick, listening } => Some(ModalView {
            fruit: None,
            title: "Map buttons".into(),
            body: "For the launcher's menus and couch mode. Pick a button with A, then press the one you want for it. The keyboard always works, and each emulator keeps its own map.".into(),
            current: None,
            rows: MAP_ROWS
                .iter()
                .enumerate()
                .map(|(i, (name, bit))| {
                    let bound = if *listening && i == *pick { "press a button…".to_string() } else { pad_button_name(map.button_for(*bit)).to_string() };
                    (name.to_string(), bound)
                })
                .collect(),
            pics: Vec::new(),
            pick: *pick,
            tick: None,
            cancel: "Cancel",
            confirm: "Done",
            extra: Some("Reset"),
        }),
        Modal::DeleteSave { title, slot, .. } => Some(ModalView {
            fruit: None,
            title: "Delete this save?".into(),
            body: format!("Slot {} of {title}, saved {}, is deleted for good.", slot.n, basket_ui::fmt::fmt_when(slot.saved)),
            current: None,
            rows: Vec::new(),
            pics: Vec::new(),
            pick: 0,
            tick: None,
            cancel: "Keep it",
            confirm: "Delete",
            extra: None,
        }),
    }
}

/// The fruit whose games/ (or data/) folder holds `game`, from the path:
/// the dialog has no shelf to ask.
fn game_fruit_id(basket: &Basket, game: &Path) -> Option<String> {
    let rel = game.strip_prefix(&basket.root).ok()?;
    rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned())
}

/// Couch mode's view: the system tabs, the games, and the hero's panel.
fn couch_view<'a>(
    c: &Couch,
    shelf: &'a crate::shelf::Shelf,
    feed: Option<&'a Feed>,
    basket: &Basket,
    installed: &HashMap<String, Current>,
    controllers: &'a [String],
) -> CouchView<'a> {
    let systems: Vec<&'a Fruit> = feed.iter().flat_map(|f| f.fruits.iter()).filter(|f| installed.contains_key(&f.id)).collect();
    let fruit = c.system.checked_sub(1).and_then(|i| systems.get(i)).copied();
    let rows = shelf.couch_rows(feed, fruit.map(|f| f.id.as_str()));
    let heading = match fruit {
        Some(f) => format!("{} · {}", f.name, rows.len()),
        None => format!("All games · {}", rows.len()),
    };
    let pick = c.pick.min(rows.len().saturating_sub(1));
    let game = rows.get(pick);
    let gfruit = game.and_then(|r| feed?.fruit(&r.game.fruit));
    let slots = match (game, gfruit) {
        (Some(r), Some(f)) => shelf.slots(&r.game.path, basket, f),
        _ => Vec::new(),
    };
    let panel = match (c.panel, game) {
        (CouchPanel::Details, Some(r)) => Panel::Details(shelf.detail(r, basket, feed)),
        (CouchPanel::Saves(i), Some(_)) => Panel::Saves {
            pics: if slots.iter().any(|s| s.picture().is_some()) {
                slots.iter().map(|s| s.picture().and_then(|p| shelf.pics.get(p))).collect()
            } else {
                Vec::new()
            },
            rows: slots
                .iter()
                .map(|s| (format!("Slot {}", s.n), basket_ui::fmt::fmt_when(s.saved)))
                .chain(std::iter::once(("Start fresh".to_string(), String::new())))
                .collect(),
            pick: i.min(slots.len()),
            note: gfruit.filter(|f| !f.loads_slots() && !slots.is_empty()).map(|f| format!("{} can't start from a save yet: Start fresh, then load in the game.", f.name)),
        },
        _ => Panel::Buttons,
    };
    let resume = game.zip(gfruit).and_then(|(r, f)| shelf.resume(&r.game.path, basket, f)).map(|s| s.n);
    let hints = match c.panel {
        CouchPanel::Buttons => vec![("A", if resume.is_some() { "Continue" } else { "Play" }), ("B", "Back"), ("X", "Saves"), ("LB RB", "Switch system")],
        CouchPanel::Saves(_) => vec![("A", "Load"), ("B", "Back"), ("X", "Delete save")],
        CouchPanel::Details => vec![("B", "Back"), ("LB RB", "Switch system")],
    };
    CouchView {
        systems: std::iter::once("All").chain(systems.iter().map(|f| f.name.as_str())).collect(),
        system: c.system.min(systems.len()),
        heading,
        rows,
        pick,
        controller: controllers.first().map(String::as_str),
        panel,
        saves: slots.len(),
        playing: shelf.running(),
        resume,
        hero: match c.panel {
            CouchPanel::Saves(i) => slots.get(i).and_then(Slot::picture).and_then(|p| shelf.pics.get(p)),
            _ => None,
        },
        hints,
    }
}

fn settings_view<'a>(
    settings: &'a Settings,
    basket: &Basket,
    feed: Option<&Feed>,
    storage: &(String, Vec<FruitStorage>),
    scroll: f32,
    os_dark: bool,
    controller: Option<String>,
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
        controller,
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
            let games = dir_size(&basket.games_dir(&f.id)) + dir_size(&basket.data_dir(&f.id));
            let old = builds.len().saturating_sub(1);
            let kept = match old {
                0 => "no old builds".to_string(),
                1 => "1 old build kept".to_string(),
                n => format!("{n} old builds kept"),
            };
            Some(FruitStorage {
                id: f.id.clone(),
                name: f.name.clone(),
                detail: format!("program {} · games {} · {kept}", crate::ui::fmt_size(program), crate::ui::fmt_size(games)),
            })
        })
        .collect();
    let used = format!("{} used", crate::ui::fmt_size(used));
    let detail = match platform::free_space(&basket.root) {
        Some(free) => format!("{used} · {} free", crate::ui::fmt_size(free)),
        None => used,
    };
    (detail, rows)
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
                    builds: basket.builds(&f.id),
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
    use crate::ui::library::PlayState;

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
    /// match a game against its compat list, switch it to nightly, roll
    /// back and forward, feed a job a wrong hash, move the basket and
    /// uninstall. Downloads real builds. Run with
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

        // Roll back to the kept stable build through the dialog, and forward.
        let nightly = app.installed["strawberry"].build.clone();
        app.apply(Cmd::AskRollback("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert_eq!(app.installed["strawberry"], cur, "back on the kept stable build");
        app.apply(Cmd::AskRollback("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert_eq!(app.installed["strawberry"].build, nightly);

        let mut bad = job(&app, "pomegranate");
        bad.asset.sha256 = "0".repeat(64);
        app.enqueue(bad);
        wait(&mut app);
        assert_eq!(app.queue.failure("pomegranate").map(|f| f.kind), Some(FailKind::Signature));
        assert!(!app.installed.contains_key("pomegranate"));
        assert!(!app.basket.builds_dir("pomegranate").exists(), "nothing written");
        assert_eq!(history::read(&app.basket.launcher_dir()).len(), 3);

        // Move the basket, then uninstall keeping the games.
        let elsewhere = tempfile::tempdir().unwrap();
        let to = elsewhere.path().join("FruitBasket");
        app.modal = Some(Modal::Move { to: to.clone() });
        app.apply(Cmd::ModalConfirm);
        while app.moving.is_some() {
            app.poll();
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(app.basket.root, to, "{:?}", app.notice);
        assert!(app.basket.exe("strawberry", "strawberry").is_some(), "the build still runs from the new place");
        app.apply(Cmd::AskUninstall("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert!(!app.installed.contains_key("strawberry"));
        assert!(app.basket.games_dir("strawberry").join("Final Fantasy IV Advance (USA).gba").is_file());
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

    /// An older Strawberry build on disk beside the current one.
    fn keep_build(app: &App, build: &str) {
        let dir = app.basket.build_dir("strawberry", build);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".installed"), "").unwrap();
    }

    /// Roll back to a kept build and forward again, uninstall keeping the
    /// games, and move the basket; dialogs open, pick and confirm by command.
    #[test]
    fn rollback_uninstall_and_move() {
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("FruitBasket");
        let mut app = App::with(Settings { root: Some(root.clone()), ..Settings::default() }, stocked(&root), Some(feed));
        keep_build(&app, "v1.2.0");
        let current = |app: &App| app.installed.get("strawberry").map(|c| c.build.clone());

        app.apply(Cmd::AskRollback("strawberry".into()));
        match &app.modal {
            Some(Modal::Rollback { options, .. }) => assert_eq!(options[0], RollOpt { build: "v1.2.0".into(), channel: Channel::Stable, download: None }),
            other => panic!("{other:?}"),
        }
        app.apply(Cmd::ModalCancel);
        assert!(app.modal.is_none());
        assert_eq!(current(&app).as_deref(), Some("v1.3.0"), "cancel changes nothing");

        app.apply(Cmd::AskRollback("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert_eq!(current(&app).as_deref(), Some("v1.2.0"));
        app.apply(Cmd::AskRollback("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert_eq!(current(&app).as_deref(), Some("v1.3.0"), "and forward again");

        // A dialog takes the keys: Esc closes it instead of quitting.
        app.apply(Cmd::AskUninstall("strawberry".into()));
        let esc = UiInput { pressed: vec![Key::Escape], ..UiInput::default() };
        app.draw(&esc, 1280, 900);
        assert!(app.modal.is_none() && !app.quit);

        let games = app.basket.games_dir("strawberry");
        app.apply(Cmd::AskUninstall("strawberry".into()));
        app.apply(Cmd::ModalConfirm);
        assert_eq!(current(&app), None);
        assert!(games.join("homebrew.gba").is_file(), "games stay unless ticked");

        // Move: the games and play history follow, and settings point there.
        let rom = games.join("homebrew.gba");
        app.shelf.played_mut().record(&rom, std::time::SystemTime::now(), 60);
        let to = tmp.path().join("Elsewhere/FruitBasket");
        app.modal = Some(Modal::Move { to: to.clone() });
        app.apply(Cmd::ModalConfirm);
        let start = Instant::now();
        while app.moving.is_some() {
            assert!(start.elapsed() < Duration::from_secs(30), "move timed out");
            app.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.settings.root.as_deref(), Some(to.as_path()));
        assert_eq!(app.basket.root, to);
        assert!(!root.exists());
        let moved = to.join("strawberry/games/homebrew.gba");
        assert!(moved.is_file());
        assert_eq!(app.shelf.played_mut().secs(&moved), 60);
    }

    /// A fruit whose launch takes `{data}`: its first Play moves the cards
    /// from beside the exe into data/, and the emulator gets that folder.
    /// The "emulator" is this test binary, which `--list` makes exit at once.
    #[test]
    fn data_moves_on_first_play() {
        let mut feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let pom = feed.fruits.iter_mut().find(|f| f.id == "pomegranate").unwrap();
        pom.bin = Some("pom".into());
        pom.launch = ["--list", "{rom}", "{data}"].map(String::from).to_vec();
        pom.carry = ["ps2emu.toml", "cards", "states"].map(String::from).to_vec();
        let tmp = tempfile::tempdir().unwrap();
        let b = Basket::new(tmp.path());
        let build = b.build_dir("pomegranate", "v0.3.0");
        std::fs::create_dir_all(build.join("cards")).unwrap();
        std::fs::write(build.join(".installed"), "").unwrap();
        std::fs::write(build.join("cards/card1.ps2"), "save").unwrap();
        let exe = build.join(if cfg!(windows) { "pom.exe" } else { "pom" });
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        std::fs::write(b.fruit_dir("pomegranate").join("current"), "v0.3.0\tstable\n").unwrap();
        std::fs::create_dir_all(b.games_dir("pomegranate")).unwrap();
        let game = b.games_dir("pomegranate").join("Game.iso");
        std::fs::write(&game, "x").unwrap();

        let mut app = App::with(Settings::default(), b, Some(feed));
        app.apply(Cmd::Play(game));
        assert_eq!(app.play_error, None);
        assert!(app.basket.data_dir("pomegranate").join("cards/card1.ps2").is_file());
        assert!(!build.join("cards").exists());
        let start = Instant::now();
        while app.shelf.running().is_some() {
            assert!(start.elapsed() < Duration::from_secs(30), "the stand-in never exited");
            app.poll();
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// The controller reaches every control: the D-pad walks Settings and
    /// A ticks a box; Y goes into the Library aside and B comes back.
    #[test]
    fn pad_walks_the_controls() {
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::with(Settings::default(), stocked(tmp.path()), Some(feed));
        let act = |a: Action| UiInput { actions: vec![a], ..UiInput::default() };
        let idle = UiInput::default();

        app.tab = Tab::Settings;
        app.draw(&idle, 1280, 900);
        app.draw(&act(Action::Down), 1280, 900);
        let first = app.focus.clone().expect("the D-pad picks a control");
        app.draw(&act(Action::Down), 1280, 900);
        assert_ne!(app.focus.as_ref(), Some(&first), "and moves on");

        let label = "Rescan when the launcher opens";
        app.apply(Cmd::Focus(Some(label.into())));
        app.draw(&idle, 1280, 900);
        assert!(app.spots.iter().any(|s| s.key == label));
        let was = app.settings.rescan_on_open;
        app.draw(&act(Action::Confirm), 1280, 900);
        assert_eq!(app.settings.rescan_on_open, !was, "A presses the focused control");

        // Far down the page: focusing it scrolls it into view.
        let last = app.spots.iter().filter(|s| s.area == Area::Main).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap().key.clone();
        app.apply(Cmd::Focus(Some(last.clone())));
        app.draw(&idle, 1280, 680);
        app.draw(&idle, 1280, 680);
        let s = app.spots.iter().find(|s| s.key == last).unwrap();
        assert!(s.y + s.h <= app.band.1, "scrolled into view: {} > {}", s.y + s.h, app.band.1);

        app.apply(Cmd::Tab(Tab::Library));
        assert_eq!(app.focus, None);
        app.draw(&idle, 1280, 900);
        let y = UiInput { pressed: vec![Key::C], ..UiInput::default() };
        app.draw(&y, 1280, 900);
        let spot = app.focus.as_deref().and_then(|k| app.spots.iter().find(|s| s.key == k)).expect("Y focuses the aside");
        assert_eq!(spot.area, Area::Aside);
        app.draw(&act(Action::Back), 1280, 900);
        assert_eq!(app.focus, None, "B goes back to the grid");
        assert!(!app.quit);
    }

    /// Strawberry installed with this test binary as its program (`--list`
    /// makes it exit at once), a game, and saves in slots 2 and 5.
    fn couch_basket(root: &Path) -> (Basket, Feed, PathBuf) {
        let mut feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let sb = feed.fruits.iter_mut().find(|f| f.id == "strawberry").unwrap();
        sb.bin = Some("sb".into());
        sb.launch = ["--list", "{rom}"].map(String::from).to_vec();
        sb.load_slot = Some(["--list", "{rom}", "--slot", "{slot}"].map(String::from).to_vec());
        sb.couch = vec!["--fullscreen".into()];
        let b = stocked(root);
        let exe = b.build_dir("strawberry", "v1.3.0").join(if cfg!(windows) { "sb.exe" } else { "sb" });
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        let rom = b.games_dir("strawberry").join("homebrew.gba");
        for n in [2, 5] {
            std::fs::write(b.games_dir("strawberry").join(format!("homebrew.s{n}.state")), "x").unwrap();
        }
        (b, feed, rom)
    }

    /// Couch mode end to end: in by Start, switch system, open the saves,
    /// delete one through the dialog, load another, back out with B.
    #[test]
    fn couch_saves_load_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let (b, feed, rom) = couch_basket(tmp.path());
        let mut app = App::with(Settings::default(), b, Some(feed));
        let act = |a: Action| UiInput { actions: vec![a], ..UiInput::default() };
        let key = |k: Key| UiInput { pressed: vec![k], ..UiInput::default() };

        app.draw(&act(Action::Start), 1280, 720);
        assert!(app.couch.is_some(), "Start opens couch mode");
        app.draw(&act(Action::NextPage), 1280, 720);
        assert_eq!(app.couch_fruit(), Some("strawberry"));
        assert_eq!(app.couch_game().as_deref(), Some(rom.as_path()));

        app.draw(&key(Key::S), 1280, 720);
        assert_eq!(app.couch.as_ref().unwrap().panel, CouchPanel::Saves(0));
        assert_eq!(app.couch_slots().len(), 2);
        // Delete the first listed save: X asks, the dialog's A confirms.
        let first = app.couch_slots()[0].clone();
        app.draw(&key(Key::S), 1280, 720);
        assert!(matches!(app.modal, Some(Modal::DeleteSave { .. })));
        app.draw(&act(Action::Confirm), 1280, 720);
        assert!(!first.path.exists());
        assert_eq!(app.couch_slots().len(), 1);

        // Load the one left; the emulator gets its slot.
        let left = app.couch_slots()[0].n;
        app.draw(&act(Action::Confirm), 1280, 720);
        assert_eq!(app.play_error, None, "loading slot {left}");
        assert!(app.shelf.running().is_some());
        assert!(app.couch_keys(&act(Action::Back)).is_empty(), "the emulator has the pad while it runs");
        let start = Instant::now();
        while app.shelf.running().is_some() {
            assert!(start.elapsed() < Duration::from_secs(30), "the stand-in never exited");
            app.poll();
            thread::sleep(Duration::from_millis(20));
        }

        app.draw(&act(Action::Back), 1280, 720);
        assert!(app.couch.is_none(), "B on the top level leaves couch mode");
        assert!(!app.quit);
    }

    /// Map buttons: A listens, the next pad button binds (swapping), Done
    /// keeps the map; Reset goes back to the defaults.
    #[test]
    fn map_buttons() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::with(Settings::default(), Basket::new(tmp.path()), None);
        let act = |a: Action| UiInput { actions: vec![a], ..UiInput::default() };
        app.apply(Cmd::MapButtons);
        app.draw(&act(Action::Confirm), 1280, 900);
        assert!(matches!(app.modal, Some(Modal::Map { listening: true, pick: 0, .. })), "A listens for row A");
        // While listening, even the pad's B is an answer, not Cancel.
        let east = UiInput { pad_buttons: vec![Button::East], actions: vec![Action::Back], ..UiInput::default() };
        app.draw(&east, 1280, 900);
        match &app.modal {
            Some(Modal::Map { map, listening: false, .. }) => {
                assert_eq!(map.button_for(A), Button::East);
                assert_eq!(map.button_for(B), Button::South, "swapped");
            }
            other => panic!("{other:?}"),
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        app.settings.theme = ThemePref::Paper;
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("settings-map-regular-paper.png")).unwrap();
        // A 680-high window takes two columns, so Done stays on screen;
        // Right jumps across.
        app.draw(&UiInput::default(), 1024, 680);
        app.canvas.save_png(&dir.join("settings-map-compact-paper.png")).unwrap();
        assert_eq!(app.modal_cols, 2);
        app.draw(&act(Action::Right), 1024, 680);
        assert!(matches!(app.modal, Some(Modal::Map { pick: 6, .. })));
        app.draw(&act(Action::Left), 1024, 680);
        assert!(matches!(app.modal, Some(Modal::Map { pick: 0, .. })));
        app.draw(&act(Action::Start), 1280, 900);
        assert!(app.modal.is_none());
        assert_eq!(app.padmap.button_for(A), Button::East);
        assert_eq!(app.settings.gamepad.as_ref().and_then(|g| g.0.get("A")).map(String::as_str), Some("East"));

        app.apply(Cmd::MapButtons);
        app.apply(Cmd::ModalExtra);
        app.apply(Cmd::ModalConfirm);
        assert_eq!(app.padmap, padmap(), "Reset");
    }

    /// Couch mode and the saves dialogs at TV sizes, for comparing with
    /// Couch*.png by eye.
    #[test]
    fn couch_shots() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        type Setup = fn(&mut App);
        let states: [(&str, Setup); 6] = [
            ("couch-all", |_| {}),
            ("couch-strawberry", |app| app.apply(Cmd::CouchSystem(1))),
            ("couch-saves", |app| app.apply(Cmd::CouchSaves)),
            ("couch-details", |app| app.apply(Cmd::CouchDetails)),
            ("couch-delete", |app| {
                app.apply(Cmd::CouchSaves);
                app.apply(Cmd::CouchSaves);
            }),
            ("couch-empty", |app| app.shelf.games.clear()),
        ];
        for (w, h) in [(1280, 720), (1920, 1080)] {
            for (name, setup) in states {
                let tmp = tempfile::tempdir().unwrap();
                let (b, feed, _) = couch_basket(tmp.path());
                for t in ["Golden Sun (USA)", "Metroid Fusion (USA)", "Mario Kart - Super Circuit (USA)"] {
                    std::fs::write(b.games_dir("strawberry").join(format!("{t}.gba")), "x").unwrap();
                }
                let mut app = App::with(Settings::default(), b, Some(feed));
                app.controllers = vec!["Xbox Wireless Controller".into()];
                app.apply(Cmd::Couch);
                let rom = app.basket.games_dir("strawberry").join("homebrew.gba");
                app.shelf.played_mut().record(&rom, std::time::SystemTime::now(), 5_400);
                setup(&mut app);
                app.draw(&UiInput::default(), w, h);
                app.canvas.save_png(&dir.join(format!("{name}-{w}.png"))).unwrap();
            }
        }
        // The desktop saves dialog, and the focus ring on Settings.
        let tmp = tempfile::tempdir().unwrap();
        let (b, feed, rom) = couch_basket(tmp.path());
        let mut app = App::with(Settings { theme: ThemePref::Paper, ..Settings::default() }, b, Some(feed));
        app.apply(Cmd::ShowSaves(rom));
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("library-saves-regular-paper.png")).unwrap();
        app.modal = None;
        app.tab = Tab::Settings;
        app.pad_used = true;
        app.draw(&UiInput::default(), 1280, 900);
        app.apply(Cmd::Focus(Some("Map buttons…".into())));
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("settings-pad-regular-paper.png")).unwrap();
    }

    /// Continue loads the newest save; Start fresh (Play) doesn't; a fruit
    /// that can't load saves, or a game with none, just plays.
    #[test]
    fn continue_loads_the_newest_save() {
        let wait = |app: &mut App| {
            let start = Instant::now();
            while app.shelf.running().is_some() {
                assert!(start.elapsed() < Duration::from_secs(30), "the stand-in never exited");
                app.poll();
                thread::sleep(Duration::from_millis(20));
            }
        };
        let tmp = tempfile::tempdir().unwrap();
        let (b, feed, rom) = couch_basket(tmp.path());
        let newer = std::time::SystemTime::now() + Duration::from_secs(60);
        std::fs::File::options().write(true).open(b.games_dir("strawberry").join("homebrew.s5.state")).unwrap().set_modified(newer).unwrap();
        let mut app = App::with(Settings::default(), b, Some(feed.clone()));
        let row = app.shelf.couch_rows(app.feed.as_ref(), None).into_iter().find(|r| r.game.path == rom).unwrap();
        assert!(matches!(app.shelf.detail(&row, &app.basket, app.feed.as_ref()).play, PlayState::Continue { slot: 5, .. }));
        app.apply(Cmd::Continue(rom.clone()));
        assert_eq!(app.play_error, None);
        assert_eq!(app.shelf.launched, ["--list", rom.to_str().unwrap(), "--slot", "5"]);
        wait(&mut app);
        app.apply(Cmd::Play(rom.clone()));
        assert_eq!(app.shelf.launched, ["--list", rom.to_str().unwrap()], "Start fresh");
        wait(&mut app);

        let mut plain = feed;
        plain.fruits.iter_mut().find(|f| f.id == "strawberry").unwrap().load_slot = None;
        let mut app = App::with(Settings::default(), Basket::new(tmp.path()), Some(plain));
        app.apply(Cmd::Continue(rom.clone()));
        assert_eq!(app.shelf.launched, ["--list", rom.to_str().unwrap()], "no load_slot: fresh");
        wait(&mut app);
        let row = app.shelf.couch_rows(app.feed.as_ref(), None).into_iter().find(|r| r.game.path == rom).unwrap();
        assert_eq!(app.shelf.detail(&row, &app.basket, app.feed.as_ref()).play, PlayState::Play);
    }

    /// Covers: a save's picture beats the feed's `art`, which beats the
    /// stripes. Saves show their pictures in couch mode and the dialog.
    #[test]
    fn pictures_for_covers_and_saves() {
        let tmp = tempfile::tempdir().unwrap();
        let (b, mut feed, rom) = couch_basket(tmp.path());
        feed.fruits.iter_mut().find(|f| f.id == "strawberry").unwrap().art = vec!["{rom_dir}/{stem}.cover.png".into()];
        let games = b.games_dir("strawberry");
        let png = |name: &str, rgb: [u8; 3]| std::fs::write(games.join(name), crate::pics::png_bytes(240, 160, rgb)).unwrap();
        png("homebrew.s5.png", [40, 90, 200]);
        for t in ["Golden Sun (USA)", "Metroid Fusion (USA)", "Cars (USA)"] {
            std::fs::write(games.join(format!("{t}.gba")), "x").unwrap();
        }
        png("Golden Sun (USA).cover.png", [40, 160, 70]);
        png("Metroid Fusion (USA).cover.png", [40, 160, 70]);
        std::fs::write(games.join("Metroid Fusion (USA).s1.state"), "x").unwrap();
        png("Metroid Fusion (USA).s1.png", [220, 120, 30]);
        let mut app = App::with(Settings { theme: ThemePref::Paper, ..Settings::default() }, b, Some(feed));
        app.shelf.settle();
        let red = |p: &Path| app.shelf.cover(p).map(|pm| pm.pixel(0, 0).unwrap().red());
        assert_eq!(red(&rom), Some(40), "the save's picture");
        assert_eq!(red(&games.join("Golden Sun (USA).gba")), Some(40), "art");
        assert_eq!(app.shelf.cover(&games.join("Golden Sun (USA).gba")).unwrap().pixel(0, 0).unwrap().green(), 160);
        assert_eq!(red(&games.join("Metroid Fusion (USA).gba")), Some(220), "a save beats art");
        assert_eq!(red(&games.join("Cars (USA).gba")), None, "stripes");

        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        app.shelf.selected = Some(rom.clone());
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("library-pictures-regular-paper.png")).unwrap();
        app.apply(Cmd::LibView(LibView::List));
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("library-pictures-list-regular-paper.png")).unwrap();
        app.apply(Cmd::ShowSaves(rom.clone()));
        app.draw(&UiInput::default(), 1280, 900);
        app.shelf.settle();
        app.draw(&UiInput::default(), 1280, 900);
        app.canvas.save_png(&dir.join("library-saves-pictures-regular-paper.png")).unwrap();
        app.modal = None;
        app.apply(Cmd::Couch);
        app.apply(Cmd::CouchSystem(1));
        let pick = app.shelf.couch_rows(app.feed.as_ref(), Some("strawberry")).iter().position(|r| r.game.path == rom).unwrap();
        app.apply(Cmd::CouchPick(pick));
        app.apply(Cmd::CouchSaves);
        app.draw(&UiInput::default(), 1280, 720);
        app.shelf.settle();
        app.draw(&UiInput::default(), 1280, 720);
        app.canvas.save_png(&dir.join("couch-saves-pictures-1280.png")).unwrap();
    }

    fn job(app: &App, id: &str) -> Job {
        let f = app.feed.as_ref().unwrap().fruit(id).unwrap();
        queue::job_for(f, app.ctx().new_channel(f), 2).expect("a build for this PC")
    }

    /// Rescan finds a new file now; a recheck that brings the same feed
    /// back doesn't rescan, and one with a newer feed does.
    #[test]
    fn rescan_and_an_unchanged_recheck() {
        let tmp = tempfile::tempdir().unwrap();
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, key::PUBLIC_KEY, None).unwrap();
        let mut app = App::with(Settings::default(), stocked(tmp.path()), Some(feed.clone()));
        assert_eq!(app.shelf.games.len(), 1);
        std::fs::write(app.basket.games_dir("strawberry").join("second.gba"), "y").unwrap();
        let fetched = |feed: Feed| {
            let (tx, rx) = channel();
            tx.send(Ok(Fetched { feed, bytes: Vec::new(), sig: String::new() })).unwrap();
            rx
        };
        app.feed_rx = Some(fetched(feed.clone()));
        app.poll();
        assert!(app.feed_rx.is_none());
        assert_eq!(app.shelf.games.len(), 1, "the same feed: no rescan");
        app.apply(Cmd::Rescan);
        assert_eq!(app.shelf.games.len(), 2, "Rescan");

        std::fs::write(app.basket.games_dir("strawberry").join("third.gba"), "z").unwrap();
        let mut newer = feed;
        newer.generated = "2099-01-01T00:00:00Z".into();
        app.feed_rx = Some(fetched(newer));
        app.poll();
        assert_eq!(app.shelf.games.len(), 3, "a newer feed rescans");
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
            ("library-list", true, |app| app.shelf.view = LibView::List),
            ("library-strawberry", true, |app| app.shelf.filter = Some("strawberry".into())),
            ("library-unverified", true, |app| {
                app.shelf.view = LibView::List;
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
        let states: [(&str, Tab, bool, Setup); 11] = [
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
            ("downloads-space", Tab::Downloads, true, |app| {
                let j = job(app, "pomegranate");
                app.queue.enqueue(j);
                let message = "not enough space: it needs 120 MB, and 80 MB is free".into();
                app.queue.on_event(&Event::Failed { fruit: "pomegranate".into(), kind: FailKind::Space, message });
            }),
            ("basket-rollback", Tab::Basket, true, |app| {
                keep_build(app, "v1.2.0");
                app.apply(Cmd::AskRollback("strawberry".into()));
                app.apply(Cmd::ModalPick(1));
            }),
            ("basket-uninstall", Tab::Basket, true, |app| app.apply(Cmd::AskUninstall("strawberry".into()))),
            ("library-ripe", Tab::Library, true, |app| app.settings.watch = vec!["pomegranate".into()]),
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

pub fn run(updated: Option<String>) -> Result<(), String> {
    let mut app = App::new();
    if let Some(from) = updated {
        app.notice = Some(format!("updated from {from} to {VERSION}"));
    }
    let (w, h) = app.window_size();
    let mut video = Video::new("Fruit Basket", w, h, &app.keymap)?;
    let mut pads = Gamepads::new(Ports::Shared);
    app.controllers = pads.connected();
    if !app.controllers.is_empty() && app.settings.couch_on_controller {
        app.apply(Cmd::Couch);
    }
    while video.is_open() && !app.quit {
        let input = app.gather(&mut video, &mut pads);
        app.frame(&mut video, &input);
        if let Err(e) = video.set_couch(app.couch.is_some()) {
            app.notice = Some(e);
            app.couch = None;
        }
        thread::sleep(Duration::from_millis(12));
    }
    let size = video.windowed_size();
    app.shutdown(size);
    Ok(())
}
