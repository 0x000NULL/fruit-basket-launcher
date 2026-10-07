//! The game library: every file an installed fruit reads, in its `games/`
//! folder and in the extra folders from Settings, plus when each was last
//! played and for how long (`launcher/played.tsv`).

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::basket::{write_atomic, Basket};
use crate::feed::Fruit;

/// Folders below a games folder that are still searched.
pub(crate) const DEPTH: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub path: PathBuf,
    /// The fruit that plays it.
    pub fruit: String,
    pub title: String,
    pub size: u64,
    pub mtime: i64,
    /// A serial or product code, when one can be read cheaply: the GBA
    /// header's game code, or a disc serial in the file name.
    pub code: Option<String>,
}

/// Scan for games. `fruits` are the installed ones, in feed order; a file
/// in an extra folder goes to the first fruit that plays it (`plays`).
pub fn scan(basket: &Basket, fruits: &[&Fruit], extra: &[PathBuf], hidden: &[PathBuf]) -> Vec<Game> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |path: PathBuf, fruit: &Fruit, out: &mut Vec<Game>| {
        if hidden.contains(&path) || !seen.insert(path.clone()) {
            return;
        }
        if let Some(g) = game(&path, fruit) {
            out.push(g);
        }
    };
    for f in fruits {
        for path in files(&basket.games_dir(&f.id), DEPTH) {
            if plays(f, &path, &mut None) {
                add(path, f, &mut out);
            }
        }
    }
    for folder in extra {
        for path in files(folder, DEPTH) {
            let mut names = None;
            if let Some(f) = fruits.iter().find(|f| plays(f, &path, &mut names)) {
                add(path, f, &mut out);
            }
        }
    }
    out
}

/// True if `fruit` plays the file at `path`: it has one of the fruit's
/// `ext`, or it is one of the fruit's `archives` with a file inside that
/// has. `names` caches the archive's file names across fruits.
pub fn plays(fruit: &Fruit, path: &Path, names: &mut Option<Vec<String>>) -> bool {
    if fruit.reads(path) {
        return true;
    }
    if !fruit.takes_archive(path) {
        return false;
    }
    names.get_or_insert_with(|| zip_names(path)).iter().any(|n| fruit.reads(Path::new(n)))
}

/// The file names inside a zip, from its central directory; none if it
/// isn't one.
fn zip_names(path: &Path) -> Vec<String> {
    let Ok(f) = fs::File::open(path) else { return Vec::new() };
    match zip::ZipArchive::new(io::BufReader::new(f)) {
        Ok(z) => z.file_names().map(str::to_string).collect(),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn files(dir: &Path, depth: u32) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        match e.file_type() {
            Ok(t) if t.is_dir() && depth > 0 => out.extend(files(&e.path(), depth - 1)),
            Ok(t) if t.is_file() => out.push(e.path()),
            _ => {}
        }
    }
    out
}

fn game(path: &Path, fruit: &Fruit) -> Option<Game> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let file_name = path.file_name()?.to_string_lossy().to_string();
    let title = display_title(&file_name);
    let code = serial_in_name(&file_name).or_else(|| gba_code(path));
    Some(Game { path: path.to_path_buf(), fruit: fruit.id.clone(), title, size: meta.len(), mtime, code })
}

/// A file name as a title: extension and `(USA)` / `[!]` tags dropped,
/// ` - ` read as a colon, and a trailing article moved to the front in
/// each part ("Legend of Zelda, The" → "The Legend of Zelda").
pub fn display_title(file_name: &str) -> String {
    let stem = Path::new(file_name).file_stem().map_or(file_name.to_string(), |s| s.to_string_lossy().into_owned());
    let mut out = String::new();
    let mut depth = 0i32;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let parts: Vec<String> = out
        .split(" - ")
        .map(|p| {
            let p = p.split_whitespace().collect::<Vec<_>>().join(" ");
            match p.rsplit_once(", ") {
                Some((head, art)) if ["The", "A", "An"].contains(&art) => format!("{art} {head}"),
                _ => p,
            }
        })
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        stem.trim().to_string()
    } else {
        parts.join(": ")
    }
}

