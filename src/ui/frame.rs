//! The window frame every tab shares: header (lockup, tabs, FIND, Update
//! all, couch button), the banner (a launcher update, a ripe fruit), and
//! the footer.

use basket_ui::text::Style;
use minifb::Key;

use crate::focus::Area;
use super::{Cmd, Size, Tab, Ui};

/// What the frame shows; borrowed from the app each frame.
pub struct FrameView<'a> {
    pub tab: Tab,
    pub find: &'a str,
    pub find_focused: bool,
    /// Fruits with an update ready and not already busy or queued.
    pub updates: usize,
    /// Downloads badge: busy + queued + failed.
    pub downloads: usize,
    /// A launcher update staged, or a watched fruit ripe.
    pub banner: Option<&'a Banner>,
    /// Footer key hints: (key, verb).
    pub hints: &'a [(&'a str, &'a str)],
    /// Footer status line: progress, a running game, or the basket path.
    pub status: &'a str,
    pub controller: bool,
}

/// The narrow header's second row: FIND and the couch button.
const NARROW_ROW2: f32 = 68.0;

/// Draw the header. Returns the y where the body starts.
pub fn header(ui: &mut Ui, v: &FrameView) -> f32 {
    ui.area = Area::Fixed;
    let (w, x0) = (ui.w(), ui.pad_x());
    let fg = ui.pal.fg;
    ui.cv.clear(ui.pal.bg);

    // Lockup: mark + wordmark, 28 high.
    let top = 20.0;
    let mark_w = ui.art.mark(ui.cv, ui.night, x0, top + 1.0, 26.0);
    let word = Style::display(23.0).tracking(0.6).color(fg);
    let lockup_w = mark_w + 8.0 + ui.cv.text(x0 + mark_w + 8.0, top + 2.0, "Fruit Basket", &word);

    // Couch-mode button, right edge; on the FIND row when narrow.
    let narrow = ui.size == Size::Narrow;
    let cb = 44.0;
    let cx = w - x0 - cb;
    let cy = if narrow { NARROW_ROW2 } else { top };
    if v.controller {
        ui.cv.fill_rect(cx, cy, cb, cb, fg);
    }
    ui.cv.stroke_rect(cx, cy, cb, cb, 1.5, fg);
    gamepad_glyph(ui, cx + cb / 2.0, cy + cb / 2.0, if v.controller { ui.pal.bg } else { fg });
    if ui.hot("Couch mode", cx, cy, cb, cb) {
        ui.emit(Cmd::Couch);
    }
    let mut right = if narrow { w - x0 } else { cx - 28.0 };

    // Update all · N.
    if v.updates > 0 && ui.size != Size::Narrow {
        let label = if ui.size == Size::Compact { format!("Update · {}", v.updates) } else { format!("Update all · {}", v.updates) };
        let st = Style::interface_bold(13.0).upper().tracking(1.8).color(fg);
        let bw = ui.cv.measure(&label, &st) + 56.0;
        let bx = right - bw;
        ui.cv.stroke_rect(bx, top, bw, cb, 2.0, fg);
        ui.cv.circle(bx + 20.0, top + cb / 2.0, 4.0, ui.pal.spot);
        ui.cv.text(bx + 33.0, top + 14.0, &label, &st);
        if ui.hot("Update all", bx, top, bw, cb) {
            ui.emit(Cmd::UpdateAll);
        }
        right = bx - 28.0;
    }

    // Tabs, after the lockup.
    let gap = match ui.size {
        Size::Regular => 26.0,
        Size::Compact => 20.0,
        Size::Narrow => 14.0,
    };
    let tab_st = |on: bool, ui: &Ui| Style::interface_bold(13.0).upper().tracking(2.0).color(if on { ui.pal.fg } else { ui.muted() });
    let mut tx = x0 + lockup_w + if ui.size == Size::Regular { 38.0 } else { 22.0 };
    for tab in Tab::ALL {
        let on = tab == v.tab;
        let st = tab_st(on, ui);
        let mut tw = ui.cv.measure(tab.label(), &st);
        ui.cv.text(tx, top + 13.0, tab.label(), &st);
        if tab == Tab::Downloads && v.downloads > 0 {
            let (bx, by) = (tx + tw + 17.0, top + 21.0);
            ui.cv.circle(bx, by, 9.0, ui.pal.spot);
            let n = Style::data_medium(11.0).color(ui.pal.bg);
            ui.cv.text_center(bx, by - 7.0, &v.downloads.min(9).to_string(), &n);
            tw += 22.0;
        }
        if on {
            ui.cv.fill_rect(tx - 10.0, top + 39.0, tw + 20.0, 3.0, fg);
        }
        if ui.clicked(tx - 10.0, top, tw + 20.0, cb) {
            ui.emit(Cmd::Tab(tab));
        }
        tx += tw + gap;
    }

    // FIND: between the tabs and the buttons, or its own row when narrow.
    let (fx, fy, fw) = if narrow {
        (x0, NARROW_ROW2, cx - 12.0 - x0)
    } else {
        // Its width, but never over the tabs.
        let fw = (if ui.size == Size::Compact { 200.0f32 } else { 240.0 }).min(right - tx - 14.0).max(80.0);
        (right - fw, top, fw)
    };
    let placeholder = if v.tab == Tab::Basket { "Fruit, console or .ext" } else { "Game or console" };
    find_field(ui, fx, fy, fw, v.find, placeholder, v.find_focused);

    let rule_y = if narrow { NARROW_ROW2 + 54.0 } else { top + 58.0 };
    ui.cv.fill_rect(x0, rule_y, w - 2.0 * x0, 3.0, fg);
    let mut body = rule_y + 3.0;

    if let Some(b) = v.banner {
        body = banner(ui, body, b);
    }
    ui.area = Area::Main;
    body
}

