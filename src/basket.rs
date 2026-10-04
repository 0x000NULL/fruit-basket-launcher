//! The basket on disk.
//!
//! ```text
//! <root>/                      ~/FruitBasket by default
//!   <fruit>/builds/<build>/    one extracted build per folder
//!   <fruit>/current            "<build>\t<channel>\n", the build that runs
//!   <fruit>/games/             always scanned; saves sit beside the games
//!   <fruit>/data/              saves and settings, for fruits that take `{data}`
//!   launcher/                  feed cache, history, downloads, library state
//! ```
//!
//! Every change is made so that a failure part-way leaves the old build
//! running: a build is extracted to `<build>.tmp` and renamed into place,
//! and `current` is only rewritten (atomically) once the build is there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::feed::Channel;

#[derive(Debug, Clone)]
pub struct Basket {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub build: String,
    pub channel: Channel,
}

/// What `uninstall` should do with the games folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Games {
    Keep,
    Delete,
}

impl Basket {
    pub fn new(root: impl Into<PathBuf>) -> Basket {
        Basket { root: root.into() }
    }

    /// `~/FruitBasket`.
    pub fn default_root() -> PathBuf {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("FruitBasket")
    }

    pub fn fruit_dir(&self, fruit: &str) -> PathBuf {
        self.root.join(fruit)
    }

    pub fn builds_dir(&self, fruit: &str) -> PathBuf {
        self.fruit_dir(fruit).join("builds")
    }

    pub fn build_dir(&self, fruit: &str, build: &str) -> PathBuf {
        self.builds_dir(fruit).join(safe_name(build))
    }

    pub fn games_dir(&self, fruit: &str) -> PathBuf {
        self.fruit_dir(fruit).join("games")
    }

    /// Where a fruit that takes `{data}` keeps its saves and settings.
    pub fn data_dir(&self, fruit: &str) -> PathBuf {
        self.fruit_dir(fruit).join("data")
    }

    pub fn launcher_dir(&self) -> PathBuf {
        self.root.join("launcher")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.launcher_dir().join("downloads")
    }

    pub fn current(&self, fruit: &str) -> Option<Current> {
        let text = fs::read_to_string(self.fruit_dir(fruit).join("current")).ok()?;
        let (build, channel) = text.trim().split_once('\t')?;
        let current = Current { build: build.to_string(), channel: Channel::parse(channel)? };
        self.build_dir(fruit, &current.build).is_dir().then_some(current)
    }

    fn set_current(&self, fruit: &str, current: &Current) -> io::Result<()> {
        let line = format!("{}\t{}\n", current.build, current.channel.name());
        write_atomic(&self.fruit_dir(fruit).join("current"), line.as_bytes())
    }

    /// Builds on disk, newest first (by when they were installed).
    pub fn builds(&self, fruit: &str) -> Vec<String> {
        let mut found: Vec<(std::time::SystemTime, String)> = fs::read_dir(self.builds_dir(fruit))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                if name.ends_with(".tmp") {
                    return None;
                }
                // The folder's own mtime moves whenever carried files move
                // in or out, so order by the marker written at install.
                let when = fs::metadata(e.path().join(".installed")).and_then(|m| m.modified()).ok()?;
                Some((when, name))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        found.into_iter().map(|(_, n)| n).collect()
    }

    /// Install the archive at `archive` as `build` and make it current.
    /// `carry` names files the emulator keeps beside its exe (Pomegranate's
    /// memory cards); they move from the old current build to the new one.
    pub fn install(&self, fruit: &str, build: &str, channel: Channel, archive: &Path, carry: &[String]) -> io::Result<()> {
        let dest = self.build_dir(fruit, build);
        let tmp = dest.with_extension("tmp");
        if tmp.exists() {
            fs::remove_dir_all(&tmp)?;
        }
        fs::create_dir_all(&tmp)?;
        if let Err(e) = extract(archive, &tmp) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
        let tmp = flatten(&tmp)?;
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::rename(&tmp, &dest)?;
        // `builds()` orders by this marker's mtime.
        touch(&dest)?;
        self.switch(fruit, Current { build: build.to_string(), channel }, carry)?;
        fs::create_dir_all(self.games_dir(fruit))
    }