/// `SLUS-20312`, `SCES_503.60` and the like, anywhere in a file name.
pub fn serial_in_name(name: &str) -> Option<String> {
    let up = name.to_ascii_uppercase();
    let b = up.as_bytes();
    for i in 0..b.len().saturating_sub(8) {
        let letters = &b[i..i + 4];
        if !letters.iter().all(u8::is_ascii_uppercase) || (i > 0 && b[i - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let sep = b[i + 4];
        if sep != b'-' && sep != b'_' {
            continue;
        }
        let digits: String = up[i + 5..].chars().filter(|c| *c != '.').take(5).collect();
        if digits.len() == 5 && digits.chars().all(|c| c.is_ascii_digit()) {
            return Some(format!("{}-{digits}", &up[i..i + 4]));
        }
    }
    None
}

/// The four-letter game code at 0xAC of a GBA cartridge header.
fn gba_code(path: &Path) -> Option<String> {
    if !path.extension()?.to_str()?.eq_ignore_ascii_case("gba") {
        return None;
    }
    let mut f = fs::File::open(path).ok()?;
    let mut head = [0u8; 0xB0];
    f.read_exact(&mut head).ok()?;
    let code = &head[0xAC..0xB0];
    code.iter().all(u8::is_ascii_alphanumeric).then(|| String::from_utf8_lossy(code).to_string())
}

/// Where `fruit` keeps save states besides the game's own folder: its data
/// folder when it takes `{data}`, else the current build (Pomegranate's
/// `<stem>.state` beside its exe before v0.3.0).
pub fn save_dirs(basket: &Basket, fruit: &Fruit) -> Vec<PathBuf> {
    let base = if fruit.uses_data() {
        basket.data_dir(&fruit.id)
    } else {
        match basket.current(&fruit.id) {
            Some(c) => basket.build_dir(&fruit.id, &c.build),
            None => return Vec::new(),
        }
    };
    vec![base.clone(), base.join("states")]
}

/// Save files the fruit keeps for this game: slot states named after it,
/// beside the game or in `dirs` (from `save_dirs`).
pub fn saves(game: &Path, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let Some(stem) = game.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else { return Vec::new() };
    let mut out = Vec::new();
    for d in game.parent().into_iter().chain(dirs.iter().map(PathBuf::as_path)) {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with(&format!("{stem}.")) && name.ends_with(".state") {
                out.push(e.path());
            }
        }
    }
    out
}

/// A save-state slot of one game.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub n: u8,
    pub path: PathBuf,
    /// When it was saved: the `saved_at` beside it if the emulator writes
    /// one (Pomegranate's `.sN.toml`), else the file's time.
    pub saved: SystemTime,
    /// Files that go with it: its picture and notes, and for slot 0 a
    /// pre-slots `<stem>.state` that Pomegranate would copy back in.
    pub extra: Vec<PathBuf>,
}

impl Slot {
    /// The picture the emulator saved with it, if it writes one
    /// (`<stem>.sN.png`).
    pub fn picture(&self) -> Option<&Path> {
        self.extra.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).map(PathBuf::as_path)
    }
}

/// The game's slots, newest first: `<stem>.s<N>.state` beside the game or
/// in `dirs`. The numbers come from the files, so Strawberry's 1–8 and
/// Pomegranate's 0–9 both work.
pub fn slots(game: &Path, dirs: &[PathBuf]) -> Vec<Slot> {
    let Some(stem) = game.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else { return Vec::new() };
    let mut out: Vec<Slot> = Vec::new();
    for d in game.parent().into_iter().chain(dirs.iter().map(PathBuf::as_path)) {
        let names: Vec<(String, PathBuf)> = fs::read_dir(d).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path())).collect();
        for (name, path) in &names {
            let Some(n) = name.strip_prefix(&format!("{stem}.s")).and_then(|r| r.strip_suffix(".state")).and_then(|n| n.parse::<u8>().ok()) else { continue };
            if out.iter().any(|s| s.n == n) {
                continue;
            }
            let sibling = |ext: &str| names.iter().find(|(m, _)| *m == format!("{stem}.s{n}.{ext}")).map(|(_, p)| p.clone());
            let mut extra: Vec<PathBuf> = ["png", "toml"].iter().filter_map(|e| sibling(e)).collect();
            if n == 0 {
                extra.extend(names.iter().filter(|(m, _)| *m == format!("{stem}.state")).map(|(_, p)| p.clone()));
            }
            let mtime = fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let saved = sibling("toml").and_then(|t| saved_at(&t)).unwrap_or(mtime);
            out.push(Slot { n, path: path.clone(), saved, extra });
        }
    }
    out.sort_by(|a, b| b.saved.cmp(&a.saved).then(a.n.cmp(&b.n)));
    out
}

