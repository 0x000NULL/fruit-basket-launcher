//! The Library tab's state: the scanned games, play history, the compat
//! and dump lists, background hashing, and the running game. `App` owns
//! one and forwards to it; `view` builds what the tab draws.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::basket::{Basket, Current};
use crate::compat::Compat;
use crate::dumps::{self, DumpDb, Hashes, Want};
use crate::feed::{Feed, Fruit};
use crate::launch::{self, Session};
use crate::library::{self, Game, Played};
use crate::lists::{self, Kind};
use crate::settings::Settings;
use crate::ui::library::{Dump, Empty, GameDetail, LibraryView, PlayState, Row};

#[derive(Default)]
pub struct Shelf {
    pub games: Vec<Game>,
    played: Played,
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
    pub list: bool,
    pub selected: Option<PathBuf>,
}

impl Shelf {
    pub fn load(launcher_dir: &Path) -> Shelf {
        Shelf { played: Played::load(launcher_dir), hashes: Hashes::load(launcher_dir), ..Shelf::default() }
    }

    fn installed<'a>(feed: Option<&'a Feed>, installed: &HashMap<String, Current>) -> Vec<&'a Fruit> {
        feed.iter().flat_map(|f| f.fruits.iter()).filter(|f| installed.contains_key(&f.id)).collect()
    }

    /// Scan the folders again, then hash whatever the dump lists need.
    pub fn rescan(&mut self, basket: &Basket, feed: Option<&Feed>, installed: &HashMap<String, Current>, settings: &Settings) {
        let fruits = Shelf::installed(feed, installed);
        self.games = library::scan(basket, &fruits, &settings.folders, &settings.hidden);
        self.start_hashing();
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
        let mut changed = false;
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
                if let Err(e) = self.played.save(launcher_dir) {
                    eprintln!("fruitbasket: saving play time: {e}");
                }
                self.running = None;
                changed = true;
            }
        }
        changed
    }

    pub fn running(&self) -> Option<&str> {
        self.running.as_ref().map(|(name, _)| name.as_str())
    }

    /// Start a game in its fruit's current build. One game at a time.
    pub fn play(&mut self, path: &Path, basket: &Basket, feed: Option<&Feed>) -> Result<(), String> {
        if self.running.is_some() {
            return Err("a game is already running".into());
        }
        let game = self.games.iter().find(|g| g.path == path).ok_or("not in the library")?;
        let fruit = feed.and_then(|f| f.fruit(&game.fruit)).ok_or("fruit not in the feed")?;
        let bin = fruit.bin.as_deref().ok_or("the fruit has no program")?;
        let exe = basket.exe(&fruit.id, bin).ok_or_else(|| format!("{} is not installed", fruit.name))?;
        let rx = launch::start(&exe, &launch::args(&fruit.launch, path, None), path).map_err(|e| e.to_string())?;
        self.running = Some((fruit.name.clone(), rx));
        self.selected = Some(path.to_path_buf());
        Ok(())
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
        }
    }

    /// The games the chip and FIND leave, in the chosen order.
    pub fn rows<'a>(&'a self, feed: Option<&'a Feed>, find: &str) -> Vec<Row<'a>> {
        let q = find.trim().to_lowercase();
        let mut rows: Vec<Row> = self
            .games
            .iter()
            .filter(|g| self.filter.as_deref().is_none_or(|f| f == g.fruit))
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

        let detail = selected.and_then(|p| rows.iter().find(|r| r.game.path == p)).map(|r| {
            let fruit = feed.and_then(|f| f.fruit(&r.game.fruit));
            let (dump, _) = self.dump_state(r.game, fruit);
            let build = basket.current(&r.game.fruit).map(|c| basket.build_dir(&r.game.fruit, &c.build));
            let play = match self.running() {
                Some(name) => PlayState::Running(name.to_string()),
                None if r.last.is_some() => PlayState::Continue,
                None => PlayState::Play,
            };
            GameDetail { row: r.clone(), dump, play, saves: library::saves(&r.game.path, build.as_deref()).len(), file: crate::platform::tilde(&r.game.path) }
        });

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
            az: self.az,
            list: self.list,
            continue_rows,
            rows,
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
