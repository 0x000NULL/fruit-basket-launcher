//! The signed feed: `fruit-basket/feed.json`, written by the site's
//! feedgen.py and signed with minisign by `make sign`.
//!
//! A feed is accepted only if its signature checks out against the
//! embedded key and it is not older than the last one accepted, so a
//! replayed old feed cannot downgrade anyone. The last good feed is kept on
//! disk so the launcher works offline.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::Path;

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;
/// Larger than any feed the site will write; stops a hostile server from
/// streaming forever.
const FEED_LIMIT: u64 = 4 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    pub schema: u32,
    /// UTC, `2026-10-02T12:00:00Z`; compares correctly as a string.
    pub generated: String,
    pub base: String,
    pub launcher: Option<Build>,
    pub fruits: Vec<Fruit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fruit {
    pub id: String,
    pub no: u32,
    pub name: String,
    pub system: String,
    pub pixel: bool,
    pub ext: Vec<String>,
    /// Archives the emulator opens itself (`[".zip"]`). An archive is this
    /// fruit's only if a file inside it has one of `ext`, since several
    /// fruits can list `.zip`.
    #[serde(default)]
    pub archives: Vec<String>,
    pub status: Status,
    pub summary: String,
    pub blurb: String,
    pub bin: Option<String>,
    pub bios: Option<String>,
    pub dump_db: Option<String>,
    pub launch: Vec<String>,
    pub load_slot: Option<Vec<String>>,
    /// Arguments for opening the fruit with no game (`--data {data}`).
    #[serde(default)]
    pub open: Vec<String>,
    /// Arguments added after `launch` or `load_slot` when a game starts
    /// from couch mode (`--fullscreen --exit-on-quit`).
    #[serde(default)]
    pub couch: Vec<String>,
    /// Files the emulator keeps beside its exe. They move to each new build;
    /// for a fruit whose templates use `{data}`, they move into `data/` once.
    #[serde(default)]
    pub carry: Vec<String>,
    /// Where the emulator keeps a game's cover picture, tried in order:
    /// paths with `{rom_dir}`, `{stem}`, `{data}`, `{code}` and `{cache}`.
    /// Only ever read as a PNG.
    #[serde(default)]
    pub art: Vec<String>,
    /// Arguments for Start fresh, in place of `launch`, for an emulator
    /// that otherwise picks up where it left off (`{rom} --no-resume`).
    /// Empty: Start fresh uses `launch`.
    #[serde(default)]
    pub fresh: Vec<String>,
    /// The save slots the emulator loads, lowest and highest (`["1", "8"]`).
    /// Others (Crabapple's resume state, `.s9`) aren't listed. Empty: all.
    #[serde(default)]
    pub slots: Vec<String>,
    pub url: String,
    pub readme_url: String,
    pub compat: Option<FileRef>,
    pub dumps: Option<FileRef>,
    pub changelog: Vec<ChangelogEntry>,
    pub stable: Option<Build>,
    pub nightly: Option<Build>,
    pub releases: Vec<Build>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Released,
    Growing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Nightly => "nightly",
        }
    }

    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "stable" => Some(Channel::Stable),
            "nightly" => Some(Channel::Nightly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Build {
    /// A release tag (`v1.3.1`) or a nightly commit (`7c1239a`).
    pub build: String,
    pub date: Option<String>,
    pub notes: Vec<String>,
    pub notes_url: String,
    /// Keyed by platform: `windows-x64`, `macos-arm64`, `linux-x64`.
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangelogEntry {
    pub date: String,
    pub text: String,
}

impl Feed {
    pub fn fruit(&self, id: &str) -> Option<&Fruit> {
        self.fruits.iter().find(|f| f.id == id)
    }
}

impl Fruit {
    pub fn channel(&self, ch: Channel) -> Option<&Build> {
        match ch {
            Channel::Stable => self.stable.as_ref(),
            Channel::Nightly => self.nightly.as_ref(),
        }
    }

    /// Every build this fruit can be installed at, nightly first.
    pub fn builds(&self) -> impl Iterator<Item = (Channel, &Build)> {
        self.nightly
            .iter()
            .map(|b| (Channel::Nightly, b))
            .chain(self.releases.iter().map(|b| (Channel::Stable, b)))
    }

    pub fn build(&self, id: &str) -> Option<(Channel, &Build)> {
        self.builds().find(|(_, b)| b.build == id)
    }

    /// The `couch` arguments, if `build` takes them: only builds the feed
    /// still lists. The site sets `couch` together with `oldest`, so a kept
    /// or installed older build (Pomegranate v0.3.0) never gets flags it
    /// doesn't know.
    pub fn couch_args(&self, build: &str) -> &[String] {
        if self.build(build).is_some() { &self.couch } else { &[] }
    }

    /// True if any launch template takes `{data}`: the fruit keeps its
    /// saves and settings in `<root>/<fruit>/data/`, not beside its exe.
    pub fn uses_data(&self) -> bool {
        self.launch.iter().chain(self.load_slot.iter().flatten()).chain(&self.open).chain(&self.couch).any(|a| a.contains("{data}"))
    }

    /// Whether the launcher lists slot `n`: inside the feed's `slots`, or
    /// any slot when the fruit doesn't say.
    pub fn lists_slot(&self, n: u8) -> bool {
        match self.slots.iter().map(|s| s.parse::<u8>()).collect::<Result<Vec<_>, _>>().as_deref() {
            Ok([lo, hi]) => (*lo..=*hi).contains(&n),
            _ => true,
        }
    }

    /// The emulator can start a game from a save slot.
    pub fn loads_slots(&self) -> bool {
        self.load_slot.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// What moves from build to build: `carry`, until the fruit has a data
    /// folder.
    pub fn build_carry(&self) -> &[String] {
        if self.uses_data() { &[] } else { &self.carry }
    }

    /// What moves into the data folder, once.
    pub fn data_carry(&self) -> &[String] {
        if self.uses_data() { &self.carry } else { &[] }
    }

    /// True if `path` has one of this fruit's extensions (case-insensitive).
    pub fn reads(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        self.ext.iter().any(|e| name.ends_with(&e.to_ascii_lowercase()))
    }

    /// True if `path` has one of this fruit's `archives` extensions; whether
    /// the archive holds a game of this fruit's is `library::plays`.
    pub fn takes_archive(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        self.archives.iter().any(|e| name.ends_with(&e.to_ascii_lowercase()))
    }
}

/// The platform key of this PC, as the feed spells it.
pub fn this_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => "unknown",
    }
}

/// `windows-x64` → `Windows x64`, for the Builds-for chips.
pub fn platform_label(key: &str) -> String {
    let (os, arch) = key.split_once('-').unwrap_or((key, ""));
    let os = match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    format!("{os} {arch}").trim().to_string()
}

#[derive(Debug)]
pub enum FeedError {
    /// The site could not be reached; `String` is the host.
    Network(String, String),
    /// The signature is missing, malformed or wrong.
    Signature(String),
    /// Signed, but older than the feed already accepted.
    Stale { got: String, have: String },
    Parse(String),
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Network(host, e) => write!(f, "couldn't reach {host}: {e}"),
            FeedError::Signature(e) => write!(f, "feed signature didn't check out: {e}"),
            FeedError::Stale { got, have } => {
                write!(f, "feed from {got} is older than the one already accepted ({have})")
            }
            FeedError::Parse(e) => write!(f, "feed didn't parse: {e}"),
        }
    }
}

