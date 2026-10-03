//! The minifb window: present a canvas, gather one frame of input. The same
//! shape as Strawberry's `window.rs`, minus the game path.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use basket_app::pads::PadPoll;
use basket_ui::input::{actions, KeyMap, PadRepeat, UiInput};
use basket_ui::Canvas;
use minifb::{InputCallback, Key, KeyRepeat, MouseButton, MouseMode, Scale, ScaleMode, Window, WindowOptions};

struct CharSink(Arc<Mutex<Vec<char>>>);

impl InputCallback for CharSink {
    fn add_char(&mut self, uni_char: u32) {
        if let Some(c) = char::from_u32(uni_char) {
            if !c.is_control() {
                self.0.lock().unwrap().push(c);
            }
        }
    }
}

pub struct Video {
    pub window: Window,
    /// Couch mode: borderless over the whole screen. Holds the windowed
    /// size to go back to.
    pub couch: Option<(usize, usize)>,
    buf: Vec<u32>,
    chars: Arc<Mutex<Vec<char>>>,
    mouse_was_down: bool,
    pad_repeat: PadRepeat,
}

impl Video {
    pub fn new(title: &str, w: usize, h: usize, map: &KeyMap) -> Result<Video, String> {
        let chars = Arc::new(Mutex::new(Vec::new()));
        let window = open(title, w, h, false, &chars)?;
        Ok(Video { window, couch: None, buf: Vec::new(), chars, mouse_was_down: false, pad_repeat: PadRepeat::new(map.set()) })
    }

    /// Into couch mode (a borderless window the size of the screen) or
    /// back to the window it came from. minifb fixes a window's style when
    /// it is made, so this makes a new one.
    pub fn set_couch(&mut self, on: bool) -> Result<(), String> {
        if on == self.couch.is_some() {
            return Ok(());
        }
        let (w, h) = match self.couch {
            Some(windowed) => windowed,
            None => crate::platform::screen_size().unwrap_or_else(|| self.size()),
        };
        let windowed = self.size();
        let mut window = open("Fruit Basket", w, h, on, &self.chars)?;
        if on {
            window.set_position(0, 0);
        }
        self.window = window;
        self.couch = on.then_some(windowed);
        self.mouse_was_down = false;
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    pub fn size(&self) -> (usize, usize) {
        let (w, h) = self.window.get_size();
        (w.max(1), h.max(1))
    }

    pub fn present(&mut self, canvas: &Canvas) {
        let (w, h) = self.size();
        canvas.to_0rgb(&mut self.buf);
        if self.buf.len() != w * h {
            // Resized between layout and present: draw again next frame.
            self.window.update();
            return;
        }
        let _ = self.window.update_with_buffer(&self.buf, w, h);
    }

    pub fn gather_input(&mut self, map: &KeyMap, pad: PadPoll, now: Instant) -> UiInput {
        let pressed = self.window.get_keys_pressed(KeyRepeat::No);
        let repeated = self.window.get_keys_pressed(KeyRepeat::Yes);
        let down = self.window.get_keys();
        let chars: Vec<char> = std::mem::take(&mut *self.chars.lock().unwrap());
        let mouse = self.window.get_mouse_pos(MouseMode::Pass).unwrap_or((-1.0, -1.0));
        let mouse_down = self.window.get_mouse_down(MouseButton::Left);
        let clicked = mouse_down && !self.mouse_was_down;
        self.mouse_was_down = mouse_down;
        let wheel = self.window.get_scroll_wheel().map(|(_, y)| y).unwrap_or(0.0);
        let (pad_pressed, pad_repeated) = self.pad_repeat.update(pad.mask, now);
        let acts = actions(map, &pressed, &repeated, pad_pressed, pad_repeated);
        let game_mask = map.mask(|k| down.contains(&k)) | pad.mask;
        UiInput { pressed, repeated, down, chars, mouse, mouse_down, clicked, wheel, actions: acts, pad_buttons: pad.pressed, game_mask }
    }

    /// The windowed size, also while in couch mode: what to save on exit.
    pub fn windowed_size(&self) -> (usize, usize) {
        self.couch.unwrap_or_else(|| self.size())
    }

    pub fn ctrl(&self) -> bool {
        self.window.is_key_down(Key::LeftCtrl) || self.window.is_key_down(Key::RightCtrl)
    }
}

fn open(title: &str, w: usize, h: usize, borderless: bool, chars: &Arc<Mutex<Vec<char>>>) -> Result<Window, String> {
    let opts = WindowOptions { resize: !borderless, borderless, scale: Scale::X1, scale_mode: ScaleMode::Stretch, ..WindowOptions::default() };
    let mut window = Window::new(title, w, h, opts).map_err(|e| format!("creating window: {e}"))?;
    window.set_target_fps(0);
    window.set_input_callback(Box::new(CharSink(chars.clone())));
    Ok(window)
}
