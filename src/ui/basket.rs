//! The Basket tab: every fruit in the feed, in three sections (in the
//! basket, ready to install, still growing), and an aside for the selected
//! one. When narrow, the aside is a bottom sheet over the list.

use crate::ui::fmt_size;
use basket_ui::text::Style;
use basket_ui::widgets;

use super::{capitalise, Cmd, Size, Ui};
use crate::art::ICON_L;
use crate::basket::Current;
use crate::feed::{self, Build, Channel, Fruit};
use crate::jobs::{FailKind, Step};

/// The status line on a card.
#[derive(Debug, Clone, PartialEq)]
pub enum CardState {
    UpToDate(Channel),
    UpdateReady,
    Busy(Step, u8),
    Queued,
    Failed(FailKind),
    NotInstalled,
    /// Released, but nothing built for this PC.
    NotForThisPc,
}

/// The aside's big button.
#[derive(Debug, Clone, PartialEq)]
pub enum Primary {
    Open,
    Update,
    Install,
    TryAgain,
    Busy(Step, u8),
    Queued,
    Unavailable,
}

pub struct Card<'a> {
    pub fruit: &'a Fruit,
    pub state: CardState,
}

pub struct Detail<'a> {
    pub fruit: &'a Fruit,
    pub current: Option<&'a Current>,
    /// The channel the segmented control shows.
    pub channel: Channel,
    pub primary: Primary,
    /// The build "What's new" and the program row describe.
    pub target: Option<&'a Build>,
    pub games: usize,
    pub program: u64,
    pub games_size: u64,
    pub watching: bool,
    pub key_id: &'a str,
    /// Roll back… works: there is something to roll back to, nothing is
    /// installing, and none of the fruit's games is running.
    pub can_roll_back: bool,
    /// Uninstall works: nothing is installing and no game is running.
    pub can_uninstall: bool,
}

pub struct BasketView<'a> {
    pub installed: Vec<Card<'a>>,
    pub ready: Vec<Card<'a>>,
    pub growing: Vec<(&'a Fruit, bool)>,
    pub selected: Option<&'a str>,
    pub detail: Option<Detail<'a>>,
    /// Narrow only: the detail is open as a sheet.
    pub sheet: bool,
    /// `~/FruitBasket/`, for the empty-basket note.
    pub root: String,
    pub scroll: f32,
    pub aside_scroll: f32,
    /// Shown when there is no feed to list.
    pub feed_note: Option<&'a str>,
}

const CARD_H: f32 = 105.0;
const TILE_W: f32 = 102.0;

pub fn aside_width(size: Size) -> f32 {
    match size {
        Size::Regular => 400.0,
        Size::Compact => 340.0,
        Size::Narrow => 0.0,
    }
}

