// SPDX-License-Identifier: GPL-3.0-or-later
//! Flatpak apps in the app menu, to flag AppImages that duplicate one: with
//! both installed the menu shows the app twice and it is unclear which runs.
//! Read from the menu entries Flatpak exports (user and system installations);
//! nothing is run.
use crate::{desktop::DesktopFile, paths::Paths};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatpakApp {
    /// The Flatpak app id, e.g. ai.storyteller.deckcraft.
    pub id: String,
    pub name: String,
    pub wm_class: Option<String>,
}

/// Folders with the menu entries Flatpak exports. GEARLEVER_COSMIC_SYSTEM_ROOT
/// moves the system installation (for tests, as in the requirements check).
fn export_dirs(paths: &Paths) -> Vec<PathBuf> {
    let system_root = std::env::var_os("GEARLEVER_COSMIC_SYSTEM_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if crate::host::sandboxed() {
                "/run/host".into()
            } else {
                "/".into()
            }
        });
    vec![
        paths.share.join("flatpak/exports/share/applications"),
        system_root.join("var/lib/flatpak/exports/share/applications"),
    ]
}

/// Every Flatpak app with a menu entry. A user installation shadows a system
/// one with the same id.
pub fn installed(paths: &Paths) -> Vec<FlatpakApp> {
    let mut apps: Vec<FlatpakApp> = Vec::new();
    for dir in export_dirs(paths) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        files.sort();
        for file in files {
            let id = file
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if apps.iter().any(|a| a.id == id) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let desktop = DesktopFile::parse(&text);
            let Some(name) = desktop.main("Name") else {
                continue;
            };
            apps.push(FlatpakApp {
                id,
                name,
                wm_class: desktop.main("StartupWMClass").filter(|c| !c.is_empty()),
            });
        }
    }
    apps
}

/// The Flatpak an AppImage duplicates: the same window class (the most exact
/// sign), a Flatpak id equal to that class, or the same name.
pub fn matching<'a>(
    flatpaks: &'a [FlatpakApp],
    names: &[&str],
    wm_class: Option<&str>,
) -> Option<&'a FlatpakApp> {
    let wm_class = wm_class.filter(|c| !c.is_empty());
    if let Some(c) = wm_class
        && let Some(f) = flatpaks.iter().find(|f| {
            f.wm_class
                .as_deref()
                .is_some_and(|w| w.eq_ignore_ascii_case(c))
                || f.id.eq_ignore_ascii_case(c)
        })
    {
        return Some(f);
    }
    flatpaks.iter().find(|f| {
        names
            .iter()
            .any(|n| !n.is_empty() && f.name.eq_ignore_ascii_case(n))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, wm: Option<&str>) -> FlatpakApp {
        FlatpakApp {
            id: id.into(),
            name: name.into(),
            wm_class: wm.map(Into::into),
        }
    }

    #[test]
    fn matches_by_window_class_id_or_name() {
        let list = [
            app(
                "ai.storyteller.deckcraft",
                "DeckCraft",
                Some("ai.storyteller.deckcraft"),
            ),
            app("org.example.Other", "Other", None),
        ];
        let by_class = matching(&list, &["Something"], Some("ai.storyteller.deckcraft"));
        assert_eq!(by_class.unwrap().id, "ai.storyteller.deckcraft");
        let by_id = matching(&list, &[], Some("org.example.other"));
        assert_eq!(by_id.unwrap().id, "org.example.Other");
        let by_name = matching(&list, &["deckcraft"], None);
        assert_eq!(by_name.unwrap().id, "ai.storyteller.deckcraft");
        assert!(matching(&list, &["Mendeley"], Some("Mendeley")).is_none());
        assert!(matching(&list, &[""], Some("")).is_none());
    }

    #[test]
    fn reads_the_user_installation_exports() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        let user = paths.share.join("flatpak/exports/share/applications");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("ai.storyteller.deckcraft.desktop"),
            "[Desktop Entry]\nName=DeckCraft\nStartupWMClass=ai.storyteller.deckcraft\n\n[Desktop Action x]\nName=Other\n",
        )
        .unwrap();
        fs::write(user.join("notes.txt"), "not an entry").unwrap();
        let found: Vec<_> = installed(&paths)
            .into_iter()
            .filter(|a| a.id == "ai.storyteller.deckcraft")
            .collect();
        assert_eq!(
            found,
            [app(
                "ai.storyteller.deckcraft",
                "DeckCraft",
                Some("ai.storyteller.deckcraft")
            )]
        );
    }
}
