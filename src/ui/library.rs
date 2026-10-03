//! The Library tab: filter chips, sort and view toggles, Continue, then all
//! games as covers or a list, and an aside for the selected game. Narrow
//! windows show the aside as a sheet, like the Basket tab.

use std::path::Path;
use std::time::SystemTime;

use basket_ui::fmt::fmt_play;

use crate::ui::fmt_size;
use basket_ui::text::Style;
use basket_ui::tokens::{hex, Rgb};
use basket_ui::widgets::{self, mix};
use tiny_skia::{PathBuilder, Transform};

use crate::focus::Area;
use super::basket::aside_width;
use super::{Cmd, Size, Ui};
use crate::compat::Level;
use crate::library::Game;

#[derive(Debug, Clone)]
pub struct Row<'a> {
    pub game: &'a Game,
    pub system: &'a str,
    pub fruit_name: &'a str,
    pub level: Option<Level>,
    pub last: Option<SystemTime>,
    pub secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dump<'a> {
    /// The fruit's feed entry has no dump list.
    NoList,
    Checking,
    Verified(&'a str),
    NoMatch(&'a str),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayState {
    Play,
    Continue,
    /// A game is running; nothing else starts until it exits.
    Running(String),
}

pub struct GameDetail<'a> {
    pub row: Row<'a>,
    pub dump: Dump<'a>,
    pub play: PlayState,
    pub saves: usize,
    pub file: String,
}

pub enum Empty {
    /// Something to list.
    No,
    NoGames,
    NoMatch(String),
}

pub struct LibraryView<'a> {
    /// (fruit id or None for All, label, count).
    pub chips: Vec<(Option<&'a str>, &'a str, usize)>,
    pub filter: Option<&'a str>,
    pub az: bool,
    pub list: bool,
    pub continue_rows: Vec<Row<'a>>,
    pub rows: Vec<Row<'a>>,
    /// "All games" or "Strawberry games".
    pub heading: String,
    pub selected: Option<&'a Path>,
    pub detail: Option<GameDetail<'a>>,
    pub empty: Empty,
    pub sheet: bool,
    pub scroll: f32,
    pub aside_scroll: f32,
}

/// The covers' colours, picked by a hash of the title: the mocks' set.
const COVERS: [Rgb; 10] = [
    hex(0x3E5E4A),
    hex(0x3E5A7E),
    hex(0x7A4A3A),
    hex(0x5A4A72),
    hex(0x6E6E34),
    hex(0x2E4A5A),
    hex(0x8E5A2E),
    hex(0x6A3446),
    hex(0x3E6A4E),
    hex(0x4A5470),
];
const COVER_TEXT: Rgb = hex(0xF2EDE2);

pub fn cover_color(title: &str) -> Rgb {
    // FNV-1a: stable across runs and platforms.
    let h = title.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    COVERS[(h % COVERS.len() as u64) as usize]
}