/// The N of a `<stem>.s<N>.state` file name; `None` for any other file.
pub fn slot_number(path: &Path) -> Option<u8> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let rest = name.strip_suffix(".state")?;
    let (_, n) = rest.rsplit_once(".s")?;
    n.parse().ok()
}

/// `saved_at = <unix secs>` from a slot's notes.
fn saved_at(toml: &Path) -> Option<SystemTime> {
    let text = fs::read_to_string(toml).ok()?;
    let line = text.lines().find(|l| l.trim_start().starts_with("saved_at"))?;
    let secs: u64 = line.split('=').nth(1)?.trim().parse().ok()?;
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

/// Delete a slot and the files that go with it.
pub fn delete_slot(slot: &Slot) -> std::io::Result<()> {
    fs::remove_file(&slot.path)?;
    for p in &slot.extra {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

/// `path` with the prefix `old` swapped for `new`, after the basket moves;
/// a path outside `old` is returned as it was.
pub fn rebase(path: &Path, old: &Path, new: &Path) -> PathBuf {
    match path.strip_prefix(old) {
        Ok(rest) => new.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// When each game was last played and for how long, by path.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Played {
    map: HashMap<PathBuf, (i64, u64)>,
}

impl Played {
    fn path(launcher_dir: &Path) -> PathBuf {
        launcher_dir.join("played.tsv")
    }

    /// `<last played unix secs>\t<play secs>\t<path>` per line.
    pub fn load(launcher_dir: &Path) -> Played {
        let text = fs::read_to_string(Played::path(launcher_dir)).unwrap_or_default();
        let map = text
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(3, '\t');
                let last = f.next()?.parse().ok()?;
                let secs = f.next()?.parse().ok()?;
                Some((PathBuf::from(f.next()?), (last, secs)))
            })
            .collect();
        Played { map }
    }

    pub fn save(&self, launcher_dir: &Path) -> io::Result<()> {
        let mut rows: Vec<_> = self.map.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let text: String = rows.iter().map(|(p, (last, secs))| format!("{last}\t{secs}\t{}\n", p.display())).collect();
        write_atomic(&Played::path(launcher_dir), text.as_bytes())
    }

    pub fn last(&self, path: &Path) -> Option<SystemTime> {
        self.map.get(path).filter(|(t, _)| *t > 0).map(|(t, _)| UNIX_EPOCH + std::time::Duration::from_secs(*t as u64))
    }

    pub fn secs(&self, path: &Path) -> u64 {
        self.map.get(path).map_or(0, |(_, s)| *s)
    }

    /// A session that started at `started` and ran `secs`.
    /// After the basket moves from `old` to `new`.
    pub fn rebase(&mut self, old: &Path, new: &Path) {
        self.map = self.map.drain().map(|(p, v)| (rebase(&p, old, new), v)).collect();
    }

    pub fn record(&mut self, path: &Path, started: SystemTime, secs: u64) {
        let at = started.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let e = self.map.entry(path.to_path_buf()).or_insert((0, 0));
        e.0 = e.0.max(at);
        e.1 += secs;
    }
}

/// Every finished session: `launcher/sessions.tsv`,
/// `<started unix secs>\t<secs>\t<path>` a line, appended as games exit.
/// `played.tsv` stays the summary; sessions from before v1.1 are only there.
#[derive(Debug, Default)]
pub struct Sessions {
    /// Oldest first.
    pub list: Vec<(i64, u64, PathBuf)>,
}

impl Sessions {
    fn path(launcher_dir: &Path) -> PathBuf {
        launcher_dir.join("sessions.tsv")
    }

    pub fn load(launcher_dir: &Path) -> Sessions {
        let text = fs::read_to_string(Sessions::path(launcher_dir)).unwrap_or_default();
        let list = text
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(3, '\t');
                Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?, PathBuf::from(f.next()?)))
            })
            .collect();
        Sessions { list }
    }

    pub fn append(&mut self, launcher_dir: &Path, game: &Path, started: SystemTime, secs: u64) -> io::Result<()> {
        use std::io::Write;
        let at = started.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        fs::create_dir_all(launcher_dir)?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(Sessions::path(launcher_dir))?;
        writeln!(f, "{at}\t{secs}\t{}", game.display())?;
        self.list.push((at, secs, game.to_path_buf()));
        Ok(())
    }

    /// After a basket move: the whole log, rewritten.
    pub fn rebase(&mut self, launcher_dir: &Path, old: &Path, new: &Path) -> io::Result<()> {
        for s in &mut self.list {
            s.2 = rebase(&s.2, old, new);
        }
        let text: String = self.list.iter().map(|(at, secs, p)| format!("{at}\t{secs}\t{}\n", p.display())).collect();
        write_atomic(&Sessions::path(launcher_dir), text.as_bytes())
    }

    /// One game's sessions, newest first.
    pub fn of(&self, game: &Path) -> Vec<(i64, u64)> {
        let mut out: Vec<(i64, u64)> = self.list.iter().filter(|s| s.2 == game).map(|s| (s.0, s.1)).collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.0));
        out
    }
}

