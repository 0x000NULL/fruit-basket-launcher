//! The launcher updating itself. The signed feed's `launcher` entry names
//! the newest build; a newer one for this PC is downloaded, checked against
//! the feed's SHA-256, unpacked under `<launcher dir>/update/<build>/`, and
//! asked for its `--version`. Only then is it staged. The next start swaps
//! it in before the window opens: the running exe is renamed aside
//! (`fruitbasket.old`), the new one copied to its place, and started. Any
//! failure puts the old exe back and carries on with it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use crate::basket::{extract, find_exe, write_atomic};
use crate::feed::Asset;

pub const BIN: &str = "fruitbasket";

/// What the update thread reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Upd {
    /// Downloaded, verified and staged: it goes in at the next start.
    Staged(String),
    Failed(String),
}

/// `a` is a later version than `b` (`v1.10.0` > `v1.9.2`); text after a
/// `-` (a pre-release) sorts before the release itself.
pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

fn key(v: &str) -> (Vec<u64>, bool) {
    let v = v.trim().trim_start_matches('v');
    let (main, pre) = match v.split_once('-') {
        Some((m, _)) => (m, true),
        None => (v, false),
    };
    (main.split('.').map(|n| n.parse().unwrap_or(0)).collect(), !pre)
}

/// Where staged builds live.
/// Whether to offer `build`: newer than what runs, and not the one the
/// player said Later to (a recheck would otherwise bring the banner back).
pub fn offer(build: &str, running: &str, dismissed: Option<&str>) -> bool {
    newer(build, running) && dismissed != Some(build)
}

pub fn dir(launcher_dir: &Path) -> PathBuf {
    launcher_dir.join("update")
}

/// The renamed-aside old exe beside `exe`.
fn old_path(exe: &Path) -> PathBuf {
    let ext = exe.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    exe.with_file_name(format!("{BIN}.old{ext}"))
}

/// The folder holding the exe can be written: the swap can happen. Not in
/// Program Files and the like, where the update has to come from the site.
pub fn can_replace(exe: &Path) -> bool {
    let Some(parent) = exe.parent() else { return false };
    let probe = parent.join(".fruitbasket-update-probe");
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Download and stage `build` on a thread.
pub fn start(build: String, asset: Asset, dir: PathBuf) -> Receiver<Upd> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let mut fetch = |url: &str, to: &Path, size: u64| crate::jobs::download(url, to, size, &mut |_| {});
        let r = match stage(&build, &asset, &dir, &mut fetch, &smoke_test) {
            Ok(_) => Upd::Staged(build),
            Err(e) => Upd::Failed(e),
        };
        let _ = tx.send(r);
    });
    rx
}

/// The staged exe must run and say it is `build`.
fn smoke_test(exe: &Path, build: &str) -> Result<(), String> {
    let out = Command::new(exe).arg("--version").output().map_err(|e| format!("the new launcher didn't start: {e}"))?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if said == build {
        Ok(())
    } else {
        Err(format!("the new launcher says it is {said:?}, not {build}"))
    }
}

type Fetch<'a> = dyn FnMut(&str, &Path, u64) -> io::Result<String> + 'a;
type Check<'a> = dyn Fn(&Path, &str) -> Result<(), String> + 'a;

