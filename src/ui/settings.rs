//! The Settings tab: game folders, storage, updates, controller, look,
//! launcher. Two columns (section names left), one when narrow.

use basket_ui::text::Style;

use super::{Cmd, Flag, Size, Ui};
use crate::feed::Channel;
use crate::settings::{Settings, ThemePref};

#[derive(Debug, Clone)]
pub struct FruitStorage {
    pub id: String,
    pub name: String,
    /// "program 12 MB · games 1.4 GB · 2 old builds kept"
    pub detail: String,
}

pub struct SettingsView<'a> {
    pub settings: &'a Settings,
    /// Each installed fruit's games/ folder, always scanned.
    pub fruit_folders: Vec<String>,
    pub root: String,
    /// "4.1 GB used · 210 GB free"
    pub root_detail: String,
    pub storage: Vec<FruitStorage>,
    pub controller: Option<String>,
    pub os_dark: bool,
    pub version: &'a str,
    pub key_id: &'a str,
    pub site: String,
    pub scroll: f32,
}

/// Draw the tab between `top` and `bottom`.
pub fn draw(ui: &mut Ui, v: &SettingsView, top: f32, bottom: f32) {
    let (x0, w) = (ui.pad_x(), ui.w());
    let narrow = ui.size == Size::Narrow;
    let label_w = if narrow { 0.0 } else { 220.0 };
    let cx = x0 + label_w;
    let cw = (w - 2.0 * x0 - label_w).min(760.0);
    ui.clip(0.0, top, w, bottom - top);
    if ui.input.wheel != 0.0 && ui.input.mouse.1 >= top && ui.input.mouse.1 < bottom {
        ui.emit(Cmd::Scroll(-ui.input.wheel * 40.0));
    }
    let s = v.settings;
    let mut y = top + 28.0 - v.scroll;

    // GAME FOLDERS
    y = heading(ui, x0, y, "Game folders", narrow);
    for folder in &v.fruit_folders {
        y = path_row(ui, cx, y, cw, folder, Some("always scanned"), None);
    }
    for (i, folder) in s.folders.iter().enumerate() {
        y = path_row(ui, cx, y, cw, &folder.display().to_string(), None, Some(Cmd::RemoveFolder(i)));
    }
    let (_, add) = ui.small_button(cx, y + 6.0, "Add folder…");
    if add {
        ui.emit(Cmd::AddFolder);
    }
    y += 52.0;
    y = check(ui, cx, y, "Rescan when the launcher opens", s.rescan_on_open, Flag::RescanOnOpen);
    y = rule(ui, x0, y, w);

    // STORAGE
    y = heading(ui, x0, y, "Storage", narrow);
    let mono = ui.mono();
    let path_st = Style::data_medium(13.0).color(ui.pal.fg);
    ui.cv.text(cx, y, &v.root, &path_st);
    ui.cv.text(cx, y + 20.0, &v.root_detail, &mono);
    let mw = ui.small_button_width("Move basket…");
    let (_, mv) = ui.small_button(cx + cw - mw, y + 2.0, "Move basket…");
    if mv {
        ui.emit(Cmd::MoveBasket);
    }
    y += 54.0;
    for f in &v.storage {
        ui.art.icon(ui.cv, &f.id, ui.night, crate::art::ICON_S, cx, y, 1.0);
        let name = Style::interface_bold(13.0).upper().tracking(1.6).color(ui.pal.fg);
        ui.cv.text(cx + 44.0, y, &f.name, &name);
        ui.cv.text(cx + 44.0, y + 18.0, &f.detail, &mono);
        let lw = ui.small_button_width("Clear old builds");
        let (_, clear) = ui.small_button(cx + cw - lw, y, "Clear old builds");
        if clear {
            ui.emit(Cmd::ClearOldBuilds(f.id.clone()));
        }
        y += 48.0;
    }
    let body = ui.body();
    ui.cv.text(cx, y + 8.0, "Old builds to keep for rolling back", &body);
    let tw = ui.cv.measure("Old builds to keep for rolling back", &body);
    let (_, hit) = ui.segmented(cx + tw + 18.0, y, &["1", "2", "3"], (s.keep.clamp(1, 3) - 1) as usize);
    if let Some(i) = hit {
        ui.emit(Cmd::Keep(i as u8 + 1));
    }
    y += 52.0;
    y = rule(ui, x0, y, w);

    // UPDATES
    y = heading(ui, x0, y, "Updates", narrow);
    ui.cv.text(cx, y + 8.0, "New installs use", &body);
    let tw = ui.cv.measure("New installs use", &body);
    let sel = if s.new_channel == Channel::Stable { 0 } else { 1 };
    let (_, hit) = ui.segmented(cx + tw + 18.0, y, &["Stable", "Nightly"], sel);
    if let Some(i) = hit {
        ui.emit(Cmd::NewChannel(if i == 0 { Channel::Stable } else { Channel::Nightly }));
    }
    y += 50.0;
    y = check(ui, cx, y, "Check every fruit when the launcher opens", s.check_on_open, Flag::CheckOnOpen);
    y = check(ui, cx, y, "Install updates without asking", s.install_without_asking, Flag::InstallWithoutAsking);
    let signed = format!("Only install builds signed with {} · always on", v.key_id);
    let (h, _) = ui.check(cx, y, &signed, true, false);
    y += h + 12.0;
    y = rule(ui, x0, y, w);

    // CONTROLLER
    y = heading(ui, x0, y, "Controller", narrow);
    let (dot, text) = match &v.controller {
        Some(name) => (ui.pal.fg, format!("{name} connected")),
        None => (ui.faded(), "No controller connected".to_string()),
    };
    ui.cv.circle(cx + 4.0, y + 9.0, 4.0, dot);
    ui.cv.text(cx + 16.0, y + 1.0, &text, &body);
    let mw = ui.small_button_width("Map buttons…");
    let (_, map) = ui.small_button(cx + cw - mw, y - 8.0, "Map buttons…");
    if map {
        ui.emit(Cmd::MapButtons);
    }
    y += 40.0;
    y = check(ui, cx, y, "Open in couch mode when a controller connects", s.couch_on_controller, Flag::CouchOnController);
    y = rule(ui, x0, y, w);

    // LOOK
    y = heading(ui, x0, y, "Look", narrow);
    ui.cv.text(cx, y + 8.0, "Theme", &body);
    let sel = match s.theme {
        ThemePref::System => 0,
        ThemePref::Paper => 1,
        ThemePref::Night => 2,
    };
    let (sw, hit) = ui.segmented(cx + 70.0, y, &["System", "Paper", "Night"], sel);
    if let Some(i) = hit {
        ui.emit(Cmd::Theme([ThemePref::System, ThemePref::Paper, ThemePref::Night][i]));
    }
    if s.theme == ThemePref::System && !narrow {
        let st = Style::reading(14.0).color(ui.muted());
        let now = if v.os_dark { "night" } else { "paper" };
        ui.cv.text(cx + 70.0 + sw + 18.0, y + 8.0, &format!("Follows your computer: {now} right now."), &st);
    }
    y += 50.0;
    y = rule(ui, x0, y, w);

    // LAUNCHER
    y = heading(ui, x0, y, "Launcher", narrow);
    ui.cv.text(cx, y, v.version, &path_st);
    let cw2 = ui.small_button_width("Check now");
    let (_, check_now) = ui.small_button(cx + cw - cw2, y - 8.0, "Check now");
    if check_now {
        ui.emit(Cmd::CheckNow);
    }
    ui.cv.text(cx, y + 22.0, &format!("builds from {}<fruit>/ · key {}", v.site, v.key_id), &mono);
    y += 70.0;

    ui.unclip();
    // How far the content can scroll, so the app can clamp.
    ui.emit(Cmd::ScrollMax((y + v.scroll - top - (bottom - top)).max(0.0)));
}

