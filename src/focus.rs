//! Moving the controller's focus between the controls a frame drew.
//!
//! Each frame, every control that can be pressed registers a [`Spot`]: a
//! key (its label, plus a count when a label repeats) and its box. The
//! D-pad then steps to the nearest spot in that direction, so a new screen
//! needs no navigation code of its own.

/// Where a control sits: the scrolling list, the aside (or narrow sheet),
/// or a part that doesn't scroll (header, banner, footer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Main,
    Aside,
    Fixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub key: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub area: Area,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Spot {
    fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The spot to focus after pressing `dir` on `current`. With no current
/// spot (or one that is gone), the first spot. `None` when nothing lies
/// that way: the focus stays.
pub fn next<'a>(spots: &'a [Spot], current: Option<&str>, dir: Dir) -> Option<&'a Spot> {
    let Some(at) = current.and_then(|k| spots.iter().find(|s| s.key == k)) else {
        return spots.first();
    };
    let (cx, cy) = at.centre();
    spots
        .iter()
        .filter(|s| s.key != at.key)
        .filter_map(|s| {
            let (sx, sy) = s.centre();
            // Distance along the direction, and off to the side of it.
            let (along, side) = match dir {
                Dir::Up => (cy - sy, sx - cx),
                Dir::Down => (sy - cy, sx - cx),
                Dir::Left => (cx - sx, sy - cy),
                Dir::Right => (sx - cx, sy - cy),
            };
            // Ahead, beyond overlapping (a row's own neighbours don't count as "below").
            let gap = match dir {
                Dir::Up => at.y - (s.y + s.h),
                Dir::Down => s.y - (at.y + at.h),
                Dir::Left => at.x - (s.x + s.w),
                Dir::Right => s.x - (at.x + at.w),
            };
            // A spot level with this one (overlapping across the direction) beats any that isn't.
            let level = match dir {
                Dir::Up | Dir::Down => s.x < at.x + at.w && at.x < s.x + s.w,
                Dir::Left | Dir::Right => s.y < at.y + at.h && at.y < s.y + s.h,
            };
            let penalty = if level { 0.0 } else { 1.0e5 };
            (along > 0.0 && gap > -4.0).then_some((s, penalty + along + 2.0 * side.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| s)
}

/// The spot after (or before) `current` in drawing order, wrapping: Tab
/// and Shift+Tab.
pub fn cycle<'a>(spots: &'a [Spot], current: Option<&str>, back: bool) -> Option<&'a Spot> {
    if spots.is_empty() {
        return None;
    }
    let n = spots.len();
    let i = match current.and_then(|k| spots.iter().position(|s| s.key == k)) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None if back => n - 1,
        None => 0,
    };
    spots.get(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(key: &str, x: f32, y: f32, w: f32, h: f32) -> Spot {
        Spot { key: key.into(), x, y, w, h, area: Area::Main }
    }

    #[test]
    fn steps_to_the_nearest_spot_that_way() {
        // Settings-like: a row of three segments, a button under them at the right, a tick box at the left.
        let spots = vec![
            spot("1", 100.0, 0.0, 40.0, 34.0),
            spot("2", 139.0, 0.0, 40.0, 34.0),
            spot("3", 178.0, 0.0, 40.0, 34.0),
            spot("Check now", 400.0, 60.0, 90.0, 34.0),
            spot("tick", 100.0, 110.0, 40.0, 22.0),
        ];
        let key = |s: Option<&Spot>| s.map(|s| s.key.clone());
        assert_eq!(key(next(&spots, Some("1"), Dir::Right)), Some("2".into()));
        assert_eq!(key(next(&spots, Some("3"), Dir::Right)), Some("Check now".into()), "nothing level, so the nearest ahead");
        assert_eq!(key(next(&spots, Some("1"), Dir::Down)), Some("tick".into()), "straight below");
        assert_eq!(key(next(&spots, Some("tick"), Dir::Up)), Some("1".into()));
        assert_eq!(key(next(&spots, Some("Check now"), Dir::Up)), Some("3".into()), "nothing level: the nearest");
        assert_eq!(key(next(&spots, Some("Check now"), Dir::Left)), Some("3".into()));
        assert_eq!(next(&spots, Some("1"), Dir::Left), None, "nothing that way: stay");
        assert_eq!(next(&spots, Some("1"), Dir::Up), None);
    }

    #[test]
    fn a_vanished_or_missing_focus_starts_at_the_first() {
        let spots = vec![spot("a", 0.0, 0.0, 10.0, 10.0), spot("b", 0.0, 50.0, 10.0, 10.0)];
        assert_eq!(next(&spots, None, Dir::Down).unwrap().key, "a");
        assert_eq!(next(&spots, Some("gone"), Dir::Down).unwrap().key, "a");
        assert!(next(&[], None, Dir::Down).is_none());
    }

    #[test]
    fn tab_cycles_in_drawing_order() {
        let spots = vec![spot("a", 0.0, 0.0, 1.0, 1.0), spot("b", 0.0, 9.0, 1.0, 1.0), spot("c", 0.0, 19.0, 1.0, 1.0)];
        assert_eq!(cycle(&spots, None, false).unwrap().key, "a");
        assert_eq!(cycle(&spots, Some("a"), false).unwrap().key, "b");
        assert_eq!(cycle(&spots, Some("c"), false).unwrap().key, "a");
        assert_eq!(cycle(&spots, Some("a"), true).unwrap().key, "c");
        assert_eq!(cycle(&spots, None, true).unwrap().key, "c");
    }
}
