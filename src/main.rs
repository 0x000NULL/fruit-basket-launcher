//! Fruit Basket launcher: the game library and the emulators, from one window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod art;
mod basket;
mod feed;
mod history;
mod jobs;
mod key;
mod platform;
mod queue;
mod settings;
mod ui;
mod window;

fn main() {
    if let Err(e) = app::run() {
        eprintln!("fruitbasket: {e}");
        std::process::exit(1);
    }
}