fn find_field(ui: &mut Ui, x: f32, y: f32, w: f32, text: &str, placeholder: &str, focused: bool) {
    let fg = ui.pal.fg;
    let mut fx = x;
    if ui.size != Size::Compact {
        let st = Style::interface_bold(13.0).upper().tracking(2.0).color(fg);
        let lw = ui.cv.text(x, y + 13.0, "Find", &st);
        fx = x + lw + 14.0;
    }
    let st = if text.is_empty() { Style::interface(15.0).color(ui.faded()) } else { Style::interface(15.0).color(fg) };
    let shown = if text.is_empty() && !focused { placeholder } else { text };
    let tw = ui.cv.text(fx + 2.0, y + 11.0, shown, &st);
    if focused {
        let caret_x = fx + 2.0 + if text.is_empty() { 0.0 } else { tw } + 1.0;
        ui.cv.fill_rect(caret_x, y + 11.0, 1.5, 19.0, ui.pal.spot);
    }
    ui.cv.fill_rect(x, y + 41.0, w, 2.0, fg);
    if ui.clicked(x, y, w, 44.0) {
        ui.emit(Cmd::FocusFind(true));
    } else if focused && ui.input.clicked {
        ui.emit(Cmd::FocusFind(false));
    }
    if focused {
        let mut s = text.to_string();
        let mut changed = false;
        for &c in &ui.input.chars {
            s.push(c);
            changed = true;
        }
        if ui.input.repeated.contains(&Key::Backspace) {
            changed |= s.pop().is_some();
        }
        if changed {
            ui.emit(Cmd::Find(s));
        }
        if ui.pressed(Key::Escape) || ui.pressed(Key::Enter) {
            ui.emit(Cmd::FocusFind(false));
        }
    }
}

/// The strip under the header: a dot, a bold lead, a sentence, an action
/// and Later. "Launcher v0.5.0 is downloaded and verified…"; "Fig is ripe:
/// v0.1.0 for PS1 is ready to install."
pub struct Banner {
    pub lead: String,
    pub text: String,
    pub action: (String, Cmd),
    pub later: Cmd,
}