pub fn draw(ui: &mut Ui, v: &LibraryView, top: f32, bottom: f32) {
    let (x0, w) = (ui.pad_x(), ui.w());
    if let Empty::NoGames = v.empty {
        no_games(ui, x0, top + 22.0, w - 2.0 * x0);
        return;
    }
    let aside_w = aside_width(ui.size);
    let list_r = if aside_w > 0.0 { w - aside_w - x0 } else { w - x0 };
    let list_w = list_r - x0;
    let sheet_open = v.sheet && ui.size == Size::Narrow && v.detail.is_some();

    if ui.input.wheel != 0.0 && ui.input.mouse.0 < list_r + x0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom && !sheet_open {
        ui.emit(Cmd::Scroll(-ui.input.wheel * 40.0));
    }
    ui.clip(0.0, top, list_r + x0, bottom - top);
    let mut y = top + 20.0 - v.scroll;
    y = toolbar(ui, v, x0, y, list_w) + 30.0;

    if let Empty::NoMatch(q) = &v.empty {
        let st = Style::reading(16.0).color(ui.pal.fg);
        ui.cv.text(x0, y, &format!("No games match “{q}”."), &st);
        ui.unclip();
        return;
    }

    if !v.continue_rows.is_empty() {
        y = ui.section(x0, y, list_w, "Continue", Some("last played")) + 12.0;
        y = continue_cards(ui, v, x0, y, list_w) + 30.0;
    }
    let note = if v.az { "a to z" } else { "most recent first" };
    y = ui.section(x0, y, list_w, &format!("{} · {}", v.heading, v.rows.len()), Some(note));
    y = if v.list { list(ui, v, x0, y, list_w) } else { covers(ui, v, x0, y + 14.0, list_w) };
    y += 24.0;
    ui.unclip();
    ui.emit(Cmd::ScrollMax((y + v.scroll - bottom).max(0.0)));

    let Some(d) = &v.detail else { return };
    if aside_w > 0.0 {
        let ax = w - aside_w;
        let panel = ui.panel();
        ui.cv.fill_rect(ax, top, aside_w, bottom - top, panel);
        ui.cv.vrule(ax, top, bottom, 1.0, ui.pal.line);
        let pad = if ui.size == Size::Regular { 33.0 } else { 25.0 };
        ui.area = Area::Aside;
        aside(ui, d, ax + pad, top, aside_w - pad - x0, bottom, v.aside_scroll);
        ui.area = Area::Main;
    } else if sheet_open {
        ui.area = Area::Aside;
        let sy = top + 140.0;
        let panel = ui.panel();
        ui.cv.fill_rect(0.0, sy, w, bottom - sy, panel);
        ui.cv.fill_rect(0.0, sy, w, 2.0, ui.pal.fg);
        ui.cv.fill_rect(x0, sy + 30.0, 44.0, 4.0, ui.pal.line);
        let cw = ui.cv.measure("Close", &Style::interface(13.0));
        let fg = ui.pal.fg;
        let (_, close) = ui.link(w - x0 - cw, sy + 24.0, "Close", fg);
        if close || ui.pressed(minifb::Key::Escape) {
            ui.emit(Cmd::CloseSheet);
        }
        ui.area = Area::Aside;
        aside(ui, d, x0, sy + 44.0, w - 2.0 * x0, bottom, v.aside_scroll);
        ui.area = Area::Main;
    }
}

fn no_games(ui: &mut Ui, x: f32, y: f32, w: f32) {
    let h = 226.0;
    ui.cv.dashed_rect(x, y, w, h, 1.0, 4.0, ui.pal.line);
    ui.art.mark(ui.cv, ui.night, x + 28.0, y + 70.0, 84.0);
    let tx = x + 149.0;
    ui.cv.text(tx, y + 28.0, "No games yet", &Style::display(30.0).upper().color(ui.pal.fg));
    let st = Style::reading(16.0).color(ui.muted());
    let text = "Install a fruit from the basket, then put your games in its games/ folder. They show up here on their own.";
    let ph = ui.cv.paragraph(tx, y + 72.0, (w - 180.0).min(390.0), text, &st, 1.5, 4);
    let (_, open) = ui.small_button_filled(tx, y + 84.0 + ph, "Open the basket");
    if open {
        ui.emit(Cmd::Tab(super::Tab::Basket));
    }
}

fn toolbar(ui: &mut Ui, v: &LibraryView, x: f32, y: f32, w: f32) -> f32 {
    // Chips, left.
    let mut cx = x;
    for (id, label, n) in &v.chips {
        let on = v.filter == *id;
        let st = Style::interface_bold(13.0).color(if on { ui.pal.bg } else { ui.pal.fg });
        let ns = Style::interface(12.0).color(if on { mix(ui.pal.bg, ui.pal.fg, 0.4) } else { ui.muted() });
        let lw = ui.cv.measure(label, &st);
        let nw = ui.cv.measure(&n.to_string(), &ns);
        let cw = (lw + nw + 30.0).round();
        if on {
            ui.cv.fill_rect(cx, y, cw, 34.0, ui.pal.fg);
        }
        ui.cv.stroke_rect(cx, y, cw, 34.0, 1.5, ui.pal.fg);
        ui.cv.text(cx + 13.0, y + 9.0, label, &st);
        ui.cv.text(cx + 17.0 + lw, y + 10.0, &n.to_string(), &ns);
        if ui.hot(label, cx, y, cw, 34.0) && !on {
            ui.emit(Cmd::LibFilter(id.map(str::to_string)));
        }
        cx += cw + 6.0;
    }

    // Toggles and Add folder…, right; Add folder… drops to its own row
    // when there is no room.
    let add_w = ui.small_button_width("Add folder…");
    let sort_w = seg_width(ui, &["Recent", "A–Z"]);
    let view_w = seg_width(ui, &["Covers", "List"]);
    let all = sort_w + 14.0 + view_w + 14.0 + add_w;
    let one_row = cx + 20.0 + all <= x + w;
    let mut rx = x + w - if one_row { all } else { sort_w + 14.0 + view_w };
    let ty = if one_row || ui.size != Size::Narrow { y } else { y + 44.0 };
    let (_, hit) = ui.segmented(rx, ty, &["Recent", "A–Z"], v.az as usize);
    if let Some(i) = hit {
        ui.emit(Cmd::LibSort(i == 1));
    }
    rx += sort_w + 14.0;
    let (_, hit) = ui.segmented(rx, ty, &["Covers", "List"], v.list as usize);
    if let Some(i) = hit {
        ui.emit(Cmd::LibView(i == 1));
    }
    let (ax, ay, end) = if one_row { (rx + view_w + 14.0, y, y + 34.0) } else { (x, ty + 44.0, ty + 78.0) };
    let (_, add) = ui.small_button(ax, ay, "Add folder…");
    if add {
        ui.emit(Cmd::AddFolder);
    }
    end
}

