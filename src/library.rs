// SPDX-License-Identifier: GPL-3.0-or-later
//! The installed-apps inventory. An app is a menu entry in
//! ~/.local/share/applications that starts an AppImage, exactly as Gear Lever
//! finds them, so apps integrated by Gear Lever appear here too.
//!
//! Name and version come from the AppImage itself, never only from the menu
//! entry (Gear Lever issue #497). When the two differ the app is marked
//! `menu_outdated` and can be synced.
use crate::{
    appimage::{self, Fingerprint, Metadata},
    desktop::{DesktopFile, Exec},
    paths::Paths,
    settings::Settings,
    store::{self, Store},
    updates::{Origin, Source},
};
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedBy {
    /// Integrated by Gear Lever COSMIC.
    GearLeverCosmic,
    /// Integrated by Gear Lever (X-AppImage-Name, TryExec).
    GearLever,
    /// A menu entry made by the app itself or another tool.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct App {
    /// Stable identifier: the menu entry's file name without .desktop.
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    /// Version read from the AppImage file now.
    pub version: Option<String>,
    pub path: PathBuf,
    pub size: u64,
    pub desktop_file: PathBuf,
    /// Other menu entries that start the same file (for example one the app wrote itself).
    pub other_desktop_files: Vec<PathBuf>,
    pub icon: Option<PathBuf>,
    /// A theme icon name from the menu entry, when it uses one instead of a file.
    pub icon_name: Option<String>,
    /// The menu entry uses an icon other than the one shipped in the AppImage.
    pub icon_custom: bool,
    /// Outside the AppImages folder.
    pub external: bool,
    pub managed_by: ManagedBy,
    pub terminal: bool,
    pub arch: String,
    pub appimage_type: u8,
    pub arguments: Vec<String>,
    pub default_arguments: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Name and version shown by the menu entry.
    pub menu_name: Option<String>,
    pub menu_version: Option<String>,
    /// The menu entry no longer matches the file (version, name or icon).
    pub menu_outdated: Vec<String>,
    /// The file changed since this app last wrote its menu entry (replaced or self-updated).
    pub changed_since_sync: bool,
    pub update_info: Option<String>,
    pub self_updater: Option<appimage::SelfUpdater>,
    pub update_source: Option<Source>,
    pub update_source_origin: Option<Origin>,
    /// A Gear Lever source that could not be used, with the reason.
    pub update_source_error: Option<String>,
    /// A Flatpak of the same app is installed too (its app id): the app menu
    /// shows both, and it is unclear which one starts.
    pub flatpak: Option<String>,
    /// The standard file name this app could be renamed to (`app.rename-file`),
    /// when its current name differs, e.g. a version left from a first install.
    pub standard_path: Option<PathBuf>,
    pub website: Option<String>,
    pub metadata_error: Option<String>,
    #[serde(skip)]
    pub metadata: Option<Metadata>,
}

