// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings shared by the window and the CLI. Gear Lever's preferences map
//! one to one; the last three are new here.
use crate::paths::Paths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Folder that holds integrated AppImages. A leading ~ means the home folder.
    pub appimages_folder: String,
    /// true: move the opened file into the folder. false: copy it and keep the original.
    pub move_on_integrate: bool,
    /// Also list menu entries whose AppImage is outside the folder.
    pub show_external: bool,
    /// Name terminal apps after their executable without an extension (for $PATH use).
    pub terminal_apps_use_executable_name: bool,
    /// Read metadata only with the built-in reader; never run the AppImage to extract it.
    pub safe_metadata: bool,
    /// Check for updates after login and send a notification. Never installs anything.
    pub check_updates_on_startup: bool,
    /// Open new files in a compact preview.
    pub preview_minimal: bool,
    /// More detailed logs on stderr.
    pub debug_logs: bool,
    /// Keep the version an update replaces in the desktop Trash (recoverable).
    pub keep_replaced_versions_in_trash: bool,
    pub theme: Theme,
    /// Removed in 0.1.4 together with the "Menu outdated" check. Still
    /// accepted so settings files written by 0.1.3 load; never written back.
    #[serde(rename = "sync_menu_entries", skip_serializing)]
    #[schemars(skip)]
    legacy_sync_menu_entries: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appimages_folder: "~/AppImages".into(),
            move_on_integrate: true,
            show_external: true,
            terminal_apps_use_executable_name: false,
            safe_metadata: true,
            check_updates_on_startup: false,
            preview_minimal: true,
            debug_logs: false,
            keep_replaced_versions_in_trash: true,
            theme: Theme::System,
            legacy_sync_menu_entries: None,
        }
    }
}

impl Settings {
    pub fn path(paths: &Paths) -> std::path::PathBuf {
        paths.config.join("settings.json")
    }
    pub fn load(paths: &Paths) -> Result<Self, String> {
        let path = Self::path(paths);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Cannot read settings {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
        }
    }
    pub fn save(&self, paths: &Paths) -> Result<(), String> {
        write_json(&Self::path(paths), self)
    }
    pub fn folder(&self, paths: &Paths) -> std::path::PathBuf {
        paths.expand(&self.appimages_folder)
    }
}

/// Atomically replace a small JSON file. An unchanged file is left alone; the
/// previous version of a changed file is kept as `NAME.bak-STAMP` (the last
/// five are retained).
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if fs::read(path).is_ok_and(|old| old == data) {
        return Ok(());
    }
    let parent = path.parent().ok_or("The file has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    let stamp = crate::now_nanos();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if path.exists() {
        fs::copy(path, parent.join(format!("{name}.bak-{stamp}")))
            .map_err(|e| format!("Cannot back up {}: {e}", path.display()))?;
        prune_backups(parent, &name, 5);
    }
    let temporary = parent.join(format!(".{name}-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

fn prune_backups(dir: &Path, name: &str, keep: usize) {
    let prefix = format!("{name}.bak-");
    let mut backups: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    backups.sort();
    while backups.len() > keep {
        let _ = fs::remove_file(backups.remove(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_0_1_3_settings_file_with_the_removed_sync_option_still_loads() {
        let s: Settings =
            serde_json::from_str(r#"{"sync_menu_entries":false,"debug_logs":true}"#).unwrap();
        assert!(s.debug_logs);
        let back = serde_json::to_string(&s).unwrap();
        assert!(!back.contains("sync_menu_entries"), "{back}");
        assert!(serde_json::from_str::<Settings>(r#"{"nope":1}"#).is_err());
    }
    #[test]
    fn settings_round_trip_and_keep_backups() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        assert_eq!(Settings::load(&paths).unwrap(), Settings::default());
        let mut s = Settings::default();
        for i in 0..8 {
            s.appimages_folder = format!("~/Apps{i}");
            s.save(&paths).unwrap();
        }
        assert_eq!(Settings::load(&paths).unwrap().appimages_folder, "~/Apps7");
        let backups = fs::read_dir(&paths.config)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".bak-")
            })
            .count();
        assert_eq!(backups, 5);
        assert!(serde_json::from_str::<Settings>(r#"{"unknown":1}"#).is_err());
    }
}
