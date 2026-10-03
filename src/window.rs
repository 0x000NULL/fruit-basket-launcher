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
    buf: Vec<u32>,
    chars: Arc<Mutex<Vec<char>>>,
    mouse_was_down: bool,
    pad_repeat: PadRepeat,
}

impl Video {
    pub fn new(title: &str, w: usize, h: usize, map: &KeyMap) -> Result<Video, String> {
        let opts = WindowOptions { resize: true, scale: Scale::X1, scale_mode: ScaleMode::Stretch, ..WindowOptions::default() };
        let mut window = Window::new(title, w, h, opts).map_err(|e| format!("creating window: {e}"))?;
        window.set_target_fps(0);
        let chars = Arc::new(Mutex::new(Vec::new()));
        window.set_input_callback(Box::new(CharSink(chars.clone())));
        Ok(Video { window, buf: Vec::new(), chars, mouse_was_down: false, pad_repeat: PadRepeat::new(map.set()) })
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

    pub fn ctrl(&self) -> bool {
        self.window.is_key_down(Key::LeftCtrl) || self.window.is_key_down(Key::RightCtrl)
    }
}
