// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-app configuration kept by this app (apps.json), and read-only access
//! to Gear Lever's saved update sources (gearlever.conf) for migration.
use crate::{appimage::Fingerprint, paths::Paths, updates::Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AppConfig {
    pub update_source: Option<Source>,
    pub website: Option<String>,
    /// Arguments the AppImage itself shipped when it was integrated. Used to
    /// tell customized arguments from defaults on update.
    pub default_arguments: Option<Vec<String>>,
    /// The file as it was when this app last wrote its menu entry.
    pub synced: Option<Fingerprint>,
    pub synced_version: Option<String>,
    /// An icon the user chose; kept across updates and menu refreshes.
    pub custom_icon: Option<PathBuf>,
    pub integrated_at: Option<u64>,
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub schema: u32,
    /// Keyed by the AppImage's absolute path.
    pub apps: BTreeMap<PathBuf, AppConfig>,
}

impl Store {
    pub fn path(paths: &Paths) -> PathBuf {
        paths.config.join("apps.json")
    }
    pub fn load(paths: &Paths) -> Result<Self, String> {
        let path = Self::path(paths);
        match fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| format!("Cannot read {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema: 1,
                ..Self::default()
            }),
            Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
        }
    }
    pub fn save(&self, paths: &Paths) -> Result<(), String> {
        crate::settings::write_json(&Self::path(paths), self)
    }
    pub fn get(&self, path: &Path) -> AppConfig {
        self.apps.get(path).cloned().unwrap_or_default()
    }
    pub fn entry(&mut self, path: &Path) -> &mut AppConfig {
        self.schema = 1;
        self.apps.entry(path.to_path_buf()).or_default()
    }
    /// Move an app's configuration when its file is renamed.
    pub fn rename(&mut self, from: &Path, to: &Path) {
        if let Some(c) = self.apps.remove(from) {
            self.apps.insert(to.to_path_buf(), c);
        }
    }
}

/// Gear Lever's per-app settings, keyed by AppImage path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GearLeverApp {
    pub file: Option<PathBuf>,
    pub source: Option<Source>,
    /// A saved but unusable source, with the reason.
    pub source_error: Option<String>,
    pub website: Option<String>,
    pub default_arguments: Option<String>,
}

fn md5_hex(text: &str) -> String {
    use md5::Digest;
    crate::appimage::hex(&md5::Md5::digest(text.as_bytes()))
}

/// Parse gearlever.conf (Python configparser format). Section `app.MD5`
/// holds file_path/website/default_exec_arguments; `app.MD5.update_manager`
/// holds manager plus that manager's keys. MD5 is of the file path.
pub fn parse_gearlever(text: &str) -> BTreeMap<String, GearLeverApp> {
    let mut sections: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut current = None;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            current = Some(t[1..t.len() - 1].to_string());
            sections.entry(current.clone().unwrap()).or_default();
            continue;
        }
        if let (Some(s), Some((k, v))) = (&current, t.split_once(['=', ':'])) {
            sections
                .get_mut(s)
                .unwrap()
                .push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut out: BTreeMap<String, GearLeverApp> = BTreeMap::new();
    for (name, pairs) in &sections {
        let Some(rest) = name.strip_prefix("app.") else {
            continue;
        };
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };
        if let Some(hash) = rest.strip_suffix(".update_manager") {
            let app = out.entry(hash.to_string()).or_default();
            if let Some(manager) = get("manager") {
                let fields: Vec<(String, String)> = pairs
                    .iter()
                    .filter(|(k, _)| k != "manager")
                    .cloned()
                    .collect();
                match Source::from_pairs(&manager, &fields) {
                    Ok(s) => app.source = Some(s),
                    Err(e) => app.source_error = Some(e),
                }
            }
        } else {
            let app = out.entry(rest.to_string()).or_default();
            app.file = get("file_path").map(PathBuf::from);
            app.website = get("website");
            app.default_arguments = get("default_exec_arguments");
        }
    }
    out
}

/// Gear Lever's settings for a file, if it saved any.
pub fn gearlever_for(paths: &Paths, file: &Path) -> Option<GearLeverApp> {
    let key = md5_hex(&file.to_string_lossy());
    for conf in paths.gearlever_configs() {
        if let Ok(text) = fs::read_to_string(&conf)
            && let Some(app) = parse_gearlever(&text).remove(&key)
        {
            return Some(app);
        }
    }
    None
}

pub fn gearlever_all(paths: &Paths) -> BTreeMap<String, GearLeverApp> {
    let mut all = BTreeMap::new();
    for conf in paths.gearlever_configs() {
        if let Ok(text) = fs::read_to_string(&conf) {
            all.extend(parse_gearlever(&text));
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gear_lever_settings_are_read_by_file_path() {
        let path = "/home/u/AppImages/chatgpt.appimage";
        let h = md5_hex(path);
        let conf = format!(
            "[DEFAULT]\nfetch-updates-in-background = no\n\n[app.{h}]\ndefault_exec_arguments = --show %u\nname = x\nfile_path = {path}\n\n[app.{h}.update_manager]\nrepo = bruglet/chatgpt-desktop-linux\nrepo_filename = codex-desktop-*-x86_64.AppImage\nallow_prereleases = False\nmanager = GithubUpdater\n\n[app.ffff.update_manager]\n\n"
        );
        let all = parse_gearlever(&conf);
        let app = &all[&h];
        assert_eq!(app.file.as_deref(), Some(Path::new(path)));
        assert_eq!(
            app.source,
            Some(Source::Github {
                repo: "bruglet/chatgpt-desktop-linux".into(),
                repo_filename: "codex-desktop-*-x86_64.AppImage".into(),
                allow_prereleases: false
            })
        );
        assert_eq!(app.default_arguments.as_deref(), Some("--show %u"));
        assert!(all["ffff"].source.is_none());
    }
    #[test]
    fn store_renames_keep_config() {
        let mut s = Store::default();
        s.entry(Path::new("/a")).website = Some("https://x".into());
        s.rename(Path::new("/a"), Path::new("/b"));
        assert_eq!(s.get(Path::new("/b")).website.as_deref(), Some("https://x"));
    }
}
