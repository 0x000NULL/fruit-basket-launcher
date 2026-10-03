//! The downloads queue as the UI sees it: what is running, what waits,
//! and what failed. The worker thread runs one job at a time; this side
//! decides which job goes next, so queued jobs can be deduplicated and a
//! failure can be retried. No I/O here, so it is tested directly.

use std::collections::{BTreeMap, VecDeque};

use crate::basket::Current;
use crate::feed::{self, Channel, Feed, Fruit};
use crate::jobs::{Event, FailKind, Job, Step};

#[derive(Debug, Clone)]
pub struct Active {
    pub job: Job,
    pub step: Step,
    /// Overall percent, 0–100.
    pub pct: u8,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub job: Job,
    pub kind: FailKind,
    pub message: String,
}

/// What a finished job means for the rest of the app.
#[derive(Debug, Clone)]
pub enum Finished {
    Installed(Job),
    Failed(Failure),
}

#[derive(Debug, Default)]
pub struct Queue {
    active: Option<Active>,
    waiting: VecDeque<Job>,
    failed: BTreeMap<String, Failure>,
}

impl Queue {
    /// Queue `job`. Returns it back if the worker should start it now.
    /// A fruit already running or waiting is not queued twice; queueing a
    /// fruit clears its last failure (that is what Try again does).
    pub fn enqueue(&mut self, job: Job) -> Option<Job> {
        if self.busy(&job.fruit).is_some() || self.is_waiting(&job.fruit) {
            return None;
        }
        self.failed.remove(&job.fruit);
        if self.active.is_none() {
            self.active = Some(Active { job: job.clone(), step: Step::Download, pct: 0 });
            Some(job)
        } else {
            self.waiting.push_back(job);
            None
        }
    }

    /// Fold in a worker event. Returns what finished, if anything, and the
    /// next job for the worker, if one should start.
    pub fn on_event(&mut self, event: &Event) -> (Option<Finished>, Option<Job>) {
        let Some(active) = &mut self.active else { return (None, None) };
        let finished = match event {
            Event::Progress { fruit, step, pct } if *fruit == active.job.fruit => {
                active.step = *step;
                active.pct = *pct;
                return (None, None);
            }
            Event::Done { fruit, .. } if *fruit == active.job.fruit => Finished::Installed(active.job.clone()),
            Event::Failed { fruit, kind, message } if *fruit == active.job.fruit => {
                let f = Failure { job: active.job.clone(), kind: *kind, message: message.clone() };
                self.failed.insert(fruit.clone(), f.clone());
                Finished::Failed(f)
            }
            _ => return (None, None),
        };
        self.active = None;
        let next = self.waiting.pop_front().map(|job| {
            self.active = Some(Active { job: job.clone(), step: Step::Download, pct: 0 });
            job
        });
        (Some(finished), next)
    }

    pub fn active(&self) -> Option<&Active> {
        self.active.as_ref()
    }

    pub fn waiting(&self) -> impl Iterator<Item = &Job> {
        self.waiting.iter()
    }

    pub fn failures(&self) -> impl Iterator<Item = &Failure> {
        self.failed.values()
    }

    pub fn busy(&self, fruit: &str) -> Option<&Active> {
        self.active.as_ref().filter(|a| a.job.fruit == fruit)
    }

    pub fn is_waiting(&self, fruit: &str) -> bool {
        self.waiting.iter().any(|j| j.fruit == fruit)
    }

    pub fn failure(&self, fruit: &str) -> Option<&Failure> {
        self.failed.get(fruit)
    }

    /// The Downloads badge: running + waiting + failed.
    pub fn count(&self) -> usize {
        self.active.iter().count() + self.waiting.len() + self.failed.len()
    }
}

/// A job installing `fruit`'s `channel` build for this PC, if there is one.
pub fn job_for(fruit: &Fruit, channel: Channel, keep: usize) -> Option<Job> {
    let build = fruit.channel(channel)?;
    let asset = build.assets.get(feed::this_platform())?;
    Some(Job {
        fruit: fruit.id.clone(),
        build: build.build.clone(),
        channel,
        asset: asset.clone(),
        carry: fruit.carry.clone(),
        keep,
    })
}

