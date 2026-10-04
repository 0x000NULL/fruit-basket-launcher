//! Couch mode: the TV view from `Couch.png`, `Couch-Strawberry.png` and
//! `Couch-Saves.png`. Always night, laid out at the mocks' 1280×720 and
//! scaled to the screen. The pad drives it (LB/RB systems, ←/→ games,
//! A Continue, X Saves, Y Details, B back, ☰ desktop); clicks work too.

use basket_ui::text::Style;
use tiny_skia::Pixmap;

use super::library::{cover, level_label, picture, squares, when, Dump, GameDetail, Row};
use super::{Cmd, Ui};

pub enum Panel<'a> {
    /// Continue · Saves · Details.
    Buttons,
    /// The game's saves, newest first, then START FRESH. `pics` has a
    /// picture per save when the fruit writes them, else is empty.
    Saves { rows: Vec<(String, String)>, pics: Vec<Option<&'a Pixmap>>, pick: usize, note: Option<String> },
    Details(GameDetail<'a>),
}

pub struct CouchView<'a> {
    /// "All", then each installed fruit's name.
    pub systems: Vec<&'a str>,
    pub system: usize,
    /// "All games · 12", "Strawberry · 6".
    pub heading: String,
    pub rows: Vec<Row<'a>>,
    pub pick: usize,
    pub controller: Option<&'a str>,
    pub panel: Panel<'a>,
    pub saves: usize,
    /// The fruit a game is running in.
    pub playing: Option<&'a str>,
    /// The slot A loads from the buttons (the newest save), if any.
    pub resume: Option<u8>,
    /// The picked save's picture, shown in the cover's place.
    pub hero: Option<&'a Pixmap>,
    pub hints: Vec<(&'static str, &'static str)>,
}

/// Mock units to pixels.
struct Scale {
    s: f32,
    oy: f32,
}

impl Scale {
    fn y(&self, y: f32) -> f32 {
        self.oy + y * self.s
    }
    fn v(&self, v: f32) -> f32 {
        v * self.s
    }
}

pub fn draw(ui: &mut Ui, v: &CouchView) {
    let (w, h) = (ui.w(), ui.h());
    let s = (w / 1280.0).min(h / 720.0);
    let k = Scale { s, oy: ((h - 720.0 * s) / 2.0).max(0.0) };
    ui.cv.clear(ui.pal.bg);
    let x0 = k.v(56.0);

    header(ui, v, &k, x0);
    let Some(r) = v.rows.get(v.pick) else {
        let big = Style::display(k.v(54.0)).upper().color(ui.pal.fg);
        ui.cv.text(x0, k.y(160.0), "No games yet", &big);
        let st = Style::reading(k.v(20.0)).color(ui.muted());
        ui.cv.text(x0, k.y(240.0), "Put games in a fruit's games/ folder, or add a folder in desktop mode.", &st);
        footer(ui, v, &k, x0);
        return;
    };

    // The hero: cover, eyebrow, title, compat, then the panel.
    let (cw, ch) = (k.v(228.0), k.v(304.0));
    match v.hero {
        Some(pm) => cover(ui, &Row { picture: Some(pm), ..r.clone() }, x0, k.y(108.0), cw, ch, false),
        None => cover(ui, r, x0, k.y(108.0), cw, ch, false),
    }
    ui.cv.text(x0 + k.v(14.0), k.y(122.0), r.system, &Style::data(k.v(12.0)).color(basket_ui::tokens::hex(0xF2EDE2)));
    let tx = x0 + cw + k.v(48.0);
    let tw = w - x0 - tx;
    let eyebrow = Style::interface_bold(k.v(14.0)).upper().tracking(k.v(2.4)).color(ui.muted());
    ui.cv.text(tx, k.y(116.0), &format!("{} · {}", r.system, r.fruit_name), &eyebrow);
    let sizes = [72.0, 60.0, 48.0, 40.0].map(|z| k.v(z));
    let (tsize, lines) = ui.cv.fonts.fit_display(Style::display(sizes[0]).upper(), &r.game.title, tw, 1, &sizes);
    let title = Style::display(tsize).upper().color(ui.pal.fg);
    if let Some(line) = lines.first() {
        ui.cv.text(tx, k.y(140.0) + (sizes[0] - tsize) * 0.6, line, &title);
    }
    let sy = k.y(232.0);
    let sq = squares_scaled(ui, tx, sy + k.v(2.0), r, &k);
    let lx = tx + sq + k.v(10.0);
    let lw = ui.cv.text(lx, sy - k.v(2.0), level_label(r.level), &Style::interface(k.v(17.0)).color(ui.pal.fg));
    let played = match r.last {
        Some(_) => format!("last played {} · {} played", when(r.last), basket_ui::fmt::fmt_play(r.secs)),
        None => "never played".to_string(),
    };
    ui.cv.text(lx + lw + k.v(22.0), sy, &played, &Style::data(k.v(15.0)).color(ui.muted()));

    let py = k.y(276.0);
    if let Some(name) = v.playing {
        let st = Style::display(k.v(28.0)).upper().color(ui.pal.fg);
        ui.cv.text(tx, py + k.v(8.0), &format!("Playing in {name}"), &st);
        let note = Style::reading(k.v(18.0)).color(ui.muted());
        ui.cv.text(tx, py + k.v(52.0), "The launcher waits here until you quit the game.", &note);
    } else {
        match &v.panel {
            Panel::Buttons => buttons(ui, v, &k, tx, py),
            Panel::Saves { rows, pics, pick, note } => saves(ui, rows, pics, *pick, note.as_deref(), &k, tx, k.y(271.0)),
            Panel::Details(d) => details(ui, d, &k, tx, py, tw),
        }
    }

    strip(ui, v, &k, x0);
    footer(ui, v, &k, x0);
}

