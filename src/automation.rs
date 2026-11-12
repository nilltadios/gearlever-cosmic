// SPDX-License-Identifier: GPL-3.0-or-later
//! Process-per-invocation CLI. Backend commands never start the GUI.
use crate::{
    appimage,
    library::{self, App, Inventory},
    ops::{self, Conflict, Job, Operation},
    paths::Paths,
    settings::Settings,
    store,
    updates::{self, Source, Status},
};
use clap::{Parser, Subcommand};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Parser)]
#[command(
    name = "gearlever-cosmic-cli",
    version,
    about = "AppImage management and explicit GUI controls. Changes require --apply.",
    long_about = "Gear Lever COSMIC CLI uses the same Rust backend as the desktop app. Backend commands run without a window; the ui subcommands inspect and operate the open window.\n\nStart with `guide`. Discover commands with `commands --json`, plan with `run --cmd`, apply with `run --apply`. Gear Lever's options (--integrate, --update, --remove, --list-installed, --list-updates, --list-update-managers, --set-update-source, --fetch-updates) are accepted too; their changes also need --apply (or -y)."
)]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Start here: how agents discover, plan, apply and verify, for the CLI and the live GUI.
    Guide,
    /// Print bundled documentation. Without a topic, list the topics.
    Docs { topic: Option<String> },
    /// Inspect and control the running GUI through its own D-Bus interface.
    Ui(crate::ui_client::Args),
    /// List command IDs, descriptions and parameter schemas.
    Commands {
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One command's schema, effects and example as JSON.
    Describe { command: String },
    /// The complete inventory as JSON: apps, running state, settings.
    Inspect,
    /// JSON Schema for script commands.
    Schema,
    /// Run commands in order, stopping at the first error. Applied steps are not rolled back.
    Run {
        #[arg(
            long = "cmd",
            conflicts_with = "script",
            required_unless_present = "script"
        )]
        commands: Vec<String>,
        #[arg(long, conflicts_with = "commands")]
        script: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        /// If either root is supplied, external file reads/writes require their root.
        #[arg(long)]
        automation_read_root: Option<PathBuf>,
        #[arg(long)]
        automation_write_root: Option<PathBuf>,
        /// Also write the result envelope to a new file.
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Gear Lever: integrate an AppImage file (--keep-both | --replace; -y applies).
    Integrate {
        file: PathBuf,
        #[arg(long, conflicts_with = "replace")]
        keep_both: bool,
        #[arg(long)]
        replace: bool,
        /// Gear Lever's --update-url: a static download URL for updates.
        #[arg(long)]
        update_url: Option<String>,
        #[arg(short = 'y', long = "yes", alias = "apply")]
        yes: bool,
    },
    /// Gear Lever: update an integrated AppImage, or --all (-y applies). Running apps are updated in place.
    Update {
        /// The app: id, name, AppImage path or menu entry.
        #[arg(value_name = "APP")]
        file: Option<PathBuf>,
        #[arg(long)]
        all: bool,
        /// Accepted for Gear Lever compatibility; running apps are never skipped.
        #[arg(long)]
        force: bool,
        #[arg(short = 'y', long = "yes", alias = "apply")]
        yes: bool,
    },
    /// Gear Lever: move an AppImage, its menu entry and icon to the Trash (-y applies).
    Remove {
        /// The app: id, name, AppImage path or menu entry.
        #[arg(value_name = "APP")]
        file: PathBuf,
        /// Not supported: removal always uses the Trash.
        #[arg(long)]
        delete: bool,
        #[arg(short = 'y', long = "yes", alias = "apply")]
        yes: bool,
    },
    /// Gear Lever: list integrated apps (--json for schema_version 1).
    ListInstalled {
        #[arg(long)]
        json: bool,
    },
    /// Gear Lever: check every app and list available updates (--json; --all-statuses for every result).
    ListUpdates {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all_statuses: bool,
    },
    /// Gear Lever: list update managers and their fields.
    ListUpdateManagers,
    /// Gear Lever: set (or --unset) an app's update source: APP --manager NAME KEY=VALUE... (-y applies).
    SetUpdateSource {
        /// The app: id, name, AppImage path or menu entry.
        #[arg(value_name = "APP")]
        file: PathBuf,
        #[arg(long)]
        manager: Option<String>,
        #[arg(long)]
        unset: bool,
        pairs: Vec<String>,
        #[arg(short = 'y', long = "yes", alias = "apply")]
        yes: bool,
    },
    /// Check all apps and send one desktop notification if updates exist. Installs nothing.
    FetchUpdates,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppList {
    /// Case-insensitive filter on name, id and path.
    #[serde(default)]
    pub search: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CliLink {
    /// true: link ~/.local/bin/gearlever-cosmic-cli to this AppImage (or the
    /// native CLI). false: move the link to the Trash.
    pub enabled: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// App id from app.list, the AppImage path, the menu entry path, or the exact name.
    pub app: String,
}
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    /// App ids, paths or names. Empty means every app (where the command says so).
    #[serde(default)]
    pub apps: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileInspect {
    pub file: PathBuf,
    /// Also compute md5, sha1 and sha256 (reads the whole file).
    #[serde(default)]
    pub hashes: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileList {
    pub file: PathBuf,
    /// Only paths starting with this prefix, e.g. /usr/share/applications.
    #[serde(default)]
    pub prefix: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Integrate {
    pub file: PathBuf,
    /// What to do when another version of the app is integrated: ask (refuse and list it), keep_both, replace.
    #[serde(default)]
    pub conflict: Conflict,
    /// App to replace when conflict is replace and several versions exist.
    #[serde(default)]
    pub replace: Option<String>,
    #[serde(default)]
    pub update_source: Option<Source>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateApply {
    /// Apps to update. Empty with all=true updates every app with an installable update.
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub all: bool,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Arguments {
    pub app: String,
    /// One line, shell-style quoting, field codes like %U kept as is.
    pub arguments: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Env {
    pub app: String,
    /// The complete environment for the launcher (replaces the current one).
    pub env: BTreeMap<String, String>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Website {
    pub app: String,
    /// http(s) URL, or null to forget it.
    pub url: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSource {
    pub app: String,
    /// The source (see update.managers), or null to remove the custom one.
    pub source: Option<Source>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Icon {
    pub app: String,
    /// A PNG or SVG file to use as the app's icon.
    pub file: PathBuf,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    /// An integrated app, or omit and give file.
    #[serde(default)]
    pub app: Option<String>,
    /// Any AppImage file.
    #[serde(default)]
    pub file: Option<PathBuf>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsSet {
    /// Only the settings to change; see settings.get for names and values.
    pub settings: serde_json::Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum Request {
    #[serde(rename = "app.list")]
    List(AppList),
    #[serde(rename = "app.info")]
    Info(Target),
    #[serde(rename = "file.inspect")]
    FileInspect(FileInspect),
    #[serde(rename = "file.list")]
    FileList(FileList),
    #[serde(rename = "update.check")]
    Check(Targets),
    #[serde(rename = "update.managers")]
    Managers(Empty),
    #[serde(rename = "settings.get")]
    SettingsGet(Empty),
    #[serde(rename = "gearlever.list")]
    GearLeverList(Empty),
    #[serde(rename = "app.requirements")]
    Requirements(Requirements),
    #[serde(rename = "app.integrate")]
    Integrate(Integrate),
    #[serde(rename = "app.update")]
    Update(UpdateApply),
    #[serde(rename = "app.remove")]
    Remove(Targets),
    #[serde(rename = "app.set-arguments")]
    SetArguments(Arguments),
    #[serde(rename = "app.set-env")]
    SetEnv(Env),
    #[serde(rename = "app.set-website")]
    SetWebsite(Website),
    #[serde(rename = "app.sync-menu")]
    SyncMenu(Targets),
    #[serde(rename = "app.rename-file")]
    RenameFile(Target),
    #[serde(rename = "app.set-icon")]
    SetIcon(Icon),
    #[serde(rename = "app.reset-icon")]
    ResetIcon(Target),
    #[serde(rename = "update.set-source")]
    SetSource(SetSource),
    #[serde(rename = "settings.set")]
    SettingsSet(SettingsSet),
    #[serde(rename = "cli.link")]
    CliLink(CliLink),
    #[serde(rename = "gearlever.import")]
    GearLeverImport(Targets),
    #[serde(rename = "app.launch")]
    Launch(Target),
    #[serde(rename = "app.open-folder")]
    OpenFolder(Target),
}

#[derive(Debug, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip)]
    pub exit: u8,
}
type Result<T> = std::result::Result<T, Error>;
fn error(code: &'static str, exit: u8, message: impl Into<String>) -> Error {
    Error {
        code,
        exit,
        message: message.into(),
        details: None,
    }
}
fn invalid(message: impl Into<String>) -> Error {
    error("invalid_argument", 2, message)
}
fn failure(message: impl Into<String>) -> Error {
    error("backend_error", 1, message)
}
fn not_found(message: impl Into<String>) -> Error {
    error("not_found", 3, message)
}

impl Request {
    pub fn name(&self) -> String {
        serde_json::to_value(self).expect("Request serializes")["command"]
            .as_str()
            .unwrap_or_default()
            .into()
    }
    pub fn mutates(&self) -> bool {
        !matches!(
            self,
            Self::List(_)
                | Self::Info(_)
                | Self::FileInspect(_)
                | Self::FileList(_)
                | Self::Check(_)
                | Self::Managers(_)
                | Self::SettingsGet(_)
                | Self::GearLeverList(_)
                | Self::Requirements(_)
        )
    }
}

fn entry<T: JsonSchema>(name: &str, description: &str, mutates: bool, example: Value) -> Value {
    json!({"command":name,"description":description,"mutates":mutates,"params_schema":schema_for!(T),"example":example,"dry_run_by_default":mutates})
}

pub fn catalog() -> Vec<Value> {
    vec![
        entry::<AppList>(
            "app.list",
            "List integrated AppImages with the version read from each file, update source, and running state (pids; old_version means a replaced file is still running; the window offers Relaunch).",
            false,
            json!({"search":""}),
        ),
        entry::<Target>(
            "app.info",
            "One app in full, with file hashes and running state.",
            false,
            json!({"app":"vscodium"}),
        ),
        entry::<FileInspect>(
            "file.inspect",
            "Read any AppImage file's metadata without integrating it: name, version, architecture, type, embedded update information, built-in updater, and whether it is already integrated or is another version of an integrated app.",
            false,
            json!({"file":"/home/user/Downloads/App.AppImage","hashes":true}),
        ),
        entry::<FileList>(
            "file.list",
            "List the files inside an AppImage's embedded image (SquashFS or DwarFS) with sizes, read in-process without running it. prefix narrows the list.",
            false,
            json!({"file":"/home/user/Downloads/App.AppImage","prefix":"/usr/share/applications"}),
        ),
        entry::<Targets>(
            "update.check",
            "Check apps for updates (empty apps = all). Each result has status available | up_to_date | self_updating | no_source | not_found | failed, a plain message, what was compared (sha1/sha256/sha512/size), and can_install. no_source and failed never mean up to date.",
            false,
            json!({"apps":[]}),
        ),
        entry::<Empty>(
            "update.managers",
            "Update source types and the fields each one takes (Gear Lever names).",
            false,
            json!({}),
        ),
        entry::<Empty>(
            "settings.get",
            "Current settings with their meanings.",
            false,
            json!({}),
        ),
        entry::<Empty>(
            "gearlever.list",
            "Update sources and websites saved by Gear Lever for files that are integrated here (read-only).",
            false,
            json!({}),
        ),
        entry::<Requirements>(
            "app.requirements",
            "Will it start here? Which FUSE its runtime needs (fuse2 runtime vs static fuse3/uruntime), whether libfuse.so.2, fusermount/fusermount3 and /dev/fuse exist, and which libraries its main program needs that are neither bundled nor installed. verdict: ok | missing | unknown, with problems and fixes.",
            false,
            json!({"app":"vscodium"}),
        ),
        entry::<Integrate>(
            "app.integrate",
            "Add an AppImage to the app menu: move or copy it into the AppImages folder (see settings), extract its icon and write a menu entry. If another version is integrated, conflict decides: ask (refuse, the default), keep_both, or replace (in place, keeping arguments and environment).",
            true,
            json!({"file":"/home/user/Downloads/App.AppImage","conflict":"ask"}),
        ),
        entry::<UpdateApply>(
            "app.update",
            "Download, verify and install updates. Running apps are updated in place: their open instance keeps the old version until restarted. Apps without an installable update are listed as skipped with the reason.",
            true,
            json!({"apps":["vscodium"]}),
        ),
        entry::<Targets>(
            "app.remove",
            "Move AppImages, their menu entries and extracted icons to the desktop Trash. Nothing is deleted permanently.",
            true,
            json!({"apps":["vscodium"]}),
        ),
        entry::<Arguments>(
            "app.set-arguments",
            "Set the command line arguments in the app's menu entry.",
            true,
            json!({"app":"vscodium","arguments":"--ozone-platform=wayland %F"}),
        ),
        entry::<Env>(
            "app.set-env",
            "Replace the environment variables the menu entry sets (env K=V before the program).",
            true,
            json!({"app":"vscodium","env":{"DESKTOPINTEGRATION":"1"}}),
        ),
        entry::<Website>(
            "app.set-website",
            "Save or forget the app's website.",
            true,
            json!({"app":"vscodium","url":"https://vscodium.com"}),
        ),
        entry::<Target>(
            "app.rename-file",
            "Rename an AppImage, its menu entry and icons to the standard name a fresh install gets (name.appimage), e.g. to drop a version left in the name by a first install. app.list shows the target as standard_path; refused when the name is already standard or taken. A running instance keeps working.",
            true,
            json!({"app":"vscodium"}),
        ),
        entry::<Icon>(
            "app.set-icon",
            "Use your own PNG or SVG as the app's icon. It is copied next to the AppImage and kept across updates and menu refreshes.",
            true,
            json!({"app":"vscodium","file":"/home/user/Pictures/my-icon.png"}),
        ),
        entry::<Target>(
            "app.reset-icon",
            "Use the icon shipped inside the AppImage again (the copied custom icon goes to the Trash).",
            true,
            json!({"app":"vscodium"}),
        ),
        entry::<Targets>(
            "app.sync-menu",
            "Rewrite menu entries from each AppImage's own metadata (name, icon, window matching), keeping arguments and environment. Empty apps = every integrated app. Gear Lever's \"Reload metadata\".",
            true,
            json!({"apps":[]}),
        ),
        entry::<SetSource>(
            "update.set-source",
            "Set or remove an app's custom update source.",
            true,
            json!({"app":"vscodium","source":{"manager":"GithubUpdater","repo":"VSCodium/vscodium","repo_filename":"VSCodium-*.AppImage","allow_prereleases":false}}),
        ),
        entry::<SettingsSet>(
            "settings.set",
            "Change settings (only the given keys).",
            true,
            json!({"settings":{"keep_replaced_versions_in_trash":true}}),
        ),
        entry::<CliLink>(
            "cli.link",
            "Put the CLI on PATH as ~/.local/bin/gearlever-cosmic-cli (a link to this AppImage or the native CLI), or remove that link. A file of that name that is not a link to Gear Lever COSMIC is never replaced. settings.get shows the link as cli_link. Gear Lever COSMIC creates it when it adds itself to the app menu.",
            true,
            json!({"enabled":true}),
        ),
        entry::<Targets>(
            "gearlever.import",
            "Copy Gear Lever's update sources and websites into this app's settings (empty apps = all found). Gear Lever's files are only read.",
            true,
            json!({"apps":[]}),
        ),
        entry::<Target>(
            "app.launch",
            "Start the app through its menu entry. Starts another program, so it needs --apply.",
            true,
            json!({"app":"vscodium"}),
        ),
        entry::<Target>(
            "app.open-folder",
            "Open the folder holding the AppImage in the file manager. Needs --apply.",
            true,
            json!({"app":"vscodium"}),
        ),
    ]
}

pub fn parse_command(text: &str) -> Result<Request> {
    if text.trim_start().starts_with('{') {
        return serde_json::from_str(text).map_err(|e| invalid(e.to_string()));
    }
    let (name, params) = text.split_once('=').unwrap_or((text, "{}"));
    let params: Value = serde_json::from_str(params).map_err(|e| invalid(e.to_string()))?;
    serde_json::from_value(json!({"command":name.trim(),"params":params}))
        .map_err(|e| invalid(e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    schema_version: u32,
    commands: Vec<Request>,
}
pub fn parse_script(text: &str) -> Result<Vec<Request>> {
    if let Ok(script) = serde_json::from_str::<Script>(text) {
        if script.schema_version != 1 {
            return Err(invalid("Unsupported script schema_version; expected 1"));
        }
        return Ok(script.commands);
    }
    if let Ok(commands) = serde_json::from_str::<Vec<Request>>(text) {
        return Ok(commands);
    }
    text.lines()
        .enumerate()
        .filter(|(_, s)| !s.trim().is_empty() && !s.trim_start().starts_with('#'))
        .map(|(i, s)| {
            parse_command(s).map_err(|mut e| {
                e.message = format!("Line {}: {}", i + 1, e.message);
                e
            })
        })
        .collect()
}

#[derive(Default)]
pub struct Policy {
    pub apply: bool,
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
}
impl Policy {
    fn external(&self, path: &Path, write: bool) -> Result<PathBuf> {
        let resolved = if write {
            let abs = std::path::absolute(path).map_err(|e| invalid(e.to_string()))?;
            let parent = abs
                .parent()
                .ok_or_else(|| invalid("Destination needs a parent directory"))?;
            fs::canonicalize(parent)
                .map_err(|e| invalid(e.to_string()))?
                .join(
                    abs.file_name()
                        .ok_or_else(|| invalid("Destination needs a file name"))?,
                )
        } else {
            fs::canonicalize(path).map_err(|e| not_found(format!("{}: {e}", path.display())))?
        };
        if self.read_root.is_some() || self.write_root.is_some() {
            let root = if write {
                &self.write_root
            } else {
                &self.read_root
            };
            let root = root.as_ref().ok_or_else(|| {
                error(
                    "path_denied",
                    4,
                    if write {
                        "No write root was granted"
                    } else {
                        "No read root was granted"
                    },
                )
            })?;
            if !resolved.starts_with(root) {
                return Err(error(
                    "path_denied",
                    4,
                    format!("{} is outside {}", resolved.display(), root.display()),
                ));
            }
        }
        Ok(resolved)
    }
}

pub struct Context {
    pub paths: Paths,
    pub settings: Settings,
}
impl Context {
    pub fn discover() -> Result<Self> {
        let paths = Paths::discover().map_err(failure)?;
        let settings = Settings::load(&paths).map_err(failure)?;
        Ok(Self { paths, settings })
    }
    fn inventory(&self) -> Inventory {
        library::load(&self.paths, &self.settings)
    }
}

fn find<'a>(inv: &'a Inventory, target: &str) -> Result<&'a App> {
    inv.find(target).map_err(not_found)
}
fn targets(inv: &Inventory, list: &[String]) -> Result<Vec<App>> {
    list.iter().map(|t| find(inv, t).cloned()).collect()
}

pub fn app_json(app: &App, running: &BTreeMap<String, library::Running>) -> Value {
    let mut v = serde_json::to_value(app).unwrap_or_default();
    v["running"] = json!(running.get(&app.id));
    v
}

fn settings_doc(s: &Settings) -> Value {
    json!({"settings":s,"descriptions":{
        "appimages_folder":"Folder for integrated AppImages; ~ is the home folder",
        "move_on_integrate":"true moves the opened file into the folder, false copies it",
        "show_external":"List menu entries whose AppImage is outside the folder",
        "terminal_apps_use_executable_name":"Save terminal apps without an extension, named like their executable",
        "safe_metadata":"Never run an AppImage to read its metadata (DwarFS images then show no metadata)",
        "check_updates_on_startup":"Check for updates after login and notify; installs nothing",
        "preview_minimal":"Show opened files in a compact preview",
        "debug_logs":"More detailed logs",
        "keep_replaced_versions_in_trash":"Put the version an update replaces in the Trash, so it can be restored",
        "theme":"system, dark or light"}})
}

/// Prepare a mutation from a request; also used by the GUI.
pub fn prepare(
    request: &Request,
    ctx: &Context,
    inv: &Inventory,
    policy: &Policy,
) -> Result<Vec<Operation>> {
    Ok(match request {
        Request::Integrate(p) => {
            let file = policy.external(&p.file, false)?;
            let meta = appimage::inspect(
                &file,
                &appimage::Options {
                    cache_dir: None,
                    allow_extract: !ctx.settings.safe_metadata,
                },
            )
            .map_err(invalid)?;
            if let Some(same) = ops::same_file(inv, &file) {
                return Err(error(
                    "already_integrated",
                    2,
                    format!(
                        "This AppImage is already integrated as {} ({})",
                        same.name, same.id
                    ),
                ));
            }
            let others = ops::same_app(inv, &meta);
            let replace = match (p.conflict, others.as_slice()) {
                (Conflict::Ask, [_, ..]) => {
                    let mut e = error(
                        "conflict",
                        2,
                        format!(
                            "Another version of {} is integrated: {}. Choose conflict keep_both or replace.",
                            meta.name.as_deref().unwrap_or("this app"),
                            others
                                .iter()
                                .map(|o| format!(
                                    "{} ({}, {})",
                                    o.id,
                                    o.display_version(),
                                    o.path.display()
                                ))
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    );
                    e.details = Some(
                        json!({"integrated": others.iter().map(|o| json!({"id":o.id,"version":o.version,"path":o.path})).collect::<Vec<_>>()}),
                    );
                    return Err(e);
                }
                (Conflict::Replace, []) => {
                    return Err(invalid(
                        "Nothing to replace: no other version is integrated; use conflict ask",
                    ));
                }
                (Conflict::Replace, [one]) if p.replace.is_none() => Some(one.clone()),
                (Conflict::Replace, _) => {
                    let id = p.replace.as_ref().ok_or_else(|| {
                        invalid("Several versions are integrated; say which with replace")
                    })?;
                    Some(find(inv, id)?.clone())
                }
                _ => None,
            };
            if let Some(s) = &p.update_source {
                s.validate().map_err(invalid)?;
            }
            vec![Operation::Integrate {
                file,
                conflict: p.conflict,
                replace,
                source: p.update_source.clone(),
            }]
        }
        Request::Update(p) => {
            let apps = if p.all {
                if !p.apps.is_empty() {
                    return Err(invalid("Use either apps or all"));
                }
                inv.apps.clone()
            } else if p.apps.is_empty() {
                return Err(invalid("Name apps to update, or set all: true"));
            } else {
                targets(inv, &p.apps)?
            };
            let mut list = Vec::new();
            for app in apps {
                let check = check_app(&ctx.paths, &app);
                match (&check.status, check.can_install, &check.download) {
                    (Status::Available, true, Some(d)) => list.push(Operation::Update {
                        app,
                        download: d.clone(),
                        version: check.available_version.clone(),
                    }),
                    _ if !p.all => {
                        let mut e =
                            error("no_update", 3, format!("{}: {}", app.name, check.message));
                        e.details = Some(json!({"check":check}));
                        return Err(e);
                    }
                    _ => {}
                }
            }
            list
        }
        Request::Remove(p) => {
            if p.apps.is_empty() {
                return Err(invalid("Name the apps to remove"));
            }
            vec![Operation::Remove(targets(inv, &p.apps)?)]
        }
        Request::SetArguments(p) => {
            let args =
                crate::desktop::split_command(&p.arguments.replace('\n', " ")).map_err(invalid)?;
            vec![Operation::SetArguments(find(inv, &p.app)?.clone(), args)]
        }
        Request::SetEnv(p) => {
            for k in p.env.keys() {
                if !crate::desktop::valid_env_key(k) {
                    return Err(invalid(format!(
                        "{k} is not a valid environment variable name"
                    )));
                }
            }
            vec![Operation::SetEnv(
                find(inv, &p.app)?.clone(),
                p.env.clone().into_iter().collect(),
            )]
        }
        Request::SetWebsite(p) => {
            if let Some(u) = &p.url
                && !(u.starts_with("https://") || u.starts_with("http://"))
            {
                return Err(invalid("The website must be an http or https URL"));
            }
            vec![Operation::SetWebsite(
                find(inv, &p.app)?.clone(),
                p.url.clone(),
            )]
        }
        Request::SyncMenu(p) => {
            let apps = if p.apps.is_empty() {
                inv.apps.clone()
            } else {
                targets(inv, &p.apps)?
            };
            if apps.is_empty() {
                return Ok(vec![]);
            }
            vec![Operation::SyncMenu(apps)]
        }
        Request::SetIcon(p) => {
            let file = policy.external(&p.file, false)?;
            let data = fs::read(&file).map_err(|e| invalid(e.to_string()))?;
            if appimage::image_kind(&data).is_none() {
                return Err(invalid("Choose a PNG or SVG image"));
            }
            vec![Operation::SetIcon(find(inv, &p.app)?.clone(), file)]
        }
        Request::RenameFile(p) => {
            let app = find(inv, &p.app)?;
            let target = ops::standard_path(&ctx.paths, &ctx.settings, app).ok_or_else(|| {
                invalid(format!(
                    "{} already has its standard name, or that name is taken",
                    app.name
                ))
            })?;
            vec![Operation::RenameFile(app.clone(), target)]
        }
        Request::ResetIcon(p) => vec![Operation::ResetIcon(find(inv, &p.app)?.clone())],
        Request::SetSource(p) => {
            if let Some(s) = &p.source {
                s.validate().map_err(invalid)?;
            }
            vec![Operation::SetSource(
                find(inv, &p.app)?.clone(),
                p.source.clone(),
            )]
        }
        Request::CliLink(p) => {
            let target = if p.enabled {
                Some(crate::cli_link::default_target().ok_or_else(|| {
                    invalid("No CLI to link: in the Flatpak use flatpak run --command=gearlever-cosmic-cli dev.nilltadios.GearLeverCosmic")
                })?)
            } else {
                None
            };
            vec![Operation::CliLink(target)]
        }
        Request::SettingsSet(p) => {
            let mut value = serde_json::to_value(&ctx.settings).unwrap_or_default();
            for (k, v) in &p.settings {
                if value.get(k).is_none() {
                    return Err(invalid(format!("Unknown setting {k}")));
                }
                value[k] = v.clone();
            }
            let new: Settings =
                serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
            if new.appimages_folder.trim().is_empty() {
                return Err(invalid("appimages_folder cannot be empty"));
            }
            vec![Operation::Settings(Box::new(new))]
        }
        Request::GearLeverImport(p) => {
            let apps = if p.apps.is_empty() {
                inv.apps.clone()
            } else {
                targets(inv, &p.apps)?
            };
            let list: Vec<_> = apps
                .into_iter()
                .filter_map(|a| {
                    let g = store::gearlever_for(&ctx.paths, &a.path)?;
                    (g.source.is_some() || g.website.is_some()).then_some((a, g.source, g.website))
                })
                .collect();
            if list.is_empty() {
                return Ok(vec![]);
            }
            vec![Operation::ImportGearLever(list)]
        }
        _ => return Err(invalid("Expected a mutating command")),
    })
}

pub fn check_app(paths: &Paths, app: &App) -> updates::Check {
    let Some(meta) = &app.metadata else {
        return updates::Check {
            status: Status::Failed,
            message: format!(
                "The AppImage's metadata could not be read: {}",
                app.metadata_error.clone().unwrap_or_default()
            ),
            origin: None,
            source: None,
            source_detail: None,
            current_version: None,
            available_version: None,
            compared_by: None,
            download: None,
            can_install: false,
            update_known: None,
            self_updater: None,
        };
    };
    let _ = paths;
    let mut check = updates::check(
        meta,
        app.update_source.as_ref().zip(app.update_source_origin),
    );
    if check.status == Status::NoSource
        && let Some(e) = &app.update_source_error
    {
        check.message = format!(
            "Gear Lever's saved update source cannot be used ({e}). Add a source to check for updates."
        );
    }
    check
}

pub fn run_request(request: &Request, ctx: &Context, policy: &Policy, job: &Job) -> Result<Value> {
    job.check().map_err(|e| error("cancelled", 5, e))?;
    let inv = ctx.inventory();
    let read = match request {
        Request::List(p) => {
            let running = library::running(&inv.apps).unwrap_or_default();
            let q = p.search.to_lowercase();
            let apps: Vec<Value> = inv
                .apps
                .iter()
                .filter(|a| format!("{} {} {}", a.name, a.id, a.path.display()).to_lowercase().contains(&q))
                .map(|a| app_json(a, &running))
                .collect();
            Some(json!({"apps":apps,"summary":{"listed":apps.len(),"integrated":inv.apps.len(),"folder":inv.folder,"applications_dir":inv.applications_dir,"scanned_menu_entries":inv.scanned_entries,"show_external":ctx.settings.show_external}}))
        }
        Request::Info(p) => {
            let app = find(&inv, &p.app)?;
            let running = library::running(std::slice::from_ref(app)).unwrap_or_default();
            let mut v = app_json(app, &running);
            v["hashes"] = json!(appimage::hashes(&app.path).ok());
            v["requirements"] = json!(crate::requirements::check(&app.path, &crate::requirements::System::host()).ok());
            v["embedded_desktop_entry"] = json!(app.metadata.as_ref().and_then(|m| m.desktop_entry.clone()));
            Some(v)
        }
        Request::FileInspect(p) => {
            let file = policy.external(&p.file, false)?;
            let meta = appimage::inspect_cached(&ctx.paths, &file, !ctx.settings.safe_metadata).map_err(invalid)?;
            let same = ops::same_file(&inv, &file);
            let versions = ops::same_app(&inv, &meta);
            Some(json!({"metadata":meta,"host_arch":crate::elf::host_arch(),"arch_matches":meta.arch == crate::elf::host_arch(),
                "already_integrated":same.map(|a| a.id),
                "requirements":crate::requirements::check(&file, &crate::requirements::System::host()).ok(),
                "other_versions":versions.iter().map(|a| json!({"id":a.id,"version":a.version,"path":a.path})).collect::<Vec<_>>(),
                "hashes": if p.hashes { json!(appimage::hashes(&file).map_err(failure)?) } else { Value::Null }}))
        }
        Request::FileList(p) => {
            let file = policy.external(&p.file, false)?;
            let tree = appimage::open_tree(&file).map_err(invalid)?;
            let mut files: Vec<Value> = tree
                .files()
                .into_iter()
                .filter(|e| e.path.starts_with(&p.prefix))
                .map(|e| json!({"path":e.path,"size":e.size,"executable":e.executable}))
                .collect();
            files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            Some(json!({"root":tree.root_names(),"files_listed":files.len(),"files":files}))
        }
        Request::Check(p) => {
            let apps = if p.apps.is_empty() { inv.apps.clone() } else { targets(&inv, &p.apps)? };
            let results: Vec<Value> = apps.iter().map(|a| json!({"app":a.id,"name":a.name,"check":check_app(&ctx.paths, a)})).collect();
            let count = |s: &str| results.iter().filter(|r| r["check"]["status"] == s).count();
            Some(json!({"results":results,"summary":{"checked":results.len(),"available":count("available"),"up_to_date":count("up_to_date"),"self_updating":count("self_updating"),"no_source":count("no_source"),"not_found":count("not_found"),"failed":count("failed")}}))
        }
        Request::Requirements(p) => {
            let file = match (&p.app, &p.file) {
                (Some(a), None) => find(&inv, a)?.path.clone(),
                (None, Some(f)) => policy.external(f, false)?,
                _ => return Err(invalid("Give either app or file")),
            };
            Some(json!(crate::requirements::check(&file, &crate::requirements::System::host()).map_err(invalid)?))
        }
        Request::Managers(_) => Some(json!(updates::MANAGERS.iter().map(|m| json!({"name":m.name,"label":m.label,"fields":m.fields.iter().map(|(f, d)| json!({"name":f,"description":d})).collect::<Vec<_>>()})).collect::<Vec<_>>())),
        Request::SettingsGet(_) => {
            let mut doc = settings_doc(&ctx.settings);
            doc["cli_link"] = json!(crate::cli_link::status(&ctx.paths));
            Some(doc)
        }
        Request::GearLeverList(_) => Some(json!({
            "configs": ctx.paths.gearlever_configs().iter().filter(|p| p.is_file()).collect::<Vec<_>>(),
            "apps": inv.apps.iter().filter_map(|a| store::gearlever_for(&ctx.paths, &a.path).map(|g| json!({"app":a.id,"gearlever":g}))).collect::<Vec<_>>()})),
        _ => None,
    };
    if let Some(data) = read {
        return Ok(
            json!({"command":request.name(),"mode":"read","data":data,"warnings":inv.warnings}),
        );
    }
    if let Request::Launch(p) | Request::OpenFolder(p) = request {
        let app = find(&inv, &p.app)?;
        let launch = matches!(request, Request::Launch(_));
        let description = if launch {
            format!("Start {} through {}", app.name, app.desktop_file.display())
        } else {
            format!(
                "Open {} in the file manager",
                app.path.parent().unwrap_or(Path::new("/")).display()
            )
        };
        if !policy.apply {
            return Ok(
                json!({"command":request.name(),"mode":"dry_run","plan":[description],"applied":false}),
            );
        }
        if launch {
            launch_app(app).map_err(failure)?;
        } else {
            crate::host::open(
                &app.path
                    .parent()
                    .unwrap_or(Path::new("/"))
                    .to_string_lossy(),
            )
            .map_err(failure)?;
        }
        return Ok(json!({"command":request.name(),"mode":"applied","done":[description]}));
    }
    let ops = prepare(request, ctx, &inv, policy)?;
    if ops.is_empty() {
        return Ok(
            json!({"command":request.name(),"mode":if policy.apply {"applied"} else {"dry_run"},"plan":[],"message":"Nothing to do"}),
        );
    }
    let plans: Vec<Value> = ops.iter().map(|op| Ok(json!({"title":op.title(),"effects":op.plan(&ctx.paths, &ctx.settings).map_err(invalid)?}))).collect::<Result<_>>()?;
    if !policy.apply {
        return Ok(json!({"command":request.name(),"mode":"dry_run","plan":plans,"applied":false}));
    }
    let mut reports = Vec::new();
    for op in ops {
        let report = ops::execute(op, &ctx.paths, &ctx.settings, job).map_err(|e| {
            if e.starts_with("Another") {
                error("busy", 6, e)
            } else {
                failure(e)
            }
        })?;
        if report.cancelled {
            let mut e = error(
                "cancelled",
                5,
                format!(
                    "Cancelled; completed steps remain applied. Report: {}",
                    report.log.display()
                ),
            );
            e.details = Some(json!({"reports":reports,"report":report}));
            return Err(e);
        }
        if !report.errors.is_empty() {
            let mut e = error(
                "operation_failed",
                1,
                format!(
                    "{}\nReport: {}",
                    report.errors.join("\n"),
                    report.log.display()
                ),
            );
            e.details = Some(json!({"reports":reports,"report":report}));
            return Err(e);
        }
        reports.push(report);
    }
    Ok(json!({"command":request.name(),"mode":"applied","reports":reports}))
}

/// Start an integrated app the way the app menu would. A launch that fails
/// right away (for example missing FUSE) is reported with its output.
pub fn launch_app(app: &App) -> std::result::Result<(), String> {
    let stem = app.desktop_file.file_stem().unwrap_or_default();
    let log = crate::host::launch_log(&stem.to_string_lossy());
    let mut child = crate::host::launch_desktop_file(&app.desktop_file, log.as_deref())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let err = log
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        if err.contains("FUSE") || err.contains("fuse") {
            return Err(format!(
                "{} needs FUSE to run. Install fuse2 (libfuse.so.2), or add --appimage-extract-and-run to its arguments. Details: {}",
                app.name,
                err.trim()
            ));
        }
        return Err(format!("Starting {} failed: {}", app.name, err.trim()));
    }
    Ok(())
}

fn emit(value: &Value) -> io::Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)
}
fn envelope(results: Vec<Value>, failure: Option<&Error>) -> Value {
    json!({"schema":"gearlever-cosmic/1","ok":failure.is_none(),"results":results,"error":failure})
}
fn canonical_root(path: Option<PathBuf>) -> Result<Option<PathBuf>> {
    path.map(|p| {
        fs::canonicalize(p)
            .map_err(|e| invalid(e.to_string()))
            .and_then(|p| {
                if p.is_dir() {
                    Ok(p)
                } else {
                    Err(invalid("Automation root must be a directory"))
                }
            })
    })
    .transpose()
}

pub const DOCS: [(&str, &str, &str); 6] = [
    (
        "llms",
        "Short index for agents",
        include_str!("../llms.txt"),
    ),
    (
        "cli",
        "Command contract, scripts, exit codes, file roots",
        include_str!("../docs/cli.md"),
    ),
    (
        "agents",
        "Plan, apply and verify workflow",
        include_str!("../docs/agents.md"),
    ),
    (
        "ui-cli",
        "Operating and testing the live window",
        include_str!("../docs/ui-cli.md"),
    ),
    (
        "updates",
        "Update sources, statuses and in-place updates",
        include_str!("../docs/updates.md"),
    ),
    (
        "parity",
        "Feature comparison with Gear Lever",
        include_str!("../docs/parity.md"),
    ),
];

fn table(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|i| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |c| c.chars().count()))
                .max()
                .unwrap_or(0)
                + 3
        })
        .collect();
    rows.iter()
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
                .collect::<String>()
                .trim_end()
                .to_string()
                + "\n"
        })
        .collect()
}

fn print(text: &str) -> Result<Value> {
    io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|e| failure(e.to_string()))?;
    Ok(Value::Null)
}

/// Gear Lever's JSON entries (schema_version 1), plus status fields.
fn compat_json(
    app: &App,
    check: Option<&updates::Check>,
    running: &BTreeMap<String, library::Running>,
) -> Value {
    json!({"name":app.name,"path":app.path,"desktop_id":app.desktop_file.file_name().map(|f| f.to_string_lossy()),
        "current_version":app.version,"available_version":check.and_then(|c| c.available_version.clone()),
        "download_size":check.and_then(|c| c.download.as_ref().and_then(|d| d.size)),
        "manager":check.and_then(|c| c.source.clone()).or(app.update_source.as_ref().map(|s| s.manager().name.to_string())),
        "embedded_source":check.map(|c| c.origin == Some(updates::Origin::Embedded)).unwrap_or(app.update_info.is_some() && app.update_source.is_none()),
        "running":running.contains_key(&app.id),
        "status":check.map(|c| &c.status),"message":check.map(|c| &c.message),"can_install":check.map(|c| c.can_install)})
}

fn compat(action: Action) -> Result<Value> {
    let ctx = Context::discover()?;
    let run = |req: Request, apply: bool| -> Result<Value> {
        let value = run_request(
            &req,
            &ctx,
            &Policy {
                apply,
                ..Policy::default()
            },
            &Job::default(),
        );
        match value {
            Ok(v) => {
                if v["mode"] == "dry_run" {
                    let mut out = String::new();
                    for p in v["plan"].as_array().into_iter().flatten() {
                        if let Some(t) = p["title"].as_str() {
                            out.push_str(&format!("{t}:\n"));
                        }
                        for e in p["effects"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .chain(p.as_str().map(|_| p))
                        {
                            out.push_str(&format!("  - {}\n", e.as_str().unwrap_or_default()));
                        }
                    }
                    out.push_str("\nNothing was changed. Add -y (or --apply) to do this.\n");
                    print(&out)
                } else {
                    let mut out = String::new();
                    for r in v["reports"].as_array().into_iter().flatten() {
                        for s in r["steps"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .chain(r["notes"].as_array().into_iter().flatten())
                        {
                            out.push_str(&format!("{}\n", s.as_str().unwrap_or_default()));
                        }
                    }
                    if out.is_empty() {
                        out = v["message"].as_str().unwrap_or("Done").to_string() + "\n";
                    }
                    print(&out)
                }
            }
            Err(e) => Err(e),
        }
    };
    // APP as `run` takes it: id, exact name, AppImage path or menu entry.
    // A relative file path is also tried from the current folder. On failure
    // the error names what was typed and lists the known ids.
    let app_for = |target: &Path| -> Result<String> {
        let inv = ctx.inventory();
        let typed = inv.find(&target.to_string_lossy());
        if let Ok(a) = typed {
            return Ok(a.id.clone());
        }
        let abs = std::path::absolute(target).map_err(|e| invalid(e.to_string()))?;
        inv.find(&abs.to_string_lossy())
            .map(|a| a.id.clone())
            .or_else(|_| typed.map(|a| a.id.clone()))
            .map_err(not_found)
    };
    match action {
        Action::Integrate {
            file,
            keep_both,
            replace,
            update_url,
            yes,
        } => run(
            Request::Integrate(Integrate {
                file,
                conflict: if replace {
                    Conflict::Replace
                } else if keep_both {
                    Conflict::KeepBoth
                } else {
                    Conflict::Ask
                },
                replace: None,
                update_source: update_url.map(|url| Source::StaticUrl { url }),
            }),
            yes,
        ),
        Action::Update {
            file,
            all,
            force: _,
            yes,
        } => {
            if all {
                run(
                    Request::Update(UpdateApply {
                        apps: vec![],
                        all: true,
                    }),
                    yes,
                )
            } else {
                let file = file.ok_or_else(|| invalid("Usage: update FILE, or update --all"))?;
                run(
                    Request::Update(UpdateApply {
                        apps: vec![app_for(&file)?],
                        all: false,
                    }),
                    yes,
                )
            }
        }
        Action::Remove { file, delete, yes } => {
            if delete {
                return Err(invalid(
                    "--delete is not supported: removal moves files to the Trash so it can be undone. Run without --delete.",
                ));
            }
            run(
                Request::Remove(Targets {
                    apps: vec![app_for(&file)?],
                }),
                yes,
            )
        }
        Action::ListInstalled { json: as_json } => {
            let inv = ctx.inventory();
            let running = library::running(&inv.apps).unwrap_or_default();
            if as_json {
                return Ok(
                    json!({"schema_version":1,"installed":inv.apps.iter().map(|a| compat_json(a, None, &running)).collect::<Vec<_>>()}),
                );
            }
            let rows: Vec<Vec<String>> = inv
                .apps
                .iter()
                .map(|a| {
                    let src = match (&a.update_source, &a.update_info, &a.self_updater) {
                        (Some(s), ..) => s.manager().name.to_string(),
                        (None, Some(_), _) => "embedded".into(),
                        (None, None, Some(_)) => "updates itself".into(),
                        _ => "UpdatesNotAvailable".into(),
                    };
                    let v = format!("[{}]", a.display_version());
                    let mut src = format!("[{src}]");
                    if let Some(f) = &a.flatpak {
                        src.push_str(&format!(" [also Flatpak {f}]"));
                    }
                    vec![a.name.clone(), v, src, a.path.display().to_string()]
                })
                .collect();
            if rows.is_empty() {
                return print("No integrated AppImages\n");
            }
            print(&table(&rows))
        }
        Action::ListUpdates {
            json: as_json,
            all_statuses,
        } => {
            let inv = ctx.inventory();
            let running = library::running(&inv.apps).unwrap_or_default();
            let checks: Vec<(App, updates::Check)> = inv
                .apps
                .iter()
                .map(|a| (a.clone(), check_app(&ctx.paths, a)))
                .collect();
            let shown = |c: &updates::Check| {
                all_statuses
                    || c.status == Status::Available
                    || (c.status == Status::SelfUpdating && c.update_known == Some(true))
            };
            if as_json {
                return Ok(
                    json!({"schema_version":1,"updates":checks.iter().filter(|(_, c)| shown(c)).map(|(a, c)| compat_json(a, Some(c), &running)).collect::<Vec<_>>()}),
                );
            }
            let rows: Vec<Vec<String>> = checks
                .iter()
                .filter(|(_, c)| shown(c))
                .map(|(a, c)| {
                    vec![
                        a.name.clone(),
                        format!("[{}]", status_label(c)),
                        c.message.clone(),
                    ]
                })
                .collect();
            let mut out = if rows.is_empty() {
                String::new()
            } else {
                table(&rows)
            };
            if !all_statuses {
                let unknown: Vec<String> = checks
                    .iter()
                    .filter(|(_, c)| !shown(c) && c.status != Status::UpToDate)
                    .map(|(a, c)| format!("  {}: {}", a.name, status_label(c)))
                    .collect();
                if rows.is_empty() {
                    out.push_str(&format!(
                        "No updates found for the {} app(s) that could be checked.\n",
                        checks
                            .iter()
                            .filter(|(_, c)| c.status == Status::UpToDate)
                            .count()
                    ));
                }
                if !unknown.is_empty() {
                    out.push_str(&format!(
                        "Not checked or unknown ({}); use --all-statuses for details:\n{}\n",
                        unknown.len(),
                        unknown.join("\n")
                    ));
                }
            }
            print(&out)
        }
        Action::ListUpdateManagers => print(&table(
            &updates::MANAGERS
                .iter()
                .map(|m| {
                    vec![
                        m.label.to_string(),
                        m.name.to_string(),
                        m.fields
                            .iter()
                            .map(|(f, _)| *f)
                            .collect::<Vec<_>>()
                            .join(", "),
                    ]
                })
                .collect::<Vec<_>>(),
        )),
        Action::SetUpdateSource {
            file,
            manager,
            unset,
            pairs,
            yes,
        } => {
            let app = app_for(&file)?;
            let source = if unset {
                None
            } else {
                let m = manager.ok_or_else(|| {
                    invalid(format!(
                        "Use --manager with one of: {}",
                        updates::MANAGERS.map(|m| m.name).join(", ")
                    ))
                })?;
                let pairs: Vec<(String, String)> = pairs
                    .iter()
                    .map(|p| {
                        p.split_once('=')
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .ok_or_else(|| invalid(format!("{p} is not KEY=VALUE")))
                    })
                    .collect::<Result<_>>()?;
                Some(Source::from_pairs(&m, &pairs).map_err(invalid)?)
            };
            run(Request::SetSource(SetSource { app, source }), yes)
        }
        Action::FetchUpdates => fetch_updates(&ctx),
        _ => unreachable!(),
    }
}

pub fn status_label(c: &updates::Check) -> String {
    match c.status {
        Status::Available if c.can_install => format!(
            "Update available{}",
            c.available_version
                .as_ref()
                .map(|v| format!(": {v}"))
                .unwrap_or_default()
        ),
        Status::Available => "Update available in the app".into(),
        Status::UpToDate => "Up to date".into(),
        Status::SelfUpdating if c.update_known == Some(true) => {
            "Update available in the app (open it to update)".into()
        }
        Status::SelfUpdating => "Updates itself (open the app to check)".into(),
        Status::NoSource => "No update source".into(),
        Status::NotFound => "Release file not found".into(),
        Status::Failed => "Check failed".into(),
    }
}

/// The startup check: one notification, nothing installed.
fn fetch_updates(ctx: &Context) -> Result<Value> {
    let inv = ctx.inventory();
    let checks: Vec<(App, updates::Check)> = inv
        .apps
        .iter()
        .map(|a| (a.clone(), check_app(&ctx.paths, a)))
        .collect();
    let installable: Vec<&str> = checks
        .iter()
        .filter(|(_, c)| c.status == Status::Available && c.can_install)
        .map(|(a, _)| a.name.as_str())
        .collect();
    let in_app: Vec<&str> = checks
        .iter()
        .filter(|(_, c)| c.status == Status::SelfUpdating && c.update_known == Some(true))
        .map(|(a, _)| a.name.as_str())
        .collect();
    let summary = json!({"installable":installable,"update_in_app":in_app,"checked":checks.len(),
        "failed":checks.iter().filter(|(_, c)| c.status == Status::Failed).map(|(a, c)| json!({"app":a.name,"message":c.message})).collect::<Vec<_>>()});
    if installable.is_empty() && in_app.is_empty() {
        eprintln!("No updates found among the apps that could be checked");
        return Ok(summary);
    }
    let title = match installable.len() {
        0 => "Updates available in apps".to_string(),
        1 => "1 AppImage update available".to_string(),
        n => format!("{n} AppImage updates available"),
    };
    let mut body = installable.join(", ");
    if !in_app.is_empty() {
        if !body.is_empty() {
            body.push_str(". ");
        }
        body.push_str(&format!("Open to update itself: {}", in_app.join(", ")));
    }
    if let Ok(true) = crate::host::notify(&title, &body, "Open Gear Lever COSMIC") {
        let _ = crate::ui_client::run(crate::ui_client::Args {
            instance: None,
            command: crate::ui_client::Command::Start { config_dir: None },
        });
    }
    Ok(summary)
}

fn run_cli(cli: Cli) -> Result<Value> {
    match cli.action {
        Action::Ui(args) => crate::ui_client::run(args).map_err(failure),
        Action::Guide => print(&crate::with_cli_command(include_str!("guide.txt"))),
        Action::Docs { topic } => {
            let text = match topic.as_deref() {
                None => {
                    let mut list = format!(
                        "Documentation topics ({} docs TOPIC):\n",
                        crate::cli_command()
                    );
                    for (name, summary, _) in DOCS {
                        list.push_str(&format!("  {name:<10} {summary}\n"));
                    }
                    list
                }
                Some(name) => DOCS
                    .iter()
                    .find(|(t, ..)| *t == name.trim_end_matches(".md"))
                    .map(|(.., text)| crate::with_cli_command(text))
                    .ok_or_else(|| {
                        invalid(format!(
                            "Unknown topic {name}; available: {}",
                            DOCS.iter().map(|(t, ..)| *t).collect::<Vec<_>>().join(", ")
                        ))
                    })?,
            };
            print(&text)
        }
        Action::Commands {
            search,
            json: as_json,
        } => {
            let entries: Vec<_> = catalog()
                .into_iter()
                .filter(|c| {
                    search.as_ref().is_none_or(|s| {
                        format!("{} {}", c["command"], c["description"])
                            .to_lowercase()
                            .contains(&s.to_lowercase())
                    })
                })
                .collect();
            if !as_json {
                let mut out = String::new();
                for c in &entries {
                    out.push_str(&format!(
                        "{:<20} {}{}\n",
                        c["command"].as_str().unwrap_or_default(),
                        if c["mutates"] == true { "[apply] " } else { "" },
                        c["description"].as_str().unwrap_or_default()
                    ));
                }
                return print(&out);
            }
            Ok(json!({"schema":"gearlever-cosmic-commands/1","commands":entries}))
        }
        Action::Describe { command } => catalog()
            .into_iter()
            .find(|c| c["command"] == command)
            .ok_or_else(|| invalid(format!("Unknown command {command}"))),
        Action::Inspect => {
            let ctx = Context::discover()?;
            let inv = ctx.inventory();
            let running = library::running(&inv.apps).unwrap_or_default();
            Ok(envelope(
                vec![
                    json!({"command":"inspect","mode":"read","data":{"apps":inv.apps.iter().map(|a| app_json(a, &running)).collect::<Vec<_>>(),"folder":inv.folder,"settings":ctx.settings,"warnings":inv.warnings}}),
                ],
                None,
            ))
        }
        Action::Schema => {
            Ok(serde_json::to_value(schema_for!(Request)).expect("Schema serializes"))
        }
        Action::Run {
            commands,
            script,
            apply,
            automation_read_root,
            automation_write_root,
            report,
        } => {
            let requests = if let Some(path) = script {
                parse_script(&fs::read_to_string(path).map_err(|e| invalid(e.to_string()))?)?
            } else {
                commands
                    .iter()
                    .map(|s| parse_command(s))
                    .collect::<Result<Vec<_>>>()?
            };
            if requests.is_empty() {
                return Err(invalid("A run needs at least one command"));
            }
            let policy = Policy {
                apply,
                read_root: canonical_root(automation_read_root)?,
                write_root: canonical_root(automation_write_root)?,
            };
            let report_file = report
                .map(|path| {
                    let path = policy.external(&path, true)?;
                    fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(path)
                        .map_err(|e| invalid(format!("Cannot create report: {e}")))
                })
                .transpose()?;
            let ctx = Context::discover()?;
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Relaxed))
                .map_err(|e| failure(e.to_string()))?;
            let job = Job {
                cancel,
                ..Job::default()
            };
            let mut results = vec![];
            let mut failed = None;
            for request in requests {
                match run_request(&request, &ctx, &policy, &job) {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            let value = envelope(results, failed.as_ref());
            if let Some(mut file) = report_file {
                serde_json::to_writer_pretty(&mut file, &value)
                    .map_err(|e| failure(e.to_string()))?;
                file.write_all(b"\n").map_err(|e| failure(e.to_string()))?;
            }
            if let Some(e) = failed {
                let _ = emit(&value);
                eprintln!("{}: {}", e.code, e.message);
                std::process::exit(e.exit.into());
            }
            Ok(value)
        }
        other => compat(other),
    }
}

/// Accept Gear Lever's `--integrate FILE` style as well as `integrate FILE`.
fn normalize_args(mut args: Vec<String>) -> Vec<String> {
    const COMPAT: [&str; 9] = [
        "integrate",
        "update",
        "remove",
        "list-installed",
        "list-updates",
        "list-update-managers",
        "set-update-source",
        "fetch-updates",
        "set-update-url",
    ];
    if let Some(first) = args
        .get(1)
        .and_then(|a| a.strip_prefix("--"))
        .map(str::to_string)
        && COMPAT.contains(&first.as_str())
    {
        args[1] = if first == "set-update-url" {
            "set-update-source".into()
        } else {
            first
        };
    }
    args
}

pub fn main_entry() -> ExitCode {
    use clap::{CommandFactory, FromArgMatches};
    let bin_name = crate::cli_command();
    let parsed = Cli::command()
        .bin_name(bin_name)
        .try_get_matches_from(normalize_args(std::env::args().collect()))
        .and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(c) => c,
        Err(e) => {
            if e.exit_code() == 0 {
                let _ = e.print();
                return ExitCode::SUCCESS;
            }
            let err = invalid(e.to_string());
            let _ = emit(&envelope(vec![], Some(&err)));
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let compat_text = matches!(
        cli.action,
        Action::Integrate { .. }
            | Action::Update { .. }
            | Action::Remove { .. }
            | Action::SetUpdateSource { .. }
    );
    match run_cli(cli) {
        Ok(Value::Null) => ExitCode::SUCCESS,
        Ok(value) => match emit(&value) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            if !compat_text {
                let _ = emit(&envelope(vec![], Some(&e)));
            }
            eprintln!("{}: {}", e.code, e.message);
            ExitCode::from(e.exit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_parameters_and_commands_are_errors() {
        assert!(parse_command("app.remove={\"apps\":[],\"force\":true}").is_err());
        assert!(parse_command("app.nuke").is_err());
    }
    #[test]
    fn scripts_are_parsed_before_any_effect() {
        assert!(parse_script("app.list\nunknown.command").is_err());
        assert_eq!(
            parse_script(
                "# note\napp.list\n{\"command\":\"update.check\",\"params\":{\"apps\":[]}}"
            )
            .unwrap()
            .len(),
            2
        );
    }
    #[test]
    fn catalog_examples_parse_and_mutation_flags_match() {
        for entry in catalog() {
            let req = parse_command(&format!(
                "{}={}",
                entry["command"].as_str().unwrap(),
                entry["example"]
            ))
            .unwrap();
            assert_eq!(
                req.mutates(),
                entry["mutates"] == true,
                "{}",
                entry["command"]
            );
            assert_eq!(entry["params_schema"]["type"], "object");
        }
    }
    #[test]
    fn gear_lever_flags_are_accepted() {
        let args = normalize_args(vec![
            "x".into(),
            "--integrate".into(),
            "/a".into(),
            "-y".into(),
        ]);
        assert_eq!(args[1], "integrate");
        assert!(Cli::try_parse_from(args).is_ok());
        assert!(
            Cli::try_parse_from(normalize_args(vec![
                "x".into(),
                "--list-installed".into(),
                "--json".into()
            ]))
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(normalize_args(vec![
                "x".into(),
                "--set-update-source".into(),
                "/a".into(),
                "--manager".into(),
                "GithubUpdater".into(),
                "repo=o/r".into()
            ]))
            .is_ok()
        );
    }
}
