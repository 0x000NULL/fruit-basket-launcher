//! Watching the game folders without a watcher: a hash of every file's
//! path, size and modified time, taken on a thread every few seconds. When
//! it changes, the Library rescans.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::library;

/// The folders' state as one number. A missing folder counts as empty.
pub fn signature(dirs: &[PathBuf]) -> u64 {
    let mut files: Vec<_> = dirs.iter().flat_map(|d| library::files(d, library::DEPTH)).collect();
    files.sort();
    let mut h = DefaultHasher::new();
    for f in &files {
        f.hash(&mut h);
        if let Ok(m) = std::fs::metadata(f) {
            m.len().hash(&mut h);
            m.modified().ok().hash(&mut h);
        }
    }
    h.finish()
}

/// `signature` on a thread.
pub fn start(dirs: Vec<PathBuf>) -> Receiver<u64> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(signature(&dirs));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_with_the_files_and_only_then() {
        let tmp = tempfile::tempdir().unwrap();
        let games = tmp.path().join("games");
        std::fs::create_dir_all(games.join("sub")).unwrap();
        std::fs::write(games.join("a.gba"), "x").unwrap();
        let dirs = vec![games.clone(), tmp.path().join("missing")];
        let first = signature(&dirs);
        assert_eq!(signature(&dirs), first, "nothing changed");
        std::fs::write(games.join("sub").join("b.gba"), "y").unwrap();
        let added = signature(&dirs);
        assert_ne!(added, first, "a new file");
        std::fs::write(games.join("a.gba"), "longer").unwrap();
        let grown = signature(&dirs);
        assert_ne!(grown, added, "a changed file");
        std::fs::remove_file(games.join("sub").join("b.gba")).unwrap();
        assert_ne!(signature(&dirs), grown, "a removed file");
    }
}
