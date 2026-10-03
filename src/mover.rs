//! Moving the basket to another folder (Settings → Move basket…). A rename
//! when the new place is on the same volume; otherwise a copy, checked file
//! for file and byte for byte, and only then is the old folder deleted. A
//! failure deletes the partial copy and leaves the old basket as it was.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::basket::copy_tree;

pub enum Progress {
    /// Percent copied (a rename jumps straight to Done).
    Pct(u8),
    Done(Result<(), String>),
}

/// Where the basket goes when the player picks `picked`: the folder itself
/// if it is empty or already called FruitBasket, else a FruitBasket folder
/// inside it. `Err` says why that won't do.
pub fn destination(root: &Path, picked: &Path) -> Result<PathBuf, String> {
    let empty = fs::read_dir(picked).map(|mut d| d.next().is_none()).unwrap_or(false);
    let dest = if empty || picked.file_name().is_some_and(|n| n == "FruitBasket") { picked.to_path_buf() } else { picked.join("FruitBasket") };
    if dest.starts_with(root) || root.starts_with(&dest) {
        return Err("that folder is the basket, or inside it".into());
    }
    if fs::read_dir(&dest).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!("{} isn't empty", dest.display()));
    }
    Ok(dest)
}

/// Move `from` to `to` on a thread.
pub fn start(from: PathBuf, to: PathBuf) -> Receiver<Progress> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = run(&from, &to, &mut |pct| {
            let _ = tx.send(Progress::Pct(pct));
        });
        let _ = tx.send(Progress::Done(result.map_err(|e| e.to_string())));
    });
    rx
}

fn run(from: &Path, to: &Path, progress: &mut dyn FnMut(u8)) -> io::Result<()> {
    // A copy into itself would never end.
    if to.starts_with(from) || from.starts_with(to) {
        return Err(io::Error::other("the new place is inside the basket"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // An empty folder picked as the destination is in the rename's way.
    if to.is_dir() {
        fs::remove_dir(to)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let want = tally(from)?;
    let mut done = 0u64;
    let mut last = 0u8;
    let copied = copy_tree(from, to, &mut |n| {
        done += n;
        let pct = (done * 100 / want.1.max(1)).min(99) as u8;
        if pct != last {
            last = pct;
            progress(pct);
        }
    });
    let checked = copied.and_then(|_| {
        let got = tally(to)?;
        if got == want {
            Ok(())
        } else {
            Err(io::Error::other(format!("copied {} files ({} bytes) of {} ({} bytes)", got.0, got.1, want.0, want.1)))
        }
    });
    if let Err(e) = checked {
        let _ = fs::remove_dir_all(to);
        return Err(e);
    }
    // The copy is whole; a leftover old folder is only wasted space.
    if let Err(e) = fs::remove_dir_all(from) {
        eprintln!("fruitbasket: removing the old basket at {}: {e}", from.display());
    }
    progress(100);
    Ok(())
}

/// (files, bytes) under `dir`.
fn tally(dir: &Path) -> io::Result<(u64, u64)> {
    let mut out = (0, 0);
    for e in fs::read_dir(dir)? {
        let e = e?;
        let t = e.file_type()?;
        if t.is_dir() {
            let (n, b) = tally(&e.path())?;
            out = (out.0 + n, out.1 + b);
        } else {
            out = (out.0 + 1, out.1 + e.metadata()?.len());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock(root: &Path) {
        fs::create_dir_all(root.join("strawberry/games")).unwrap();
        fs::write(root.join("strawberry/games/a.gba"), b"rom").unwrap();
        fs::write(root.join("strawberry/current"), b"v1\tstable\n").unwrap();
    }

    #[test]
    fn picks_a_fruitbasket_folder_and_refuses_bad_ones() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("FruitBasket");
        stock(&root);
        let other = t.path().join("D");
        fs::create_dir_all(other.join("stuff")).unwrap();
        assert_eq!(destination(&root, &other).unwrap(), other.join("FruitBasket"));
        let empty = t.path().join("Empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(destination(&root, &empty).unwrap(), empty);
        assert!(destination(&root, &root.join("strawberry")).is_err(), "inside the basket");
        assert!(destination(&root, t.path()).is_err(), "the basket itself");
        fs::create_dir_all(other.join("FruitBasket/x")).unwrap();
        assert!(destination(&root, &other).is_err(), "not empty");
    }

    #[test]
    fn moves_and_copies() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("A/FruitBasket");
        stock(&root);
        let to = t.path().join("B/FruitBasket");
        run(&root, &to, &mut |_| {}).unwrap();
        assert!(to.join("strawberry/games/a.gba").is_file());
        assert!(!root.exists());

        // The copy path, as across volumes: same result, checked.
        let back = t.path().join("C/FruitBasket");
        copy_tree(&to, &back, &mut |_| {}).unwrap();
        assert_eq!(tally(&back).unwrap(), tally(&to).unwrap());
    }

    #[test]
    fn a_failed_move_leaves_the_basket() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("FruitBasket");
        stock(&root);
        // A file where the destination's parent should be: nothing can go there.
        fs::write(t.path().join("blocked"), b"x").unwrap();
        assert!(run(&root, &t.path().join("blocked/FruitBasket"), &mut |_| {}).is_err());
        assert!(run(&root, &root.join("inner"), &mut |_| {}).is_err(), "not into itself");
        assert!(root.join("strawberry/games/a.gba").is_file());
    }
}