impl std::error::Error for FeedError {}

/// Check `sig` over `bytes` with `key`, then parse. `newest` is the
/// `generated` stamp of the last feed accepted, if any.
pub fn verify(bytes: &[u8], sig: &str, key: &str, newest: Option<&str>) -> Result<Feed, FeedError> {
    let pk = PublicKey::from_base64(key).map_err(|e| FeedError::Signature(format!("bad key: {e}")))?;
    let sig = Signature::decode(sig).map_err(|e| FeedError::Signature(e.to_string()))?;
    pk.verify(bytes, &sig, false).map_err(|e| FeedError::Signature(e.to_string()))?;
    let feed: Feed = serde_json::from_slice(bytes).map_err(|e| FeedError::Parse(e.to_string()))?;
    if feed.schema != SCHEMA {
        return Err(FeedError::Parse(format!("schema {} (this launcher reads {SCHEMA})", feed.schema)));
    }
    if let Some(have) = newest {
        if feed.generated.as_str() < have {
            return Err(FeedError::Stale { got: feed.generated, have: have.to_string() });
        }
    }
    Ok(feed)
}

pub fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).to_string()
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, FeedError> {
    let net = |e: String| FeedError::Network(host(url), e);
    let resp = ureq::get(url).call().map_err(|e| net(e.to_string()))?;
    let mut out = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut out).map_err(|e| net(e.to_string()))?;
    Ok(out)
}

/// The feed as fetched and checked, with the raw bytes so the caller can
/// cache exactly what was signed.
pub struct Fetched {
    pub feed: Feed,
    pub bytes: Vec<u8>,
    pub sig: String,
}

