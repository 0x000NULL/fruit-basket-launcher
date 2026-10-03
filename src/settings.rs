//! Launcher settings: `<config_dir>/fruitbasket/settings.toml`, read and
//! written with `basket_app::prefs` like the emulators' own settings, so
//! unknown keys survive and a bad value costs only that one key.

use std::path::PathBuf;

use basket_app::prefs::{self, Hidden};
use serde::{Deserialize, Serialize};

use crate::basket::Basket;
use crate::feed::Channel;

pub const APP_DIR: &str = "fruitbasket";
const APP_NAME: &str = "Fruit Basket";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    System,
    Paper,
    Night,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    /// The basket folder; `None` is `~/FruitBasket`.
    pub root: Option<PathBuf>,
    /// Game folders scanned besides each fruit's own `games/`.
    pub folders: Vec<PathBuf>,
    pub rescan_on_open: bool,
    /// Channel for a fruit's first install.
    pub new_channel: Channel,
    pub check_on_open: bool,
    pub install_without_asking: bool,
    /// Old builds kept for rolling back: 1, 2 or 3.
    pub keep: u8,
    pub couch_on_controller: bool,
    pub theme: ThemePref,
    /// Growing fruits to tell the player about when they ripen.
    pub watch: Vec<String>,
    /// Games hidden from the library with Remove (the files stay).
    pub hidden: Vec<PathBuf>,
    pub window: Option<(u32, u32)>,
    #[serde(skip)]
    pub extra: Hidden<toml::Table>,
}

const KNOWN_KEYS: [&str; 12] = [
    "root",
    "folders",
    "rescan_on_open",
    "new_channel",
    "check_on_open",
    "install_without_asking",
    "keep",
    "couch_on_controller",
    "theme",
    "watch",
    "hidden",
    "window",
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            root: None,
            folders: Vec::new(),
            rescan_on_open: true,
            new_channel: Channel::Stable,
            check_on_open: true,
            install_without_asking: false,
            keep: 2,
            couch_on_controller: true,
            theme: ThemePref::System,
            // The mocks watch Pear by default: the first fruit people ask about.
            watch: vec!["pear".to_string()],
            hidden: Vec::new(),
            window: None,
            extra: Hidden::default(),
        }
    }
}

impl Settings {
    /// None in tests, so no test reads or overwrites the real file.
    pub fn path() -> Option<PathBuf> {
        if cfg!(test) {
            return None;
        }
        prefs::config_path(APP_DIR)
    }

    pub fn load() -> Settings {
        Settings::path().and_then(|p| prefs::read(&p)).map(Settings::from_table).unwrap_or_default()
    }

    pub fn from_table(t: toml::Table) -> Settings {
        let mut s = Settings::default();
        macro_rules! take {
            ($($field:ident),*) => {$(
                if let Some(v) = prefs::take(&t, stringify!($field)) {
                    s.$field = v;
                }
            )*};
        }
        take!(root, folders, rescan_on_open, new_channel, check_on_open, install_without_asking, keep,
              couch_on_controller, theme, watch, hidden, window);
        s.keep = s.keep.clamp(1, 3);
        s.extra = Hidden(t);
        s
    }

    pub fn save(&self) {
        if let Some(path) = Settings::path() {
            if let Err(e) = prefs::write(&path, APP_NAME, &self.extra.0, &KNOWN_KEYS, self) {
                eprintln!("fruitbasket: saving {}: {e:#}", path.display());
            }
        }
    }

    pub fn basket(&self) -> Basket {
        Basket::new(self.root.clone().unwrap_or_else(Basket::default_root))
    }
}

impl<'de> Deserialize<'de> for ThemePref {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match String::deserialize(d)?.as_str() {
            "system" => Ok(ThemePref::System),
            "paper" => Ok(ThemePref::Paper),
            "night" => Ok(ThemePref::Night),
            other => Err(serde::de::Error::custom(format!("unknown theme {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unknown_keys_and_drops_bad_values() {
        let t: toml::Table = toml::from_str(
            "keep = 9\ntheme = \"night\"\nnew_channel = \"nightly\"\ncouch_on_controller = \"yes\"\nfuture = 1\n",
        )
        .unwrap();
        let s = Settings::from_table(t);
        assert_eq!(s.keep, 3, "clamped");
        assert_eq!(s.theme, ThemePref::Night);
        assert_eq!(s.new_channel, Channel::Nightly);
        assert!(s.couch_on_controller, "a bad value falls back to the default");
        assert!(s.extra.0.contains_key("future"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        prefs::write(&path, APP_NAME, &s.extra.0, &KNOWN_KEYS, &s).unwrap();
        let back = Settings::from_table(prefs::read(&path).unwrap());
        assert_eq!(back.theme, ThemePref::Night);
        assert_eq!(back.keep, 3);
        assert!(back.extra.0.contains_key("future"));
    }
}