    /// Make an installed build current (a rollback, or a switch back).
    pub fn switch(&self, fruit: &str, to: Current, carry: &[String]) -> io::Result<()> {
        let new_dir = self.build_dir(fruit, &to.build);
        if !new_dir.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is not on disk", to.build)));
        }
        if let Some(old) = self.current(fruit) {
            if old.build != to.build {
                carry_over(&self.build_dir(fruit, &old.build), &new_dir, carry)?;
            }
        }
        self.set_current(fruit, &to)
    }

    /// Delete builds beyond the newest `keep` old ones; the current build
    /// never counts and is never deleted. Returns what was removed.
    pub fn prune(&self, fruit: &str, keep: usize) -> io::Result<Vec<String>> {
        let current = self.current(fruit).map(|c| c.build);
        let old: Vec<String> = self.builds(fruit).into_iter().filter(|b| Some(b) != current.as_ref()).collect();
        let mut removed = Vec::new();
        for build in old.into_iter().skip(keep) {
            fs::remove_dir_all(self.build_dir(fruit, &build))?;
            removed.push(build);
        }
        Ok(removed)
    }

    /// Move `names` from the builds into the data folder, once: from the
    /// current build if it has them, else the newest build that does. A
    /// name already in `data/` is left alone, so this never overwrites and
    /// running it again does nothing. Returns what moved.
    pub fn migrate_data(&self, fruit: &str, names: &[String]) -> io::Result<Vec<String>> {
        let data = self.data_dir(fruit);
        let mut builds = self.builds(fruit);
        if let Some(c) = self.current(fruit) {
            builds.retain(|b| *b != c.build);
            builds.insert(0, c.build);
        }
        let mut moved = Vec::new();
        for name in names.iter().filter(|n| plain_name(n)) {
            let dst = data.join(name);
            if dst.exists() {
                continue;
            }
            let Some(src) = builds.iter().map(|b| self.build_dir(fruit, b).join(name)).find(|p| p.exists()) else { continue };
            fs::create_dir_all(&data)?;
            move_path(&src, &dst)?;
            moved.push(name.clone());
        }
        Ok(moved)
    }

    /// Remove the program. Games and saves stay unless asked; asked, the
    /// data folder goes too.
    pub fn uninstall(&self, fruit: &str, games: Games) -> io::Result<()> {
        let dir = self.fruit_dir(fruit);
        let _ = fs::remove_file(dir.join("current"));
        if self.builds_dir(fruit).exists() {
            fs::remove_dir_all(self.builds_dir(fruit))?;
        }
        if games == Games::Delete {
            for d in [self.games_dir(fruit), self.data_dir(fruit)] {
                if d.exists() {
                    fs::remove_dir_all(d)?;
                }
            }
        }
        Ok(())
    }

    /// The emulator's executable inside the current build.
    pub fn exe(&self, fruit: &str, bin: &str) -> Option<PathBuf> {
        let current = self.current(fruit)?;
        find_exe(&self.build_dir(fruit, &current.build), bin)
    }
}

/// Lowercase hex, as the feed writes SHA-256 and SHA-1 sums.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write `path` through a temporary file and a rename, so a reader sees the
/// old contents or the new, never half of either.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

/// A build ID made safe as one path component.
fn safe_name(build: &str) -> String {
    build
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect::<String>()
        .trim_start_matches('.')
        .to_string()
}

fn touch(dir: &Path) -> io::Result<()> {
    write_atomic(&dir.join(".installed"), b"")
}

/// Archives that wrap everything in one top folder get it lifted out, so
/// the exe is at the top of the build folder either way.
fn flatten(dir: &Path) -> io::Result<PathBuf> {
    let entries: Vec<_> = fs::read_dir(dir)?.flatten().collect();
    if entries.len() == 1 && entries[0].path().is_dir() {
        let inner = entries[0].path();
        let lifted = dir.with_extension("lift");
        if lifted.exists() {
            fs::remove_dir_all(&lifted)?;
        }
        fs::rename(&inner, &lifted)?;
        fs::remove_dir(dir)?;
        fs::rename(&lifted, dir)?;
    }
    Ok(dir.to_path_buf())
}

/// One path component: no separators, no `..`.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !name.contains('/') && !name.contains('\\')
}