pub fn draw(ui: &mut Ui, v: &BasketView, top: f32, bottom: f32) {
    let (x0, w) = (ui.pad_x(), ui.w());
    let aside_w = aside_width(ui.size);
    let list_r = if aside_w > 0.0 { w - aside_w - x0 } else { w - x0 };
    let list_w = list_r - x0;

    // --- the list -----------------------------------------------------------------
    let over_list = ui.input.mouse.0 < list_r + x0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom;
    if ui.input.wheel != 0.0 && over_list && !(v.sheet && ui.size == Size::Narrow) {
        ui.emit(Cmd::Scroll(-ui.input.wheel * 40.0));
    }
    ui.clip(0.0, top, list_r + x0, bottom - top);
    let mut y = top + 22.0 - v.scroll;

    if let Some(note) = v.feed_note {
        ui.section(x0, y, list_w, "Basket", None);
        let st = Style::reading(15.0).color(ui.pal.fg);
        ui.cv.paragraph(x0, y + 44.0, list_w.min(560.0), note, &st, 1.5, 4);
        ui.unclip();
        return;
    }

    let n = v.installed.len();
    y = ui.section(x0, y, list_w, &format!("In the basket · {n}"), Some("installed")) + 12.0;
    if n == 0 {
        y = empty_basket(ui, x0, y, list_w, &v.root);
    } else {
        y = cards(ui, &v.installed, v.selected, x0, y, list_w);
    }
    if !v.ready.is_empty() {
        y += 22.0;
        let n = v.ready.len();
        y = ui.section(x0, y, list_w, &format!("Ready to install · {n}"), Some("built and published")) + 12.0;
        y = cards(ui, &v.ready, v.selected, x0, y, list_w);
    }
    if !v.growing.is_empty() {
        y += 22.0;
        let n = v.growing.len();
        y = ui.section(x0, y, list_w, &format!("Still growing · {n}"), Some("no build yet")) + 12.0;
        y = tiles(ui, &v.growing, v.selected, x0, y, list_w);
    }
    y += 24.0;
    ui.unclip();
    ui.emit(Cmd::ScrollMax((y + v.scroll - bottom).max(0.0)));

    // --- the aside, or the sheet ----------------------------------------------------
    let Some(d) = &v.detail else { return };
    if aside_w > 0.0 {
        let ax = w - aside_w;
        let panel = ui.panel();
        ui.cv.fill_rect(ax, top, aside_w, bottom - top, panel);
        ui.cv.vrule(ax, top, bottom, 1.0, ui.pal.line);
        let pad = if ui.size == Size::Regular { 33.0 } else { 25.0 };
        aside(ui, d, ax + pad, top, aside_w - pad - x0, bottom, v.aside_scroll);
    } else if v.sheet {
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
        aside(ui, d, x0, sy + 44.0, w - 2.0 * x0, bottom, v.aside_scroll);
    }
}

fn empty_basket(ui: &mut Ui, x: f32, y: f32, w: f32, root: &str) -> f32 {
    let h = 92.0;
    ui.cv.dashed_rect(x, y, w, h, 1.0, 4.0, ui.pal.line);
    ui.art.mark(ui.cv, ui.night, x + 20.0, y + 18.0, 54.0);
    let st = Style::reading(15.0).color(ui.pal.fg);
    let text = format!("The basket is empty. Install a fruit below and it downloads into {root}.");
    ui.cv.paragraph(x + 104.0, y + 22.0, (w - 130.0).min(360.0), &text, &st, 1.55, 2);
    y + h
}

fn columns(ui: &Ui, w: f32) -> usize {
    match ui.size {
        Size::Regular => 3,
        Size::Compact | Size::Narrow => 2,
    }
    .min(((w + 10.0) / 240.0).max(1.0) as usize)
    .max(1)
}

fn cards(ui: &mut Ui, cards: &[Card], selected: Option<&str>, x: f32, y: f32, w: f32) -> f32 {
    let cols = columns(ui, w);
    let gap = 10.0;
    let cw = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let mut end = y;
    for (i, card) in cards.iter().enumerate() {
        let cx = x + (i % cols) as f32 * (cw + gap);
        let cy = y + (i / cols) as f32 * (CARD_H + gap);
        draw_card(ui, card, selected == Some(card.fruit.id.as_str()), cx, cy, cw);
        end = cy + CARD_H;
    }
    end
}