fn squares_scaled(ui: &mut Ui, x: f32, y: f32, r: &Row, k: &Scale) -> f32 {
    if k.s <= 1.05 {
        return squares(ui, x, y, r.level);
    }
    let filled = r.level.map_or(0, crate::compat::Level::squares);
    let (side, step) = (k.v(11.0), k.v(15.0));
    for i in 0..4 {
        let sx = x + i as f32 * step;
        if i < filled {
            ui.cv.fill_rect(sx, y, side, side, ui.pal.fg);
        } else {
            ui.cv.stroke_rect(sx, y, side, side, 1.5, ui.pal.fg);
        }
    }
    4.0 * step
}

fn header(ui: &mut Ui, v: &CouchView, k: &Scale, x0: f32) {
    let fg = ui.pal.fg;
    let mark_w = ui.art.mark(ui.cv, true, x0, k.y(38.0), k.v(26.0));
    let word = Style::display(k.v(21.0)).upper().tracking(k.v(0.4)).color(fg);
    let lw = mark_w + k.v(8.0) + ui.cv.text(x0 + mark_w + k.v(8.0), k.y(40.0), "Fruit Basket", &word);

    let mut x = x0 + lw + k.v(36.0);
    let ty = k.y(42.0);
    x += pad_key(ui, x, ty - k.v(2.0), "LB", k) + k.v(20.0);
    let tab = Style::interface_bold(k.v(15.0)).upper().tracking(k.v(2.4));
    for (i, name) in v.systems.iter().enumerate() {
        let on = i == v.system;
        let st = tab.color(if on { fg } else { ui.muted() });
        let tw = ui.cv.text(x, ty, name, &st);
        if on {
            ui.cv.fill_rect(x - k.v(14.0), k.y(71.0), tw + k.v(28.0), k.v(3.0), fg);
        }
        if ui.clicked(x - k.v(10.0), ty - k.v(10.0), tw + k.v(20.0), k.v(40.0)) && !on {
            ui.emit(Cmd::CouchSystem(i));
        }
        x += tw + k.v(34.0);
    }
    pad_key(ui, x - k.v(14.0), ty - k.v(2.0), "RB", k);

    let name = v.controller.unwrap_or("No controller");
    let st = Style::data(k.v(14.0)).color(if v.controller.is_some() { ui.muted() } else { ui.faded() });
    let nw = ui.cv.text_right(ui.w() - x0, k.y(44.0), name, &st);
    if v.controller.is_some() {
        ui.cv.circle(ui.w() - x0 - nw - k.v(16.0), k.y(52.0), k.v(4.0), fg);
    }
}

/// A round pad keycap, `label` centred; returns its width.
fn pad_key(ui: &mut Ui, x: f32, y: f32, label: &str, k: &Scale) -> f32 {
    let st = Style::interface_bold(k.v(11.0)).color(ui.pal.fg);
    let tw = ui.cv.measure(label, &st);
    let h = k.v(26.0);
    let w = if label.chars().count() > 1 { tw + k.v(16.0) } else { h };
    ui.cv.stroke_round_rect(x, y, w, h, [k.v(7.0); 4], 1.5, ui.pal.fg);
    ui.cv.text(x + (w - tw) / 2.0, y + k.v(6.0), label, &st);
    w
}