fn heading(ui: &mut Ui, x: f32, y: f32, title: &str, narrow: bool) -> f32 {
    let st = ui.section_label();
    ui.cv.text(x, y, title, &st);
    if narrow {
        y + 32.0
    } else {
        y
    }
}

fn rule(ui: &mut Ui, x0: f32, y: f32, w: f32) -> f32 {
    ui.cv.hrule(x0, w - x0, y + 10.0, 1.0, ui.pal.line);
    y + 38.0
}

fn check(ui: &mut Ui, x: f32, y: f32, label: &str, on: bool, flag: Flag) -> f32 {
    let (h, hit) = ui.check(x, y, label, on, true);
    if hit {
        ui.emit(Cmd::Toggle(flag));
    }
    y + h + 12.0
}

fn path_row(ui: &mut Ui, x: f32, y: f32, w: f32, path: &str, note: Option<&str>, remove: Option<Cmd>) -> f32 {
    let st = Style::data_medium(13.0).color(ui.pal.fg);
    let shown = ui.cv.fonts.ellipsize_middle(&st, path, w - 140.0);
    ui.cv.text(x, y, &shown, &st);
    if let Some(note) = note {
        let mono = ui.mono();
        ui.cv.text_right(x + w, y, note, &mono);
    }
    if let Some(cmd) = remove {
        let spot = ui.pal.fg;
        let rw = ui.cv.measure("Remove", &Style::interface(13.0));
        let (_, hit) = ui.link(x + w - rw, y, "Remove", spot);
        if hit {
            ui.emit(cmd);
        }
    }
    ui.cv.hrule(x, x + w, y + 26.0, 1.0, ui.pal.line);
    y + 36.0
}
