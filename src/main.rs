//! Fruit Basket launcher: the game library and the emulators, from one window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod art;
mod basket;
mod compat;
mod dumps;
mod feed;
mod focus;
mod history;
mod jobs;
mod key;
mod launch;
mod library;
mod lists;
mod mover;
mod platform;
mod queue;
mod settings;
mod shelf;
mod ui;
mod update;
mod window;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        // Not println!: a shell that doesn't wait for a GUI program closes the pipe.
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{}", app::VERSION);
        return;
    }
    // `--updated vX`: started by the old launcher after a swap.
    let updated = args.windows(2).find(|w| w[0] == "--updated").map(|w| w[1].clone());
    if updated.is_none() && swap_in_update() {
        return;
    }
    if let Err(e) = app::run(updated) {
        eprintln!("fruitbasket: {e}");
        std::process::exit(1);
    }
}

/// A staged launcher update goes in before the window opens: swap the exe,
/// start the new one and leave. True if it did.
fn swap_in_update() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let dir = update::dir(&settings::Settings::load().basket().launcher_dir());
    match update::apply_staged(&dir, &exe, app::VERSION) {
        Ok(Some(build)) => match std::process::Command::new(&exe).arg("--updated").arg(app::VERSION).spawn() {
            Ok(_) => return true,
            Err(e) => {
                eprintln!("fruitbasket: starting {build}: {e}; staying on {}", app::VERSION);
                update::undo(&exe);
            }
        },
        Ok(None) => update::clean(&dir, &exe),
        Err(e) => eprintln!("fruitbasket: {e}"),
    }
    false
}
