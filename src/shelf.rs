//! The Library tab's state: the scanned games, play history, the compat
//! and dump lists, background hashing, and the running game. `App` owns
//! one and forwards to it; `view` builds what the tab draws.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::basket::{Basket, Current};
use crate::compat::Compat;
use crate::dumps::{self, DumpDb, Hashes, Want};
use crate::feed::{Feed, Fruit};
use crate::launch::{self, Session};
use crate::library::{self, Favorites, Game, Played, Sessions, Slot};
use crate::pics::Pics;
use crate::lists::{self, Kind};
use crate::settings::{LibView, Settings};
use crate::ui::library::{Dump, Empty, GameDetail, LibraryView, PlayState, Row};
use crate::ui::stats::StatsView;

#[derive(Default)]
pub struct Shelf {
    pub games: Vec<Game>,
    played: Played,
    sessions: Sessions,
    favorites: Favorites,
    /// The Favorites chip: only favourites, on top of the fruit chip.
    pub fav_only: bool,
    hashes: Hashes,
    compat: HashMap<String, Compat>,
    dumps: HashMap<String, DumpDb>,
    lists_rx: Option<Receiver<(String, Kind)>>,
    hash_rx: Option<Receiver<(Want, String)>>,
    /// Files handed to the hashing thread and not back yet.
    hashing: HashSet<PathBuf>,
    /// The running game's fruit name, and its session when it exits.
    running: Option<(String, Receiver<Session>)>,
    /// Chip: a fruit id, or None for All.
    pub filter: Option<String>,
    pub az: bool,
    pub view: LibView,
    pub selected: Option<PathBuf>,
    /// Each game's cover picture: its newest save's, else the feed's `art`.
    covers: HashMap<PathBuf, PathBuf>,
    covers_rx: Option<Receiver<HashMap<PathBuf, PathBuf>>>,
    pub pics: Pics,
    /// Slots by game, read once, not every frame; cleared when saves can
    /// have changed (a game exited, a save deleted, a rescan).
    slots: RefCell<HashMap<PathBuf, Vec<Slot>>>,
    /// A game exited: the covers need looking for again.
    pub stale_covers: bool,
    /// The last game's arguments, for the tests to check.
    #[cfg(test)]
    pub launched: Vec<String>,
}

impl Shelf {
    pub fn load(launcher_dir: &Path) -> Shelf {
        Shelf { played: Played::load(launcher_dir), sessions: Sessions::load(launcher_dir), favorites: Favorites::load(launcher_dir), hashes: Hashes::load(launcher_dir), ..Shelf::default() }
    }