fn draw_card(ui: &mut Ui, card: &Card, on: bool, x: f32, y: f32, w: f32) {
    let f = card.fruit;
    if on {
        let panel = ui.panel();
        ui.cv.fill_rect(x, y, w, CARD_H, panel);
        ui.cv.stroke_rect(x, y, w, CARD_H, 2.0, ui.pal.fg);
    } else {
        ui.cv.stroke_rect(x, y, w, CARD_H, 1.0, ui.pal.line);
    }
    let icon = ICON_L as f32;
    ui.art.icon(ui.cv, &f.id, ui.night, ICON_L, x + 16.0, y + (CARD_H - icon) / 2.0, 1.0);
    let tx = x + 16.0 + icon + 26.0;
    let tw = w - (tx - x) - 12.0;
    number_line(ui, tx, y + 22.0, f, false);
    let name = Style::display(21.0).upper().color(ui.pal.fg);
    let shown = ui.cv.fonts.ellipsize(&name, &f.name, tw);
    ui.cv.text(tx, y + 38.0, &shown, &name);

    let (line, color) = match &card.state {
        CardState::UpToDate(ch) => (format!("{} · up to date", ch.name()), ui.muted()),
        CardState::UpdateReady => ("update ready".to_string(), ui.muted()),
        CardState::Busy(step, pct) => (format!("{} · {pct}%", step.name()), ui.muted()),
        CardState::Queued => ("queued".to_string(), ui.muted()),
        CardState::Failed(FailKind::Signature) => ("signature didn't match".to_string(), ui.pal.spot),
        CardState::Failed(FailKind::Network) => ("download stopped".to_string(), ui.pal.spot),
        CardState::Failed(FailKind::Install) => ("install failed".to_string(), ui.pal.spot),
        CardState::Failed(FailKind::Space) => ("not enough space".to_string(), ui.pal.spot),
        CardState::NotInstalled => ("not installed".to_string(), ui.muted()),
        CardState::NotForThisPc => ("no build for this PC yet".to_string(), ui.muted()),
    };
    ui.cv.text(tx, y + 68.0, &line, &Style::data(12.0).color(color));
    if let CardState::Busy(_, pct) = card.state {
        let bw = tw.min(142.0);
        ui.cv.fill_rect(tx, y + 86.0, bw, 3.0, ui.pal.line);
        ui.cv.fill_rect(tx, y + 86.0, bw * pct as f32 / 100.0, 3.0, ui.pal.fg);
    }
    if card.state == CardState::UpdateReady || matches!(card.state, CardState::Failed(_)) {
        ui.cv.circle(x + w - 18.0, y + 18.0, 4.0, ui.pal.spot);
    }
    if ui.clicked(x, y, w, CARD_H) {
        ui.emit(Cmd::Select(f.id.clone()));
    }
}

/// "NO. 1  PS2" (and "· EMULATOR" in the aside).
pub(crate) fn number_line(ui: &mut Ui, x: f32, y: f32, f: &Fruit, emulator: bool) {
    let no = Style::interface_bold(11.0).upper().tracking(1.6).color(ui.pal.spot);
    let nw = ui.cv.text(x, y, &format!("No. {}", f.no), &no);
    let sys = Style::interface_bold(11.0).upper().tracking(1.6).color(ui.muted());
    let label = if emulator { format!("{} · emulator", f.system) } else { f.system.clone() };
    ui.cv.text(x + nw + 8.0, y, &label, &sys);
}

fn tiles(ui: &mut Ui, growing: &[(&Fruit, bool)], selected: Option<&str>, x: f32, y: f32, w: f32) -> f32 {
    let per_row = ((w / TILE_W).floor() as usize).max(1);
    let th = 112.0;
    let mut end = y;
    for (i, (f, watching)) in growing.iter().enumerate() {
        let tx = x + (i % per_row) as f32 * TILE_W;
        let ty = y + (i / per_row) as f32 * (th + 8.0);
        if selected == Some(f.id.as_str()) {
            let panel = ui.panel();
            ui.cv.fill_rect(tx, ty, TILE_W - 6.0, th, panel);
            ui.cv.stroke_rect(tx, ty, TILE_W - 6.0, th, 2.0, ui.pal.fg);
        }
        let icon = ICON_L as f32;
        ui.art.icon(ui.cv, &f.id, ui.night, ICON_L, tx + (TILE_W - 6.0 - icon) / 2.0, ty + 8.0, 0.6);
        let cx = tx + (TILE_W - 6.0) / 2.0;
        let name = Style::interface_bold(12.0).upper().color(ui.muted());
        ui.cv.text_center(cx, ty + 78.0, &f.name, &name);
        let sub = if *watching { "watching".to_string() } else { f.system.clone() };
        ui.cv.text_center(cx, ty + 95.0, &sub, &Style::data(11.0).color(ui.muted()));
        if ui.clicked(tx, ty, TILE_W - 6.0, th) {
            ui.emit(Cmd::Select(f.id.clone()));
        }
        end = ty + th;
    }
    end
}

