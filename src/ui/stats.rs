//! The Library's Stats view: totals, play time by week, by fruit, the
//! most played games and the latest sessions. It reads the same chip,
//! Favorites and FIND as the covers.

use std::time::{Duration, UNIX_EPOCH};

use basket_ui::text::Style;

use super::library::{cover, Row};
use super::{Cmd, Size, Ui};

/// Weeks the chart covers, oldest first.
pub const WEEKS: usize = 12;

pub struct StatsView<'a> {
    /// Play time over the shown games (from `played.tsv`, so older play counts).
    pub total: u64,
    /// The last seven days, from the session log.
    pub week: u64,
    pub sessions: usize,
    /// Games with any play time.
    pub games: usize,
    /// Seconds per week, oldest first; the last is the seven days up to now.
    pub weeks: Vec<u64>,
    /// (fruit name, seconds), most first.
    pub fruits: Vec<(&'a str, u64)>,
    /// The most played, most first.
    pub top: Vec<Row<'a>>,
    /// (title, started unix secs, seconds), newest first.
    pub recent: Vec<(String, i64, u64)>,
}

/// "under a minute", "42 min", "3 h 05 min".
pub fn fmt_hm(secs: u64) -> String {
    match secs {
        0..60 => "under a minute".into(),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{} h {:02} min", secs / 3600, secs / 60 % 60),
    }
}

/// The short form, for a chart label: "42m", "3h".
fn fmt_short(secs: u64) -> String {
    if secs < 3600 { format!("{}m", secs / 60) } else { format!("{:.1}h", secs as f32 / 3600.0).replace(".0h", "h") }
}

pub fn when(at: i64) -> String {
    basket_ui::fmt::fmt_when(UNIX_EPOCH + Duration::from_secs(at.max(0) as u64))
}

pub fn draw(ui: &mut Ui, v: &StatsView, x: f32, mut y: f32, w: f32) -> f32 {
    if v.total == 0 && v.sessions == 0 {
        let st = Style::reading(16.0).color(ui.pal.fg);
        ui.cv.paragraph(x, y, w.min(520.0), "No play time yet. Play a game from the launcher and it shows up here.", &st, 1.5, 3);
        return y + 60.0;
    }
    y = tiles(ui, v, x, y, w) + 34.0;
    y = ui.section(x, y, w, "Play time by week", Some("last 12 weeks")) + 18.0;
    y = weeks(ui, &v.weeks, x, y, w) + 34.0;
    if !v.fruits.is_empty() {
        y = ui.section(x, y, w, "By fruit", None) + 8.0;
        y = fruits(ui, &v.fruits, x, y, w) + 30.0;
    }
    if !v.top.is_empty() {
        y = ui.section(x, y, w, "Most played", None);
        y = top(ui, &v.top, x, y, w) + 30.0;
    }
    if !v.recent.is_empty() {
        y = ui.section(x, y, w, "Recent sessions", None);
        y = recent(ui, &v.recent, x, y, w);
    }
    y
}

fn tiles(ui: &mut Ui, v: &StatsView, x: f32, y: f32, w: f32) -> f32 {
    let items = [
        ("Play time", fmt_hm(v.total)),
        ("This week", fmt_hm(v.week)),
        ("Sessions", v.sessions.to_string()),
        ("Games played", v.games.to_string()),
    ];
    let cols = if ui.size == Size::Narrow { 2 } else { 4 };
    let gap = 12.0;
    let tw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
    let th = 84.0;
    let label = Style::interface_bold(11.0).upper().tracking(1.6).color(ui.muted());
    for (i, (name, value)) in items.iter().enumerate() {
        let tx = x + (i % cols) as f32 * (tw + gap);
        let ty = y + (i / cols) as f32 * (th + gap);
        ui.cv.stroke_rect(tx, ty, tw, th, 1.5, ui.pal.line);
        ui.cv.text(tx + 16.0, ty + 16.0, name, &label);
        let (size, lines) = ui.cv.fonts.fit_display(Style::display(30.0).upper(), value, tw - 32.0, 1, &[30.0, 26.0, 22.0, 18.0]);
        if let Some(l) = lines.first() {
            ui.cv.text(tx + 16.0, ty + 38.0, l, &Style::display(size).upper().color(ui.pal.fg));
        }
    }
    y + (items.len().div_ceil(cols)) as f32 * (th + gap) - gap
}

