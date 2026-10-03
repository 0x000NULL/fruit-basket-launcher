//! `launcher/history.log`: one line per finished job, for the Downloads
//! tab's Earlier list. Tab-separated so a person can read it too:
//!
//! ```text
//! <unix secs>\t<fruit>\t<build>\t<channel>\t<result>\t<message>
//! ```
//!
//! `result` is `installed`, or the failure kind: `network`, `signature`,
//! `install`. A line that doesn't parse is skipped, never fatal.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::feed::Channel;
use crate::jobs::FailKind;

/// How many entries Earlier shows.
pub const SHOWN: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub when: SystemTime,
    pub fruit: String,
    pub build: String,
    pub channel: Channel,
    /// `None` is installed; `Some` is how it failed.
    pub failed: Option<FailKind>,
    pub message: String,
}

impl Entry {
    fn line(&self) -> String {
        let secs = self.when.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let result = match self.failed {
            None => "installed",
            Some(FailKind::Network) => "network",
            Some(FailKind::Signature) => "signature",
            Some(FailKind::Install) => "install",
        };
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!("{secs}\t{}\t{}\t{}\t{result}\t{}\n", clean(&self.fruit), clean(&self.build), self.channel.name(), clean(&self.message))
    }

    fn parse(line: &str) -> Option<Entry> {
        let mut f = line.splitn(6, '\t');
        let secs: u64 = f.next()?.parse().ok()?;
        let fruit = f.next()?.to_string();
        let build = f.next()?.to_string();
        let channel = Channel::parse(f.next()?)?;
        let failed = match f.next()? {
            "installed" => None,
            "network" => Some(FailKind::Network),
            "signature" => Some(FailKind::Signature),
            "install" => Some(FailKind::Install),
            _ => return None,
        };
        let message = f.next().unwrap_or("").trim_end().to_string();
        Some(Entry { when: UNIX_EPOCH + Duration::from_secs(secs), fruit, build, channel, failed, message })
    }
}

pub fn path(launcher_dir: &Path) -> PathBuf {
    launcher_dir.join("history.log")
}

pub fn append(launcher_dir: &Path, entry: &Entry) -> io::Result<()> {
    fs::create_dir_all(launcher_dir)?;
    let mut f = OpenOptions::new().create(true).append(true).open(path(launcher_dir))?;
    f.write_all(entry.line().as_bytes())
}

/// The newest `SHOWN` entries, newest first.
pub fn read(launcher_dir: &Path) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(path(launcher_dir)) else { return Vec::new() };
    let mut out: Vec<Entry> = text.lines().filter_map(Entry::parse).collect();
    out.reverse();
    out.truncate(SHOWN);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(secs: u64, fruit: &str, failed: Option<FailKind>) -> Entry {
        Entry {
            when: UNIX_EPOCH + Duration::from_secs(secs),
            fruit: fruit.into(),
            build: "v1.3.1".into(),
            channel: Channel::Stable,
            failed,
            message: "tab\tand\nnewline".into(),
        }
    }

    #[test]
    fn round_trip_newest_first_and_bad_lines_skipped() {
        let t = tempfile::tempdir().unwrap();
        append(t.path(), &entry(100, "strawberry", None)).unwrap();
        fs::OpenOptions::new().append(true).open(path(t.path())).unwrap().write_all(b"garbage\n1\tx\n").unwrap();
        append(t.path(), &entry(200, "pomegranate", Some(FailKind::Signature))).unwrap();
        let got = read(t.path());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].fruit, "pomegranate");
        assert_eq!(got[0].failed, Some(FailKind::Signature));
        assert_eq!(got[0].message, "tab and newline");
        assert_eq!(got[1], Entry { message: "tab and newline".into(), ..entry(100, "strawberry", None) });
    }

    #[test]
    fn missing_log_is_empty() {
        let t = tempfile::tempdir().unwrap();
        assert!(read(t.path()).is_empty());
    }
}