/// The selected fruit, in the aside or the sheet, starting at `top` and
/// clipped above `bottom`.
fn aside(ui: &mut Ui, d: &Detail, x: f32, top: f32, w: f32, bottom: f32, scroll: f32) {
    let in_aside = ui.input.mouse.0 >= x - 30.0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom;
    if ui.input.wheel != 0.0 && in_aside {
        ui.emit(Cmd::AsideScroll(-ui.input.wheel * 40.0));
    }
    ui.clip(x - 40.0, top, w + 80.0, bottom - top);
    let f = d.fruit;
    let mut y = top + 26.0 - scroll;

    // Head: icon, number line, name.
    let icon = ICON_L as f32;
    ui.art.icon(ui.cv, &f.id, ui.night, ICON_L, x, y + 6.0, if d.primary == Primary::Unavailable && f.status == feed::Status::Growing { 0.6 } else { 1.0 });
    let tx = x + icon + 30.0;
    number_line(ui, tx, y + 11.0, f, true);
    let (size, lines) = ui.cv.fonts.fit_display(Style::display(38.0).upper(), &f.name, w - (tx - x), 1, &[38.0, 32.0, 28.0, 24.0]);
    let name = Style::display(size).upper().color(ui.pal.fg);
    ui.cv.text(tx, y + 30.0, lines.first().map(String::as_str).unwrap_or(&f.name), &name);
    y += icon + 30.0;

    let reading = Style::reading(16.0).color(ui.pal.fg);
    y += ui.cv.paragraph(x, y, w, &f.blurb, &reading, 1.55, 3) + 18.0;

    if f.status == feed::Status::Growing {
        growing_box(ui, d, x, y, w);
        ui.unclip();
        return;
    }

    // The big button.
    y = primary(ui, d, x, y, w);

    // What's new: for an update, or a retry of one.
    if let (Some(build), Some(_)) = (d.target, d.current) {
        if matches!(d.primary, Primary::Update | Primary::TryAgain) && !build.notes.is_empty() {
            y = whats_new(ui, build, x, y + 10.0, w);
        }
    }

    // Open games/ · Roll back… · Uninstall.
    if d.current.is_some() {
        y += 10.0;
        let (gw, games) = ui.small_button(x, y, "Open games/");
        if games {
            ui.emit(Cmd::OpenGames(f.id.clone()));
        }
        let rx = x + gw + 14.0;
        if d.can_roll_back {
            if ui.small_button(rx, y, "Roll back…").1 {
                ui.emit(Cmd::AskRollback(f.id.clone()));
            }
        } else {
            ui.small_button_off(rx, y, "Roll back…");
        }
        let st = Style::interface(13.0);
        let uw = ui.cv.measure("Uninstall", &st);
        let rw = ui.small_button_width("Roll back…");
        let (ux, uy) = if rx + rw + 20.0 + uw <= x + w { (x + w - uw, y + 9.0) } else { (x, y + 50.0) };
        if d.can_uninstall {
            if ui.link(ux, uy, "Uninstall", ui.pal.fg).1 {
                ui.emit(Cmd::AskUninstall(f.id.clone()));
            }
        } else {
            ui.cv.text(ux, uy, "Uninstall", &st.color(ui.faded()));
        }
        y += if uy == y + 9.0 { 34.0 } else { 70.0 };
    }

    // Setup, or what install does.
    y += 24.0;
    let heading = if d.current.is_some() { "Setup" } else { "What install does" };
    let note = ui.note();
    ui.cv.text(x, y, heading, &note);
    y += 18.0;
    y = setup(ui, d, x, y, w);

    // Channel · On disk.
    y += 26.0;
    let half = (w / 2.0).floor();
    ui.cv.text(x, y, "Channel", &note);
    ui.cv.text(x + half + 8.0, y, "On disk", &note);
    let sel = if d.channel == Channel::Stable { 0 } else { 1 };
    let (_, hit) = ui.segmented(x, y + 18.0, &["Stable", "Nightly"], sel);
    if let Some(i) = hit {
        let ch = if i == 0 { Channel::Stable } else { Channel::Nightly };
        if f.channel(ch).is_some() {
            ui.emit(Cmd::SetChannel(f.id.clone(), ch));
        }
    }
    let mono = Style::data(13.0).color(ui.pal.fg);
    if d.current.is_some() {
        ui.cv.text(x + half + 8.0, y + 20.0, &format!("program {}", fmt_size(d.program)), &mono);
        ui.cv.text(x + half + 8.0, y + 39.0, &format!("games {}", fmt_size(d.games_size)), &mono);
    } else if let Some(a) = d.target.and_then(|b| b.assets.get(feed::this_platform())) {
        ui.cv.text(x + half + 8.0, y + 20.0, &format!("needs {}", fmt_size(a.size)), &mono);
    }
    y += 64.0;
    ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);

    // Builds for.
    if let Some(build) = d.target {
        y += 16.0;
        ui.cv.text(x, y, "Builds for", &note);
        y += 18.0;
        let here = feed::this_platform();
        let mut cx = x;
        // This PC's chip first, then the rest in feed order.
        let keys = build.assets.keys().filter(|k| *k == here).chain(build.assets.keys().filter(|k| *k != here));
        for key in keys {
            let mine = key == here;
            let label = if mine { format!("{} · this PC", feed::platform_label(key)) } else { feed::platform_label(key) };
            let st = Style::data(12.0).color(if mine { ui.pal.fg } else { ui.muted() });
            let cw = ui.cv.measure(&label, &st) + 20.0;
            if cx + cw > x + w && cx > x {
                cx = x;
                y += 34.0;
            }
            ui.cv.stroke_rect(cx, y, cw, 27.0, if mine { 1.5 } else { 1.0 }, if mine { ui.pal.fg } else { ui.pal.line });
            ui.cv.text(cx + 10.0, y + 6.0, &label, &st);
            cx += cw + 6.0;
        }
        y += 40.0;
        ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);
    }

    // Docs.
    y += 14.0;
    ui.cv.text(x, y + 2.0, "Docs", &note);
    let mut lx = x + 55.0;
    let fg = ui.pal.fg;
    let mut links = vec![("README", f.readme_url.clone())];
    if let Some(c) = &f.compat {
        links.push(("Compatibility list", c.url.clone()));
    }
    links.push(("All releases", f.url.clone()));
    for (label, url) in links {
        let lw = ui.cv.measure(label, &Style::interface(13.0));
        if lx + lw > x + w {
            lx = x + 55.0;
            y += 24.0;
        }
        let (_, hit) = ui.link(lx, y, label, fg);
        if hit {
            ui.emit(Cmd::OpenUrl(url));
        }
        lx += lw + 16.0;
    }
    y += 40.0;

    ui.unclip();
    ui.emit(Cmd::AsideScrollMax((y + scroll - bottom).max(0.0)));
}