impl App {
    pub fn display_version(&self) -> String {
        self.version
            .clone()
            .unwrap_or_else(|| "Unknown version".into())
    }
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Inventory {
    pub apps: Vec<App>,
    pub folder: PathBuf,
    pub applications_dir: PathBuf,
    pub scanned_entries: usize,
    pub warnings: Vec<String>,
}

impl Inventory {
    /// Find an app by id, AppImage path, menu entry path or exact name.
    pub fn find(&self, target: &str) -> Result<&App, String> {
        let path = Path::new(target);
        if let Some(a) = self.apps.iter().find(|a| a.id == target) {
            return Ok(a);
        }
        let by_path: Vec<_> = self
            .apps
            .iter()
            .filter(|a| {
                a.path == path
                    || a.desktop_file == path
                    || (path.is_absolute()
                        && fs::canonicalize(path).ok().as_deref() == Some(a.path.as_path()))
            })
            .collect();
        if let [a] = by_path.as_slice() {
            return Ok(a);
        }
        let by_name: Vec<_> = self
            .apps
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case(target))
            .collect();
        match by_name.as_slice() {
            [a] => Ok(a),
            [] => Err(format!(
                "No integrated AppImage {target}. Known ids: {}",
                self.apps
                    .iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            many => Err(format!(
                "Several apps are named {target}; use an id: {}",
                many.iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

pub const OWN_KEY: &str = "X-GearLeverCosmic";

struct Entry {
    file: PathBuf,
    desktop: DesktopFile,
    target: PathBuf,
    try_exec: bool,
}

fn entry_target(desktop: &DesktopFile) -> Option<(PathBuf, bool)> {
    if let Some(t) = desktop.main("TryExec") {
        return Some((PathBuf::from(t), true));
    }
    let exec = Exec::parse(&desktop.main("Exec")?).ok()?;
    exec.program
        .starts_with('/')
        .then(|| (PathBuf::from(exec.program), false))
}

pub fn load(paths: &Paths, settings: &Settings) -> Inventory {
    let store = Store::load(paths);
    let mut inv = Inventory {
        folder: settings.folder(paths),
        applications_dir: paths.applications(),
        ..Inventory::default()
    };
    let store = match store {
        Ok(s) => s,
        Err(e) => {
            inv.warnings.push(e);
            Store::default()
        }
    };
    let gearlever = store::gearlever_all(paths);
    let flatpaks = crate::flatpak::installed(paths);
    let gl_by_path: BTreeMap<PathBuf, &store::GearLeverApp> = gearlever
        .values()
        .filter_map(|a| a.file.clone().map(|f| (f, a)))
        .collect();
    let Ok(dir) = fs::read_dir(paths.applications()) else {
        return inv;
    };
    let mut files: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
        .collect();
    files.sort();
    let mut by_target: BTreeMap<PathBuf, Vec<Entry>> = BTreeMap::new();
    for file in files {
        inv.scanned_entries += 1;
        let Ok(text) = fs::read_to_string(&file) else {
            inv.warnings.push(format!("Cannot read {}", file.display()));
            continue;
        };
        let desktop = DesktopFile::parse(&text);
        let Some((target, try_exec)) = entry_target(&desktop) else {
            continue;
        };
        if !target.is_file() || !appimage::is_appimage(&target) {
            continue;
        }
        let external = target.parent() != Some(inv.folder.as_path());
        if external && !settings.show_external {
            continue;
        }
        by_target.entry(target.clone()).or_default().push(Entry {
            file,
            desktop,
            target,
            try_exec,
        });
    }
    for (target, mut entries) in by_target {
        // Prefer an entry this app or Gear Lever wrote, then one with TryExec.
        entries.sort_by_key(|e| {
            (
                e.desktop.main(OWN_KEY).is_none(),
                e.desktop.main("X-AppImage-Name").is_none(),
                !e.try_exec,
            )
        });
        let primary = entries.remove(0);
        let menu = primary.desktop.clone();
        let mut app = build(
            paths,
            settings,
            &store,
            gl_by_path.get(&target).copied(),
            primary,
            entries,
            &inv.folder,
        );
        app.flatpak = flatpak_duplicate(
            &flatpaks,
            app.metadata.as_ref(),
            &menu,
            app.menu_name.as_deref(),
        );
        inv.apps.push(app);
    }
    inv.apps.sort_by_key(|a| a.name.to_lowercase());
    for i in 0..inv.apps.len() {
        inv.apps[i].standard_path = crate::ops::standard_path(paths, settings, &inv.apps[i]);
    }
    inv
}

/// The Flatpak this AppImage duplicates, by the window class and name of its
/// menu entry or of the desktop file inside the AppImage.
pub fn flatpak_duplicate(
    flatpaks: &[crate::flatpak::FlatpakApp],
    meta: Option<&Metadata>,
    menu: &DesktopFile,
    menu_name: Option<&str>,
) -> Option<String> {
    let embedded = meta
        .and_then(|m| m.desktop_entry.as_deref())
        .map(DesktopFile::parse);
    let wm_class = menu
        .main("StartupWMClass")
        .or_else(|| embedded.as_ref().and_then(|d| d.main("StartupWMClass")));
    let names = [
        meta.and_then(|m| m.name.as_deref()).unwrap_or(""),
        menu_name.unwrap_or(""),
    ];
    crate::flatpak::matching(flatpaks, &names, wm_class.as_deref()).map(|f| f.id.clone())
}

fn build(
    paths: &Paths,
    settings: &Settings,
    store: &Store,
    gearlever: Option<&store::GearLeverApp>,
    e: Entry,
    others: Vec<Entry>,
    folder: &Path,
) -> App {
    let config = store.get(&e.target);
    let meta = appimage::inspect_cached(paths, &e.target, !settings.safe_metadata);
    let (meta, metadata_error) = match meta {
        Ok(m) => {
            let err = m.error.clone();
            (Some(m), err)
        }
        Err(err) => (None, Some(err)),
    };
    let exec = e
        .desktop
        .main("Exec")
        .and_then(|x| Exec::parse(&x).ok())
        .unwrap_or_default();
    let menu_name = e.desktop.main("Name");
    let menu_version = e.desktop.main("X-AppImage-Version");
    let original_name = e.desktop.main("X-AppImage-Name");
    let mut outdated = Vec::new();
    if let Some(m) = &meta {
        if m.version.is_some() && menu_version.is_some() && m.version != menu_version {
            outdated.push("version".into());
        }
        let name_differs = match (&m.name, &original_name) {
            (Some(n), Some(o)) => n != o,
            // Written by this app before the file's name could be read (for
            // example a DwarFS image an older reader could not open).
            (Some(n), None) => {
                e.desktop.main(OWN_KEY).is_some() && menu_name.as_ref().is_some_and(|mn| mn != n)
            }
            _ => false,
        };
        if name_differs {
            outdated.push("name".into());
        }
    }
    let changed = config
        .synced
        .is_some_and(|f| Fingerprint::of(&e.target).is_ok_and(|now| now != f));
    if changed && !outdated.iter().any(|o| o == "file") {
        outdated.push("file".into());
    }
    let icon_value = e.desktop.main("Icon");
    let menu_icon = icon_value
        .as_ref()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file());
    let icon_name = icon_value.clone().filter(|v| !v.starts_with('/'));
    let icon_custom = config.custom_icon.is_some()
        || icon_value.as_deref().is_some_and(|v| {
            !crate::ops::is_official_icon(&e.target, v)
                && !v.contains("/metadata-cache/")
                && v != "applications-other"
        });
    let managed_by = if e.desktop.main(OWN_KEY).is_some() {
        ManagedBy::GearLeverCosmic
    } else if original_name.is_some() && e.try_exec {
        ManagedBy::GearLever
    } else {
        ManagedBy::Other
    };
    let (update_source, origin, source_error) = match (&config.update_source, gearlever) {
        (Some(s), _) => (Some(s.clone()), Some(Origin::Custom), None),
        (None, Some(g)) if g.source.is_some() => (g.source.clone(), Some(Origin::GearLever), None),
        (None, Some(g)) => (None, None, g.source_error.clone()),
        _ => (None, None, None),
    };
    let id = e
        .file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    App {
        name: meta
            .as_ref()
            .and_then(|m| m.name.clone())
            .filter(|_| managed_by != ManagedBy::Other || menu_name.is_none())
            .or(menu_name.clone())
            .unwrap_or_else(|| id.clone()),
        comment: e
            .desktop
            .main("Comment")
            .or(meta.as_ref().and_then(|m| m.comment.clone())),
        version: meta.as_ref().and_then(|m| m.version.clone()),
        size: fs::metadata(&e.target).map(|m| m.len()).unwrap_or(0),
        external: e.target.parent() != Some(folder),
        icon: menu_icon.or(meta.as_ref().and_then(|m| m.icon.clone())),
        icon_name,
        icon_custom,
        terminal: e.desktop.bool("Terminal"),
        arch: meta
            .as_ref()
            .map(|m| m.arch.clone())
            .unwrap_or_else(|| "unknown".into()),
        appimage_type: meta.as_ref().map(|m| m.appimage_type).unwrap_or(0),
        arguments: exec.args.clone(),
        default_arguments: config
            .default_arguments
            .clone()
            .or_else(|| {
                gearlever
                    .and_then(|g| g.default_arguments.as_deref())
                    .and_then(|a| crate::desktop::split_command(a).ok())
            })
            .or(meta.as_ref().map(|m| m.default_arguments.clone()))
            .unwrap_or_default(),
        env: exec.env,
        menu_name,
        menu_version,
        menu_outdated: outdated,
        changed_since_sync: changed,
        update_info: meta.as_ref().and_then(|m| m.update_info.clone()),
        self_updater: meta.as_ref().and_then(|m| m.self_updater.clone()),
        update_source,
        update_source_origin: origin,
        update_source_error: source_error,
        flatpak: None,
        standard_path: None,
        website: config
            .website
            .clone()
            .or(gearlever.and_then(|g| g.website.clone())),
        metadata_error,
        metadata: meta,
        managed_by,
        id,
        other_desktop_files: others.into_iter().map(|o| o.file).collect(),
        desktop_file: e.file,
        path: e.target,
    }
}

/// Which apps are running, by process executable. A path ending in
/// " (deleted)" means an instance still runs a version that was replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Running {
    pub pids: Vec<u32>,
    /// At least one instance runs a file that has since been replaced.
    pub old_version: bool,
}

pub fn running(apps: &[App]) -> Result<BTreeMap<String, Running>, String> {
    let procs = crate::host::process_executables()?;
    let mut out = BTreeMap::new();
    for app in apps {
        let mut r = Running::default();
        let deleted = format!("{} (deleted)", app.path.display());
        for (pid, exe) in &procs {
            if exe == &app.path {
                r.pids.push(*pid);
            } else if exe.to_string_lossy() == deleted {
                r.pids.push(*pid);
                r.old_version = true;
            }
        }
        if !r.pids.is_empty() {
            out.insert(app.id.clone(), r);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appimage::tests::fixture;
    #[test]
    fn lists_gear_lever_entries_and_reads_the_version_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        let folder = dir.path().join("AppImages");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(paths.applications()).unwrap();
        // The file was replaced in place by a newer build (issue #497).
        let app = fixture(
            &folder,
            "mendeley.appimage",
            "",
            &[(
                "/m.desktop",
                b"[Desktop Entry]\nName=Mendeley\nExec=m %U\nX-AppImage-Version=2.148.0\n",
            )],
            &[],
        );
        fs::write(
            paths.applications().join("mendeley.desktop"),
            format!("[Desktop Entry]\nName=Mendeley\nTryExec={0}\nExec=env DESKTOPINTEGRATION=1 {0} %U\nX-AppImage-Version=2.138.0\nX-AppImage-Name=Mendeley\n", app.display()),
        )
        .unwrap();
        // A second entry the app wrote for itself, and an unrelated one.
        fs::write(
            paths.applications().join("com.mendeley.desktop"),
            format!(
                "[Desktop Entry]\nName=Mendeley\nExec=\"{}\" %U\n",
                app.display()
            ),
        )
        .unwrap();
        fs::write(
            paths.applications().join("other.desktop"),
            "[Desktop Entry]\nName=Other\nExec=/usr/bin/true\n",
        )
        .unwrap();
        let inv = load(&paths, &Settings::default());
        assert_eq!(inv.apps.len(), 1);
        let a = &inv.apps[0];
        assert_eq!(a.id, "mendeley");
        assert_eq!(a.version.as_deref(), Some("2.148.0"));
        assert_eq!(a.menu_version.as_deref(), Some("2.138.0"));
        assert_eq!(a.menu_outdated, ["version"]);
        assert_eq!(a.managed_by, ManagedBy::GearLever);
        assert_eq!(
            a.other_desktop_files,
            [paths.applications().join("com.mendeley.desktop")]
        );
        assert_eq!(a.env, [("DESKTOPINTEGRATION".to_string(), "1".to_string())]);
        assert!(!a.external);
        assert_eq!(inv.find("Mendeley").unwrap().id, "mendeley");
        assert!(inv.find("nope").is_err());
    }
}