fn seg_width(ui: &mut Ui, options: &[&str]) -> f32 {
    options.iter().map(|o| (ui.cv.measure(o, &Style::interface_bold(13.0)) + 26.0).round() - 1.5).sum::<f32>() + 1.5
}

fn columns(ui: &Ui) -> usize {
    match ui.size {
        Size::Regular => 3,
        Size::Compact | Size::Narrow => 2,
    }
}

fn continue_cards(ui: &mut Ui, v: &LibraryView, x: f32, y: f32, w: f32) -> f32 {
    let cols = columns(ui);
    let gap = 10.0;
    let cw = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let h = 96.0;
    for (i, r) in v.continue_rows.iter().take(cols).enumerate() {
        let cx = x + i as f32 * (cw + gap);
        let on = v.selected == Some(r.game.path.as_path());
        if on {
            let panel = ui.panel();
            ui.cv.fill_rect(cx, y, cw, h, panel);
            ui.cv.stroke_rect(cx, y, cw, h, 2.0, ui.pal.fg);
        } else {
            ui.cv.stroke_rect(cx, y, cw, h, 1.0, ui.pal.line);
        }
        cover(ui, r, cx + 11.0, y + 12.0, 54.0, 72.0, false);
        let tx = cx + 80.0;
        let tw = cw - 80.0 - 66.0;
        let st = Style::display(17.0).upper().color(ui.pal.fg);
        let t = ui.cv.fonts.ellipsize(&st, &r.game.title, tw);
        ui.cv.text(tx, y + 28.0, &t, &st);
        let when = r.last.map(basket_ui::fmt::fmt_when).unwrap_or_default();
        let mono = Style::data(12.0).color(ui.muted());
        let sub = ui.cv.fonts.ellipsize(&mono, &format!("{} · {when}", r.fruit_name), tw);
        ui.cv.text(tx, y + 52.0, &sub, &mono);
        let (bx, by) = (cx + cw - 55.0, y + 26.0);
        play_button(ui, bx, by, 44.0);
        if ui.hot("Play", bx, by, 44.0, 44.0) {
            ui.emit(Cmd::Play(r.game.path.clone()));
        } else if ui.clicked(cx, y, cw, h) {
            ui.emit(Cmd::SelectGame(r.game.path.clone()));
        }
    }
    y + h
}

fn play_button(ui: &mut Ui, x: f32, y: f32, s: f32) {
    ui.cv.fill_rect(x, y, s, s, ui.pal.fg);
    let mut pb = PathBuilder::new();
    let (cx, cy) = (x + s / 2.0 + 1.0, y + s / 2.0);
    pb.move_to(cx - 4.0, cy - 5.5);
    pb.line_to(cx + 5.0, cy);
    pb.line_to(cx - 4.0, cy + 5.5);
    pb.close();
    if let Some(p) = pb.finish() {
        ui.cv.fill_path(&p, ui.pal.bg, Transform::identity());
    }
}