fn primary(ui: &mut Ui, d: &Detail, x: f32, y: f32, w: f32) -> f32 {
    let name = &d.fruit.name;
    let label = match &d.primary {
        Primary::Open => Some((format!("Open {name}"), Cmd::Open(d.fruit.id.clone()))),
        Primary::Update => Some((format!("Update {name}"), Cmd::Update(d.fruit.id.clone()))),
        Primary::Install => Some((format!("Install {name}"), Cmd::Install(d.fruit.id.clone()))),
        Primary::TryAgain => Some(("Try again".to_string(), Cmd::Retry(d.fruit.id.clone()))),
        _ => None,
    };
    let h = 52.0;
    match label {
        Some((label, cmd)) => {
            ui.cv.fill_rect(x, y, w, h, ui.pal.fg);
            let st = Style::interface_bold(17.0).color(ui.pal.bg);
            let ks = Style::data(13.0).color(widgets::mix(ui.pal.bg, ui.pal.fg, 0.55));
            let lw = ui.cv.measure(&label, &st);
            let kw = ui.cv.measure("Z", &ks) + 12.0;
            let lx = x + (w - lw - kw) / 2.0;
            ui.cv.text(lx, y + 16.0, &label, &st);
            ui.cv.text(lx + lw + 12.0, y + 19.0, "Z", &ks);
            if ui.clicked(x, y, w, h) {
                ui.emit(cmd);
            }
            y + h
        }
        None => {
            let text = match d.primary {
                Primary::Busy(step, _) => format!("{}…", capitalise(step.name())),
                Primary::Queued => "Queued".to_string(),
                _ => "No build for this PC yet".to_string(),
            };
            ui.cv.stroke_rect(x, y, w, h, 1.5, ui.pal.line);
            let st = Style::interface_bold(17.0).color(ui.muted());
            ui.cv.text_center(x + w / 2.0, y + 15.0, &text, &st);
            let mut end = y + h;
            if let Primary::Busy(_, pct) = d.primary {
                end += 10.0 + ui.step_bars(x, end + 10.0, w, pct);
            }
            end
        }
    }
}

