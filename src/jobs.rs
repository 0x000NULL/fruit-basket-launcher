//! The download worker: one thread, one fruit at a time.
//!
//! Each job runs Download (0–70%), Verify (70–85%) and Install (85–100%),
//! the split the Downloads tab draws. The hash is computed while the bytes
//! arrive; Verify compares it with the signed feed and deletes the file on
//! a mismatch, before anything on disk is touched. The UI thread sends
//! jobs in and polls events out; it never blocks on the network.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;

use sha2::{Digest, Sha256};

use crate::basket::Basket;
use crate::feed::{self, Asset, Channel};

#[derive(Debug, Clone)]
pub struct Job {
    pub fruit: String,
    pub build: String,
    pub channel: Channel,
    pub asset: Asset,
    /// Files that move from the old build to the new one.
    pub carry: Vec<String>,
    /// Files that move from the builds into `data/`, once.
    pub migrate: Vec<String>,
    /// Builds to keep for rolling back, besides the current one.
    pub keep: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Download,
    Verify,
    Install,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Download => "downloading",
            Step::Verify => "verifying",
            Step::Install => "installing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// Couldn't reach the site; nothing on disk changed.
    Network,
    /// The hash didn't match the signed feed; the download was deleted.
    Signature,
    /// The archive was bad or the disk refused; the old build still runs.
    Install,
    /// Too little free space to start; nothing was downloaded.
    Space,
}

#[derive(Debug, Clone)]
pub enum Event {
    Progress { fruit: String, step: Step, pct: u8 },
    Done { fruit: String, build: String, channel: Channel },
    Failed { fruit: String, kind: FailKind, message: String },
}

/// Overall percent for `frac` (0..=1) of the way through `step`.
pub fn overall(step: Step, frac: f32) -> u8 {
    let (lo, hi) = match step {
        Step::Download => (0.0, 70.0),
        Step::Verify => (70.0, 85.0),
        Step::Install => (85.0, 100.0),
    };
    (lo + (hi - lo) * frac.clamp(0.0, 1.0)).round() as u8
}

/// The step an overall percent falls in, and how far through it (0–100):
/// overall 76 is "Verify · 40%".
pub fn step_of(pct: u8) -> (Step, u8) {
    let p = pct as f32;
    let (step, lo, hi) = if p < 70.0 {
        (Step::Download, 0.0, 70.0)
    } else if p < 85.0 {
        (Step::Verify, 70.0, 85.0)
    } else {
        (Step::Install, 85.0, 100.0)
    };
    (step, ((p - lo) / (hi - lo) * 100.0).round().min(100.0) as u8)
}

pub struct Worker {
    jobs: Sender<Job>,
    events: Receiver<Event>,
}

impl Worker {
    pub fn start(basket: Basket) -> Worker {
        let (jobs, job_rx) = channel::<Job>();
        let (event_tx, events) = channel();
        thread::Builder::new()
            .name("downloads".into())
            .spawn(move || {
                for job in job_rx {
                    run(&basket, &job, &event_tx, &mut |url, to, size, progress| download(url, to, size, progress));
                }
            })
            .expect("spawn download thread");
        Worker { jobs, events }
    }

    /// Queue a job. Jobs run in the order sent.
    pub fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// Everything that happened since the last poll.
    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(e) => out.push(e),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

type Fetch<'a> = dyn FnMut(&str, &Path, u64, &mut dyn FnMut(u64)) -> io::Result<String> + 'a;

/// One job, start to finish. `fetch` downloads `url` to a path and returns
/// the SHA-256 of what it wrote; tests swap it out.
fn run(basket: &Basket, job: &Job, tx: &Sender<Event>, fetch: &mut Fetch) {
    let send = |e: Event| {
        let _ = tx.send(e);
    };
    let fruit = job.fruit.clone();
    let progress = |step: Step, frac: f32| Event::Progress { fruit: fruit.clone(), step, pct: overall(step, frac) };
    let fail = |kind: FailKind, message: String| Event::Failed { fruit: fruit.clone(), kind, message };

    let free = crate::platform::free_space(&basket.root);
    if let Some(message) = short_of_space(job.asset.size, free) {
        send(fail(FailKind::Space, message));
        return;
    }

    send(progress(Step::Download, 0.0));
    let part = part_path(basket, &job.asset.name);
    let mut last = 0u8;
    let size = job.asset.size.max(1);
    let got = fetch(&job.asset.url, &part, job.asset.size, &mut |bytes| {
        let pct = overall(Step::Download, bytes as f32 / size as f32);
        if pct != last {
            last = pct;
            send(progress(Step::Download, bytes as f32 / size as f32));
        }
    });
    let got = match got {
        Ok(hash) => hash,
        Err(e) => {
            let _ = fs::remove_file(&part);
            send(fail(FailKind::Network, format!("couldn't reach {}: {e}", feed::host(&job.asset.url))));
            return;
        }
    };

    send(progress(Step::Verify, 0.5));
    if !got.eq_ignore_ascii_case(&job.asset.sha256) {
        let _ = fs::remove_file(&part);
        send(fail(FailKind::Signature, format!("{} hashed to {got}, not the signed {}", job.asset.name, job.asset.sha256)));
        return;
    }

    send(progress(Step::Install, 0.0));
    let result = basket
        .install(&job.fruit, &job.build, job.channel, &part, &job.carry)
        // Before pruning, so an old build's cards are moved, not deleted.
        .and_then(|_| basket.migrate_data(&job.fruit, &job.migrate))
        .and_then(|_| basket.prune(&job.fruit, job.keep).map(|_| ()));
    let _ = fs::remove_file(&part);
    match result {
        Ok(()) => {
            send(progress(Step::Install, 1.0));
            send(Event::Done { fruit: job.fruit.clone(), build: job.build.clone(), channel: job.channel });
        }
        Err(e) => send(fail(FailKind::Install, e.to_string())),
    }
}

/// Why a job of `size` bytes can't start with `free` bytes free, if it
/// can't: it needs room for the download, the extracted build, and the
/// build it replaces, kept for rolling back.
fn short_of_space(size: u64, free: Option<u64>) -> Option<String> {
    let need = size.saturating_mul(3);
    let free = free?;
    (free < need).then(|| {
        use crate::ui::fmt_size;
        format!("not enough space: it needs {}, and {} is free", fmt_size(need), fmt_size(free))
    })
}

fn part_path(basket: &Basket, name: &str) -> PathBuf {
    let safe: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':')).collect();
    basket.downloads_dir().join(format!("{safe}.part"))
}