/// The games marked as favourites: `launcher/favorites.tsv`, one path a line.
#[derive(Debug, Default)]
pub struct Favorites {
    set: std::collections::BTreeSet<PathBuf>,
}

impl Favorites {
    fn path(launcher_dir: &Path) -> PathBuf {
        launcher_dir.join("favorites.tsv")
    }

    pub fn load(launcher_dir: &Path) -> Favorites {
        let text = fs::read_to_string(Favorites::path(launcher_dir)).unwrap_or_default();
        Favorites { set: text.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect() }
    }

    pub fn save(&self, launcher_dir: &Path) -> io::Result<()> {
        let text: String = self.set.iter().map(|p| format!("{}\n", p.display())).collect();
        write_atomic(&Favorites::path(launcher_dir), text.as_bytes())
    }

    pub fn contains(&self, game: &Path) -> bool {
        self.set.contains(game)
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Mark or unmark; true if it is a favourite now.
    pub fn toggle(&mut self, game: &Path) -> bool {
        if self.set.remove(game) {
            false
        } else {
            self.set.insert(game.to_path_buf());
            true
        }
    }

    /// After a basket move.
    pub fn rebase(&mut self, old: &Path, new: &Path) {
        self.set = std::mem::take(&mut self.set).into_iter().map(|p| rebase(&p, old, new)).collect();
    }
}

/// Lowercase letters and digits only, articles moved: for matching a
/// file's title against a list's ("Legend of Zelda, The" = "The Legend of Zelda").
pub fn norm_title(title: &str) -> String {
    let t = title.trim();
    let t = match t.rsplit_once(", ") {
        Some((head, art)) if ["the", "a", "an"].contains(&art.to_lowercase().as_str()) => format!("{art} {head}"),
        _ => t.to_string(),
    };
    t.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Status;

    pub(crate) fn fruit(id: &str, ext: &[&str]) -> Fruit {
        Fruit {
            id: id.into(),
            no: 2,
            name: id.into(),
            system: "GBA".into(),
            pixel: true,
            ext: ext.iter().map(|s| s.to_string()).collect(),
            archives: vec![],
            status: Status::Released,
            summary: String::new(),
            blurb: String::new(),
            bin: Some(id.into()),
            bios: None,
            dump_db: None,
            launch: vec!["{rom}".into()],
            load_slot: None,
            open: vec![],
            couch: vec![],
            carry: vec![],
            art: vec![],
            fresh: vec![],
            slots: vec![],
            url: String::new(),
            readme_url: String::new(),
            compat: None,
            dumps: None,
            changelog: vec![],
            stable: None,
            nightly: None,
            releases: vec![],
        }
    }

    #[test]
    fn scans_games_extra_folders_and_skips_hidden() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path().join("basket"));
        let games = b.games_dir("strawberry");
        fs::create_dir_all(games.join("sub")).unwrap();
        let mut rom = vec![0u8; 0xC0];
        rom[0xAC..0xB0].copy_from_slice(b"BZ4E");
        fs::write(games.join("Final Fantasy IV Advance (USA).gba"), &rom).unwrap();
        fs::write(games.join("sub/Hidden.gba"), b"x").unwrap();
        fs::write(games.join("notes.txt"), b"x").unwrap();
        let extra = t.path().join("extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(extra.join("Legend of Zelda, The (USA).gba"), b"x").unwrap();
        fs::write(extra.join("disc.iso"), b"x").unwrap();

        let berry = fruit("strawberry", &[".gba"]);
        let got = scan(&b, &[&berry], std::slice::from_ref(&extra), &[games.join("sub/Hidden.gba")]);
        let mut titles: Vec<_> = got.iter().map(|g| g.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Final Fantasy IV Advance", "The Legend of Zelda"]);
        let ff = got.iter().find(|g| g.title.starts_with("Final")).unwrap();
        assert_eq!(ff.code.as_deref(), Some("BZ4E"));
    }

    fn zip_with(path: &Path, inner: &str) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        w.start_file(inner, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut w, b"rom").unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn zips_go_to_the_fruit_that_reads_what_is_inside() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path().join("basket"));
        let extra = t.path().join("extra");
        fs::create_dir_all(&extra).unwrap();
        zip_with(&extra.join("Golden Sun (USA).zip"), "Golden Sun (USA).gba");
        zip_with(&extra.join("Super Mario Bros. (World).zip"), "smb.nes");
        zip_with(&extra.join("Readme.zip"), "readme.txt");
        fs::write(extra.join("Broken.zip"), b"not a zip").unwrap();
        // A fruit that doesn't list .zip in archives never takes one.
        zip_with(&extra.join("Disc.zip"), "disc.iso");