/// A circled pad button letter, as on the buttons and in the footer.
fn pad_circle(ui: &mut Ui, cx: f32, cy: f32, label: &str, k: &Scale, c: basket_ui::tokens::Rgb) {
    let r = k.v(13.0);
    ui.cv.stroke_circle(cx, cy, r, 1.5, c);
    let st = Style::interface_bold(k.v(12.0)).color(c);
    ui.cv.text_center(cx, cy - k.v(7.0), label, &st);
}

fn buttons(ui: &mut Ui, v: &CouchView, k: &Scale, x: f32, y: f32) {
    let bh = k.v(60.0);
    let st = Style::interface_bold(k.v(21.0));
    let saves = format!("Saves · {}", v.saves);
    let go = match v.resume {
        Some(n) => format!("Continue · Slot {n}"),
        None => "Play".to_string(),
    };
    let items: [(&str, &str, Cmd, bool); 3] = [("A", &go, Cmd::CouchContinue, true), ("X", &saves, Cmd::CouchSaves, false), ("Y", "Details", Cmd::CouchDetails, false)];
    let mut bx = x;
    for (key, label, cmd, primary) in items {
        let bw = ui.cv.measure(label, &st) + k.v(98.0);
        let (fill, ink) = if primary { (ui.pal.fg, ui.pal.bg) } else { (ui.pal.bg, ui.pal.fg) };
        ui.cv.fill_rect(bx, y, bw, bh, fill);
        ui.cv.stroke_rect(bx, y, bw, bh, 2.0, ui.pal.fg);
        if primary {
            // The focus: what A does, ringed as in the mock.
            ui.cv.stroke_rect(bx - k.v(6.0), y - k.v(6.0), bw + k.v(12.0), bh + k.v(12.0), 3.0, ui.pal.fg);
        }
        pad_circle(ui, bx + k.v(44.0), y + bh / 2.0, key, k, ink);
        ui.cv.text(bx + k.v(70.0), y + k.v(17.0), label, &st.color(ink));
        if ui.clicked(bx, y, bw, bh) {
            ui.emit(cmd);
        }
        bx += bw + k.v(14.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn saves(ui: &mut Ui, rows: &[(String, String)], pics: &[Option<&Pixmap>], pick: usize, note: Option<&str>, k: &Scale, x: f32, y: f32) {
    let rw = k.v(640.0);
    let rh = k.v(58.0);
    let label = Style::display(k.v(21.0)).upper();
    let date = Style::data(k.v(15.0));
    // Show at most four rows, keeping the pick in view.
    let first = pick.saturating_sub(3);
    let mut ry = y;
    for (i, (name, when)) in rows.iter().enumerate().skip(first).take(4) {
        let on = i == pick;
        if on {
            ui.cv.fill_rect(x, ry, rw, rh, ui.pal.fg);
        }
        let ink = if on { ui.pal.bg } else { ui.pal.fg };
        let mut lx = x + k.v(16.0);
        if !pics.is_empty() {
            // A 4:3 thumbnail at the left; Start fresh keeps the space.
            let (tw, th) = (k.v(60.0), k.v(45.0));
            let ty = ry + (rh - th) / 2.0;
            match pics.get(i).copied().flatten() {
                Some(pm) => picture(ui, pm, x + k.v(8.0), ty, tw, th),
                None if i < pics.len() => ui.cv.stroke_rect(x + k.v(8.0), ty, tw, th, 1.0, ui.pal.line),
                None => {}
            }
            lx = x + k.v(8.0) + tw + k.v(16.0);
        }
        ui.cv.text(lx, ry + k.v(18.0), name, &label.color(ink));
        ui.cv.text_right(x + rw - k.v(16.0), ry + k.v(21.0), when, &date.color(if on { ui.pal.bg } else { ui.muted() }));
        ui.cv.hrule(x, x + rw, ry + rh, 1.0, ui.pal.line);
        if ui.clicked(x, ry, rw, rh) {
            ui.emit(if on { Cmd::CouchContinue } else { Cmd::CouchSavePick(i) });
        }
        ry += rh;
    }
    if let Some(note) = note {
        ui.cv.text(x, ry + k.v(14.0), note, &Style::reading(k.v(16.0)).color(ui.muted()));
    }
}

fn details(ui: &mut Ui, d: &GameDetail, k: &Scale, x: f32, y: f32, w: f32) {
    let label = Style::interface_bold(k.v(12.0)).upper().tracking(k.v(2.0)).color(ui.muted());
    let value = Style::data(k.v(16.0)).color(ui.pal.fg);
    let dump = match &d.dump {
        Dump::NoList => "no dump list for this fruit yet".to_string(),
        Dump::Checking => "checking…".to_string(),
        Dump::Verified(db) => format!("verified · matches {db}"),
        Dump::NoMatch(db) => format!("not in {db}"),
    };
    let rows = [("Dump", dump), ("Saves", d.saves.to_string()), ("File", d.file.clone())];
    let mut ry = y;
    for (name, text) in rows {
        ui.cv.text(x, ry, name, &label);
        let t = ui.cv.fonts.ellipsize_middle(&value, &text, w);
        ui.cv.text(x, ry + k.v(20.0), &t, &value);
        ry += k.v(54.0);
    }
}

fn strip(ui: &mut Ui, v: &CouchView, k: &Scale, x0: f32) {
    let note = Style::interface_bold(k.v(13.0)).upper().tracking(k.v(2.4)).color(ui.muted());
    ui.cv.text(x0, k.y(449.0), &v.heading, &note);
    let (cw, ch, step) = (k.v(96.0), k.v(128.0), k.v(116.0));
    let fit = ((ui.w() - x0) / step).floor().max(1.0) as usize;
    // Keep the pick in view, a cover from the right edge.
    let first = (v.pick + 2).saturating_sub(fit).min(v.pick);
    let y = k.y(499.0);
    for (i, r) in v.rows.iter().enumerate().skip(first) {
        // The picked cover is bigger: the ones after it move over.
        let x = x0 + (i - first) as f32 * step + if i > v.pick { k.v(16.0) } else { 0.0 };
        if x > ui.w() {
            break;
        }
        let on = i == v.pick;
        let (bx, by, bw, bh) = if on { (x - k.v(0.0), y - k.v(22.0), k.v(112.0), k.v(150.0)) } else { (x, y, cw, ch) };
        if on {
            ui.cv.stroke_rect(bx - k.v(8.0), by - k.v(8.0), bw + k.v(16.0), bh + k.v(16.0), 3.0, ui.pal.fg);
        }
        cover(ui, r, bx, by, bw, bh, false);
        let st = Style::interface_bold(k.v(11.0)).upper().color(basket_ui::tokens::hex(0xF2EDE2));
        let lines = ui.cv.fonts.wrap(&st, &r.game.title, bw - k.v(16.0), 2);
        let n = lines.len() as f32;
        for (j, l) in lines.iter().enumerate() {
            ui.cv.text(bx + k.v(8.0), by + bh - k.v(22.0) - (n - 1.0 - j as f32) * k.v(12.0), l, &st);
        }
        if ui.clicked(bx, by, bw, bh) {
            ui.emit(if on { Cmd::CouchContinue } else { Cmd::CouchPick(i) });
        }
    }
}

fn footer(ui: &mut Ui, v: &CouchView, k: &Scale, x0: f32) {
    let w = ui.w();
    let top = k.y(653.0);
    ui.cv.hrule(x0, w - x0, top, 1.0, ui.pal.line);
    let cy = k.y(684.0);
    let verb = Style::interface(k.v(17.0)).color(ui.pal.fg);
    let mut x = x0;
    for (key, text) in &v.hints {
        if key.contains(' ') {
            for part in key.split(' ') {
                x += pad_key(ui, x, cy - k.v(13.0), part, k) + k.v(6.0);
            }
            x += k.v(4.0);
        } else {
            pad_circle(ui, x + k.v(13.0), cy, key, k, ui.pal.fg);
            x += k.v(38.0);
        }
        x += ui.cv.text(x, cy - k.v(10.0), text, &verb) + k.v(28.0);
    }
    let dw = ui.cv.measure("Desktop mode", &verb);
    let dx = w - x0 - dw;
    ui.cv.text(dx, cy - k.v(10.0), "Desktop mode", &verb);
    let kw = pad_key(ui, dx - k.v(38.0), cy - k.v(13.0), "", k);
    // The menu glyph (the fonts have no ☰): three bars.
    for i in 0..3 {
        let bx = dx - k.v(38.0) + kw / 2.0 - k.v(6.0);
        ui.cv.fill_rect(bx, cy - k.v(5.0) + i as f32 * k.v(4.5), k.v(12.0), k.v(1.6), ui.pal.fg);
    }
    if ui.clicked(dx - k.v(38.0), cy - k.v(16.0), kw + dw + k.v(38.0), k.v(32.0)) {
        ui.emit(Cmd::Couch);
    }
}
