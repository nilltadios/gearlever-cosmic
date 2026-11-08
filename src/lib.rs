// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared AppImage backend used by both the GUI and the headless CLI.
pub mod appimage;
pub mod automation;
pub mod desktop;
pub mod elf;
pub mod host;
pub mod library;
pub mod ops;
pub mod paths;
pub mod requirements;
pub mod settings;
pub mod store;
pub mod ui_client;
pub mod ui_protocol;
pub mod updates;

pub const APP_ID: &str = "dev.nilltadios.GearLeverCosmic";

pub fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
pub fn now() -> u64 {
    (now_nanos() / 1_000_000_000) as u64
}

/// The AppImage this program runs from, if any. `APPIMAGE` alone is not
/// proof: a parent AppImage (a terminal or another app started from one)
/// passes it on, so this program's executable must also live inside that
/// AppImage's `APPDIR`.
pub fn appimage_self() -> Option<std::path::PathBuf> {
    let appdir = std::path::PathBuf::from(std::env::var_os("APPDIR")?);
    let exe = std::env::current_exe().ok()?;
    if !appdir.is_absolute() || !exe.starts_with(&appdir) {
        return None;
    }
    std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
}

/// Variables an AppImage runtime sets for itself; programs this app starts
/// must not inherit them.
pub const APPIMAGE_ENV: [&str; 5] = ["APPIMAGE", "APPDIR", "ARGV0", "ARG0", "OWD"];

#[cfg(test)]
mod tests {
    #[test]
    fn an_inherited_appimage_variable_is_not_our_appimage() {
        // Tests run from target/, never inside an APPDIR.
        unsafe {
            std::env::set_var("APPIMAGE", "/bin/sh");
            std::env::set_var("APPDIR", "/tmp/.mount_other");
        }
        assert_eq!(super::appimage_self(), None);
    }
}
