//! A fruit's compatibility list, `compat.txt` on the site:
//! `title<TAB>serial<TAB>status<TAB>notes`, a header line first. A game is
//! matched by serial when it has one, else by title.

use std::collections::HashMap;

use crate::library::{norm_title, Game};

/// How far a game gets: the mocks' four squares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Nothing,
    Boots,
    Menus,
    InGame,
    Playable,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.trim().to_lowercase().as_str() {
            "playable" | "perfect" => Level::Playable,
            "in-game" | "ingame" => Level::InGame,
            "menus" | "menu" | "intro" => Level::Menus,
            "boots" => Level::Boots,
            "nothing" | "broken" | "black" => Level::Nothing,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Playable => "Playable",
            Level::InGame => "In-game",
            Level::Menus => "Menus",
            Level::Boots => "Boots",
            Level::Nothing => "Doesn't boot",
        }
    }

    /// Filled squares out of four.
    pub fn squares(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Default, Clone)]
pub struct Compat {
    by_serial: HashMap<String, Level>,
    by_title: HashMap<String, Level>,
}

impl Compat {
    pub fn parse(text: &str) -> Compat {
        let mut c = Compat::default();
        for line in text.lines().skip(1) {
            let mut f = line.split('\t');
            let (Some(title), Some(serial), Some(status)) = (f.next(), f.next(), f.next()) else { continue };
            let Some(level) = Level::parse(status) else { continue };
            if !serial.trim().is_empty() {
                c.by_serial.insert(serial.trim().to_uppercase(), level);
            }
            c.by_title.insert(norm_title(title), level);
        }
        c
    }

    /// The game's level: by its code, then a serial its dump matched, then title.
    pub fn level(&self, game: &Game, dump_serial: Option<&str>) -> Option<Level> {
        let serials = game.code.as_deref().into_iter().chain(dump_serial);
        for s in serials {
            if let Some(l) = self.by_serial.get(&s.to_uppercase()) {
                return Some(*l);
            }
        }
        self.by_title.get(&norm_title(&game.title)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game(title: &str, code: Option<&str>) -> Game {
        Game { path: PathBuf::from("x"), fruit: "f".into(), title: title.into(), size: 0, mtime: 0, code: code.map(Into::into) }
    }

    #[test]
    fn matches_by_serial_then_title() {
        let c = Compat::parse("title\tserial\tstatus\tnotes\nFinal Fantasy X\tSLUS-20312\tin-game\tok\nLegend of Zelda, The\t\tmenus\t\nOdd\tX\tunknown\t\n");
        assert_eq!(c.level(&game("Anything", Some("slus-20312")), None), Some(Level::InGame));
        assert_eq!(c.level(&game("Something", None), Some("SLUS-20312")), Some(Level::InGame));
        assert_eq!(c.level(&game("The Legend of Zelda", None), None), Some(Level::Menus));
        assert_eq!(c.level(&game("Odd", Some("X")), None), None, "unknown status skipped");
        assert_eq!(Level::InGame.squares(), 3);
    }
}