/// One bar a week on a shared baseline; the tallest week and the hovered
/// one are labelled, the rest read off their neighbours.
fn weeks(ui: &mut Ui, weeks: &[u64], x: f32, y: f32, w: f32) -> f32 {
    let ch = 150.0;
    let base = y + ch;
    let max = weeks.iter().copied().max().unwrap_or(0).max(1);
    let n = weeks.len().max(1) as f32;
    let gap = 6.0;
    let bw = ((w - gap * (n - 1.0)) / n).max(2.0);
    let label = Style::data(12.0).color(ui.muted());
    let value = Style::interface_bold(13.0).color(ui.pal.fg);
    let peak = weeks.iter().enumerate().max_by_key(|(_, s)| **s).map(|(i, _)| i);
    let (mx, my) = ui.input.mouse;
    for (i, &secs) in weeks.iter().enumerate() {
        let bx = x + i as f32 * (bw + gap);
        let bh = (secs as f32 / max as f32 * (ch - 22.0)).round();
        let hover = mx >= bx && mx < bx + bw + gap && my >= y && my < base + 24.0;
        if secs > 0 {
            let c = if hover { ui.pal.fg2 } else { ui.pal.fg };
            ui.cv.fill_rect(bx, base - bh, bw, bh.max(2.0), c);
        }
        if (hover || Some(i) == peak) && secs > 0 {
            ui.cv.text_center(bx + bw / 2.0, base - bh - 20.0, &fmt_short(secs), &value);
        }
    }
    ui.cv.hrule(x, x + w, base, 1.0, ui.pal.fg);
    let last = weeks.len().saturating_sub(1) as f32;
    ui.cv.text(x, base + 8.0, &format!("{} weeks ago", weeks.len().saturating_sub(1)), &label);
    ui.cv.text_right(x + last * (bw + gap) + bw, base + 8.0, "this week", &label);
    base + 26.0
}

fn fruits(ui: &mut Ui, fruits: &[(&str, u64)], x: f32, y: f32, w: f32) -> f32 {
    let max = fruits.first().map_or(1, |f| f.1).max(1);
    let name_w = 130.0;
    let value_w = 120.0;
    let bar_w = (w - name_w - value_w).max(40.0);
    let st = Style::interface_bold(14.0).color(ui.pal.fg);
    let mono = Style::data(13.0).color(ui.pal.fg);
    let mut ry = y;
    for (name, secs) in fruits {
        ui.cv.text(x, ry + 10.0, name, &st);
        let bw = (*secs as f32 / max as f32 * bar_w).max(2.0);
        ui.cv.fill_rect(x + name_w, ry + 12.0, bw, 14.0, ui.pal.fg);
        ui.cv.text_right(x + w, ry + 11.0, &fmt_hm(*secs), &mono);
        ry += 38.0;
    }
    ry
}

fn top(ui: &mut Ui, rows: &[Row], x: f32, y: f32, w: f32) -> f32 {
    let h = 49.0;
    let title = Style::display(16.0).upper().color(ui.pal.fg);
    let mono = Style::data(13.0);
    let mut ry = y;
    for (i, r) in rows.iter().enumerate() {
        ui.cv.text(x, ry + 16.0, &format!("{:>2}", i + 1), &mono.color(ui.muted()));
        cover(ui, r, x + 30.0, ry + 8.0, 24.0, 32.0, false);
        let t = ui.cv.fonts.ellipsize(&title, &r.game.title, w * 0.5);
        ui.cv.text(x + 68.0, ry + 15.0, &t, &title);
        ui.cv.text(x + 68.0 + w * 0.5 + 12.0, ry + 17.0, r.fruit_name, &mono.color(ui.muted()));
        ui.cv.text_right(x + w - 8.0, ry + 17.0, &fmt_hm(r.secs), &mono.color(ui.pal.fg));
        if ui.clicked(x, ry, w, h) {
            ui.emit(Cmd::SelectGame(r.game.path.clone()));
        }
        ry += h;
        ui.cv.hrule(x, x + w, ry, 1.0, ui.pal.line);
    }
    ry
}

fn recent(ui: &mut Ui, sessions: &[(String, i64, u64)], x: f32, y: f32, w: f32) -> f32 {
    let h = 40.0;
    let st = Style::interface(14.0).color(ui.pal.fg);
    let mono = Style::data(13.0);
    let mut ry = y;
    for (title, at, secs) in sessions {
        let t = ui.cv.fonts.ellipsize(&st, title, w * 0.5);
        ui.cv.text(x, ry + 12.0, &t, &st);
        ui.cv.text(x + w * 0.5 + 12.0, ry + 13.0, &when(*at), &mono.color(ui.muted()));
        ui.cv.text_right(x + w - 8.0, ry + 13.0, &fmt_hm(*secs), &mono.color(ui.pal.fg));
        ry += h;
        ui.cv.hrule(x, x + w, ry, 1.0, ui.pal.line);
    }
    ry
}
