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
const DEPTH: u32 = 3;

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
/// in an extra folder goes to the first fruit that reads its extension.
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
            if f.reads(&path) {
                add(path, f, &mut out);
            }
        }
    }
    for folder in extra {
        for path in files(folder, DEPTH) {
            if let Some(f) = fruits.iter().find(|f| f.reads(&path)) {
                add(path, f, &mut out);
            }
        }
    }
    out
}

fn files(dir: &Path, depth: u32) -> Vec<PathBuf> {
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

/// Save files the fruit keeps for this game: slot states named after it,
/// beside the game or in the build folder (Pomegranate's `<stem>.state`).
pub fn saves(game: &Path, build_dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(stem) = game.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = game.parent().map(Path::to_path_buf).into_iter().collect();
    if let Some(b) = build_dir {
        dirs.push(b.to_path_buf());
        dirs.push(b.join("states"));
    }
    let mut out = Vec::new();
    for d in dirs {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with(&format!("{stem}.")) && name.ends_with(".state") {
                out.push(e.path());
            }
        }
    }
    out
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
    pub fn record(&mut self, path: &Path, started: SystemTime, secs: u64) {
        let at = started.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let e = self.map.entry(path.to_path_buf()).or_insert((0, 0));
        e.0 = e.0.max(at);
        e.1 += secs;
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
            status: Status::Released,
            summary: String::new(),
            blurb: String::new(),
            bin: Some(id.into()),
            bios: None,
            dump_db: None,
            launch: vec!["{rom}".into()],
            load_slot: None,
            carry: vec![],
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
        let got = scan(&b, &[&berry], &[extra.clone()], &[games.join("sub/Hidden.gba")]);
        let mut titles: Vec<_> = got.iter().map(|g| g.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["Final Fantasy IV Advance", "The Legend of Zelda"]);
        let ff = got.iter().find(|g| g.title.starts_with("Final")).unwrap();
        assert_eq!(ff.code.as_deref(), Some("BZ4E"));
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
        assert_eq!(saves(&rom, Some(&build)).len(), 3);
    }
}
