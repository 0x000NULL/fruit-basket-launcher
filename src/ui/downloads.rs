//! The Downloads tab: what is running now (with its three steps), what
//! failed and why, what waits, and the Earlier history.

use basket_ui::fmt::fmt_size;
use basket_ui::text::Style;

use super::{capitalise, Cmd, Ui};
use crate::art::{ICON_L, ICON_S};
use crate::history::Entry;
use crate::jobs::{FailKind, Job};
use crate::queue::{Active, Failure};

pub struct DownloadsView<'a> {
    pub active: Option<&'a Active>,
    pub failures: Vec<&'a Failure>,
    pub waiting: Vec<&'a Job>,
    pub earlier: &'a [Entry],
    /// Display name for a fruit id.
    pub name: &'a dyn Fn(&str) -> String,
    /// Fruits with a build installed; a failure card says the old build
    /// still works only when there is one.
    pub has_build: &'a dyn Fn(&str) -> bool,
    pub key_id: &'a str,
    pub scroll: f32,
}

pub fn draw(ui: &mut Ui, v: &DownloadsView, top: f32, bottom: f32) {
    let x0 = ui.pad_x();
    let w = (ui.w() - 2.0 * x0).min(960.0);
    if ui.input.wheel != 0.0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom {
        ui.emit(Cmd::Scroll(-ui.input.wheel * 40.0));
    }
    let full_w = ui.w();
    ui.clip(0.0, top, full_w, bottom - top);
    let mut y = top + 22.0 - v.scroll;

    y = ui.section(x0, y, w, "Now", None) + 14.0;
    let mut any = false;
    if let Some(a) = v.active {
        y = active(ui, v, a, x0, y, w) + 18.0;
        any = true;
    }
    for f in &v.failures {
        y = failure(ui, v, f, x0, y, w) + 12.0;
        any = true;
    }
    for job in &v.waiting {
        y = row(ui, v, &job.fruit, &format!("queued · {} {}", job.channel.name(), job.build), "waiting", None, x0, y, w);
        any = true;
    }
    if !any {
        let st = Style::reading(15.0).color(ui.muted());
        ui.cv.text(x0, y + 6.0, "Nothing downloading. Installs and updates from the Basket tab run here, one at a time.", &st);
        y += 40.0;
    }

    y += 22.0;
    y = ui.section(x0, y, w, "Earlier", None);
    if v.earlier.is_empty() {
        let st = Style::reading(15.0).color(ui.muted());
        ui.cv.text(x0, y + 18.0, "Nothing yet.", &st);
        y += 50.0;
    }
    for e in v.earlier {
        let (text, bad) = match e.failed {
            None => (format!("installed · {} {}", e.channel.name(), e.build), false),
            Some(FailKind::Signature) => (format!("signature didn't match · {} {}", e.channel.name(), e.build), true),
            Some(FailKind::Network) => (format!("download stopped · {} {}", e.channel.name(), e.build), true),
            Some(FailKind::Install) => (format!("install failed · {} {}", e.channel.name(), e.build), true),
        };
        let when = basket_ui::fmt::fmt_when(e.when);
        y = row(ui, v, &e.fruit, &text, &when, Some(bad), x0, y, w);
    }
    y += 24.0;
    ui.unclip();
    ui.emit(Cmd::ScrollMax((y + v.scroll - bottom).max(0.0)));
}

/// The running job: icon, name, build and size, the three step bars.
fn active(ui: &mut Ui, v: &DownloadsView, a: &Active, x: f32, y: f32, w: f32) -> f32 {
    let icon = ICON_L as f32;
    ui.art.icon(ui.cv, &a.job.fruit, ui.night, ICON_L, x, y + 6.0, 1.0);
    let tx = x + icon + 30.0;
    let name = Style::display(24.0).upper().color(ui.pal.fg);
    ui.cv.text(tx, y + 10.0, &(v.name)(&a.job.fruit), &name);
    let what = format!("{} {} · {}", a.job.channel.name(), a.job.build, fmt_size(a.job.asset.size));
    ui.cv.text_right(x + w, y + 16.0, &what, &Style::data(13.0).color(ui.pal.fg));
    ui.step_bars(tx, y + 50.0, x + w - tx, a.pct);
    y + icon + 12.0
}