fn whats_new(ui: &mut Ui, build: &Build, x: f32, y: f32, w: f32) -> f32 {
    let inner = w - 34.0;
    let reading = Style::reading(15.0).color(ui.pal.fg);
    // Measure first so the box fits its notes.
    let notes: Vec<Vec<String>> = build.notes.iter().take(4).map(|n| ui.cv.fonts.wrap(&reading, n, inner - 18.0, 2)).collect();
    let lines: usize = notes.iter().map(Vec::len).sum();
    let h = 46.0 + lines as f32 * 23.0 + 10.0;
    let bg = ui.pal.bg;
    ui.cv.fill_rect(x, y, w, h, bg);
    ui.cv.stroke_rect(x, y, w, h, 1.0, ui.pal.line);
    let head = Style::interface_bold(11.0).upper().tracking(1.6).color(ui.pal.fg);
    ui.cv.text(x + 17.0, y + 16.0, &format!("What's new · {}", build.build), &head);
    let fw = ui.cv.measure("Full notes", &Style::interface(13.0));
    let fg = ui.pal.fg;
    let (_, full) = ui.link(x + w - 17.0 - fw, y + 14.0, "Full notes", fg);
    if full {
        ui.emit(Cmd::OpenUrl(build.notes_url.clone()));
    }
    let mut ly = y + 40.0;
    for note in notes {
        ui.cv.circle(x + 23.0, ly + 10.0, 2.5, ui.pal.fg);
        for line in note {
            ui.cv.text(x + 35.0, ly, &line, &reading);
            ly += 23.0;
        }
    }
    y + h
}