/// Rename, or copy and delete when `from` and `to` are on different volumes.
/// A failed copy is cleaned up and `from` is left as it was.
pub fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = copy_tree(from, to, &mut |_| {}) {
        let _ = if to.is_dir() { fs::remove_dir_all(to) } else { fs::remove_file(to) };
        return Err(e);
    }
    if from.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) }
}

/// Copy a file or a folder and everything in it; `copied` hears each
/// file's size as it lands.
pub fn copy_tree(from: &Path, to: &Path, copied: &mut dyn FnMut(u64)) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for e in fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()), copied)?;
        }
        Ok(())
    } else {
        copied(fs::copy(from, to)?);
        Ok(())
    }
}

fn carry_over(from: &Path, to: &Path, names: &[String]) -> io::Result<()> {
    for name in names {
        if !plain_name(name) {
            continue;
        }
        let src = from.join(name);
        let dst = to.join(name);
        if src.exists() {
            if dst.exists() {
                // The new build shipped its own (empty) copy; the player's wins.
                if dst.is_dir() { fs::remove_dir_all(&dst)? } else { fs::remove_file(&dst)? }
            }
            fs::rename(&src, &dst)?;
        }
    }
    Ok(())
}

pub(crate) fn find_exe(dir: &Path, bin: &str) -> Option<PathBuf> {
    let want = if cfg!(windows) { format!("{bin}.exe") } else { bin.to_string() };
    let direct = dir.join(&want);
    if direct.is_file() {
        return Some(direct);
    }
    // One level down covers archives with a bin/ folder.
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).find_map(|d| {
        let p = d.join(&want);
        p.is_file().then_some(p)
    })
}

