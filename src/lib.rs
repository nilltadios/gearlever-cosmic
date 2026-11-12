// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared AppImage backend used by both the GUI and the headless CLI.
pub mod appimage;
pub mod automation;
pub mod cli_link;
pub mod desktop;
pub mod elf;
pub mod flatpak;
pub mod host;
pub mod library;
pub mod ops;
pub mod paths;
pub mod relaunch;
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

/// The command that starts the agent CLI, as a user would type it: the
/// AppImage's full path plus `cli` when running from an AppImage (or the
/// symlink it was started through), the CLI's name when that resolves on
/// PATH to this build, its full path otherwise.
pub fn cli_command() -> String {
    const NAME: &str = "gearlever-cosmic-cli";
    if let Some(appimage) = appimage_self() {
        if let Some(arg0) = std::env::var_os("ARGV0").or_else(|| std::env::var_os("ARG0")) {
            let arg0 = std::path::PathBuf::from(arg0);
            if arg0
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(NAME))
            {
                return shell_quote(&arg0.to_string_lossy());
            }
        }
        return format!("{} cli", shell_quote(&appimage.to_string_lossy()));
    }
    let Some(own) = std::env::current_exe().ok().map(|e| e.with_file_name(NAME)) else {
        return NAME.into();
    };
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|d| {
            let c = d.join(NAME);
            c.is_file() && c.canonicalize().ok() == own.canonicalize().ok()
        })
    });
    if on_path || !own.is_file() {
        NAME.into()
    } else {
        shell_quote(&own.to_string_lossy())
    }
}

/// Quote a word for a POSIX shell only when it needs it.
pub fn shell_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@%~".contains(c))
    {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// Bundled text with the CLI's name replaced by [`cli_command`].
pub fn with_cli_command(text: &str) -> String {
    replace_cli_command(text, &cli_command())
}

fn replace_cli_command(text: &str, command: &str) -> String {
    if command == "gearlever-cosmic-cli" {
        return text.into();
    }
    text.replace("gearlever-cosmic-cli ", &format!("{command} "))
}

/// Variables an AppImage runtime sets for itself; programs this app starts
/// must not inherit them.
pub const APPIMAGE_ENV: [&str; 5] = ["APPIMAGE", "APPDIR", "ARGV0", "ARG0", "OWD"];

#[cfg(test)]
mod tests {
    #[test]
    fn shell_quote_leaves_plain_paths_and_quotes_the_rest() {
        assert_eq!(
            super::shell_quote("/home/me/AppImages/gear_lever_cosmic.appimage"),
            "/home/me/AppImages/gear_lever_cosmic.appimage"
        );
        assert_eq!(super::shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }

    /// The guide and bundled docs show the command that really starts the
    /// CLI. Release files are named GearLever_COSMIC-VERSION-x86_64.AppImage
    /// and users rename them, so no text may name one fixed file.
    #[test]
    fn bundled_texts_name_the_real_cli_command() {
        let guide = include_str!("guide.txt");
        let appimage = "/home/me/AppImages/gear_lever_cosmic.appimage cli";
        let shown = super::replace_cli_command(guide, appimage);
        assert!(shown.contains(&format!("Run every command as:  {appimage} ...")));
        assert!(shown.starts_with("Gear Lever COSMIC: agent guide"));
        assert!(shown.contains(&format!("{appimage} manages AppImages")));
        let texts =
            std::iter::once(guide).chain(crate::automation::DOCS.iter().map(|(.., text)| *text));
        for text in texts {
            assert!(
                !text.contains("GearLever_COSMIC-x86_64.AppImage"),
                "a bundled text names a fixed AppImage file"
            );
        }
    }

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
