//! Drawing the launcher. Immediate mode: each frame draws everything from
//! the app's state and turns this frame's clicks and keys into `Cmd`s,
//! which the app applies after drawing. Nothing here changes state itself.

pub mod frame;
pub mod settings;

use basket_ui::canvas::Clip;
use basket_ui::input::UiInput;
use basket_ui::text::{Face, Style};
use basket_ui::tokens::{Palette, Rgb, NIGHT, PAPER};
use basket_ui::widgets::{self, mix};
use basket_ui::Canvas;
use minifb::Key;

use crate::art::Art;
use crate::feed::Channel;
use crate::settings::ThemePref;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Basket,
    Downloads,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Library, Tab::Basket, Tab::Downloads, Tab::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Basket => "Basket",
            Tab::Downloads => "Downloads",
            Tab::Settings => "Settings",
        }
    }

    pub fn step(self, by: i32) -> Tab {
        let i = Tab::ALL.iter().position(|&t| t == self).unwrap() as i32;
        Tab::ALL[(i + by).rem_euclid(Tab::ALL.len() as i32) as usize]
    }
}

/// The three window classes the mocks are drawn at: 1280, 1024 and 640 wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Regular,
    Compact,
    Narrow,
}

impl Size {
    pub fn of(width: f32) -> Size {
        if width < 820.0 {
            Size::Narrow
        } else if width < 1180.0 {
            Size::Compact
        } else {
            Size::Regular
        }
    }

    /// Side padding: 32 / 24 / 16.
    pub fn pad(self) -> f32 {
        match self {
            Size::Regular => 32.0,
            Size::Compact => 24.0,
            Size::Narrow => 16.0,
        }
    }
}

/// Everything a frame changes, applied by the app after drawing.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Tab(Tab),
    FocusFind(bool),
    Find(String),
    UpdateAll,
    Couch,
    Quit,
    Scroll(f32),
    /// The furthest the current tab can scroll; sent once per frame.
    ScrollMax(f32),
    Theme(ThemePref),
    NewChannel(Channel),
    Keep(u8),
    Toggle(Flag),
    AddFolder,
    RemoveFolder(usize),
    MoveBasket,
    ClearOldBuilds(String),
    CheckNow,
    MapButtons,
    OpenUrl(String),
    RestartForUpdate,
    DismissLauncherUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    RescanOnOpen,
    CheckOnOpen,
    InstallWithoutAsking,
    CouchOnController,
}

/// One frame's drawing context.
pub struct Ui<'a> {
    pub cv: &'a mut Canvas,
    pub input: &'a UiInput,
    pub art: &'a Art,
    pub pal: Palette,
    pub night: bool,
    pub size: Size,
    /// A controller is the last thing used: show pad hints.
    pub pad: bool,
    pub cmds: Vec<Cmd>,
}