/// Draw the banner, as `Launcher-Update.png`: the lead in small capitals,
/// the sentence in the reading face, the action filled and Later as a
/// link. Returns the y below it.
fn banner(ui: &mut Ui, y: f32, b: &Banner) -> f32 {
    let (x0, w) = (ui.pad_x(), ui.w());
    let h = 58.0;
    let panel = ui.panel();
    ui.cv.fill_rect(x0, y, w - 2.0 * x0, h, panel);
    ui.cv.stroke_rect(x0, y, w - 2.0 * x0, h, 1.0, ui.pal.line);
    ui.cv.circle(x0 + 23.0, y + h / 2.0, 4.0, ui.pal.spot);
    let lead = Style::interface_bold(12.0).upper().tracking(2.0).color(ui.pal.fg);
    let lw = ui.cv.text(x0 + 45.0, y + 22.0, &b.lead, &lead);

    let later = Style::interface(14.0);
    let later_w = ui.cv.measure("Later", &later);
    let later_x = w - x0 - 22.0 - later_w;
    let (_, later_hit) = ui.link(later_x, y + 20.0, "Later", ui.pal.fg);
    let act = Style::interface_bold(14.0);
    let aw = (ui.cv.measure(&b.action.0, &act) + 32.0).round();
    let ax = later_x - 26.0 - aw;
    ui.cv.fill_rect(ax, y + 11.0, aw, 36.0, ui.pal.fg);
    ui.cv.text_center(ax + aw / 2.0, y + 20.0, &b.action.0, &act.color(ui.pal.bg));
    let act_hit = ui.hot(&b.action.0, ax, y + 11.0, aw, 36.0);

    let reading = Style::reading(16.0).color(ui.pal.fg);
    let tx = x0 + 45.0 + lw + 18.0;
    if ui.size != Size::Narrow && ui.cv.measure(&b.text, &reading) <= ax - 16.0 - tx {
        ui.cv.text(tx, y + 18.0, &b.text, &reading);
    }
    if later_hit {
        ui.emit(b.later.clone());
    }
    if act_hit {
        ui.emit(b.action.1.clone());
    }
    y + h
}

/// Draw the footer and return the y where it starts (the body ends there).
pub fn footer(ui: &mut Ui, v: &FrameView) -> f32 {
    ui.area = Area::Fixed;
    let (x0, w, h) = (ui.pad_x(), ui.w(), ui.h());
    let top = h - 52.0;
    ui.cv.fill_rect(0.0, top, w, 52.0, ui.pal.bg);
    ui.cv.hrule(x0, w - x0, top, 1.0, ui.pal.line);
    let y = top + 14.0;
    let verb = ui.body();

    // Quit (or Couch mode on a pad) at the far right.
    let (qk, qv) = if ui.pad { ("☰", "Couch mode") } else { ("Esc", "Quit") };
    let qvw = ui.cv.measure(qv, &verb);
    let qkw = ui.key_width(qk);
    let qx = w - x0 - qvw - qkw - 9.0;
    ui.key(qx, y, qk);
    ui.cv.text(qx + qkw + 9.0, y + 3.0, qv, &verb);

    let limit = if ui.size == Size::Narrow { 2 } else { v.hints.len() };
    let mut x = x0;
    for (key, verb_text) in v.hints.iter().take(limit) {
        let kw = ui.key(x, y, key);
        x += kw + 9.0;
        x += ui.cv.text(x, y + 3.0, verb_text, &verb) + 22.0;
    }

    if ui.size != Size::Narrow && !v.status.is_empty() {
        let st = Style::data(12.0).color(ui.muted());
        ui.cv.text_right(qx - 20.0, y + 4.0, v.status, &st);
    }
    top
}

/// The couch-mode glyph: a gamepad outline with a d-pad and two buttons.
fn gamepad_glyph(ui: &mut Ui, cx: f32, cy: f32, c: basket_ui::tokens::Rgb) {
    ui.cv.stroke_round_rect(cx - 11.0, cy - 6.5, 22.0, 13.0, [6.5; 4], 1.6, c);
    ui.cv.fill_rect(cx - 7.5, cy - 0.75, 6.0, 1.5, c);
    ui.cv.fill_rect(cx - 5.25, cy - 3.0, 1.5, 6.0, c);
    ui.cv.circle(cx + 4.0, cy + 1.0, 1.3, c);
    ui.cv.circle(cx + 7.0, cy - 1.5, 1.3, c);
}