/// Download, verify, unpack and check `build`; then record it in
/// `staged`. Returns the staged exe. Nothing is staged on any failure.
fn stage(build: &str, asset: &Asset, dir: &Path, fetch: &mut Fetch, check: &Check) -> Result<PathBuf, String> {
    let err = |what: &str, e: io::Error| format!("{what}: {e}");
    fs::create_dir_all(dir).map_err(|e| err("making the update folder", e))?;
    let safe: String = asset.name.chars().filter(|c| !matches!(c, '/' | '\\' | ':')).collect();
    let part = dir.join(format!("{safe}.part"));
    let got = fetch(&asset.url, &part, asset.size).map_err(|e| {
        let _ = fs::remove_file(&part);
        format!("couldn't fetch {}: {e}", asset.name)
    })?;
    if !got.eq_ignore_ascii_case(&asset.sha256) {
        let _ = fs::remove_file(&part);
        return Err(format!("{} hashed to {got}, not the signed {}", asset.name, asset.sha256));
    }
    let tmp = dir.join(format!("{build}.tmp"));
    let _ = fs::remove_dir_all(&tmp);
    let unpacked = extract(&part, &tmp).map_err(|e| err("unpacking", e));
    let _ = fs::remove_file(&part);
    unpacked?;
    let result = (|| {
        let exe = find_exe(&tmp, BIN).ok_or_else(|| format!("no {BIN} in {}", asset.name))?;
        check(&exe, build)?;
        let at = dir.join(build);
        let _ = fs::remove_dir_all(&at);
        fs::rename(&tmp, &at).map_err(|e| err("staging", e))?;
        let exe = find_exe(&at, BIN).ok_or_else(|| format!("no {BIN} in {}", asset.name))?;
        write_atomic(&dir.join("staged"), format!("{build}\t{}\n", exe.display()).as_bytes()).map_err(|e| err("staging", e))?;
        Ok(exe)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// The staged build, if one is waiting: (build, its exe).
pub fn staged(dir: &Path) -> Option<(String, PathBuf)> {
    let text = fs::read_to_string(dir.join("staged")).ok()?;
    let (build, exe) = text.trim_end().split_once('\t')?;
    Some((build.to_string(), PathBuf::from(exe)))
}

/// At start, before the window: swap in a staged build newer than
/// `current`. Returns the build swapped in; the caller starts `exe` again
/// and exits. On failure the old exe is back in place.
pub fn apply_staged(dir: &Path, exe: &Path, current: &str) -> Result<Option<String>, String> {
    let Some((build, new)) = staged(dir) else { return Ok(None) };
    let _ = fs::remove_file(dir.join("staged"));
    if !newer(&build, current) || !new.is_file() {
        return Ok(None);
    }
    let old = old_path(exe);
    let _ = fs::remove_file(&old);
    // Renaming a running exe is allowed on every OS; deleting it is not on Windows.
    fs::rename(exe, &old).map_err(|e| format!("moving the old launcher aside: {e}"))?;
    if let Err(e) = fs::copy(&new, exe) {
        let _ = fs::remove_file(exe);
        let _ = fs::rename(&old, exe);
        return Err(format!("putting the new launcher in place: {e}"));
    }
    Ok(Some(build))
}

/// Put the old exe back after the new one failed to start.
pub fn undo(exe: &Path) {
    let old = old_path(exe);
    if old.is_file() {
        let _ = fs::remove_file(exe);
        let _ = fs::rename(&old, exe);
    }
}

/// After a start with nothing to swap in: remove the old exe and staged
/// folders, best effort (the old process may still be exiting).
pub fn clean(dir: &Path, exe: &Path) {
    let _ = fs::remove_file(old_path(exe));
    if staged(dir).is_none() {
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("v1.0.1", "v1.0.0"));
        assert!(newer("v1.10.0", "v1.9.9"));
        assert!(newer("v1.0.0", "v0.5.1"));
        assert!(!newer("v1.0.0", "v1.0.0"));
        assert!(!newer("v0.5.1", "v1.0.0"));
        assert!(newer("v1.0.0", "v1.0.0-rc1"), "a release beats its pre-release");
    }

    /// Later holds for that build only: a recheck that finds it again stays
    /// quiet, and a newer one is offered.
    #[test]
    fn later_holds_for_that_build() {
        assert!(offer("v1.0.1", "v1.0.0", None));
        assert!(!offer("v1.0.1", "v1.0.0", Some("v1.0.1")));
        assert!(offer("v1.0.2", "v1.0.0", Some("v1.0.1")));
        assert!(!offer("v1.0.0", "v1.0.0", None));
    }

    fn zip_with(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file(name, opts).unwrap();
            z.write_all(body).unwrap();
            z.finish().unwrap();
        }
        out.into_inner()
    }

    fn sha(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        crate::basket::hex(&Sha256::digest(bytes))
    }

    #[test]
    fn stages_only_a_verified_build_that_runs() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("update");
        let exe_name = if cfg!(windows) { "fruitbasket.exe" } else { "fruitbasket" };
        let zip = zip_with(&format!("fruitbasket-v9.0.0-test/{exe_name}"), b"new launcher");
        let asset = Asset { name: "fruitbasket-v9.0.0-test.zip".into(), url: "x".into(), size: zip.len() as u64, sha256: sha(&zip) };
        let mut fetch = |_: &str, to: &Path, _: u64| {
            fs::write(to, &zip)?;
            Ok(sha(&zip))
        };

        // A build that doesn't say it is v9.0.0 is refused, and nothing is staged.
        let liar = |_: &Path, _: &str| Err("says v0".to_string());
        assert!(stage("v9.0.0", &asset, &dir, &mut fetch, &liar).is_err());
        assert!(staged(&dir).is_none());
        assert!(!dir.join("v9.0.0").exists() && !dir.join("v9.0.0.tmp").exists());

        // A wrong hash: the download is deleted.
        let bad = Asset { sha256: "0".repeat(64), ..asset.clone() };
        assert!(stage("v9.0.0", &bad, &dir, &mut fetch, &|_, _| Ok(())).is_err());
        assert!(fs::read_dir(&dir).unwrap().next().is_none(), "nothing left");

        let exe = stage("v9.0.0", &asset, &dir, &mut fetch, &|_, _| Ok(())).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new launcher");
        assert_eq!(staged(&dir), Some(("v9.0.0".to_string(), exe)));
    }

    #[test]
    fn swaps_in_a_newer_staged_build_and_cleans_up() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("launcher/update");
        let exe = t.path().join("app").join(if cfg!(windows) { "fruitbasket.exe" } else { "fruitbasket" });
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"old").unwrap();
        let new = dir.join("v2.0.0/fruitbasket");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&new, b"new").unwrap();

        // Staged but not newer: ignored, and the stale record goes.
        write_atomic(&dir.join("staged"), format!("v1.0.0\t{}\n", new.display()).as_bytes()).unwrap();
        assert_eq!(apply_staged(&dir, &exe, "v1.0.0").unwrap(), None);
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(staged(&dir).is_none());

        write_atomic(&dir.join("staged"), format!("v2.0.0\t{}\n", new.display()).as_bytes()).unwrap();
        assert_eq!(apply_staged(&dir, &exe, "v1.0.0").unwrap().as_deref(), Some("v2.0.0"));
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old", "kept until the next start");

        // The new one failed to start: the old one comes back.
        undo(&exe);
        assert_eq!(fs::read(&exe).unwrap(), b"old");

        clean(&dir, &exe);
        assert!(!old_path(&exe).exists() && !dir.exists());
    }

    #[test]
    fn a_folder_that_cannot_be_written_is_reported() {
        let t = tempfile::tempdir().unwrap();
        assert!(can_replace(&t.path().join("fruitbasket.exe")));
        assert!(!can_replace(&t.path().join("missing/fruitbasket.exe")));
    }
}