impl<'a> Ui<'a> {
    pub fn new(cv: &'a mut Canvas, input: &'a UiInput, art: &'a Art, night: bool, pad: bool) -> Ui<'a> {
        let size = Size::of(cv.width());
        Ui { cv, input, art, pal: if night { NIGHT } else { PAPER }, night, size, pad, cmds: Vec::new() }
    }

    pub fn w(&self) -> f32 {
        self.cv.width()
    }

    pub fn h(&self) -> f32 {
        self.cv.height()
    }

    pub fn pad_x(&self) -> f32 {
        self.size.pad()
    }

    pub fn emit(&mut self, cmd: Cmd) {
        self.cmds.push(cmd);
    }

    /// A click inside the box, and inside the current clip if any.
    pub fn clicked(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if let Some(c) = self.cv.clip() {
            let (mx, my) = self.input.mouse;
            if mx < c.x as f32 || my < c.y as f32 || mx >= (c.x + c.w) as f32 || my >= (c.y + c.h) as f32 {
                return false;
            }
        }
        self.input.click_in(x, y, w, h)
    }

    pub fn pressed(&self, k: Key) -> bool {
        self.input.pressed(k)
    }

    // --- colours the palette doesn't name -----------------------------------

    /// Muted text: `fg2`.
    pub fn muted(&self) -> Rgb {
        self.pal.fg2
    }

    /// Fainter than muted: footers, hints, disabled rows.
    pub fn faded(&self) -> Rgb {
        mix(self.pal.fg2, self.pal.bg, 0.25)
    }

    /// The aside and selected-row panel.
    pub fn panel(&self) -> Rgb {
        self.pal.bg2
    }

    // --- type ------------------------------------------------------------------

    pub fn section_label(&self) -> Style {
        Style::interface_bold(13.0).upper().tracking(2.2).color(self.pal.fg)
    }

    pub fn note(&self) -> Style {
        Style::interface_bold(11.0).upper().tracking(1.8).color(self.muted())
    }

    pub fn body(&self) -> Style {
        Style::interface(14.0).color(self.pal.fg)
    }

    pub fn mono(&self) -> Style {
        Style::data(13.0).color(self.muted())
    }

    pub fn reading(&self) -> Style {
        Style::reading(15.0).color(self.pal.fg)
    }

    // --- controls ---------------------------------------------------------------

    /// A section heading with its note at the right and a rule under both;
    /// the mocks' "ALL GAMES · 12 ........ MOST RECENT FIRST". Returns the y
    /// below the rule.
    pub fn section(&mut self, x: f32, y: f32, w: f32, title: &str, note: Option<&str>) -> f32 {
        let st = self.section_label();
        self.cv.text(x, y, title, &st);
        if let Some(note) = note {
            let st = self.note();
            self.cv.text_right(x + w, y + 2.0, note, &st);
        }
        self.cv.hrule(x, x + w, y + 24.0, 1.0, self.pal.line);
        y + 25.0
    }

    pub fn small_button_width(&mut self, label: &str) -> f32 {
        (self.cv.measure(label, &Style::interface_bold(13.0)) + 26.0).round()
    }

    /// Outline button, 34 high (toolbar size). Returns (width, clicked).
    pub fn small_button(&mut self, x: f32, y: f32, label: &str) -> (f32, bool) {
        let st = Style::interface_bold(13.0).color(self.pal.fg);
        let w = self.small_button_width(label);
        self.cv.stroke_rect(x, y, w, 34.0, 1.5, self.pal.fg);
        self.cv.text(x + 13.0, y + 9.0, label, &st);
        (w, self.clicked(x, y, w, 34.0))
    }

    /// Segmented control (Recent | A–Z, Stable | Nightly). Returns the
    /// index clicked this frame, if any.
    pub fn segmented(&mut self, x: f32, y: f32, options: &[&str], selected: usize) -> (f32, Option<usize>) {
        let mut cx = x;
        let mut hit = None;
        for (i, label) in options.iter().enumerate() {
            let on = i == selected;
            let st = Style::interface_bold(13.0).color(if on { self.pal.bg } else { self.pal.fg });
            let w = (self.cv.measure(label, &st) + 26.0).round();
            if on {
                self.cv.fill_rect(cx, y, w, 34.0, self.pal.fg);
            }
            self.cv.stroke_rect(cx, y, w, 34.0, 1.5, self.pal.fg);
            self.cv.text(cx + 13.0, y + 9.0, label, &st);
            if self.clicked(cx, y, w, 34.0) && !on {
                hit = Some(i);
            }
            cx += w - 1.5;
        }
        (cx - x + 1.5, hit)
    }

    /// Tick box + label; returns (height, clicked). Disabled rows are faded
    /// and never report a click.
    pub fn check(&mut self, x: f32, y: f32, label: &str, on: bool, enabled: bool) -> (f32, bool) {
        let mut pal = self.pal;
        if !enabled {
            pal.fg = self.faded();
        }
        let w = widgets::tick_option(self.cv, x, y, label, on, &pal, false);
        (22.0, enabled && self.clicked(x - 4.0, y - 4.0, w + 8.0, 24.0))
    }

    /// Underlined text link; returns (width, clicked).
    pub fn link(&mut self, x: f32, y: f32, label: &str, color: Rgb) -> (f32, bool) {
        let st = Style::interface(13.0).color(color);
        let w = self.cv.text(x, y, label, &st);
        let (asc, _, _) = self.cv.fonts.line_metrics(Face::Interface, 13.0);
        self.cv.underline(x, y + asc + 3.0, w, color);
        (w, self.clicked(x, y - 2.0, w, 20.0))
    }

    pub fn key_width(&mut self, label: &str) -> f32 {
        if self.pad {
            let tw = self.cv.measure(label, &Style::interface_bold(11.0));
            if label.chars().count() > 1 { tw + 14.0 } else { 24.0 }
        } else {
            widgets::keycap_width(self.cv, label)
        }
    }

    /// Keycap: square for the keyboard, round for the pad (the mocks' Ⓐ).
    pub fn key(&mut self, x: f32, y: f32, label: &str) -> f32 {
        if self.pad {
            let st = Style::interface_bold(11.0).color(self.pal.fg);
            let tw = self.cv.measure(label, &st);
            let w = self.key_width(label);
            self.cv.stroke_round_rect(x, y - 1.0, w, 24.0, [12.0; 4], 1.5, self.pal.fg);
            self.cv.text(x + (w - tw) / 2.0, y + 4.0, label, &st);
            w
        } else {
            widgets::keycap(self.cv, x, y, label, &self.pal, false)
        }
    }

    /// Clip drawing (and clicks) to a box until `unclip`.
    pub fn clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.cv.set_clip(Some(Clip { x: x as i32, y: y as i32, w: w.max(0.0) as i32, h: h.max(0.0) as i32 }));
    }

    pub fn unclip(&mut self) {
        self.cv.set_clip(None);
    }
}