/// Fetch `url` and `url.minisig`, and verify. Blocking; call it off the UI
/// thread.
pub fn fetch(url: &str, key: &str, newest: Option<&str>) -> Result<Fetched, FeedError> {
    let bytes = get(url, FEED_LIMIT)?;
    let sig = String::from_utf8_lossy(&get(&format!("{url}.minisig"), 4096)?).into_owned();
    let feed = verify(&bytes, &sig, key, newest)?;
    Ok(Fetched { feed, bytes, sig })
}

/// The cached feed, re-verified on load so a file edited on disk is not
/// trusted either.
pub fn load_cached(dir: &Path, key: &str) -> Option<Feed> {
    let bytes = std::fs::read(dir.join("feed.json")).ok()?;
    let sig = std::fs::read_to_string(dir.join("feed.json.minisig")).ok()?;
    verify(&bytes, &sig, key, None).ok()
}

pub fn save_cached(dir: &Path, fetched: &Fetched) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::basket::write_atomic(&dir.join("feed.json.minisig"), fetched.sig.as_bytes())?;
    crate::basket::write_atomic(&dir.join("feed.json"), &fetched.bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The real site feed and its signature, as committed test data, so
    /// verification is tested against the real key and real minisign output.
    pub const FEED: &[u8] = include_bytes!("../tests/data/feed.json");
    pub const SIG: &str = include_str!("../tests/data/feed.json.minisig");

#[test]
    fn couch_args_only_for_builds_the_feed_lists() {
        let mut feed = verify(FEED, SIG, crate::key::PUBLIC_KEY, None).unwrap();
        let f = feed.fruits.iter_mut().find(|f| f.id == "strawberry").unwrap();
        f.couch = vec!["--fullscreen".into()];
        let listed = f.builds().next().unwrap().1.build.clone();
        assert_eq!(f.couch_args(&listed), ["--fullscreen".to_string()]);
        assert!(f.couch_args("v0.0.1").is_empty(), "an older build kept on disk");
    }

        #[test]
    fn real_feed_verifies() {
        let feed = verify(FEED, SIG, crate::key::PUBLIC_KEY, None).unwrap();
        let pom = feed.fruit("pomegranate").unwrap();
        assert_eq!(pom.no, 1);
        assert_eq!(pom.status, Status::Released);
        assert!(pom.stable.as_ref().unwrap().assets.contains_key("windows-x64"));
        assert_eq!(feed.fruit("pear").unwrap().status, Status::Growing);
    }

    #[test]
    fn tampered_feed_is_rejected() {
        let mut bytes = FEED.to_vec();
        let at = bytes.windows(6).position(|w| w == b"sha256").unwrap() + 10;
        bytes[at] ^= 1;
        assert!(matches!(verify(&bytes, SIG, crate::key::PUBLIC_KEY, None), Err(FeedError::Signature(_))));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let other = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        assert!(matches!(verify(FEED, SIG, other, None), Err(FeedError::Signature(_))));
    }

    #[test]
    fn older_feed_is_rejected() {
        let r = verify(FEED, SIG, crate::key::PUBLIC_KEY, Some("2999-01-01T00:00:00Z"));
        assert!(matches!(r, Err(FeedError::Stale { .. })));
    }

    #[test]
    fn platform_labels() {
        assert_eq!(platform_label("windows-x64"), "Windows x64");
        assert_eq!(platform_label("macos-arm64"), "macOS arm64");
        assert_eq!(host("https://projects.ethanaldrich.net/fruit-basket/feed.json"), "projects.ethanaldrich.net");
    }

    /// Old launchers must keep reading feeds that grow keys (v1.2's
    /// `archives`), so nothing here may deny unknown fields.
    #[test]
    fn unknown_keys_are_ignored_and_archives_parse() {
        let mut v: serde_json::Value = serde_json::from_slice(FEED).unwrap();
        v["someday"] = serde_json::json!(true);
        let f = &mut v["fruits"][0];
        f["someday"] = serde_json::json!({"x": 1});
        f["archives"] = serde_json::json!([".zip"]);
        let feed: Feed = serde_json::from_value(v).unwrap();
        assert_eq!(feed.fruits[0].archives, [".zip"]);
        assert!(feed.fruits[0].takes_archive(Path::new("Game (USA).Zip")));
        assert!(feed.fruits[1].archives.is_empty(), "absent: no archives");
    }

    #[test]
    fn reads_extensions() {
        let feed = verify(FEED, SIG, crate::key::PUBLIC_KEY, None).unwrap();
        let s = feed.fruit("strawberry").unwrap();
        assert!(s.reads(Path::new("Game.GBA")));
        assert!(!s.reads(Path::new("game.nes")));
    }
}
