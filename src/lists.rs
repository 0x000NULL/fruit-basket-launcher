//! The per-fruit lists the feed points at, `compat.txt` and `dumps.txt`,
//! cached in `launcher/lists/`. A list is used only if its SHA-256 matches
//! the signed feed, so the lists are as trustworthy as the feed itself.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use sha2::{Digest, Sha256};

use crate::basket::write_atomic;
use crate::feed::{Feed, FileRef};

/// Bigger than any list the site writes.
const LIMIT: u64 = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Compat,
    Dumps,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Compat => "compat",
            Kind::Dumps => "dumps",
        }
    }
}

pub fn dir(launcher_dir: &Path) -> PathBuf {
    launcher_dir.join("lists")
}

fn path(launcher_dir: &Path, fruit: &str, kind: Kind) -> PathBuf {
    dir(launcher_dir).join(format!("{fruit}.{}.txt", kind.name()))
}

fn sha256(bytes: &[u8]) -> String {
    crate::basket::hex(&Sha256::digest(bytes))
}

/// The cached list, if it matches the feed.
pub fn load(launcher_dir: &Path, fruit: &str, kind: Kind, want: &FileRef) -> Option<String> {
    let bytes = std::fs::read(path(launcher_dir, fruit, kind)).ok()?;
    sha256(&bytes).eq_ignore_ascii_case(&want.sha256).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Download every list the cache lacks or has stale, on a thread. The
/// receiver gets one message per list saved and closes when all are done.
pub fn fetch_missing(launcher_dir: &Path, feed: &Feed) -> Receiver<(String, Kind)> {
    let wanted: Vec<(String, Kind, FileRef)> = feed
        .fruits
        .iter()
        .flat_map(|f| {
            [(Kind::Compat, &f.compat), (Kind::Dumps, &f.dumps)]
                .into_iter()
                .filter_map(|(k, r)| r.clone().map(|r| (f.id.clone(), k, r)))
        })
        .filter(|(id, k, r)| load(launcher_dir, id, *k, r).is_none())
        .collect();
    let dir = launcher_dir.to_path_buf();
    let (tx, rx) = channel();
    thread::spawn(move || {
        for (id, kind, want) in wanted {
            match fetch(&want) {
                Ok(bytes) => {
                    if let Err(e) = write_atomic(&path(&dir, &id, kind), &bytes) {
                        eprintln!("fruitbasket: saving {id} {}: {e}", kind.name());
                    } else if tx.send((id, kind)).is_err() {
                        return;
                    }
                }
                Err(e) => eprintln!("fruitbasket: {id} {}: {e}", kind.name()),
            }
        }
    });
    rx
}

fn fetch(want: &FileRef) -> Result<Vec<u8>, String> {
    let resp = ureq::get(&want.url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_body().into_reader().take(LIMIT).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let got = sha256(&bytes);
    if !got.eq_ignore_ascii_case(&want.sha256) {
        return Err(format!("{} hashed to {got}, not the signed {}", want.url, want.sha256));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_list_must_match_the_feed() {
        let t = tempfile::tempdir().unwrap();
        let text = b"title\tserial\tstatus\tnotes\nGame\tAAAA\tin-game\tok\n";
        write_atomic(&path(t.path(), "berry", Kind::Compat), text).unwrap();
        let good = FileRef { url: String::new(), sha256: sha256(text) };
        let bad = FileRef { url: String::new(), sha256: "00".repeat(32) };
        assert!(load(t.path(), "berry", Kind::Compat, &good).is_some());
        assert!(load(t.path(), "berry", Kind::Compat, &bad).is_none());
        assert!(load(t.path(), "berry", Kind::Dumps, &good).is_none());
    }
}