        // NES first in feed order: the GBA zip must still skip it.
        let mut apple = fruit("crabapple", &[".nes"]);
        apple.archives = vec![".zip".into()];
        let mut berry = fruit("strawberry", &[".gba"]);
        berry.archives = vec![".ZIP".into()];
        let pom = fruit("pomegranate", &[".iso"]);
        let got = scan(&b, &[&apple, &berry, &pom], std::slice::from_ref(&extra), &[]);
        let mut by: Vec<(&str, &str)> = got.iter().map(|g| (g.fruit.as_str(), g.title.as_str())).collect();
        by.sort();
        assert_eq!(by, [("crabapple", "Super Mario Bros."), ("strawberry", "Golden Sun")]);
        let gs = got.iter().find(|g| g.fruit == "strawberry").unwrap();
        assert_eq!(gs.path, extra.join("Golden Sun (USA).zip"), "launched as the zip itself");

        // In a fruit's own games folder too, and counted there.
        let games = b.games_dir("strawberry");
        fs::create_dir_all(&games).unwrap();
        zip_with(&games.join("Minish Cap.zip"), "minish.gba");
        let got = scan(&b, &[&berry], &[], &[]);
        assert_eq!(got.len(), 1);
        assert!(!plays(&berry, &extra.join("Broken.zip"), &mut None));
    }

    #[test]
    fn serials_and_titles() {
        assert_eq!(serial_in_name("Final Fantasy X (USA) [SLUS-20312].chd").as_deref(), Some("SLUS-20312"));
        assert_eq!(serial_in_name("SCES_503.60.iso").as_deref(), Some("SCES-50360"));
        assert_eq!(serial_in_name("Kingdom Hearts.iso"), None);
        assert_eq!(norm_title("Legend of Zelda, The"), norm_title("The Legend of Zelda"));
        assert_eq!(display_title("Mario Kart - Super Circuit (USA) [!].gba"), "Mario Kart: Super Circuit");
        assert_eq!(display_title("Legend of Zelda, The (USA).gba"), "The Legend of Zelda");
        assert_eq!(display_title("(USA).gba"), "(USA)");
        assert_eq!(norm_title("Disgaea: Hour of Darkness"), "disgaeahourofdarkness");
    }

    #[test]
    fn played_round_trip_and_saves() {
        let t = tempfile::tempdir().unwrap();
        let rom = t.path().join("Game (USA).gba");
        let mut p = Played::default();
        let start = UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        p.record(&rom, start, 60);
        p.record(&rom, start, 30);
        p.save(t.path()).unwrap();
        let back = Played::load(t.path());
        assert_eq!(back.secs(&rom), 90);
        assert_eq!(back.last(&rom), Some(start));
        assert_eq!(back, p);

        fs::write(t.path().join("Game (USA).s1.state"), b"x").unwrap();
        fs::write(t.path().join("Game (USA).s3.state"), b"x").unwrap();
        fs::write(t.path().join("Other.s1.state"), b"x").unwrap();
        let build = t.path().join("build");
        fs::create_dir_all(&build).unwrap();
        fs::write(build.join("Game (USA).state"), b"x").unwrap();
        assert_eq!(saves(&rom, std::slice::from_ref(&build)).len(), 3);
        assert_eq!(saves(&rom, &[]).len(), 2);

        // The basket moves: paths inside it follow, others stay.
        let (old, new) = (t.path().to_path_buf(), Path::new("/elsewhere/FruitBasket"));
        p.rebase(&old, new);
        assert_eq!(p.secs(&new.join("Game (USA).gba")), 90);
        assert_eq!(rebase(Path::new("/other/x.gba"), &old, new), Path::new("/other/x.gba"));
    }

    #[test]
    fn saves_live_in_data_for_fruits_that_take_it() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let mut pom = fruit("pomegranate", &[".iso"]);
        assert!(save_dirs(&b, &pom).is_empty(), "not installed, no data folder");
        pom.launch = vec!["play".into(), "{rom}".into(), "--data".into(), "{data}".into()];
        let states = b.data_dir("pomegranate").join("states");
        fs::create_dir_all(&states).unwrap();
        fs::write(states.join("Game.s0.state"), b"x").unwrap();
        assert_eq!(saves(&t.path().join("games/Game.iso"), &save_dirs(&b, &pom)).len(), 1);
    }

    #[test]
    fn slots_from_either_numbering_with_their_files() {
        let t = tempfile::tempdir().unwrap();
        let games = t.path().join("games");
        let states = t.path().join("data/states");
        fs::create_dir_all(&games).unwrap();
        fs::create_dir_all(&states).unwrap();
        // Strawberry: 1-8 beside the ROM.
        let rom = games.join("Golden Sun.gba");
        fs::write(&rom, b"x").unwrap();
        fs::write(games.join("Golden Sun.s1.state"), b"x").unwrap();
        fs::write(games.join("golden sun.S8.state"), b"x").unwrap();
        fs::write(games.join("Golden Sun.sav"), b"x").unwrap();
        let got: Vec<u8> = { let mut v: Vec<u8> = slots(&rom, &[]).iter().map(|s| s.n).collect(); v.sort(); v };
        assert_eq!(got, vec![1, 8]);

        // Pomegranate: 0-9 in data/states, with a picture, notes and a pre-slots state for 0.
        let iso = games.join("Game.iso");
        fs::write(&iso, b"x").unwrap();
        for n in [0, 9] {
            fs::write(states.join(format!("Game.s{n}.state")), b"x").unwrap();
            fs::write(states.join(format!("Game.s{n}.png")), b"x").unwrap();
        }
        fs::write(states.join("Game.s0.toml"), "frame = 3
saved_at = 2000000000
").unwrap();
        fs::write(states.join("Game.s9.toml"), "saved_at = 1000000000
").unwrap();
        fs::write(states.join("Game.state"), b"old").unwrap();
        let dirs = vec![t.path().join("data"), states.clone()];
        let s = slots(&iso, &dirs);
        assert_eq!(s.iter().map(|s| s.n).collect::<Vec<_>>(), vec![0, 9], "newest first, by saved_at");
        assert_eq!(s[0].saved, SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000));
        assert_eq!(s[0].extra.len(), 3, "png, toml and the pre-slots state");
        assert_eq!(s[1].extra.len(), 2);

        delete_slot(&s[0]).unwrap();
        assert!(!states.join("Game.s0.state").exists() && !states.join("Game.s0.png").exists());
        assert!(!states.join("Game.state").exists(), "or Pomegranate would copy it back into slot 0");
        assert_eq!(slots(&iso, &dirs).len(), 1);
    }
}