/// A failure card: what went wrong, what that means on disk, Try again.
fn failure(ui: &mut Ui, v: &DownloadsView, f: &Failure, x: f32, y: f32, w: f32) -> f32 {
    let name = (v.name)(&f.job.fruit);
    let still = if (v.has_build)(&f.job.fruit) { " and your current build still works" } else { "" };
    let (head, body) = match f.kind {
        FailKind::Signature => (
            "signature didn't match",
            format!("The download wasn't signed with {}, so it was deleted. Nothing was installed{still}.", v.key_id),
        ),
        FailKind::Network => ("download stopped", format!("{}. Nothing on disk changed{still}.", capitalise(&f.message))),
        FailKind::Install => ("install failed", format!("{}. Nothing was switched{still}.", capitalise(&f.message))),
    };
    let reading = Style::reading(15.0).color(ui.pal.fg);
    let bw = 120.0;
    let text_w = w - 90.0 - bw - 30.0;
    let lines = ui.cv.fonts.wrap(&reading, &body, text_w, 3);
    let h = (48.0 + lines.len() as f32 * 22.0).max(80.0);
    ui.cv.stroke_rect(x, y, w, h, 1.0, ui.pal.spot);
    let iy = y + (h - ICON_S as f32 - 18.0) / 2.0;
    ui.cv.stroke_round_rect(x + 16.0, iy, ICON_S as f32 + 18.0, ICON_S as f32 + 18.0, [8.0; 4], 1.0, ui.pal.fg);
    ui.art.icon(ui.cv, &f.job.fruit, ui.night, ICON_S, x + 25.0, iy + 9.0, 1.0);
    let hs = Style::interface_bold(12.0).upper().tracking(1.6).color(ui.pal.spot);
    ui.cv.text(x + 86.0, y + 16.0, &format!("{name} · {head}"), &hs);
    for (i, line) in lines.iter().enumerate() {
        ui.cv.text(x + 86.0, y + 38.0 + i as f32 * 22.0, line, &reading);
    }
    let tw = ui.cv.measure("Try again", &Style::interface_bold(13.0)) + 36.0;
    let (_, retry) = ui.small_button_filled(x + w - 16.0 - tw, y + (h - 40.0) / 2.0, "Try again");
    if retry {
        ui.emit(Cmd::Retry(f.job.fruit.clone()));
    }
    y + h
}

/// One history or queue row: icon, name, what happened, when. `bad` is
/// `None` for a queued row (no mark), else whether the job failed.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, v: &DownloadsView, fruit: &str, text: &str, right: &str, bad: Option<bool>, x: f32, y: f32, w: f32) -> f32 {
    let h = 59.0;
    ui.art.icon(ui.cv, fruit, ui.night, ICON_S, x + 3.0, y + (h - ICON_S as f32) / 2.0, 1.0);
    let name = Style::display(17.0).upper().color(ui.pal.fg);
    ui.cv.text(x + 54.0, y + 20.0, &(v.name)(fruit), &name);
    let mid = x + (w * 0.415).max(220.0);
    let mono = Style::data(13.0);
    match bad {
        Some(false) => ui.check_mark(mid, y + 23.0, ui.pal.fg),
        Some(true) => ui.cv.circle(mid + 5.0, y + 29.0, 3.5, ui.pal.spot),
        None => {}
    }
    let color = if bad == Some(true) { ui.pal.spot } else { ui.pal.fg };
    let tx = if bad.is_some() { mid + 21.0 } else { mid };
    let shown = ui.cv.fonts.ellipsize(&mono, text, x + w - 140.0 - tx);
    ui.cv.text(tx, y + 21.0, &shown, &mono.color(color));
    ui.cv.text_right(x + w, y + 21.0, right, &Style::data(13.0).color(ui.muted()));
    ui.cv.hrule(x, x + w, y + h, 1.0, ui.pal.line);
    y + h
}
