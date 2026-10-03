//! Starting a game: the feed's launch template for the fruit (`{rom}`,
//! `play {rom} --data {data}`) with the game's path filled in, run from
//! the build folder. A thread waits for the emulator to exit and reports
//! how long the game ran, for play time.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Instant, SystemTime};

/// The template with `{rom}`, `{slot}` and `{data}` filled in. A
/// placeholder with nothing to fill it is an error, never passed on as
/// text: the emulator would take `{data}` for a folder name.
pub fn args(template: &[String], rom: Option<&Path>, slot: Option<u8>, data: Option<&Path>) -> Result<Vec<String>, String> {
    let fill = |a: &String| {
        let mut a = a.clone();
        let values = [
            ("{rom}", rom.map(|p| p.to_string_lossy().into_owned())),
            ("{slot}", slot.map(|n| n.to_string())),
            ("{data}", data.map(|p| p.to_string_lossy().into_owned())),
        ];
        for (key, value) in values {
            if a.contains(key) {
                let value = value.ok_or_else(|| format!("nothing to fill {key} with"))?;
                a = a.replace(key, &value);
            }
        }
        Ok(a)
    };
    template.iter().map(fill).collect()
}

/// A finished session.
#[derive(Debug, Clone)]
pub struct Session {
    pub game: PathBuf,
    pub started: SystemTime,
    pub secs: u64,
}

/// Start `exe` with `args`; the receiver gets the session when it exits.
pub fn start(exe: &Path, args: &[String], game: &Path) -> std::io::Result<Receiver<Session>> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn()?;
    let (tx, rx) = channel();
    let game = game.to_path_buf();
    let (started, clock) = (SystemTime::now(), Instant::now());
    thread::spawn(move || {
        let _ = child.wait();
        let _ = tx.send(Session { game, started, secs: clock.elapsed().as_secs() });
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_template() {
        let t = vec!["play".to_string(), "{rom}".to_string(), "--slot".to_string(), "{slot}".to_string()];
        assert_eq!(args(&t, Some(Path::new("g/a b.iso")), Some(3), None).unwrap(), ["play", "g/a b.iso", "--slot", "3"]);
        assert_eq!(args(&["{rom}".to_string()], Some(Path::new("x.gba")), None, None).unwrap(), ["x.gba"]);
        let d = vec!["play".to_string(), "{rom}".to_string(), "--data".to_string(), "{data}".to_string()];
        assert_eq!(args(&d, Some(Path::new("x.iso")), None, Some(Path::new("FB/pom/data"))).unwrap(), ["play", "x.iso", "--data", "FB/pom/data"]);
    }

    #[test]
    fn refuses_an_unfilled_placeholder() {
        let d = vec!["play".to_string(), "{rom}".to_string(), "--data".to_string(), "{data}".to_string()];
        assert!(args(&d, Some(Path::new("x.iso")), None, None).unwrap_err().contains("{data}"));
        assert!(args(&["--slot".to_string(), "{slot}".to_string()], None, None, None).is_err());
    }

    #[test]
    fn reports_the_session_when_the_program_exits() {
        // The test binary itself, asked only to list its tests, exits at once.
        let exe = std::env::current_exe().unwrap();
        let rx = start(&exe, &["--list".to_string()], Path::new("game.gba")).unwrap();
        let s = rx.recv_timeout(std::time::Duration::from_secs(30)).expect("session");
        assert_eq!(s.game, Path::new("game.gba"));
        assert!(s.secs < 30);
    }
}