/// Extract a `.zip` or `.tar.gz` into `dest`. Entries that would land
/// outside `dest` are refused.
pub fn extract(archive: &Path, dest: &Path) -> io::Result<()> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = name.trim_end_matches(".part");
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(io::Error::other)?;
            let Some(rel) = entry.enclosed_name() else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "archive path escapes the build folder"));
            };
            let out = dest.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            io::copy(&mut entry, &mut fs::File::create(&out)?)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755))?;
            }
        }
        Ok(())
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(fs::File::open(archive)?);
        let mut tar = tar::Archive::new(gz);
        for entry in tar.entries()? {
            // unpack_in refuses paths outside dest and returns false for them.
            if !entry?.unpack_in(dest)? {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "archive path escapes the build folder"));
            }
        }
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a build archive: {name}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn exe(name: &str) -> String {
        if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
    }

    #[test]
    fn hex_matches_the_feeds_sums() {
        use sha2::{Digest, Sha256};
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn install_switch_prune_uninstall() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path().join("FruitBasket"));
        let a1 = t.path().join("a1.zip");
        let a2 = t.path().join("a2.zip");
        let a3 = t.path().join("a3.zip");
        zip_with(&a1, &[(&exe("berry"), b"one"), ("cards/Memory card 1.ps2", b"save")]);
        zip_with(&a2, &[(&format!("berry-v2/{}", exe("berry")), b"two")]);
        zip_with(&a3, &[(&exe("berry"), b"three")]);
        let carry = vec!["cards".to_string()];

        b.install("berry", "v1", Channel::Stable, &a1, &carry).unwrap();
        assert_eq!(b.current("berry").unwrap().build, "v1");
        assert!(b.games_dir("berry").is_dir());

        std::thread::sleep(std::time::Duration::from_millis(20));
        b.install("berry", "v2", Channel::Nightly, &a2, &carry).unwrap();
        let cur = b.current("berry").unwrap();
        assert_eq!((cur.build.as_str(), cur.channel), ("v2", Channel::Nightly));
        // The single top folder was lifted out, and the save card moved over.
        assert_eq!(fs::read(b.exe("berry", "berry").unwrap()).unwrap(), b"two");
        assert!(b.build_dir("berry", "v2").join("cards/Memory card 1.ps2").is_file());
        assert!(!b.build_dir("berry", "v1").join("cards").exists());

        std::thread::sleep(std::time::Duration::from_millis(20));
        b.install("berry", "v3", Channel::Stable, &a3, &carry).unwrap();
        assert_eq!(b.builds("berry"), vec!["v3", "v2", "v1"]);

        // Roll back to v2: the card follows again.
        b.switch("berry", Current { build: "v2".into(), channel: Channel::Nightly }, &carry).unwrap();
        assert!(b.build_dir("berry", "v2").join("cards/Memory card 1.ps2").is_file());

        // Keep one old build besides the current (v2): v3 is newest, v1 goes.
        assert_eq!(b.prune("berry", 1).unwrap(), vec!["v1"]);
        assert_eq!(b.builds("berry"), vec!["v3", "v2"]);

        fs::write(b.games_dir("berry").join("game.gba"), b"rom").unwrap();
        b.uninstall("berry", Games::Keep).unwrap();
        assert!(b.current("berry").is_none());
        assert!(b.games_dir("berry").join("game.gba").is_file());
        b.uninstall("berry", Games::Delete).unwrap();
        assert!(!b.games_dir("berry").exists());
    }

    #[test]
    fn data_migrates_once_and_never_overwrites() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let a1 = t.path().join("a1.zip");
        let a2 = t.path().join("a2.zip");
        zip_with(&a1, &[(&exe("pom"), b"one"), ("cards/card.ps2", b"old card"), ("ps2emu.toml", b"old")]);
        zip_with(&a2, &[(&exe("pom"), b"two"), ("cards/card.ps2", b"new card")]);
        b.install("pom", "v1", Channel::Stable, &a1, &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        b.install("pom", "v2", Channel::Stable, &a2, &[]).unwrap();
        let names: Vec<String> = ["ps2emu.toml", "cards", "states", "../x"].map(String::from).to_vec();

        // The current build's cards win; the toml only v1 has comes from v1.
        assert_eq!(b.migrate_data("pom", &names).unwrap(), vec!["ps2emu.toml", "cards"]);
        assert_eq!(fs::read(b.data_dir("pom").join("cards/card.ps2")).unwrap(), b"new card");
        assert_eq!(fs::read(b.data_dir("pom").join("ps2emu.toml")).unwrap(), b"old");
        assert!(!b.build_dir("pom", "v2").join("cards").exists());

        // Again: nothing moves, and v1's leftover card doesn't replace it.
        assert!(b.migrate_data("pom", &names).unwrap().is_empty());
        assert_eq!(fs::read(b.data_dir("pom").join("cards/card.ps2")).unwrap(), b"new card");
        assert!(b.build_dir("pom", "v1").join("cards/card.ps2").is_file());

        b.uninstall("pom", Games::Keep).unwrap();
        assert!(b.data_dir("pom").is_dir(), "saves stay");
        b.uninstall("pom", Games::Delete).unwrap();
        assert!(!b.data_dir("pom").exists());
    }

    #[test]
    fn bad_archive_leaves_current_build() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let good = t.path().join("good.zip");
        zip_with(&good, &[(&exe("berry"), b"one")]);
        b.install("berry", "v1", Channel::Stable, &good, &[]).unwrap();
        let bad = t.path().join("bad.zip");
        fs::write(&bad, b"not a zip").unwrap();
        assert!(b.install("berry", "v2", Channel::Stable, &bad, &[]).is_err());
        assert_eq!(b.current("berry").unwrap().build, "v1");
        assert!(!b.build_dir("berry", "v2").exists());
        assert!(!b.builds_dir("berry").join("v2.tmp").exists());
    }

    #[test]
    fn tar_gz_extracts() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("b.tar.gz");
        let gz = flate2::write::GzEncoder::new(fs::File::create(&path).unwrap(), flate2::Compression::fast());
        let mut tb = tar::Builder::new(gz);
        let mut h = tar::Header::new_gnu();
        h.set_size(3);
        h.set_mode(0o755);
        h.set_cksum();
        tb.append_data(&mut h, "pkg/berry", &b"abc"[..]).unwrap();
        tb.into_inner().unwrap().finish().unwrap();
        let out = t.path().join("out");
        fs::create_dir(&out).unwrap();
        extract(&path, &out).unwrap();
        assert_eq!(fs::read(out.join("pkg/berry")).unwrap(), b"abc");
    }

    #[test]
    fn build_names_are_one_component() {
        assert_eq!(safe_name("v1.3.1"), "v1.3.1");
        assert_eq!(safe_name("../evil"), "_evil");
        assert_eq!(safe_name("a/b"), "a_b");
    }
}