/// Stream `url` to `to`, hashing as it goes. Stops past `size` bytes plus a
/// little slack, so a wrong-length file fails fast instead of filling a disk.
fn download(url: &str, to: &Path, size: u64, progress: &mut dyn FnMut(u64)) -> io::Result<String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let resp = ureq::get(url).call().map_err(io::Error::other)?;
    let mut reader = resp.into_reader().take(size + 1);
    let mut out = io::BufWriter::new(fs::File::create(to)?);
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        total += n as u64;
        progress(total);
    }
    out.flush()?;
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes() -> Vec<u8> {
        let mut cur = io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut cur);
        w.start_file("berry.exe", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"exe").unwrap();
        w.finish().unwrap();
        cur.into_inner()
    }

    fn job(data: &[u8], sha256: String) -> Job {
        Job {
            fruit: "berry".into(),
            build: "v1".into(),
            channel: Channel::Stable,
            asset: Asset { name: "berry-v1.zip".into(), url: "https://example.test/berry-v1.zip".into(), size: data.len() as u64, sha256 },
            carry: vec![],
            migrate: vec![],
            keep: 2,
        }
    }

    fn events(basket: &Basket, job: &Job, data: Vec<u8>, net_ok: bool) -> Vec<Event> {
        let (tx, rx) = channel();
        run(basket, job, &tx, &mut |_, to, _, progress| {
            if !net_ok {
                return Err(io::Error::other("connection refused"));
            }
            fs::create_dir_all(to.parent().unwrap())?;
            fs::write(to, &data)?;
            progress(data.len() as u64);
            Ok(format!("{:x}", Sha256::digest(&data)))
        });
        drop(tx);
        rx.into_iter().collect()
    }

    #[test]
    fn good_build_installs() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let data = zip_bytes();
        let ev = events(&b, &job(&data, format!("{:x}", Sha256::digest(&data))), data, true);
        assert!(matches!(ev.last(), Some(Event::Done { build, .. }) if build == "v1"));
        assert_eq!(b.current("berry").unwrap().build, "v1");
        assert!(fs::read_dir(b.downloads_dir()).unwrap().next().is_none(), "the .part is cleaned up");
    }

    #[test]
    fn wrong_hash_deletes_download_and_installs_nothing() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let data = zip_bytes();
        let ev = events(&b, &job(&data, "00".repeat(32)), data, true);
        assert!(matches!(ev.last(), Some(Event::Failed { kind: FailKind::Signature, .. })));
        assert!(b.current("berry").is_none());
        assert!(!b.builds_dir("berry").exists());
        assert!(!part_path(&b, "berry-v1.zip").exists());
    }

    #[test]
    fn network_failure_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let ev = events(&b, &job(b"x", "00".repeat(32)), vec![], false);
        match ev.last() {
            Some(Event::Failed { kind: FailKind::Network, message, .. }) => assert!(message.contains("example.test")),
            other => panic!("{other:?}"),
        }
        assert!(!b.fruit_dir("berry").exists());
    }

    #[test]
    fn too_little_space_fails_before_downloading() {
        assert!(short_of_space(100, Some(300)).is_none());
        assert!(short_of_space(100, None).is_none(), "unknown free space doesn't block");
        assert!(short_of_space(100, Some(299)).unwrap().starts_with("not enough space"));
        assert!(short_of_space(u64::MAX, Some(1)).is_some());

        let t = tempfile::tempdir().unwrap();
        let b = Basket::new(t.path());
        let mut j = job(b"x", "00".repeat(32));
        j.asset.size = u64::MAX / 2;
        let ev = events(&b, &j, vec![], true);
        assert!(matches!(ev.as_slice(), [Event::Failed { kind: FailKind::Space, .. }]));
        assert!(!b.downloads_dir().exists());
    }

    #[test]
    fn step_split_matches_the_mocks() {
        assert_eq!(step_of(76), (Step::Verify, 40));
        assert_eq!(step_of(46), (Step::Download, 66));
        assert_eq!(step_of(100), (Step::Install, 100));
        assert_eq!(overall(Step::Verify, 0.4), 76);
    }
}