/// A placeholder cover: the title's colour with diagonal stripes, the
/// system top left and (large covers) the title bottom left.
pub(crate) fn cover(ui: &mut Ui, r: &Row, x: f32, y: f32, w: f32, h: f32, label: bool) {
    let c = cover_color(&r.game.title);
    ui.cv.fill_rect(x, y, w, h, c);
    let stripe = mix(c, [0, 0, 0], 0.22);
    // Lines px + py = k, clipped to the box by hand.
    let mut k = x + y + 14.0;
    while k < x + y + w + h {
        let lo = (k - y - h).max(x);
        let hi = (k - y).min(x + w);
        if hi > lo {
            ui.cv.line(lo, k - lo, hi, k - hi, 1.0, stripe);
        }
        k += 22.0;
    }
    if label {
        let sys = Style::data(10.0).color(COVER_TEXT);
        ui.cv.text(x + 10.0, y + 11.0, r.system, &sys);
        let st = Style::display(15.0).upper().color(COVER_TEXT);
        let lines = ui.cv.fonts.wrap(&st, &r.game.title, w - 20.0, 3);
        let n = lines.len() as f32;
        for (i, l) in lines.iter().enumerate() {
            ui.cv.text(x + 10.0, y + h - 22.0 - (n - 1.0 - i as f32) * 18.0, l, &st);
        }
    }
}

pub(crate) fn squares(ui: &mut Ui, x: f32, y: f32, level: Option<Level>) -> f32 {
    let filled = level.map_or(0, Level::squares);
    for i in 0..4 {
        let sx = x + i as f32 * 11.0;
        if i < filled {
            ui.cv.fill_rect(sx, y, 8.0, 8.0, ui.pal.fg);
        } else {
            ui.cv.stroke_rect(sx, y, 8.0, 8.0, 1.0, ui.pal.fg);
        }
    }
    48.0
}

pub(crate) fn when(last: Option<SystemTime>) -> String {
    last.map(basket_ui::fmt::fmt_when).unwrap_or_else(|| "never".to_string())
}

pub(crate) fn level_label(level: Option<Level>) -> &'static str {
    level.map_or("Not tested", Level::label)
}

fn covers(ui: &mut Ui, v: &LibraryView, x: f32, y: f32, w: f32) -> f32 {
    let cols: usize = match ui.size {
        Size::Regular | Size::Compact => 5,
        Size::Narrow => 4,
    };
    let gap = 16.0;
    let cw = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let ch = (cw * 4.0 / 3.0).round();
    let row_h = ch + 44.0;
    let mut end = y;
    for (i, r) in v.rows.iter().enumerate() {
        let cx = x + (i % cols) as f32 * (cw + gap);
        let cy = y + (i / cols) as f32 * row_h;
        end = cy + row_h;
        if cy > ui.h() || cy + row_h < 0.0 {
            continue;
        }
        cover(ui, r, cx, cy, cw, ch, true);
        if v.selected == Some(r.game.path.as_path()) {
            ui.cv.stroke_rect(cx - 3.0, cy - 3.0, cw + 6.0, ch + 6.0, 2.5, ui.pal.fg);
            ui.cv.stroke_rect(cx, cy, cw, ch, 2.0, ui.pal.bg);
        }
        let sw = squares(ui, cx, cy + ch + 11.0, r.level);
        let mono = Style::data(12.0).color(ui.muted());
        let line = format!("{} · {}", level_label(r.level), when(r.last));
        let shown = ui.cv.fonts.ellipsize(&mono, &line, cw - sw - 2.0);
        ui.cv.text(cx + sw + 1.0, cy + ch + 8.0, &shown, &mono);
        if ui.clicked(cx, cy, cw, ch + 28.0) {
            ui.emit(Cmd::SelectGame(r.game.path.clone()));
        }
    }
    end
}