fn setup(ui: &mut Ui, d: &Detail, x: f32, mut y: f32, w: f32) -> f32 {
    let f = d.fruit;
    let installed = d.current.is_some();
    let ext = f.ext.join(" ");
    let bios = f.bios.clone().unwrap_or_else(|| "not needed".to_string());
    let bios_ok = f.bios.as_deref().map_or(true, |b| b == "built in" || b == "not needed");
    let rows: Vec<(&str, String, bool)> = match d.current {
        Some(c) => vec![
            ("Program", format!("{} {}", c.channel.name(), c.build), true),
            ("Signature", format!("verified · {}", d.key_id), true),
            ("Games", format!("games/ · {} found", d.games), d.games > 0),
            ("BIOS", bios, bios_ok),
        ],
        None => {
            let program = match d.target {
                Some(b) => match b.assets.get(feed::this_platform()) {
                    Some(a) => format!("{} {} · {}", d.channel.name(), b.build, fmt_size(a.size)),
                    None => format!("{} {}", d.channel.name(), b.build),
                },
                None => "no build yet".to_string(),
            };
            vec![
                ("Program", program, false),
                ("Signature", "checked on download".to_string(), false),
                ("Games", format!("reads {ext}"), false),
                ("BIOS", bios, bios_ok),
            ]
        }
    };
    let label = Style::interface_bold(14.0).color(ui.pal.fg);
    let mono = Style::data(13.0).color(ui.muted());
    for (name, value, ok) in rows {
        let cy = y + 22.0;
        if ok {
            ui.check_mark(x + 2.0, cy - 6.0, ui.pal.fg);
        } else if installed {
            ui.cv.circle(x + 8.0, cy, 4.0, ui.pal.spot);
        } else {
            ui.cv.stroke_circle(x + 8.0, cy, 4.0, 1.2, ui.pal.fg);
        }
        ui.cv.text(x + 30.0, y + 13.0, name, &label);
        let shown = ui.cv.fonts.ellipsize(&mono, &value, w - 130.0);
        ui.cv.text(x + 130.0, y + 14.0, &shown, &mono);
        y += 45.0;
        ui.cv.hrule(x, x + w, y, 1.0, ui.pal.line);
    }
    y
}

/// A growing fruit: no build yet, what it will read, and the watch button.
fn growing_box(ui: &mut Ui, d: &Detail, x: f32, y: f32, w: f32) {
    let f = d.fruit;
    let reading = Style::reading(16.0).color(ui.pal.fg);
    let text = format!("No build yet. {} moves up to Ready to install the night its first build lands.", f.name);
    let lines = ui.cv.fonts.wrap(&reading, &text, w - 42.0, 4).len();
    let h = 64.0 + lines as f32 * 25.0 + 52.0 + 60.0;
    ui.cv.dashed_rect(x, y, w, h, 1.0, 3.0, ui.pal.fg);
    let note = ui.section_label();
    ui.cv.text(x + 21.0, y + 22.0, "Still growing", &Style::interface_bold(12.0).upper().tracking(2.0).color(note.color));
    let mut iy = y + 50.0;
    iy += ui.cv.paragraph(x + 21.0, iy, w - 42.0, &text, &reading, 1.55, 4) + 8.0;
    let small = ui.note();
    ui.cv.text(x + 21.0, iy, "Will read", &small);
    ui.cv.text(x + 21.0, iy + 18.0, &f.ext.join(" "), &Style::data(13.0).color(ui.pal.fg));
    iy += 48.0;
    let pal = ui.pal;
    if d.watching {
        let label = "Watching";
        let bw = widgets::button_width(ui.cv, label, None) + 16.0;
        ui.cv.fill_rect(x + 21.0, iy, bw, 44.0, pal.fg);
        widgets::button_primary(ui.cv, x + 21.0 - 8.0, iy, bw, label, None, &pal);
        ui.check_mark(x + 21.0 + bw - 26.0, iy + 17.0, pal.bg);
        if ui.clicked(x + 21.0, iy, bw, 44.0) {
            ui.emit(Cmd::Watch(f.id.clone()));
        }
    } else {
        let label = "Tell me when it's ripe";
        let bw = widgets::button_width(ui.cv, label, None);
        widgets::button_secondary(ui.cv, x + 21.0, iy, bw, label, &pal);
        if ui.clicked(x + 21.0, iy, bw, 44.0) {
            ui.emit(Cmd::Watch(f.id.clone()));
        }
    }
}
