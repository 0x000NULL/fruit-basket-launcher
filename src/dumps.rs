//! Dump checks: is this file a known good dump? A fruit's `dumps.txt`
//! (`sha1<TAB>serial<TAB>title`, from No-Intro or Redump) is compared with
//! the SHA-1 of each game. Hashing runs on one background thread and is
//! cached in `launcher/hashes.tsv` by path, size and mtime, so a file is
//! read once. A CHD is not hashed: its header carries the SHA-1 of the raw
//! data, which is the ISO's SHA-1 the lists hold.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use sha1::{Digest, Sha1};

use crate::basket::write_atomic;

#[derive(Debug, Default, Clone)]
pub struct DumpDb {
    /// sha1 → (serial, title).
    known: HashMap<String, (String, String)>,
}

impl DumpDb {
    pub fn parse(text: &str) -> DumpDb {
        let known = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut f = l.split('\t');
                let sha1 = f.next()?.trim().to_lowercase();
                (sha1.len() == 40).then(|| (sha1, (f.next().unwrap_or("").to_string(), f.next().unwrap_or("").to_string())))
            })
            .collect();
        DumpDb { known }
    }

    /// The serial and title a hash is listed under.
    pub fn get(&self, sha1: &str) -> Option<&(String, String)> {
        self.known.get(&sha1.to_lowercase())
    }
}

/// The SHA-1 a dump list would hold for `path`.
pub fn sha1_of(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("chd")) {
        return chd_raw_sha1(&mut f);
    }
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// The raw-data SHA-1 from a v4 or v5 CHD header (big-endian; offset 64
/// in v5, 88 in v4).
fn chd_raw_sha1(f: &mut File) -> io::Result<String> {
    let mut head = [0u8; 124];
    f.seek(SeekFrom::Start(0))?;
    let n = f.read(&mut head)?;
    if n < 108 || &head[..8] != b"MComprHD" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a CHD"));
    }
    let version = u32::from_be_bytes(head[12..16].try_into().unwrap());
    let at = match version {
        5 => 64,
        4 => 88,
        v => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("CHD version {v}"))),
    };
    Ok(hex(&head[at..at + 20]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hashes already computed, keyed by path with the size and mtime they
/// were computed at.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hashes {
    map: HashMap<PathBuf, (u64, i64, String)>,
}

impl Hashes {
    fn path(launcher_dir: &Path) -> PathBuf {
        launcher_dir.join("hashes.tsv")
    }

    /// `<sha1>\t<size>\t<mtime>\t<path>` per line.
    pub fn load(launcher_dir: &Path) -> Hashes {
        let text = fs::read_to_string(Hashes::path(launcher_dir)).unwrap_or_default();
        let map = text
            .lines()
            .filter_map(|l| {
                let mut f = l.splitn(4, '\t');
                let sha1 = f.next()?.to_string();
                let size = f.next()?.parse().ok()?;
                let mtime = f.next()?.parse().ok()?;
                Some((PathBuf::from(f.next()?), (size, mtime, sha1)))
            })
            .collect();
        Hashes { map }
    }

    pub fn save(&self, launcher_dir: &Path) -> io::Result<()> {
        let mut rows: Vec<_> = self.map.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let text: String = rows.iter().map(|(p, (s, m, h))| format!("{h}\t{s}\t{m}\t{}\n", p.display())).collect();
        write_atomic(&Hashes::path(launcher_dir), text.as_bytes())
    }

    /// The hash, if it was computed for this exact size and mtime.
    pub fn get(&self, path: &Path, size: u64, mtime: i64) -> Option<&str> {
        self.map.get(path).filter(|(s, m, _)| *s == size && *m == mtime).map(|(_, _, h)| h.as_str())
    }

    pub fn insert(&mut self, path: PathBuf, size: u64, mtime: i64, sha1: String) {
        self.map.insert(path, (size, mtime, sha1));
    }
}

/// One file to hash.
pub type Want = (PathBuf, u64, i64);

/// Hash `files` one after another on a thread; each result arrives as it
/// is done. Files that can't be read are skipped.
pub fn hash_in_background(files: Vec<Want>) -> Receiver<(Want, String)> {
    let (tx, rx) = channel();
    thread::Builder::new()
        .name("hashing".into())
        .spawn(move || {
            for want in files {
                match sha1_of(&want.0) {
                    Ok(h) => {
                        if tx.send((want, h)).is_err() {
                            return;
                        }
                    }
                    Err(e) => eprintln!("fruitbasket: hashing {}: {e}", want.0.display()),
                }
            }
        })
        .expect("spawn hashing thread");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_files_and_reads_chd_headers() {
        let t = tempfile::tempdir().unwrap();
        let rom = t.path().join("a.gba");
        fs::write(&rom, b"abc").unwrap();
        assert_eq!(sha1_of(&rom).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");

        let mut chd = vec![0u8; 124];
        chd[..8].copy_from_slice(b"MComprHD");
        chd[12..16].copy_from_slice(&5u32.to_be_bytes());
        for (i, b) in chd[64..84].iter_mut().enumerate() {
            *b = i as u8;
        }
        let path = t.path().join("disc.chd");
        fs::write(&path, &chd).unwrap();
        assert_eq!(sha1_of(&path).unwrap(), "000102030405060708090a0b0c0d0e0f10111213");

        let db = DumpDb::parse("# comment\na9993e364706816aba3e25717850c26c9cd0d89d\tAAAE\tGame (USA)\nshort\tx\ty\n");
        assert_eq!(db.get("A9993E364706816ABA3E25717850C26C9CD0D89D").map(|x| x.0.as_str()), Some("AAAE"));
        assert!(db.get("short").is_none());
    }

    #[test]
    fn cache_round_trip_and_staleness() {
        let t = tempfile::tempdir().unwrap();
        let mut h = Hashes::default();
        h.insert(PathBuf::from("/g/a.gba"), 3, 100, "ab".repeat(20));
        h.save(t.path()).unwrap();
        let back = Hashes::load(t.path());
        assert_eq!(back, h);
        assert!(back.get(Path::new("/g/a.gba"), 3, 100).is_some());
        assert!(back.get(Path::new("/g/a.gba"), 3, 101).is_none(), "touched since");

        let rx = hash_in_background(vec![(t.path().join("hashes.tsv"), 0, 0), (t.path().join("missing"), 0, 0)]);
        assert_eq!(rx.into_iter().count(), 1);
    }
}