fn list(ui: &mut Ui, v: &LibraryView, x: f32, y: f32, w: f32) -> f32 {
    let narrow = ui.size == Size::Narrow;
    // Column starts as fractions of the width: title, system, runs, last played; size is right-aligned.
    let col = |f: f32| x + w * f;
    let (c_sys, c_runs, c_last) = (col(0.42), col(0.57), col(0.74));
    let head = ui.note();
    ui.cv.text(x + 54.0, y + 12.0, "Title", &head);
    ui.cv.text(c_sys, y + 12.0, "System", &head);
    if !narrow {
        ui.cv.text(c_runs, y + 12.0, "Runs", &head);
        ui.cv.text(c_last, y + 12.0, "Last played", &head);
    }
    ui.cv.text_right(x + w - 8.0, y + 12.0, "Size", &head);
    let mut ry = y + 34.0;
    ui.cv.hrule(x, x + w, ry, 1.0, ui.pal.line);
    let title_st = Style::display(16.0).upper().color(ui.pal.fg);
    let mono = Style::data(13.0).color(ui.pal.fg);
    let muted = Style::data(13.0).color(ui.muted());
    for r in &v.rows {
        let h = 49.0;
        if ry + h >= 0.0 && ry <= ui.h() {
            if v.selected == Some(r.game.path.as_path()) {
                let panel = ui.panel();
                ui.cv.fill_rect(x, ry + 1.0, w, h - 1.0, panel);
            }
            cover(ui, r, x + 8.0, ry + 9.0, 24.0, 32.0, false);
            let t = ui.cv.fonts.ellipsize(&title_st, &r.game.title, c_sys - x - 70.0);
            ui.cv.text(x + 54.0, ry + 15.0, &t, &title_st);
            ui.cv.text(c_sys, ry + 17.0, r.system, &mono);
            if !narrow {
                let sw = squares(ui, c_runs, ry + 21.0, r.level);
                ui.cv.text(c_runs + sw + 1.0, ry + 15.0, level_label(r.level), &Style::interface(13.0).color(ui.pal.fg));
                ui.cv.text(c_last, ry + 17.0, &when(r.last), &muted);
            }
            ui.cv.text_right(x + w - 8.0, ry + 17.0, &fmt_size(r.game.size), &muted);
            if ui.clicked(x, ry, w, h) {
                ui.emit(Cmd::SelectGame(r.game.path.clone()));
            }
        }
        ry += h;
        ui.cv.hrule(x, x + w, ry, 1.0, ui.pal.line);
    }
    ry
}

