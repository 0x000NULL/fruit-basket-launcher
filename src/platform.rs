//! The few things that differ per OS: dark mode, free space, and handing a
//! folder, file or link to the system.

use std::path::Path;
use std::process::Command;

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flash from a windowed app.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn output(cmd: &mut Command) -> Option<String> {
    let out = quiet(cmd).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether the OS is in dark mode. Spawns a process, so call it rarely.
pub fn os_dark() -> bool {
    if cfg!(windows) {
        output(Command::new("reg").args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ]))
        .is_some_and(|s| s.contains("0x0"))
    } else if cfg!(target_os = "macos") {
        output(Command::new("defaults").args(["read", "-g", "AppleInterfaceStyle"])).is_some_and(|s| s.contains("Dark"))
    } else {
        output(Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]))
            .is_some_and(|s| s.contains("dark"))
    }
}

/// The nearest folder at or above `path` that exists: the basket's root
/// may not have been made yet.
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.is_dir())
}

/// Bytes free for this user on the volume holding `path`.
#[cfg(windows)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let wide: Vec<u16> = existing(path)?.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut avail = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the null
    // out-pointers are optional in the API.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(avail)
}

/// Bytes free for this user on the volume holding `path`.
#[cfg(not(windows))]
pub fn free_space(path: &Path) -> Option<u64> {
    let out = output(Command::new("df").arg("-Pk").arg(existing(path)?))?;
    let kb: u64 = out.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// Open a folder, file or URL with whatever the system uses for it.
pub fn open(target: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("explorer");
        c.arg(target);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    let _ = quiet(&mut cmd).spawn();
}

/// Show a file selected in its folder.
pub fn reveal(path: &Path) {
    if cfg!(windows) {
        let _ = quiet(Command::new("explorer").arg(format!("/select,{}", path.display()))).spawn();
    } else if cfg!(target_os = "macos") {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    } else if let Some(dir) = path.parent() {
        open(&dir.to_string_lossy());
    }
}

/// `C:\Users\me\FruitBasket` → `~\FruitBasket`, for paths shown in the UI.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            let sep = std::path::MAIN_SEPARATOR;
            return if rest.as_os_str().is_empty() { "~".into() } else { format!("~{sep}{}", rest.display()) };
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn free_space_of_a_folder_not_made_yet() {
        let t = tempfile::tempdir().unwrap();
        assert!(super::free_space(&t.path().join("not/yet")).is_some_and(|n| n > 0));
    }
}