    fn installed<'a>(feed: Option<&'a Feed>, installed: &HashMap<String, Current>) -> Vec<&'a Fruit> {
        feed.iter().flat_map(|f| f.fruits.iter()).filter(|f| installed.contains_key(&f.id)).collect()
    }

    /// Scan the folders again, then hash whatever the dump lists need.
    pub fn rescan(&mut self, basket: &Basket, feed: Option<&Feed>, installed: &HashMap<String, Current>, settings: &Settings) {
        let fruits = Shelf::installed(feed, installed);
        self.games = library::scan(basket, &fruits, &settings.folders, &settings.hidden);
        self.start_hashing();
        self.index_covers(basket, feed);
    }

    /// Find every game's cover on a thread: the newest save that has a
    /// picture, else the first of the feed's `art` paths that exists.
    pub fn index_covers(&mut self, basket: &Basket, feed: Option<&Feed>) {
        self.slots.borrow_mut().clear();
        self.stale_covers = false;
        let cache = dirs::cache_dir();
        let jobs: Vec<(PathBuf, Vec<PathBuf>, Vec<PathBuf>, Vec<u8>)> = self
            .games
            .iter()
            .filter_map(|g| {
                let f = feed?.fruit(&g.fruit)?;
                let data = f.uses_data().then(|| basket.data_dir(&f.id));
                let place = launch::Place { rom: &g.path, data: data.as_deref(), code: g.code.as_deref(), cache: cache.as_deref() };
                let art = f.art.iter().filter_map(|t| launch::path(t, &place)).collect();
                let listed: Vec<u8> = (0..=u8::MAX).filter(|n| f.lists_slot(*n)).collect();
                Some((g.path.clone(), library::save_dirs(basket, f), art, listed))
            })
            .collect();
        let (tx, rx) = channel();
        thread::spawn(move || {
            let found = jobs
                .into_iter()
                .filter_map(|(game, dirs, art, listed)| {
                    let slot = library::slots(&game, &dirs).iter().filter(|s| listed.contains(&s.n)).find_map(|s| s.picture().map(Path::to_path_buf));
                    slot.or_else(|| art.into_iter().find(|p| p.is_file())).map(|p| (game, p))
                })
                .collect();
            let _ = tx.send(found);
        });
        self.covers_rx = Some(rx);
    }

    /// The game's save slots, newest first, read once until something
    /// could have changed them.
    pub fn slots(&self, game: &Path, basket: &Basket, fruit: &Fruit) -> Vec<Slot> {
        self.slots
            .borrow_mut()
            .entry(game.to_path_buf())
            .or_insert_with(|| library::slots(game, &library::save_dirs(basket, fruit)).into_iter().filter(|s| fruit.lists_slot(s.n)).collect())
            .clone()
    }

    /// The save Continue loads: the newest, if the fruit can start from one.
    pub fn resume(&self, game: &Path, basket: &Basket, fruit: &Fruit) -> Option<Slot> {
        fruit.loads_slots().then(|| self.slots(game, basket, fruit).into_iter().next()).flatten()
    }

    /// Wait for the cover index and its pictures: the render tests draw
    /// what's loaded.
    #[cfg(test)]
    pub fn settle(&mut self) {
        if let Some(rx) = self.covers_rx.take() {
            if let Ok(found) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
                for p in found.values() {
                    self.pics.want(p);
                }
                self.covers = found;
            }
        }
        self.pics.settle();
    }

    /// The game's cover picture, if it has one and it's loaded.
    pub fn cover(&self, game: &Path) -> Option<&tiny_skia::Pixmap> {
        self.pics.get(self.covers.get(game)?)
    }

    /// Read the cached lists that match the feed, and fetch the rest.
    pub fn lists(&mut self, launcher_dir: &Path, feed: &Feed) {
        self.load_lists(launcher_dir, feed);
        if self.lists_rx.is_none() {
            self.lists_rx = Some(lists::fetch_missing(launcher_dir, feed));
        }
    }

    fn load_lists(&mut self, launcher_dir: &Path, feed: &Feed) {
        self.compat.clear();
        self.dumps.clear();
        for f in &feed.fruits {
            if let Some(text) = f.compat.as_ref().and_then(|r| lists::load(launcher_dir, &f.id, Kind::Compat, r)) {
                self.compat.insert(f.id.clone(), Compat::parse(&text));
            }
            if let Some(text) = f.dumps.as_ref().and_then(|r| lists::load(launcher_dir, &f.id, Kind::Dumps, r)) {
                self.dumps.insert(f.id.clone(), DumpDb::parse(&text));
            }
        }
    }

    fn start_hashing(&mut self) {
        if self.hash_rx.is_some() {
            return;
        }
        let want: Vec<Want> = self
            .games
            .iter()
            .filter(|g| self.dumps.contains_key(&g.fruit) && self.hashes.get(&g.path, g.size, g.mtime).is_none())
            .map(|g| (g.path.clone(), g.size, g.mtime))
            .collect();
        if !want.is_empty() {
            self.hashing = want.iter().map(|w| w.0.clone()).collect();
            self.hash_rx = Some(dumps::hash_in_background(want));
        }
    }

    /// Pick up lists, hashes and finished games. True if the view changed.
    pub fn poll(&mut self, launcher_dir: &Path, feed: Option<&Feed>) -> bool {
        let mut changed = self.pics.poll();
        if let Some(rx) = &self.covers_rx {
            if let Ok(found) = rx.try_recv() {
                self.covers_rx = None;
                for p in found.values() {
                    self.pics.want(p);
                }
                self.covers = found;
                changed = true;
            }
        }
        if let Some(rx) = &self.lists_rx {
            let mut got = false;
            let done = loop {
                match rx.try_recv() {
                    Ok(_) => got = true,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if done {
                self.lists_rx = None;
            }
            if got {
                if let Some(feed) = feed {
                    self.load_lists(launcher_dir, feed);
                }
                self.start_hashing();
                changed = true;
            }
        }
        if let Some(rx) = &self.hash_rx {
            let mut got = false;
            let done = loop {
                match rx.try_recv() {
                    Ok(((path, size, mtime), sha1)) => {
                        self.hashing.remove(&path);
                        self.hashes.insert(path, size, mtime, sha1);
                        got = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
                }
            };
            if got {
                if let Err(e) = self.hashes.save(launcher_dir) {
                    eprintln!("fruitbasket: saving hashes: {e}");
                }
                changed = true;
            }
            if done {
                self.hash_rx = None;
                self.hashing.clear();
                // Games added while hashing ran.
                self.start_hashing();
            }
        }
        if let Some((_, rx)) = &self.running {
            if let Ok(s) = rx.try_recv() {
                self.played.record(&s.game, s.started, s.secs);
                if let Err(e) = self.played.save(launcher_dir).and_then(|_| self.sessions.append(launcher_dir, &s.game, s.started, s.secs)) {
                    eprintln!("fruitbasket: saving play time: {e}");
                }
                self.running = None;
                // The game may have saved: new slots, new pictures.
                self.slots.borrow_mut().clear();
                self.stale_covers = true;
                self.pics.reload();
                changed = true;
            }
        }
        changed
    }

    pub fn running(&self) -> Option<&str> {
        self.running.as_ref().map(|(name, _)| name.as_str())
    }

    /// Start a game in its fruit's current build, from save `slot` if
    /// given, with the fruit's couch arguments if `couch`. One game at a time.
    pub fn play(&mut self, path: &Path, basket: &Basket, feed: Option<&Feed>, slot: Option<u8>, couch: bool) -> Result<(), String> {
        if self.running.is_some() {
            return Err("a game is already running".into());
        }
        let game = self.games.iter().find(|g| g.path == path).ok_or("not in the library")?;
        let fruit = feed.and_then(|f| f.fruit(&game.fruit)).ok_or("fruit not in the feed")?;
        let bin = fruit.bin.as_deref().ok_or("the fruit has no program")?;
        let exe = basket.exe(&fruit.id, bin).ok_or_else(|| format!("{} is not installed", fruit.name))?;
        let data = fruit.uses_data().then(|| basket.data_dir(&fruit.id));
        if data.is_some() {
            // Installs from before the fruit took `{data}` move over on first play.
            basket.migrate_data(&fruit.id, fruit.data_carry()).map_err(|e| format!("moving saves to data/: {e}"))?;
        }
        let template = match slot {
            Some(_) => fruit.load_slot.as_ref().filter(|t| !t.is_empty()).ok_or_else(|| format!("{} can't start from a save", fruit.name))?,
            None => &fruit.launch,
        };
        let mut args = launch::args(template, Some(path), slot, data.as_deref())?;
        if couch {
            let build = basket.current(&fruit.id).map(|c| c.build).unwrap_or_default();
            args.extend(launch::args(fruit.couch_args(&build), Some(path), slot, data.as_deref())?);
        }
        #[cfg(test)]
        {
            self.launched = args.clone();
        }
        let rx = launch::start(&exe, &args, path).map_err(|e| e.to_string())?;
        self.running = Some((fruit.name.clone(), rx));
        self.selected = Some(path.to_path_buf());
        Ok(())
    }

    /// After the basket moves: play history and hashes follow the games.
    pub fn rebase(&mut self, launcher_dir: &Path, old: &Path, new: &Path) {
        self.played.rebase(old, new);
        self.hashes.rebase(old, new);
        self.favorites.rebase(old, new);
        if let Err(e) = self.sessions.rebase(launcher_dir, old, new) {
            eprintln!("fruitbasket: saving sessions: {e}");
        }
        if let Err(e) = self.played.save(launcher_dir).and_then(|_| self.hashes.save(launcher_dir)).and_then(|_| self.favorites.save(launcher_dir)) {
            eprintln!("fruitbasket: saving library state: {e}");
        }
        self.selected = self.selected.as_deref().map(|p| library::rebase(p, old, new));
    }

    pub fn remove(&mut self, path: &Path) {
        self.games.retain(|g| g.path != path);
        if self.selected.as_deref() == Some(path) {
            self.selected = None;
        }
    }

    fn dump_state<'a>(&'a self, g: &Game, fruit: Option<&'a Fruit>) -> (Dump<'a>, Option<&'a str>) {
        let Some(db) = self.dumps.get(&g.fruit) else { return (Dump::NoList, None) };
        let name = fruit.and_then(|f| f.dump_db.as_deref()).unwrap_or("the dump list");
        match self.hashes.get(&g.path, g.size, g.mtime) {
            Some(h) => match db.get(h) {
                Some((serial, _)) => (Dump::Verified(name), Some(serial.as_str()).filter(|s| !s.is_empty())),
                None => (Dump::NoMatch(name), None),
            },
            None => (Dump::Checking, None),
        }
    }

    fn row<'a>(&'a self, g: &'a Game, feed: Option<&'a Feed>) -> Row<'a> {
        let fruit = feed.and_then(|f| f.fruit(&g.fruit));
        let (_, serial) = self.dump_state(g, fruit);
        Row {
            game: g,
            system: fruit.map_or("", |f| f.system.as_str()),
            fruit_name: fruit.map_or(g.fruit.as_str(), |f| f.name.as_str()),
            level: self.compat.get(&g.fruit).and_then(|c| c.level(g, serial)),
            last: self.played.last(&g.path),
            secs: self.played.secs(&g.path),
            picture: self.cover(&g.path),
            favorite: self.favorites.contains(&g.path),
        }
    }

    /// Everything the aside (or couch mode's Details) shows about a game.
    pub fn detail<'a>(&'a self, r: &Row<'a>, basket: &Basket, feed: Option<&'a Feed>) -> GameDetail<'a> {
        let fruit = feed.and_then(|f| f.fruit(&r.game.fruit));
        let (dump, _) = self.dump_state(r.game, fruit);
        let dirs = fruit.map(|f| library::save_dirs(basket, f)).unwrap_or_default();
        let play = match (self.running(), fruit.and_then(|f| self.resume(&r.game.path, basket, f))) {
            (Some(name), _) => PlayState::Running(name.to_string()),
            (None, Some(s)) => PlayState::Continue { slot: s.n, saved: s.saved },
            (None, None) => PlayState::Play,
        };
        let sessions = self.sessions.of(&r.game.path);
        GameDetail {
            row: r.clone(),
            dump,
            play,
            saves: library::saves(&r.game.path, &dirs).iter().filter(|p| library::slot_number(p).is_none_or(|n| fruit.is_none_or(|f| f.lists_slot(n)))).count(),
            file: crate::platform::tilde(&r.game.path),
            sessions: sessions.len(),
            average: if sessions.is_empty() { 0 } else { sessions.iter().map(|s| s.1).sum::<u64>() / sessions.len() as u64 },
            recent: sessions.into_iter().take(5).collect(),
        }
    }

    /// Couch mode's games: one fruit's (or all), favourites first, then
    /// most recently played. Couch mode ignores the desktop's chip, sort and FIND.
    pub fn couch_rows<'a>(&'a self, feed: Option<&'a Feed>, fruit: Option<&str>) -> Vec<Row<'a>> {
        let mut rows: Vec<Row> = self.games.iter().filter(|g| fruit.is_none_or(|f| f == g.fruit)).map(|g| self.row(g, feed)).collect();
        rows.sort_by(|a, b| b.favorite.cmp(&a.favorite).then(b.last.cmp(&a.last)).then_with(|| a.game.title.to_lowercase().cmp(&b.game.title.to_lowercase())));
        rows
    }

    /// The Stats view over `rows` (the chip, Favorites and FIND apply).
    pub fn stats<'a>(&self, rows: &[Row<'a>], feed: Option<&'a Feed>, now: std::time::SystemTime) -> StatsView<'a> {
        let now = now.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let shown: HashSet<&Path> = rows.iter().map(|r| r.game.path.as_path()).collect();
        let mut sessions: Vec<&(i64, u64, PathBuf)> = self.sessions.list.iter().filter(|s| shown.contains(s.2.as_path())).collect();
        sessions.sort_by_key(|s| s.0);
        const WEEK: i64 = 7 * 24 * 3600;
        let mut weeks = [0u64; crate::ui::stats::WEEKS];
        // Oldest week first; the last is the seven days up to now.
        for s in &sessions {
            let ago = ((now - s.0).max(0) / WEEK) as usize;
            if ago < weeks.len() {
                weeks[weeks.len() - 1 - ago] += s.1;
            }
        }
        let mut fruits: Vec<(&'a str, u64)> = Vec::new();
        for f in feed.iter().flat_map(|f| f.fruits.iter()) {
            let secs: u64 = rows.iter().filter(|r| r.game.fruit == f.id).map(|r| r.secs).sum();
            if secs > 0 {
                fruits.push((f.name.as_str(), secs));
            }
        }
        fruits.sort_by(|a, b| b.1.cmp(&a.1));
        let mut top: Vec<Row<'a>> = rows.iter().filter(|r| r.secs > 0).cloned().collect();
        top.sort_by(|a, b| b.secs.cmp(&a.secs));
        top.truncate(10);
        let title = |p: &Path| rows.iter().find(|r| r.game.path == p).map(|r| r.game.title.clone()).unwrap_or_default();
        StatsView {
            total: rows.iter().map(|r| r.secs).sum(),
            week: sessions.iter().filter(|s| now - s.0 < WEEK).map(|s| s.1).sum(),
            sessions: sessions.len(),
            games: rows.iter().filter(|r| r.secs > 0).count(),
            weeks: weeks.to_vec(),
            fruits,
            top,
            recent: sessions.iter().rev().take(20).map(|s| (title(&s.2), s.0, s.1)).collect(),
        }
    }

    /// Mark or unmark a favourite, and keep it.
    pub fn toggle_favorite(&mut self, launcher_dir: &Path, game: &Path) {
        self.favorites.toggle(game);
        if let Err(e) = self.favorites.save(launcher_dir) {
            eprintln!("fruitbasket: saving favourites: {e}");
        }
        if self.favorites.len() == 0 {
            self.fav_only = false;
        }
    }

    /// The games the chip and FIND leave, in the chosen order.
    pub fn rows<'a>(&'a self, feed: Option<&'a Feed>, find: &str) -> Vec<Row<'a>> {
        let q = find.trim().to_lowercase();
        let mut rows: Vec<Row> = self
            .games
            .iter()
            .filter(|g| self.filter.as_deref().is_none_or(|f| f == g.fruit))
            .filter(|g| !self.fav_only || self.favorites.contains(&g.path))
            .map(|g| self.row(g, feed))
            .filter(|r| q.is_empty() || r.game.title.to_lowercase().contains(&q) || r.system.to_lowercase().contains(&q) || r.fruit_name.to_lowercase().contains(&q))
            .collect();
        if self.az {
            rows.sort_by_key(|r| r.game.title.to_lowercase());
        } else {
            rows.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.game.title.to_lowercase().cmp(&b.game.title.to_lowercase())));
        }
        rows
    }

    /// The selection if it is listed, else the first game.
    pub fn selected_in<'a>(&self, rows: &[Row<'a>]) -> Option<&'a Path> {
        let sel = self.selected.as_deref();
        let game: Option<&'a Game> = rows.iter().find(|r| Some(r.game.path.as_path()) == sel).or(rows.first()).map(|r| r.game);
        game.map(|g| g.path.as_path())
    }

    pub fn view<'a>(&'a self, basket: &Basket, feed: Option<&'a Feed>, find: &str, sheet: bool, scroll: f32, aside_scroll: f32) -> LibraryView<'a> {
        let rows = self.rows(feed, find);
        let mut continue_rows: Vec<Row> = rows.iter().filter(|r| r.last.is_some()).cloned().collect();
        continue_rows.sort_by(|a, b| b.last.cmp(&a.last));
        continue_rows.truncate(3);
        let selected = self.selected_in(&rows);

        let mut counts: Vec<(Option<&str>, &str, usize)> = vec![(None, "All", self.games.len())];
        for f in feed.iter().flat_map(|f| f.fruits.iter()) {
            let n = self.games.iter().filter(|g| g.fruit == f.id).count();
            if n > 0 {
                counts.push((Some(f.id.as_str()), f.name.as_str(), n));
            }
        }
        let heading = match self.filter.as_deref().and_then(|id| feed.and_then(|f| f.fruit(id))) {
            Some(f) => format!("{} games", f.name),
            None => "All games".to_string(),
        };

        let detail = selected.and_then(|p| rows.iter().find(|r| r.game.path == p)).map(|r| self.detail(r, basket, feed));
        let stats = (self.view == LibView::Stats).then(|| self.stats(&rows, feed, std::time::SystemTime::now()));

        let favorites = self.games.iter().filter(|g| self.favorites.contains(&g.path)).count();
        let empty = if self.games.is_empty() {
            Empty::NoGames
        } else if rows.is_empty() {
            Empty::NoMatch(find.trim().to_string())
        } else {
            Empty::No
        };
        LibraryView {
            chips: counts,
            filter: self.filter.as_deref(),
            favorites,
            fav_only: self.fav_only,
            az: self.az,
            view: self.view,
            continue_rows,
            rows,
            stats,
            heading,
            selected,
            detail,
            empty,
            sheet,
            scroll,
            aside_scroll,
        }
    }

    /// For tests: record play and set lists without the network.
    #[cfg(test)]
    pub fn played_mut(&mut self) -> &mut Played {
        &mut self.played
    }

    /// For tests: a finished session, as `poll` would log it.
    #[cfg(test)]
    pub fn log_session(&mut self, launcher_dir: &Path, game: &Path, started: std::time::SystemTime, secs: u64) {
        self.played.record(game, started, secs);
        self.sessions.append(launcher_dir, game, started, secs).unwrap();
    }

    #[cfg(test)]
    pub fn set_lists(&mut self, fruit: &str, compat: Compat, dumps: Option<DumpDb>) {
        self.compat.insert(fruit.into(), compat);
        if let Some(d) = dumps {
            self.dumps.insert(fruit.into(), d);
        }
    }

    #[cfg(test)]
    pub fn set_hash(&mut self, g: &Game, sha1: &str) {
        self.hashes.insert(g.path.clone(), g.size, g.mtime, sha1.into());
    }
}