/// The build `fruit` would update to: its channel's build in the feed, if
/// that differs from what is installed and has a download for this PC.
pub fn update_for<'a>(fruit: &'a Fruit, current: &Current) -> Option<&'a feed::Build> {
    let build = fruit.channel(current.channel)?;
    (build.build != current.build && build.assets.contains_key(feed::this_platform())).then_some(build)
}

/// Update all: a job for every installed fruit that has an update and is
/// not already running, waiting, or (for the header count) failed.
pub fn updates<'a>(feed: &Feed, installed: impl Fn(&str) -> Option<&'a Current>, keep: usize) -> Vec<Job> {
    feed.fruits
        .iter()
        .filter_map(|f| {
            let current = installed(&f.id)?;
            update_for(f, current)?;
            job_for(f, current.channel, keep)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Asset;

    fn job(fruit: &str) -> Job {
        Job {
            fruit: fruit.into(),
            build: "v1".into(),
            channel: Channel::Stable,
            asset: Asset { name: "a.zip".into(), url: "https://x.test/a.zip".into(), size: 1, sha256: "00".into() },
            carry: vec![],
            keep: 2,
        }
    }

    fn done(fruit: &str) -> Event {
        Event::Done { fruit: fruit.into(), build: "v1".into(), channel: Channel::Stable }
    }

    #[test]
    fn runs_in_order_without_duplicates() {
        let mut q = Queue::default();
        assert!(q.enqueue(job("a")).is_some(), "idle: starts now");
        assert!(q.enqueue(job("b")).is_none());
        assert!(q.enqueue(job("a")).is_none(), "already running");
        assert!(q.enqueue(job("b")).is_none(), "already waiting");
        assert_eq!(q.count(), 2);

        q.on_event(&Event::Progress { fruit: "a".into(), step: Step::Verify, pct: 76 });
        assert_eq!(q.busy("a").map(|a| (a.step, a.pct)), Some((Step::Verify, 76)));

        let (fin, next) = q.on_event(&done("a"));
        assert!(matches!(fin, Some(Finished::Installed(j)) if j.fruit == "a"));
        assert_eq!(next.map(|j| j.fruit), Some("b".to_string()));
        assert!(q.busy("b").is_some());
        let (_, next) = q.on_event(&done("b"));
        assert!(next.is_none());
        assert_eq!(q.count(), 0);
    }

    #[test]
    fn failure_is_kept_until_retried_and_the_next_job_starts() {
        let mut q = Queue::default();
        q.enqueue(job("a"));
        q.enqueue(job("b"));
        let (fin, next) = q.on_event(&Event::Failed { fruit: "a".into(), kind: FailKind::Network, message: "couldn't reach x.test".into() });
        assert!(matches!(fin, Some(Finished::Failed(f)) if f.kind == FailKind::Network));
        assert_eq!(next.map(|j| j.fruit), Some("b".to_string()));
        assert_eq!(q.failure("a").map(|f| f.kind), Some(FailKind::Network));
        assert_eq!(q.count(), 2, "b running + a failed");

        assert!(q.enqueue(job("a")).is_none(), "b is still running");
        assert!(q.failure("a").is_none(), "retrying clears the failure");
        assert!(q.is_waiting("a"));
    }

    #[test]
    fn stray_events_are_ignored() {
        let mut q = Queue::default();
        assert!(q.on_event(&done("a")).0.is_none());
        q.enqueue(job("a"));
        assert!(q.on_event(&done("zzz")).0.is_none());
        assert!(q.busy("a").is_some());
    }

    #[test]
    fn update_all_picks_only_out_of_date_fruits() {
        let feed = feed::verify(feed::tests::FEED, feed::tests::SIG, crate::key::PUBLIC_KEY, None).unwrap();
        let berry = feed.fruit("strawberry").unwrap();
        let stable = berry.stable.as_ref().unwrap().build.clone();
        let old = Current { build: "v0.0.1".into(), channel: Channel::Stable };
        let fresh = Current { build: stable, channel: Channel::Stable };

        let jobs = updates(&feed, |id| (id == "strawberry").then_some(&old), 2);
        if feed::this_platform() == "windows-x64" {
            assert_eq!(jobs.iter().map(|j| j.fruit.as_str()).collect::<Vec<_>>(), ["strawberry"]);
        }
        assert!(updates(&feed, |id| (id == "strawberry").then_some(&fresh), 2).is_empty());
        assert!(updates(&feed, |_| None, 2).is_empty(), "nothing installed, nothing to update");
    }
}