fn aside(ui: &mut Ui, d: &GameDetail, x: f32, top: f32, w: f32, bottom: f32, scroll: f32) {
    if ui.input.wheel != 0.0 && ui.input.mouse.0 >= x - 30.0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom {
        ui.emit(Cmd::AsideScroll(-ui.input.wheel * 40.0));
    }
    ui.clip(x - 40.0, top, w + 80.0, bottom - top);
    let r = &d.row;
    let mut y = top + 26.0 - scroll;

    // Cover, and how far the game gets.
    cover(ui, r, x, y, 120.0, 160.0, false);
    ui.cv.text(x + 10.0, y + 11.0, r.system, &Style::data(10.0).color(COVER_TEXT));
    let lx = x + 138.0;
    let sw = squares(ui, lx, y + 106.0, r.level);
    ui.cv.text(lx + sw + 3.0, y + 102.0, level_label(r.level), &Style::interface_bold(14.0).color(ui.pal.fg));
    let source = match r.level {
        Some(_) => format!("from {}’s compatibility list", r.fruit_name),
        None => format!("not on {}’s compatibility list", r.fruit_name),
    };
    let mono = Style::data(12.0).color(ui.muted());
    ui.cv.paragraph(lx, y + 126.0, w - 138.0, &source, &mono, 1.4, 2);
    y += 184.0;

    let note = ui.note();
    ui.cv.text(x, y, &format!("{} · {}", r.system, r.fruit_name), &note);
    let (size, lines) = ui.cv.fonts.fit_display(Style::display(38.0).upper(), &r.game.title, w, 2, &[38.0, 32.0, 28.0, 24.0]);
    let st = Style::display(size).upper().color(ui.pal.fg);
    let mut ty = y + 18.0;
    for l in &lines {
        ui.cv.text(x, ty, l, &st);
        ty += size * 1.05;
    }
    y = ty + 14.0;

    // Play / Continue / Running.
    let h = 54.0;
    match &d.play {
        PlayState::Running(fruit) => {
            ui.cv.stroke_rect(x, y, w, h, 1.5, ui.pal.line);
            let st = Style::interface_bold(17.0).color(ui.muted());
            ui.cv.text_center(x + w / 2.0, y + 16.0, &format!("Running in {fruit}"), &st);
        }
        p => {
            let label = if *p == PlayState::Continue { "Continue" } else { "Play" };
            ui.cv.fill_rect(x, y, w, h, ui.pal.fg);
            let st = Style::interface_bold(18.0).color(ui.pal.bg);
            let ks = Style::data(13.0).color(widgets::mix(ui.pal.bg, ui.pal.fg, 0.55));
            let lw = ui.cv.measure(label, &st);
            let lx = x + (w - lw - 20.0) / 2.0;
            ui.cv.text(lx, y + 16.0, label, &st);
            ui.cv.text(lx + lw + 12.0, y + 20.0, "Z", &ks);
            if ui.hot(label, x, y, w, h) {
                ui.emit(Cmd::Play(r.game.path.clone()));
            }
        }
    }
    y += h + 10.0;

    // Saves · N, Show file, Remove.
    let saves = format!("Saves · {}", d.saves);
    let (sw, hit) = if d.saves > 0 { ui.small_button(x, y + 3.0, &saves) } else { (ui.small_button_off(x, y + 3.0, &saves), false) };
    if hit {
        ui.emit(Cmd::ShowSaves(r.game.path.clone()));
    }
    let (_, show) = ui.small_button(x + sw + 16.0, y + 3.0, "Show file");
    if show {
        ui.emit(Cmd::ShowFile(r.game.path.clone()));
    }
    let rw = ui.cv.measure("Remove", &Style::interface(13.0));
    let muted = ui.muted();
    let (_, remove) = ui.link(x + w - rw, y + 12.0, "Remove", muted);
    if remove {
        ui.emit(Cmd::RemoveGame(r.game.path.clone()));
    }
    y += 58.0;
    ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);

    // Last played · Play time.
    y += 16.0;
    let half = (w / 2.0).floor();
    ui.cv.text(x, y, "Last played", &note);
    ui.cv.text(x + half + 20.0, y, "Play time", &note);
    let data = Style::data(13.0).color(ui.pal.fg);
    ui.cv.text(x, y + 20.0, &when(r.last), &data);
    let played = if r.secs == 0 { "—".to_string() } else { fmt_play(r.secs) };
    ui.cv.text(x + half + 20.0, y + 20.0, &played, &data);
    y += 48.0;
    ui.cv.hrule(x, x + half, y, 1.0, ui.pal.line);
    ui.cv.hrule(x + half + 20.0, x + w, y, 1.0, ui.pal.line);

    // Dump.
    y += 16.0;
    ui.cv.text(x, y, "Dump", &note);
    let reading = Style::reading(15.0).color(ui.pal.fg);
    let ry = y + 20.0;
    match d.dump {
        Dump::Verified(db) => {
            ui.check_mark(x + 1.0, ry + 3.0, ui.pal.fg);
            ui.cv.text(x + 24.0, ry, &format!("Verified · matches {db}"), &reading);
        }
        Dump::NoMatch(db) => {
            ui.cv.circle(x + 7.0, ry + 9.0, 4.0, ui.pal.spot);
            ui.cv.text(x + 24.0, ry, &format!("No match in {db} · may not run right"), &reading);
        }
        Dump::Checking => {
            ui.cv.text(x, ry, "Checking…", &reading.color(ui.muted()));
        }
        Dump::NoList => {
            ui.cv.text(x, ry, "No dump list for this fruit yet", &reading.color(ui.muted()));
        }
    }
    y = ry + 32.0;
    ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);

    // File.
    y += 16.0;
    ui.cv.text(x, y, "File", &note);
    let fh = file_lines(ui, x, y + 20.0, w, &d.file);
    y += 20.0 + fh + 12.0;
    ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);
    y += 30.0;

    ui.unclip();
    ui.emit(Cmd::AsideScrollMax((y + scroll - bottom).max(0.0)));
}

/// A path in the data face, broken at any character to fit (paths have
/// few spaces to wrap at).
fn file_lines(ui: &mut Ui, x: f32, y: f32, w: f32, path: &str) -> f32 {
    let st = Style::data(13.0).color(ui.pal.fg);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in path.chars() {
        cur.push(ch);
        if ui.cv.measure(&cur, &st) > w {
            cur.pop();
            lines.push(std::mem::take(&mut cur));
            cur.push(ch);
        }
    }
    lines.push(cur);
    for (i, l) in lines.iter().take(4).enumerate() {
        ui.cv.text(x, y + i as f32 * 20.0, l, &st);
    }
    lines.len().min(4) as f32 * 20.0
}
