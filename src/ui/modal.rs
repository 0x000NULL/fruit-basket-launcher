//! A dialog over a dimmed frame: Roll back, Uninstall, Move basket. The
//! page under it is drawn with its input taken away; the app routes keys
//! to the dialog (Z confirms, X and Esc cancel, arrows pick).

use basket_ui::text::Style;
use tiny_skia::{Pixmap, Transform};

use super::basket::number_line;
use super::{Cmd, Size, Ui};
use crate::feed::Fruit;

pub struct ModalView<'a> {
    /// The eyebrow (`No. 1 PS2`); none for Move basket.
    pub fruit: Option<&'a Fruit>,
    pub title: String,
    pub body: String,
    /// A greyed first row: what runs now ("nightly f1a2c09", "installed now").
    pub current: Option<(String, String)>,
    /// Radio rows: label and note.
    pub rows: Vec<(String, String)>,
    pub pick: usize,
    /// A tick box under the body: label and state.
    pub tick: Option<(&'a str, bool)>,
    pub cancel: &'a str,
    pub confirm: &'a str,
    /// A third button at the left, for the picked row (Delete).
    pub extra: Option<&'a str>,
}

const PAD: f32 = 28.0;
const ROW_H: f32 = 46.0;
const BUTTON_H: f32 = 46.0;

/// Dim everything drawn so far.
fn scrim(ui: &mut Ui) {
    let a = if ui.night { 0.55 } else { 0.38 };
    let Some(mut px) = Pixmap::new(1, 1) else { return };
    px.fill(tiny_skia::Color::from_rgba(0.0, 0.0, 0.0, a).unwrap_or(tiny_skia::Color::BLACK));
    let (w, h) = (ui.w(), ui.h());
    ui.cv.draw_pixmap(&px, 0.0, 0.0, Transform::from_scale(w, h));
}

pub fn draw(ui: &mut Ui, v: &ModalView) {
    scrim(ui);
    let gutter = if ui.size == Size::Narrow { 16.0 } else { 32.0 };
    let cw = (ui.w() - 2.0 * gutter).min(540.0);
    let inner = cw - 2.0 * PAD;
    let cx = ((ui.w() - cw) / 2.0).round();

    // Measure first, so the card can be centred.
    let title_sizes = [38.0, 32.0, 28.0, 24.0];
    let (tsize, tlines) = ui.cv.fonts.fit_display(Style::display(38.0).upper(), &v.title, inner, 2, &title_sizes);
    let reading = Style::reading(16.0).color(ui.pal.fg);
    let blines = ui.cv.fonts.wrap(&reading, &v.body, inner, 5);
    let rows = v.rows.len() + v.current.is_some() as usize;
    let mut ch = PAD + if v.fruit.is_some() { 22.0 } else { 0.0 };
    ch += tlines.len() as f32 * tsize * 1.05 + 14.0;
    ch += blines.len() as f32 * 24.0 + 16.0;
    ch += rows as f32 * ROW_H + if rows > 0 { 16.0 } else { 0.0 };
    ch += if v.tick.is_some() { 38.0 } else { 0.0 };
    ch += BUTTON_H + PAD;
    let cy = ((ui.h() - ch) / 2.0).max(16.0).round();

    ui.cv.fill_rect(cx, cy, cw, ch, ui.pal.bg);
    ui.cv.stroke_rect(cx, cy, cw, ch, 2.0, ui.pal.fg);
    let x = cx + PAD;
    let mut y = cy + PAD;

    if let Some(f) = v.fruit {
        number_line(ui, x, y, f, false);
        y += 22.0;
    }
    let title = Style::display(tsize).upper().color(ui.pal.fg);
    for line in &tlines {
        ui.cv.text(x, y, line, &title);
        y += tsize * 1.05;
    }
    y += 14.0;
    for line in &blines {
        ui.cv.text(x, y, line, &reading);
        y += 24.0;
    }
    y += 16.0;

    if rows > 0 {
        ui.cv.hrule(x, x + inner, y, 1.0, ui.pal.line);
        let mono = Style::data(14.0);
        let note = Style::interface(13.0);
        if let Some((label, what)) = &v.current {
            ui.cv.text(x + 30.0, y + 15.0, label, &mono.color(ui.muted()));
            ui.cv.text_right(x + inner, y + 16.0, what, &note.color(ui.muted()));
            y += ROW_H;
            ui.cv.hrule(x, x + inner, y, 1.0, ui.pal.line);
        }
        for (i, (label, what)) in v.rows.iter().enumerate() {
            let on = i == v.pick;
            ui.cv.stroke_circle(x + 9.0, y + ROW_H / 2.0, 8.0, 2.0, ui.pal.fg);
            if on {
                ui.cv.circle(x + 9.0, y + ROW_H / 2.0, 4.0, ui.pal.fg);
            }
            ui.cv.text(x + 30.0, y + 15.0, label, &mono.color(ui.pal.fg));
            ui.cv.text_right(x + inner, y + 16.0, what, &note.color(ui.muted()));
            if ui.clicked(x, y, inner, ROW_H) && !on {
                ui.emit(Cmd::ModalPick(i));
            }
            y += ROW_H;
            ui.cv.hrule(x, x + inner, y, 1.0, ui.pal.line);
        }
        y += 16.0;
    }

    if let Some((label, on)) = v.tick {
        let (_, hit) = ui.check(x, y + 2.0, label, on, true);
        if hit {
            ui.emit(Cmd::ModalToggle);
        }
        y += 38.0;
    }

    // Cancel · Confirm, at the right.
    let bold = Style::interface_bold(16.0);
    let confirm_w = (ui.cv.measure(v.confirm, &bold) + 40.0).round();
    let cancel_w = (ui.cv.measure(v.cancel, &bold) + 40.0).round();
    let bx = x + inner - confirm_w;
    ui.cv.fill_rect(bx, y, confirm_w, BUTTON_H, ui.pal.fg);
    ui.cv.text_center(bx + confirm_w / 2.0, y + 14.0, v.confirm, &bold.color(ui.pal.bg));
    if ui.clicked(bx, y, confirm_w, BUTTON_H) {
        ui.emit(Cmd::ModalConfirm);
    }
    let kx = bx - 10.0 - cancel_w;
    ui.cv.stroke_rect(kx, y, cancel_w, BUTTON_H, 2.0, ui.pal.fg);
    ui.cv.text_center(kx + cancel_w / 2.0, y + 14.0, v.cancel, &bold.color(ui.pal.fg));
    if ui.clicked(kx, y, cancel_w, BUTTON_H) {
        ui.emit(Cmd::ModalCancel);
    }
    if let Some(extra) = v.extra {
        let ew = (ui.cv.measure(extra, &bold) + 40.0).round();
        ui.cv.stroke_rect(x, y, ew, BUTTON_H, 2.0, ui.pal.fg);
        ui.cv.text_center(x + ew / 2.0, y + 14.0, extra, &bold.color(ui.pal.fg));
        if ui.clicked(x, y, ew, BUTTON_H) {
            ui.emit(Cmd::ModalExtra);
        }
    }
}
