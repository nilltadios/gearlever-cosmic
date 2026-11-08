// SPDX-License-Identifier: GPL-3.0-or-later
//! Changes to files: integrate, update, remove and edit AppImages and their
//! menu entries. Every operation can describe itself as a plan before it runs,
//! takes the shared operation lock, and writes a durable report.
//!
//! Updates never wait for the app to close: the new file is written beside
//! the old one and renamed over it. A running instance keeps the old file's
//! data until it exits; the next start uses the new version.
use crate::{
    appimage::{self, Fingerprint, Metadata},
    desktop::{DesktopFile, Exec, MAIN},
    library::{App, Inventory, OWN_KEY},
    paths::Paths,
    settings::Settings,
    store::Store,
    updates::{self, Download, Source},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    /// Refuse when another version of the app is integrated (the default for agents).
    #[default]
    Ask,
    /// Integrate next to the existing version, with the version in the menu name.
    KeepBoth,
    /// Replace the existing version's file in place, keeping its settings.
    Replace,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Integrate {
        file: PathBuf,
        conflict: Conflict,
        /// The app to replace when conflict is Replace.
        replace: Option<App>,
        source: Option<Source>,
    },
    Update {
        app: App,
        download: Download,
        version: Option<String>,
    },
    Remove(Vec<App>),
    SetArguments(App, Vec<String>),
    SetEnv(App, Vec<(String, String)>),
    SetWebsite(App, Option<String>),
    SetSource(App, Option<Source>),
    /// Use the user's own image as the app's icon; it survives updates.
    SetIcon(App, PathBuf),
    /// Rename the AppImage, its menu entry and icons to the name a fresh
    /// install would get (see `standard_path`).
    RenameFile(App, PathBuf),
    /// Go back to the icon shipped inside the AppImage.
    ResetIcon(App),
    SyncMenu(Vec<App>),
    ImportGearLever(Vec<(App, Option<Source>, Option<String>)>),
    Settings(Box<Settings>),
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub fraction: Option<f32>,
    pub step: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<Mutex<Progress>>,
}
impl Job {
    pub fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("Cancelled. Completed steps remain applied.".into())
        } else {
            Ok(())
        }
    }
    pub fn step(&self, text: impl Into<String>) {
        let text = text.into();
        if let Ok(mut p) = self.progress.lock() {
            p.step = text.clone();
            p.fraction = None;
            p.lines.push(text);
            if p.lines.len() > 300 {
                p.lines.remove(0);
            }
        }
    }
    pub fn fraction(&self, f: f32) {
        if let Ok(mut p) = self.progress.lock() {
            p.fraction = Some(f);
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Report {
    pub title: String,
    pub started: u64,
    pub finished: u64,
    pub steps: Vec<String>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
    pub cancelled: bool,
    /// Files created or replaced (for agents to verify).
    pub changed: Vec<PathBuf>,
    pub trashed: Vec<PathBuf>,
    pub log: PathBuf,
}

/// Turn a display name into Gear Lever's file name style.
pub fn file_stem(name: &str) -> String {
    let lower = name.to_lowercase().replace(' ', "_");
    let clean: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == '-')
        .collect();
    let clean = clean.trim_matches('.').to_string();
    if clean.is_empty() {
        "app".into()
    } else {
        clean
    }
}

impl Operation {
    pub fn title(&self) -> String {
        match self {
            Self::Integrate {
                file,
                conflict: Conflict::Replace,
                ..
            } => format!("Replace with {}", name_of(file)),
            Self::Integrate { file, .. } => format!("Add {} to the app menu", name_of(file)),
            Self::Update { app, .. } => format!("Update {}", app.name),
            Self::Remove(apps) if apps.len() == 1 => format!("Remove {}", apps[0].name),
            Self::Remove(apps) => format!("Remove {} apps", apps.len()),
            Self::SetArguments(a, _) => format!("Change {} arguments", a.name),
            Self::SetEnv(a, _) => format!("Change {} environment", a.name),
            Self::SetWebsite(a, _) => format!("Change {} website", a.name),
            Self::SetSource(a, _) => format!("Change {} update source", a.name),
            Self::SetIcon(a, _) => format!("Change {} icon", a.name),
            Self::RenameFile(a, _) => format!("Give {} its standard file name", a.name),
            Self::ResetIcon(a) => format!("Use {} official icon", a.name),
            Self::SyncMenu(apps) if apps.len() == 1 => {
                format!("Refresh {} menu entry", apps[0].name)
            }
            Self::SyncMenu(apps) => format!("Refresh {} menu entries", apps.len()),
            Self::ImportGearLever(list) => format!("Import {} Gear Lever settings", list.len()),
            Self::Settings(_) => "Change settings".into(),
        }
    }
    /// Human-readable effects, with exact paths, for dry-run plans and dialogs.
    pub fn plan(&self, paths: &Paths, settings: &Settings) -> Result<Vec<String>, String> {
        let short = |p: &Path| paths.short(p);
        Ok(match self {
            Self::Integrate { file, conflict, replace, source } => {
                let meta = appimage::inspect(file, &appimage::Options { cache_dir: None, allow_extract: !settings.safe_metadata })?;
                let mut out = Vec::new();
                if let (Conflict::Replace, Some(app)) = (conflict, replace) {
                    out.push(format!("Write {} over {} without waiting for it to close; running instances keep the old version until restarted", short(file), short(&app.path)));
                    if settings.keep_replaced_versions_in_trash {
                        out.push("Keep the replaced file in the Trash".to_string());
                    }
                    out.push(format!("Rewrite the menu entry {} with the new name, version and icon, keeping custom arguments and environment", short(&app.desktop_file)));
                } else {
                    let target = target_path(paths, settings, &meta, *conflict)?;
                    let move_it = settings.move_on_integrate && file.parent() != target.parent();
                    out.push(format!("{} {} to {}", if move_it { "Move" } else { "Copy" }, short(file), short(&target)));
                    out.push(format!("Write the menu entry {}", short(&desktop_path(paths, &target))));
                    if meta.icon.is_some() || meta.desktop_entry.is_some() {
                        out.push(format!("Save the app icon in {}", short(&target.parent().unwrap().join(".icons"))));
                    }
                    if *conflict == Conflict::KeepBoth {
                        out.push("Show the version in the menu name, since another version stays installed".into());
                    }
                }
                if let Some(s) = source {
                    out.push(format!("Set the update source to {} ({})", s.manager().label, s.summary()));
                }
                out
            }
            Self::Update { app, download, version } => {
                let mut out = vec![format!(
                    "Download {}{} from {}",
                    download.name.as_deref().unwrap_or("the new file"),
                    version.as_ref().map(|v| format!(" ({v})")).unwrap_or_default(),
                    download.url
                )];
                let checks: Vec<&str> = [("size", download.size.is_some()), ("SHA-1", download.sha1.is_some()), ("SHA-256", download.sha256.is_some()), ("SHA-512", download.sha512.is_some())]
                    .iter()
                    .filter(|(_, k)| *k)
                    .map(|(n, _)| *n)
                    .collect();
                out.push(if checks.is_empty() {
                    "Check that the download is an AppImage (the source publishes no checksum)".into()
                } else {
                    format!("Verify the download ({}) and that it is an AppImage", checks.join(", "))
                });
                out.push(format!("Replace {} in place; a running instance keeps the old version until it is restarted", short(&app.path)));
                if settings.keep_replaced_versions_in_trash {
                    out.push("Keep the replaced file in the Trash".into());
                }
                out.push(format!("Refresh the menu entry {}", short(&app.desktop_file)));
                out
            }
            Self::Remove(apps) => {
                let mut out = Vec::new();
                for a in apps {
                    out.push(format!("Move {} to the Trash", short(&a.path)));
                    out.push(format!("Move the menu entry {} to the Trash", short(&a.desktop_file)));
                    if let Some(i) = own_icon(a) {
                        out.push(format!("Move the icon {} to the Trash", short(&i)));
                    }
                    for o in &a.other_desktop_files {
                        out.push(format!("Leave {} in place: the app or another tool made it", short(o)));
                    }
                }
                out
            }
            Self::SetArguments(a, args) => vec![format!("Set the arguments in {} to: {}", short(&a.desktop_file), crate::desktop::join_command(args))],
            Self::SetEnv(a, env) => vec![format!(
                "Set the environment in {} to: {}",
                short(&a.desktop_file),
                if env.is_empty() { "nothing".into() } else { env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ") }
            )],
            Self::SetWebsite(a, w) => vec![match w {
                Some(w) => format!("Save {w} as {}'s website", a.name),
                None => format!("Forget {}'s website", a.name),
            }],
            Self::SetSource(a, s) => vec![match s {
                Some(s) => format!("Check {} for updates with {} ({})", a.name, s.manager().label, s.summary()),
                None => format!("Remove {}'s custom update source; embedded or Gear Lever information is used again", a.name),
            }],
            Self::RenameFile(a, target) => vec![
                format!("Rename {} to {}; a running instance keeps working", short(&a.path), short(target)),
                format!("Rename the menu entry to {} and point it at the new file", short(&desktop_path(paths, target))),
                format!("Rename its icons in {} to match", short(&a.path.parent().unwrap_or(Path::new("/")).join(".icons"))),
            ],
            Self::SetIcon(a, file) => vec![
                format!("Copy {} to {}", short(file), short(&custom_icon_path(&a.path, file))),
                format!("Use it as the icon in {}; updates and menu refreshes keep it", short(&a.desktop_file)),
            ],
            Self::ResetIcon(a) => {
                let mut v = vec![format!("Extract the icon shipped in {} and use it in {}", short(&a.path), short(&a.desktop_file))];
                if let Some(c) = own_custom_icon(a) {
                    v.push(format!("Move the custom icon {} to the Trash", short(&c)));
                }
                v
            }
            Self::SyncMenu(apps) => apps
                .iter()
                .map(|a| format!("Rewrite {} from the AppImage's own metadata (name, version {}, icon), keeping arguments and environment", short(&a.desktop_file), a.display_version()))
                .collect(),
            Self::ImportGearLever(list) => list
                .iter()
                .map(|(a, s, w)| {
                    let mut parts = vec![];
                    if let Some(s) = s {
                        parts.push(format!("update source {} ({})", s.manager().label, s.summary()));
                    }
                    if let Some(w) = w {
                        parts.push(format!("website {w}"));
                    }
                    format!("Save Gear Lever's {} for {}", parts.join(" and "), a.name)
                })
                .collect(),
            Self::Settings(s) => {
                let old = serde_json::to_value(settings).unwrap_or_default();
                let new = serde_json::to_value(s.as_ref()).unwrap_or_default();
                let mut out = Vec::new();
                if let (Some(o), Some(n)) = (old.as_object(), new.as_object()) {
                    for (k, v) in n {
                        if o.get(k) != Some(v) {
                            out.push(format!("Set {k} to {v}"));
                        }
                    }
                }
                if s.check_updates_on_startup != settings.check_updates_on_startup {
                    out.push(format!(
                        "{} the autostart entry ~/.config/autostart/{}.desktop",
                        if s.check_updates_on_startup { "Create" } else { "Move to Trash" },
                        crate::APP_ID
                    ));
                }
                if out.is_empty() {
                    out.push("Nothing changes".into());
                }
                out
            }
        })
    }
}

/// The name a fresh install of this app would get (`name.appimage`), when it
/// differs from the current one and is free. None when the name is already
/// standard, the app is outside the folder, its name is unknown, or the
/// standard name belongs to another app (a "keep both" copy keeps its
/// version on purpose then).
pub fn standard_path(paths: &Paths, settings: &Settings, app: &App) -> Option<PathBuf> {
    let meta = app.metadata.as_ref()?;
    let name = meta.name.as_ref()?;
    if app.external {
        return None;
    }
    let plain = meta.terminal && settings.terminal_apps_use_executable_name;
    let target = settings.folder(paths).join(format!(
        "{}{}",
        file_stem(name),
        if plain { "" } else { ".appimage" }
    ));
    if target == app.path || target.exists() || desktop_path(paths, &target).exists() {
        return None;
    }
    Some(target)
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn desktop_path(paths: &Paths, appimage: &Path) -> PathBuf {
    let stem = appimage
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(' ', "_");
    paths.applications().join(format!("{stem}.desktop"))
}

fn own_icon(app: &App) -> Option<PathBuf> {
    let icon = app.icon.as_ref()?;
    is_official_icon(&app.path, &icon.to_string_lossy()).then(|| icon.clone())
}

/// The icon this app (or Gear Lever) extracted from the AppImage: in the
/// folder's .icons, named after the AppImage, with or without an extension.
pub fn is_official_icon(appimage: &Path, icon: &str) -> bool {
    let p = Path::new(icon);
    let dir = appimage.parent().map(|d| d.join(".icons"));
    p.parent() == dir.as_deref() && p.file_stem() == appimage.file_stem()
}

pub fn custom_icon_path(appimage: &Path, source: &Path) -> PathBuf {
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());
    appimage
        .parent()
        .unwrap_or(Path::new("/"))
        .join(".icons")
        .join(format!(
            "{}-custom.{ext}",
            appimage.file_stem().unwrap_or_default().to_string_lossy()
        ))
}

/// A custom icon file this app copied into .icons.
fn own_custom_icon(app: &App) -> Option<PathBuf> {
    let icon = app.icon.as_ref()?;
    let stem = format!("{}-custom", app.path.file_stem()?.to_string_lossy());
    (icon.parent() == app.path.parent().map(|d| d.join(".icons")).as_deref()
        && icon
            .file_stem()
            .is_some_and(|s| s.to_string_lossy() == stem))
    .then(|| icon.clone())
}

/// The icon to keep instead of the official one: the user's chosen file, or
/// any Icon value someone set by hand (a file elsewhere or a theme icon name).
fn kept_icon(
    stored: Option<&Path>,
    existing: Option<&DesktopFile>,
    appimage: &Path,
) -> Option<String> {
    if let Some(p) = stored.filter(|p| p.is_file()) {
        return Some(p.to_string_lossy().into_owned());
    }
    let value = existing?.main("Icon")?;
    if is_official_icon(appimage, &value)
        || value.contains("/metadata-cache/")
        || value == "applications-other"
    {
        return None;
    }
    Some(value)
}

/// Where a new AppImage goes: Gear Lever's naming, never overwriting.
fn target_path(
    paths: &Paths,
    settings: &Settings,
    meta: &Metadata,
    conflict: Conflict,
) -> Result<PathBuf, String> {
    let folder = settings.folder(paths);
    let fallback = format!(
        "gearlever_{}",
        meta.path.file_stem().unwrap_or_default().to_string_lossy()
    );
    let base = file_stem(meta.name.as_deref().unwrap_or(&fallback));
    let plain = meta.terminal && settings.terminal_apps_use_executable_name;
    let ext = if plain { "" } else { ".appimage" };
    let free = |name: &str| {
        let p = folder.join(name);
        !p.exists() && !desktop_path(paths, &p).exists()
    };
    let first = format!("{base}{ext}");
    if free(&first) && conflict != Conflict::KeepBoth {
        return Ok(folder.join(first));
    }
    if let Some(v) = &meta.version {
        let name = format!("{base}_{}{ext}", file_stem(&v.replace('.', "_")));
        if free(&name) {
            return Ok(folder.join(name));
        }
    }
    for i in 1..1000 {
        let name = format!("{base}_{i}{ext}");
        if free(&name) {
            return Ok(folder.join(name));
        }
    }
    Err("Cannot find a free file name in the AppImages folder".into())
}

/// Write a file atomically: temp file in the same folder, fsync, rename.
fn write_atomic(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let parent = path.parent().ok_or("No parent folder")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    let tmp = parent.join(format!(".{}.tmp-{}", name_of(path), crate::now_nanos()));
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// Keep a copy of a menu entry before rewriting it (in this app's data
/// folder, so the applications folder stays clean).
fn backup_desktop(paths: &Paths, file: &Path, report: &mut Report) {
    if let Ok(text) = fs::read(file) {
        let dir = paths.data.join("menu-entry-backups");
        let dest = dir.join(format!("{}.{}", name_of(file), crate::now_nanos()));
        if fs::create_dir_all(&dir).is_ok() && fs::write(&dest, text).is_ok() {
            report.notes.push(format!(
                "Previous menu entry saved as {}",
                paths.short(&dest)
            ));
        }
    }
}

struct EntryInput<'a> {
    meta: &'a Metadata,
    appimage: &'a Path,
    icon: Option<String>,
    arguments: Vec<String>,
    env: Vec<(String, String)>,
    name_suffix: Option<String>,
    /// Existing entry to update instead of starting from the embedded one.
    existing: Option<&'a DesktopFile>,
}

/// Keys a refresh copies from the AppImage's own entry when the menu entry
/// lacks them. StartupWMClass lets the panel match the running window.
const FILLED_KEYS: [&str; 8] = [
    "Comment",
    "Categories",
    "Keywords",
    "MimeType",
    "StartupWMClass",
    "StartupNotify",
    "Terminal",
    "GenericName",
];

fn compose_entry(input: EntryInput) -> DesktopFile {
    let mut d = match (input.existing, &input.meta.desktop_entry) {
        (Some(e), embedded) => {
            // Keep the existing entry, but fill keys it lacks from the
            // AppImage's own entry (an entry written before the metadata could
            // be read has none of them). Present keys are never overwritten.
            let mut d = e.clone();
            if let Some(text) = embedded {
                let own = DesktopFile::parse(text);
                for key in FILLED_KEYS {
                    if d.main(key).is_none()
                        && let Some(v) = own.main(key)
                    {
                        d.set(MAIN, key, &v);
                    }
                }
            }
            d
        }
        (None, Some(text)) => DesktopFile::parse(text),
        (None, None) => DesktopFile::parse(&format!(
            "[{MAIN}]\nType=Application\nName={}\n",
            input
                .appimage
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        )),
    };
    let path = input.appimage.to_string_lossy().into_owned();
    if let Some(name) = &input.meta.name {
        let shown = match &input.name_suffix {
            Some(s) => format!("{name} ({s})"),
            None => name.clone(),
        };
        // A Gear Lever "keep both" entry carries the version in its name.
        let keep_suffix = input.existing.and_then(|e| e.main("Name")).filter(|n| {
            input
                .existing
                .and_then(|e| e.main("X-AppImage-Name"))
                .is_some_and(|o| n.starts_with(&format!("{o} (")))
        });
        if keep_suffix.is_none() || input.name_suffix.is_some() {
            d.set(MAIN, "Name", &shown);
        } else if let Some(v) = &input.meta.version {
            d.set(MAIN, "Name", &format!("{name} ({v})"));
        }
        d.set(MAIN, "X-AppImage-Name", name);
    }
    d.set(MAIN, "Type", "Application");
    d.set(MAIN, "TryExec", &path);
    let exec = Exec {
        env: input.env,
        program: path.clone(),
        args: input.arguments,
    };
    d.set(MAIN, "Exec", &exec.render());
    if let Some(icon) = &input.icon {
        d.set(MAIN, "Icon", icon);
    }
    match &input.meta.version {
        Some(v) => d.set(MAIN, "X-AppImage-Version", v),
        None => d.remove(MAIN, "X-AppImage-Version"),
    }
    d.set(MAIN, OWN_KEY, "true");
    // Activation through D-Bus would start a different binary than this file.
    d.remove(MAIN, "DBusActivatable");
    for group in d.action_groups() {
        if let Some(exec) = d.get(&group, "Exec").and_then(|e| Exec::parse(&e).ok()) {
            let rewritten = Exec {
                env: exec.env,
                program: path.clone(),
                args: exec.args,
            };
            d.set(&group, "Exec", &rewritten.render());
        }
        if let Some(icon) = &input.icon {
            d.set(&group, "Icon", icon);
        }
    }
    d
}

fn save_icon(meta: &Metadata, appimage: &Path) -> Result<Option<PathBuf>, String> {
    let Some(src) = &meta.icon else {
        return Ok(None);
    };
    let ext = src
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let dir = appimage.parent().ok_or("No folder")?.join(".icons");
    let dest = dir.join(format!(
        "{}{ext}",
        appimage.file_stem().unwrap_or_default().to_string_lossy()
    ));
    let data = fs::read(src).map_err(|e| e.to_string())?;
    write_atomic(&dest, &data, 0o644)?;
    Ok(Some(dest))
}

/// Replace `target` with `new` (a verified file in the same folder) by
/// rename. Returns the trashed old file, if kept.
fn swap_in(new: &Path, target: &Path, keep_old: bool, report: &mut Report) -> Result<(), String> {
    fs::set_permissions(new, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let mut kept = None;
    if keep_old && target.exists() {
        // A second name for the old data survives the rename and goes to Trash.
        let link = target.with_file_name(format!("{}.replaced-{}", name_of(target), crate::now()));
        match fs::hard_link(target, &link) {
            Ok(()) => kept = Some(link),
            Err(e) => report.notes.push(format!(
                "The replaced version could not be kept ({e}); it is overwritten"
            )),
        }
    }
    fs::rename(new, target).map_err(|e| format!("Cannot replace {}: {e}", target.display()))?;
    report.changed.push(target.to_path_buf());
    if let Some(link) = kept {
        match crate::host::trash(&link) {
            Ok(()) => {
                report.trashed.push(link);
                report
                    .notes
                    .push("The replaced version is in the Trash".into());
            }
            Err(e) => report.notes.push(format!(
                "The replaced version stays as {}: {e}",
                link.display()
            )),
        }
    }
    Ok(())
}

/// Keep customized arguments across an update; adopt the new defaults when
/// the user never changed them (as Gear Lever does).
fn merged_arguments(app: &App, new_defaults: &[String]) -> Vec<String> {
    if app.arguments == app.default_arguments {
        new_defaults.to_vec()
    } else {
        app.arguments.clone()
    }
}

fn refresh_entry(
    paths: &Paths,
    app: &App,
    meta: &Metadata,
    arguments: Vec<String>,
    custom: Option<&Path>,
    report: &mut Report,
) -> Result<(), String> {
    let existing = fs::read_to_string(&app.desktop_file)
        .map(|t| DesktopFile::parse(&t))
        .ok();
    let icon = match kept_icon(custom, existing.as_ref(), &app.path) {
        Some(kept) => {
            report.notes.push(format!("Kept your custom icon ({kept}). Use the official icon from the app's page to change back."));
            Some(kept)
        }
        None => save_icon(meta, &app.path)?
            .or(app.icon.clone())
            .map(|p| p.to_string_lossy().into_owned()),
    };
    let entry = compose_entry(EntryInput {
        meta,
        appimage: &app.path,
        icon,
        arguments,
        env: app.env.clone(),
        name_suffix: None,
        existing: existing.as_ref(),
    });
    backup_desktop(paths, &app.desktop_file, report);
    write_atomic(&app.desktop_file, entry.text().as_bytes(), 0o644)?;
    report.changed.push(app.desktop_file.clone());
    Ok(())
}

fn is_inside_icons(appimage: &Path, p: &Path) -> bool {
    p.parent() == appimage.parent().map(|d| d.join(".icons")).as_deref()
}

fn set_icon(paths: &Paths, app: &App, icon: &str, report: &mut Report) -> Result<(), String> {
    let text = fs::read_to_string(&app.desktop_file)
        .map_err(|e| format!("Cannot read {}: {e}", app.desktop_file.display()))?;
    let mut d = DesktopFile::parse(&text);
    d.set(MAIN, "Icon", icon);
    for g in d.action_groups() {
        if d.get(&g, "Icon").is_some() {
            d.set(&g, "Icon", icon);
        }
    }
    backup_desktop(paths, &app.desktop_file, report);
    write_atomic(&app.desktop_file, d.text().as_bytes(), 0o644)?;
    report.changed.push(app.desktop_file.clone());
    Ok(())
}

fn edit_exec(
    paths: &Paths,
    app: &App,
    report: &mut Report,
    f: impl FnOnce(&mut Exec),
) -> Result<(), String> {
    let text = fs::read_to_string(&app.desktop_file)
        .map_err(|e| format!("Cannot read {}: {e}", app.desktop_file.display()))?;
    let mut d = DesktopFile::parse(&text);
    let mut exec = Exec::parse(&d.main("Exec").unwrap_or_default())?;
    f(&mut exec);
    d.set(MAIN, "Exec", &exec.render());
    backup_desktop(paths, &app.desktop_file, report);
    write_atomic(&app.desktop_file, d.text().as_bytes(), 0o644)?;
    report.changed.push(app.desktop_file.clone());
    Ok(())
}

/// Find an integrated app holding the same contents as `file`.
pub fn same_file(inv: &Inventory, file: &Path) -> Option<App> {
    let size = fs::metadata(file).ok()?.len();
    let candidates: Vec<&App> = inv.apps.iter().filter(|a| a.size == size).collect();
    if candidates.is_empty() {
        return None;
    }
    let hash = appimage::hashes(file).ok()?.sha256;
    candidates
        .into_iter()
        .find(|a| appimage::hashes(&a.path).is_ok_and(|h| h.sha256 == hash))
        .cloned()
}

/// Other integrated versions of the same app (same name in their metadata).
pub fn same_app(inv: &Inventory, meta: &Metadata) -> Vec<App> {
    let Some(name) = &meta.name else {
        return vec![];
    };
    inv.apps
        .iter()
        .filter(|a| {
            a.metadata.as_ref().and_then(|m| m.name.as_ref()) == Some(name) || &a.name == name
        })
        .cloned()
        .collect()
}

pub fn execute(
    op: Operation,
    paths: &Paths,
    settings: &Settings,
    job: &Job,
) -> Result<Report, String> {
    fs::create_dir_all(&paths.data).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.lock())
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|_| {
        "Another Gear Lever COSMIC operation is running; try again when it finishes".to_string()
    })?;
    let mut report = Report {
        title: op.title(),
        started: crate::now(),
        finished: 0,
        steps: vec![],
        errors: vec![],
        notes: vec![],
        cancelled: false,
        changed: vec![],
        trashed: vec![],
        log: PathBuf::new(),
    };
    let result = run(&op, paths, settings, job, &mut report);
    if let Err(e) = result {
        if job.cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
        }
        report.errors.push(e);
    }
    if report
        .changed
        .iter()
        .chain(&report.trashed)
        .any(|p| p.starts_with(paths.applications()))
    {
        let _ = crate::host::update_desktop_database(&paths.applications());
    }
    report.finished = crate::now();
    let dir = paths.reports();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    report.log = dir.join(format!(
        "{}-{}-{}.json",
        report.started,
        std::process::id(),
        crate::now_nanos() % 1_000_000
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&report.log)
        .map_err(|e| format!("Cannot save the operation report: {e}"))?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(report)
}

fn run(
    op: &Operation,
    paths: &Paths,
    settings: &Settings,
    job: &Job,
    report: &mut Report,
) -> Result<(), String> {
    let mut store = Store::load(paths)?;
    let allow_extract = !settings.safe_metadata;
    match op {
        Operation::Integrate {
            file,
            conflict,
            replace,
            source,
        } => {
            job.step(format!("Reading {}", name_of(file)));
            let meta = appimage::inspect_cached(paths, file, allow_extract)?;
            if let (Conflict::Replace, Some(app)) = (conflict, replace) {
                job.step(format!("Replacing {}", app.name));
                let staging = app.path.with_file_name(format!(
                    ".{}.incoming-{}",
                    name_of(&app.path),
                    crate::now_nanos()
                ));
                fs::copy(file, &staging)
                    .map_err(|e| format!("Cannot copy {}: {e}", file.display()))?;
                swap_in(
                    &staging,
                    &app.path,
                    settings.keep_replaced_versions_in_trash,
                    report,
                )?;
                let meta = appimage::inspect_cached(paths, &app.path, allow_extract)?;
                refresh_entry(
                    paths,
                    app,
                    &meta,
                    merged_arguments(app, &meta.default_arguments),
                    store.get(&app.path).custom_icon.as_deref(),
                    report,
                )?;
                let c = store.entry(&app.path);
                c.synced = Some(meta.fingerprint);
                c.synced_version = meta.version.clone();
                c.default_arguments = Some(meta.default_arguments.clone());
                c.updated_at = Some(crate::now());
                if let Some(s) = source {
                    c.update_source = Some(s.clone());
                }
                if settings.move_on_integrate && file.parent() != app.path.parent() {
                    trash_original(file, report);
                }
                report
                    .steps
                    .push(format!("Replaced {}", paths.short(&app.path)));
            } else {
                let target = target_path(paths, settings, &meta, *conflict)?;
                let folder = target.parent().unwrap();
                fs::create_dir_all(folder)
                    .map_err(|e| format!("Cannot create {}: {e}", folder.display()))?;
                job.step(format!("Copying to {}", paths.short(&target)));
                let staging = folder.join(format!(
                    ".{}.incoming-{}",
                    name_of(&target),
                    crate::now_nanos()
                ));
                let moved = settings.move_on_integrate
                    && file.parent() != Some(folder)
                    && fs::rename(file, &staging).is_ok();
                if !moved {
                    fs::copy(file, &staging)
                        .map_err(|e| format!("Cannot copy {}: {e}", file.display()))?;
                }
                fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
                fs::rename(&staging, &target).map_err(|e| e.to_string())?;
                report.changed.push(target.clone());
                let mut meta = meta;
                meta.path = target.clone();
                let icon = save_icon(&meta, &target)?;
                if let Some(i) = &icon {
                    report.changed.push(i.clone());
                }
                let icon = icon.map(|i| i.to_string_lossy().into_owned());
                let mut env = vec![];
                if !meta
                    .desktop_entry
                    .as_deref()
                    .unwrap_or_default()
                    .contains("DESKTOPINTEGRATION")
                {
                    env.push(("DESKTOPINTEGRATION".to_string(), "1".to_string()));
                }
                let entry = compose_entry(EntryInput {
                    meta: &meta,
                    appimage: &target,
                    icon,
                    arguments: meta.default_arguments.clone(),
                    env,
                    name_suffix: (*conflict == Conflict::KeepBoth)
                        .then(|| meta.version.clone().unwrap_or_else(|| "copy".into())),
                    existing: None,
                });
                let desktop = desktop_path(paths, &target);
                write_atomic(&desktop, entry.text().as_bytes(), 0o644)?;
                report.changed.push(desktop.clone());
                let fp = Fingerprint::of(&target)?;
                let c = store.entry(&target);
                *c = Default::default();
                c.synced = Some(fp);
                c.synced_version = meta.version.clone();
                c.default_arguments = Some(meta.default_arguments.clone());
                c.integrated_at = Some(crate::now());
                c.update_source = source.clone();
                if settings.move_on_integrate && !moved && file.parent() != Some(folder) {
                    trash_original(file, report);
                }
                report.steps.push(format!(
                    "Added {} to the app menu as {}",
                    meta.name.as_deref().unwrap_or("the app"),
                    paths.short(&target)
                ));
            }
        }
        Operation::Update {
            app,
            download,
            version,
        } => {
            let folder = app.path.parent().ok_or("No folder")?;
            let staging = folder.join(format!(
                ".{}.download-{}",
                name_of(&app.path),
                crate::now_nanos()
            ));
            job.step(format!(
                "Downloading {}",
                download.name.as_deref().unwrap_or("update")
            ));
            let verified =
                updates::download(download, &staging, &|f| job.fraction(f), &job.cancel)?;
            report
                .steps
                .push(format!("Downloaded and checked: {}", verified.join(", ")));
            job.check().inspect_err(|_| {
                let _ = fs::remove_file(&staging);
            })?;
            job.step(format!("Installing {}", app.name));
            let running = crate::library::running(std::slice::from_ref(app))
                .ok()
                .and_then(|r| r.get(&app.id).cloned());
            swap_in(
                &staging,
                &app.path,
                settings.keep_replaced_versions_in_trash,
                report,
            )?;
            let meta = appimage::inspect_cached(paths, &app.path, allow_extract)?;
            refresh_entry(
                paths,
                app,
                &meta,
                merged_arguments(app, &meta.default_arguments),
                store.get(&app.path).custom_icon.as_deref(),
                report,
            )?;
            let c = store.entry(&app.path);
            c.synced = Some(meta.fingerprint);
            c.synced_version = meta.version.clone();
            c.default_arguments = Some(meta.default_arguments.clone());
            c.updated_at = Some(crate::now());
            report.steps.push(format!(
                "Updated {} to {}",
                app.name,
                meta.version
                    .clone()
                    .or(version.clone())
                    .unwrap_or_else(|| "the new file".into())
            ));
            if running.is_some() {
                report.notes.push(format!("{} is still running the previous version. The update takes effect the next time it starts.", app.name));
            }
        }
        Operation::Remove(apps) => {
            for app in apps {
                job.check()?;
                job.step(format!("Removing {}", app.name));
                crate::host::trash(&app.path)?;
                report.trashed.push(app.path.clone());
                if app.desktop_file.exists() {
                    crate::host::trash(&app.desktop_file)?;
                    report.trashed.push(app.desktop_file.clone());
                }
                if let Some(icon) = own_icon(app)
                    && icon.exists()
                {
                    crate::host::trash(&icon)?;
                    report.trashed.push(icon);
                }
                if let Some(icon) = store.get(&app.path).custom_icon.filter(|p| p.is_file())
                    && icon.parent() == app.path.parent().map(|d| d.join(".icons")).as_deref()
                {
                    crate::host::trash(&icon)?;
                    report.trashed.push(icon);
                }
                store.apps.remove(&app.path);
                report
                    .steps
                    .push(format!("Moved {} to the Trash", app.name));
                if !app.other_desktop_files.is_empty() {
                    report.notes.push(format!(
                        "{} still has menu entries made outside this app: {}",
                        app.name,
                        app.other_desktop_files
                            .iter()
                            .map(|p| paths.short(p))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        Operation::SetArguments(app, args) => {
            edit_exec(paths, app, report, |e| e.args = args.clone())?;
            report.steps.push(format!(
                "Arguments set to: {}",
                crate::desktop::join_command(args)
            ));
        }
        Operation::SetEnv(app, env) => {
            for (k, _) in env {
                if !crate::desktop::valid_env_key(k) {
                    return Err(format!("{k} is not a valid variable name"));
                }
            }
            edit_exec(paths, app, report, |e| e.env = env.clone())?;
            report.steps.push("Environment saved".into());
        }
        Operation::SetWebsite(app, w) => {
            store.entry(&app.path).website = w.clone();
            report.steps.push("Website saved".into());
        }
        Operation::SetSource(app, s) => {
            if let Some(s) = s {
                s.validate()?;
            }
            store.entry(&app.path).update_source = s.clone();
            report.steps.push("Update source saved".into());
        }
        Operation::RenameFile(app, target) => {
            if target.exists() || desktop_path(paths, target).exists() {
                return Err(format!(
                    "{} is taken; nothing was renamed",
                    target.display()
                ));
            }
            let text = fs::read_to_string(&app.desktop_file)
                .map_err(|e| format!("Cannot read {}: {e}", app.desktop_file.display()))?;
            let mut text = text.replace(&*app.path.to_string_lossy(), &target.to_string_lossy());
            fs::rename(&app.path, target)
                .map_err(|e| format!("Cannot rename {}: {e}", app.path.display()))?;
            report.changed.push(target.clone());
            let old_stem = app
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let new_stem = target
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let icons = app.path.parent().unwrap_or(Path::new("/")).join(".icons");
            let mut custom = store.get(&app.path).custom_icon;
            for entry in fs::read_dir(&icons).into_iter().flatten().flatten() {
                let old = entry.path();
                let name = name_of(&old);
                let rest = match name.strip_prefix(&old_stem) {
                    Some(r) if r.is_empty() || r.starts_with('.') || r.starts_with("-custom") => {
                        r.to_string()
                    }
                    _ => continue,
                };
                let new = icons.join(format!("{new_stem}{rest}"));
                if new.exists() {
                    continue;
                }
                if fs::rename(&old, &new).is_ok() {
                    text = text.replace(&*old.to_string_lossy(), &new.to_string_lossy());
                    if custom.as_deref() == Some(old.as_path()) {
                        custom = Some(new.clone());
                    }
                    report.changed.push(new);
                }
            }
            let new_desktop = desktop_path(paths, target);
            backup_desktop(paths, &app.desktop_file, report);
            write_atomic(&new_desktop, text.as_bytes(), 0o644)?;
            report.changed.push(new_desktop.clone());
            if new_desktop != app.desktop_file {
                // Its content lives on in the new entry and in the backup.
                fs::remove_file(&app.desktop_file)
                    .map_err(|e| format!("Cannot remove the old menu entry: {e}"))?;
                report.trashed.push(app.desktop_file.clone());
            }
            store.rename(&app.path, target);
            store.entry(target).custom_icon = custom;
            report.steps.push(format!(
                "Renamed {} to {}",
                paths.short(&app.path),
                paths.short(target)
            ));
            if crate::library::running(std::slice::from_ref(app))
                .is_ok_and(|r| r.contains_key(&app.id))
            {
                report.notes.push(format!(
                    "{} keeps running from the renamed file; nothing needs a restart.",
                    app.name
                ));
            }
        }
        Operation::SetIcon(app, file) => {
            let data =
                fs::read(file).map_err(|e| format!("Cannot read {}: {e}", file.display()))?;
            let kind = appimage::image_kind(&data).ok_or("Choose a PNG or SVG image")?;
            let dest = custom_icon_path(&app.path, &file.with_extension(kind));
            write_atomic(&dest, &data, 0o644)?;
            report.changed.push(dest.clone());
            set_icon(paths, app, &dest.to_string_lossy(), report)?;
            store.entry(&app.path).custom_icon = Some(dest);
            report
                .steps
                .push(format!("{} now uses your icon", app.name));
        }
        Operation::ResetIcon(app) => {
            let meta = appimage::inspect_cached(paths, &app.path, allow_extract)?;
            let official =
                save_icon(&meta, &app.path)?.ok_or("The AppImage has no icon of its own")?;
            set_icon(paths, app, &official.to_string_lossy(), report)?;
            report.changed.push(official);
            if let Some(c) = store
                .get(&app.path)
                .custom_icon
                .filter(|p| p.is_file() && is_inside_icons(&app.path, p))
            {
                crate::host::trash(&c)?;
                report.trashed.push(c);
            }
            store.entry(&app.path).custom_icon = None;
            report
                .steps
                .push(format!("{} uses its official icon again", app.name));
        }
        Operation::SyncMenu(apps) => {
            for app in apps {
                job.check()?;
                job.step(format!("Refreshing {}", app.name));
                let meta = appimage::inspect_cached(paths, &app.path, allow_extract)?;
                // No arguments and none customized (the defaults were unknown
                // too): adopt the AppImage's own, e.g. %F for opened files.
                let arguments = if app.arguments.is_empty() && app.default_arguments.is_empty() {
                    meta.default_arguments.clone()
                } else {
                    app.arguments.clone()
                };
                refresh_entry(
                    paths,
                    app,
                    &meta,
                    arguments,
                    store.get(&app.path).custom_icon.as_deref(),
                    report,
                )?;
                let c = store.entry(&app.path);
                c.synced = Some(meta.fingerprint);
                c.synced_version = meta.version.clone();
                if c.default_arguments.as_ref().is_none_or(|d| d.is_empty()) {
                    c.default_arguments = Some(meta.default_arguments.clone());
                }
                report.steps.push(format!(
                    "{} menu entry now shows {}",
                    app.name,
                    meta.version.as_deref().unwrap_or("no version")
                ));
            }
        }
        Operation::ImportGearLever(list) => {
            for (app, s, w) in list {
                let c = store.entry(&app.path);
                if s.is_some() {
                    c.update_source = s.clone();
                }
                if w.is_some() {
                    c.website = w.clone();
                }
                report
                    .steps
                    .push(format!("Imported Gear Lever settings for {}", app.name));
            }
        }
        Operation::Settings(new) => {
            if new.check_updates_on_startup != settings.check_updates_on_startup {
                autostart(paths, new.check_updates_on_startup, report)?;
            }
            new.save(paths)?;
            report.changed.push(Settings::path(paths));
            report.steps.push("Settings saved".into());
        }
    }
    store.save(paths)?;
    Ok(())
}

fn trash_original(file: &Path, report: &mut Report) {
    match crate::host::trash(file) {
        Ok(()) => {
            report.trashed.push(file.to_path_buf());
            report
                .notes
                .push(format!("The original {} is in the Trash", file.display()));
        }
        Err(e) => report
            .notes
            .push(format!("The original file was kept: {e}")),
    }
}

/// The startup update check runs from an XDG autostart entry. Inside the
/// Flatpak it starts the packaged CLI; natively the installed CLI.
fn autostart(paths: &Paths, enable: bool, report: &mut Report) -> Result<(), String> {
    let file = paths
        .home
        .join(".config/autostart")
        .join(format!("{}.desktop", crate::APP_ID));
    if !enable {
        if file.exists() {
            crate::host::trash(&file)?;
            report.trashed.push(file);
        }
        return Ok(());
    }
    let exec = if let Some(appimage) = crate::appimage_self() {
        format!(
            "{} cli fetch-updates",
            crate::desktop::quote_arg(&appimage.to_string_lossy())
        )
    } else {
        let cli = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("gearlever-cosmic-cli");
        format!(
            "{} fetch-updates",
            crate::desktop::quote_arg(&cli.to_string_lossy())
        )
    };
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Gear Lever COSMIC update check\nComment=Checks AppImages for updates and sends a notification; installs nothing\nExec={exec}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    );
    write_atomic(&file, text.as_bytes(), 0o644)?;
    report.changed.push(file);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appimage::tests::{PNG, fixture};

    fn setup() -> (tempfile::TempDir, Paths, Settings) {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        fs::create_dir_all(paths.applications()).unwrap();
        (dir, paths, Settings::default())
    }
    fn demo(dir: &Path, name: &str, version: &str) -> PathBuf {
        let desktop = format!(
            "[Desktop Entry]\nName=Demo App\nExec=demo --flag %U\nIcon=demo\nX-AppImage-Version={version}\nDBusActivatable=true\nStartupWMClass=dev.example.Demo\n\n[Desktop Action win]\nName=New window\nExec=demo --new\n"
        );
        fixture(
            dir,
            name,
            "",
            &[("/demo.desktop", desktop.as_bytes()), ("/demo.png", PNG)],
            &[],
        )
    }

    #[test]
    fn integrate_moves_the_file_and_writes_a_complete_menu_entry() {
        let (dir, paths, settings) = setup();
        let downloads = dir.path().join("Downloads");
        fs::create_dir_all(&downloads).unwrap();
        let file = demo(&downloads, "Demo-1.0-x86_64.AppImage", "1.0");
        let op = Operation::Integrate {
            file: file.clone(),
            conflict: Conflict::Ask,
            replace: None,
            source: None,
        };
        let plan = op.plan(&paths, &settings).unwrap();
        assert!(
            plan[0].starts_with(
                "Move ~/Downloads/Demo-1.0-x86_64.AppImage to ~/AppImages/demo_app.appimage"
            ),
            "{plan:?}"
        );
        // Without trash support in the test sandbox, rename is used (no trash needed).
        let report = execute(op, &paths, &settings, &Job::default()).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(!file.exists());
        let target = dir.path().join("AppImages/demo_app.appimage");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let entry = DesktopFile::parse(
            &fs::read_to_string(paths.applications().join("demo_app.desktop")).unwrap(),
        );
        assert_eq!(entry.main("TryExec").unwrap(), target.to_string_lossy());
        assert_eq!(
            entry.main("Exec").unwrap(),
            format!("env DESKTOPINTEGRATION=1 {} --flag %U", target.display())
        );
        assert_eq!(entry.main("X-AppImage-Version").as_deref(), Some("1.0"));
        assert_eq!(
            entry.main("Icon").unwrap(),
            dir.path()
                .join("AppImages/.icons/demo_app.png")
                .to_string_lossy()
        );
        assert!(entry.main("DBusActivatable").is_none());
        assert_eq!(
            entry.get("Desktop Action win", "Exec").unwrap(),
            format!("{} --new", target.display())
        );
        let inv = crate::library::load(&paths, &settings);
        assert_eq!(inv.apps.len(), 1);
        assert!(inv.apps[0].menu_outdated.is_empty());
        assert_eq!(
            inv.apps[0].managed_by,
            crate::library::ManagedBy::GearLeverCosmic
        );
        assert!(report.log.is_file());
    }

    #[test]
    fn replacing_a_running_app_keeps_its_open_file_and_custom_arguments() {
        let (dir, paths, mut settings) = setup();
        settings.keep_replaced_versions_in_trash = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let v1 = demo(&src, "v1.AppImage", "1.0");
        settings.move_on_integrate = false;
        execute(
            Operation::Integrate {
                file: v1,
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let inv = crate::library::load(&paths, &settings);
        let app = inv.apps[0].clone();
        execute(
            Operation::SetArguments(app.clone(), vec!["--custom".into()]),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        // Hold the old file open as a running AppImage's runtime does.
        let mut open_old = fs::File::open(&app.path).unwrap();
        let v2 = demo(&src, "v2.AppImage", "2.0");
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        let report = execute(
            Operation::Integrate {
                file: v2,
                conflict: Conflict::Replace,
                replace: Some(app.clone()),
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut old = Vec::new();
        use std::io::Read;
        open_old.read_to_end(&mut old).unwrap();
        assert!(
            String::from_utf8_lossy(&old).contains("X-AppImage-Version=1.0") || !old.is_empty()
        );
        let now = crate::library::load(&paths, &settings).apps[0].clone();
        assert_eq!(now.version.as_deref(), Some("2.0"));
        assert_eq!(now.menu_version.as_deref(), Some("2.0"));
        assert_eq!(now.arguments, ["--custom"]);
        assert_eq!(now.path, app.path);
    }

    #[test]
    fn keep_both_adds_a_second_entry_with_the_version_in_its_name() {
        let (dir, paths, mut settings) = setup();
        settings.move_on_integrate = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "a.AppImage", "1.0"),
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "b.AppImage", "2.0"),
                conflict: Conflict::KeepBoth,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let inv = crate::library::load(&paths, &settings);
        let names: Vec<_> = inv
            .apps
            .iter()
            .map(|a| a.menu_name.clone().unwrap())
            .collect();
        assert_eq!(names, ["Demo App", "Demo App (2.0)"]);
        assert!(dir.path().join("AppImages/demo_app_2_0.appimage").is_file());
    }

    #[test]
    fn sync_fixes_a_menu_entry_left_behind_by_a_self_update() {
        let (dir, paths, mut settings) = setup();
        settings.move_on_integrate = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "a.AppImage", "1.0"),
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        // The app replaces its own file, as electron-updater does.
        let newer = demo(&src, "n.AppImage", "1.5");
        fs::rename(&newer, &app.path).unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        assert_eq!(app.version.as_deref(), Some("1.5"));
        assert!(app.menu_outdated.contains(&"version".to_string()));
        assert!(app.changed_since_sync);
        execute(
            Operation::SyncMenu(vec![app]),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        assert!(app.menu_outdated.is_empty(), "{:?}", app.menu_outdated);
        assert_eq!(app.menu_version.as_deref(), Some("1.5"));
    }

    #[test]
    fn a_custom_icon_survives_updates_and_the_official_one_can_be_restored() {
        let (dir, paths, mut settings) = setup();
        settings.move_on_integrate = false;
        settings.keep_replaced_versions_in_trash = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "a.AppImage", "1.0"),
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        let mine = src.join("mine.svg");
        fs::write(&mine, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        execute(
            Operation::SetIcon(app.clone(), mine),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let custom = dir.path().join("AppImages/.icons/demo_app-custom.svg");
        let menu_icon = |p: &Paths| {
            DesktopFile::parse(
                &fs::read_to_string(p.applications().join("demo_app.desktop")).unwrap(),
            )
            .main("Icon")
            .unwrap()
        };
        assert_eq!(menu_icon(&paths), custom.to_string_lossy());
        // An update and a menu refresh keep it.
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        assert!(app.icon_custom);
        let report = execute(
            Operation::Integrate {
                file: demo(&src, "b.AppImage", "2.0"),
                conflict: Conflict::Replace,
                replace: Some(app.clone()),
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        assert!(
            report.notes.iter().any(|n| n.contains("custom icon")),
            "{:?}",
            report.notes
        );
        assert_eq!(menu_icon(&paths), custom.to_string_lossy());
        // A theme icon name set by hand is kept as well.
        let entry = paths.applications().join("demo_app.desktop");
        fs::write(
            &entry,
            fs::read_to_string(&entry)
                .unwrap()
                .replace(&*custom.to_string_lossy(), "my-theme-icon"),
        )
        .unwrap();
        let mut store = Store::load(&paths).unwrap();
        store.entry(&app.path).custom_icon = None;
        store.save(&paths).unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        execute(
            Operation::SyncMenu(vec![app.clone()]),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        assert_eq!(menu_icon(&paths), "my-theme-icon");
        // Back to the official icon.
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        execute(
            Operation::ResetIcon(app),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        assert_eq!(
            menu_icon(&paths),
            dir.path()
                .join("AppImages/.icons/demo_app.png")
                .to_string_lossy()
        );
        assert!(!crate::library::load(&paths, &settings).apps[0].icon_custom);
    }

    #[test]
    fn refreshing_a_bare_entry_restores_window_matching_and_arguments() {
        let (dir, paths, mut settings) = setup();
        settings.move_on_integrate = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "a.AppImage", "1.0"),
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        // Simulate an entry written when nothing could be read.
        let entry = paths.applications().join("demo_app.desktop");
        let target = dir.path().join("AppImages/demo_app.appimage");
        fs::write(&entry, format!("[Desktop Entry]\nType=Application\nName=demo_app\nTryExec={0}\nExec=env DESKTOPINTEGRATION=1 {0}\nX-GearLeverCosmic=true\n", target.display())).unwrap();
        let mut store = Store::load(&paths).unwrap();
        store.entry(&target).default_arguments = Some(vec![]);
        store.save(&paths).unwrap();
        let app = crate::library::load(&paths, &settings).apps[0].clone();
        assert!(
            app.menu_outdated.contains(&"name".to_string()),
            "{:?}",
            app.menu_outdated
        );
        execute(
            Operation::SyncMenu(vec![app]),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let d = DesktopFile::parse(&fs::read_to_string(&entry).unwrap());
        assert_eq!(d.main("Name").as_deref(), Some("Demo App"));
        assert_eq!(
            d.main("StartupWMClass").as_deref(),
            Some("dev.example.Demo")
        );
        assert_eq!(
            d.main("Exec").unwrap(),
            format!("env DESKTOPINTEGRATION=1 {} --flag %U", target.display())
        );
        let after = crate::library::load(&paths, &settings).apps[0].clone();
        assert!(after.menu_outdated.is_empty(), "{:?}", after.menu_outdated);
    }

    #[test]
    fn a_versioned_file_name_is_renamed_to_the_standard_one_while_open() {
        let (dir, paths, mut settings) = setup();
        settings.move_on_integrate = false;
        settings.keep_replaced_versions_in_trash = false;
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        // A second copy gets a versioned name; then the first goes away.
        execute(
            Operation::Integrate {
                file: demo(&src, "a.AppImage", "1.0"),
                conflict: Conflict::Ask,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        execute(
            Operation::Integrate {
                file: demo(&src, "b.AppImage", "2.0"),
                conflict: Conflict::KeepBoth,
                replace: None,
                source: None,
            },
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        let inv = crate::library::load(&paths, &settings);
        let versioned = inv
            .apps
            .iter()
            .find(|a| a.path.ends_with("demo_app_2_0.appimage"))
            .unwrap()
            .clone();
        assert_eq!(
            standard_path(&paths, &settings, &versioned),
            None,
            "standard name is taken"
        );
        let first = inv
            .apps
            .iter()
            .find(|a| a.path.ends_with("demo_app.appimage"))
            .unwrap()
            .clone();
        fs::remove_file(&first.path).unwrap();
        fs::remove_file(&first.desktop_file).unwrap();
        // As a real removal does: the first copy's icon goes too.
        fs::remove_file(dir.path().join("AppImages/.icons/demo_app.png")).unwrap();
        let mut open = fs::File::open(&versioned.path).unwrap();
        let target = standard_path(&paths, &settings, &versioned).unwrap();
        assert!(target.ends_with("AppImages/demo_app.appimage"));
        let report = execute(
            Operation::RenameFile(versioned.clone(), target.clone()),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut buf = Vec::new();
        use std::io::Read;
        open.read_to_end(&mut buf).unwrap();
        assert!(!buf.is_empty());
        assert!(!versioned.desktop_file.exists());
        let d = DesktopFile::parse(
            &fs::read_to_string(paths.applications().join("demo_app.desktop")).unwrap(),
        );
        assert_eq!(d.main("TryExec").unwrap(), target.to_string_lossy());
        assert!(d.main("Exec").unwrap().contains(&*target.to_string_lossy()));
        assert_eq!(
            d.main("Icon").unwrap(),
            dir.path()
                .join("AppImages/.icons/demo_app.png")
                .to_string_lossy()
        );
        let after = crate::library::load(&paths, &settings);
        assert_eq!(after.apps.len(), 1);
        assert_eq!(after.apps[0].path, target);
        assert!(after.apps[0].standard_path.is_none());
    }

    #[test]
    fn the_lock_refuses_overlapping_operations() {
        let (_dir, paths, settings) = setup();
        fs::create_dir_all(&paths.data).unwrap();
        let held = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.lock())
            .unwrap();
        held.try_lock().unwrap();
        let err = execute(
            Operation::Settings(Box::new(settings.clone())),
            &paths,
            &settings,
            &Job::default(),
        )
        .unwrap_err();
        assert!(err.contains("Another"));
    }
}
