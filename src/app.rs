// SPDX-License-Identifier: GPL-3.0-or-later
//! The desktop window. Every change goes through the same backend
//! operations as the CLI (`gearlever_cosmic::ops`).
use crate::window_state::{self, WindowState};
use cosmic::{
    Application,
    app::Task,
    iced::{self, Alignment, Length, Subscription},
    prelude::*,
    widget::{self, icon, nav_bar},
};
use gearlever_cosmic::{
    appimage::{self, Hashes, Metadata},
    automation,
    library::{self, App as AppItem, Inventory, ManagedBy, Running},
    ops::{self, Conflict, Job, Operation, Report},
    paths::Paths,
    requirements::{self, Requirements, Verdict},
    settings::{Settings, Theme},
    updates::{self, Check, MANAGERS, Source, Status},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[path = "ui_view.rs"]
mod ui_view;

/// Page width that fits the narrowest list (260), the divider (9), the
/// narrowest details pane (320) and spacing; below it one pane is shown.
pub const MIN_SPLIT: f32 = 610.;
/// Narrowest sidebar that still shows its longest label ("Add AppImages")
/// whole: libcosmic's sidebar cuts labels without an ellipsis.
pub const NAV_MIN: f32 = 236.;
const FILE_TRANSFER_MIME: &str = "application/vnd.portal.filetransfer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Apps,
    Updates,
    Add,
}
impl Page {
    pub const ALL: [Page; 3] = [Page::Apps, Page::Updates, Page::Add];
    pub fn name(self) -> &'static str {
        match self {
            Page::Apps => "Installed Apps",
            Page::Updates => "Updates",
            Page::Add => "Add AppImages",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Page::Apps => "apps",
            Page::Updates => "updates",
            Page::Add => "add",
        }
    }
}

/// The two draggable dividers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Between the navigation sidebar and the page.
    Nav,
    /// Between the app list and the details.
    List,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Search,
    Args,
    EnvKey(usize),
    EnvValue(usize),
    Website,
    Source(String),
    IconPath,
}

#[derive(Debug, Clone)]
pub enum Dialog {
    Remove(Vec<AppItem>),
    /// A reviewed operation: title, plan lines, confirm label.
    Confirm(Box<Operation>, Vec<String>),
    About,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub path: PathBuf,
    pub meta: Option<Result<Metadata, String>>,
    pub requirements: Option<Requirements>,
    pub conflict: Conflict,
    pub same_file: Option<String>,
    pub others: Vec<(String, Option<String>)>,
    pub hashes: Option<Result<Hashes, String>>,
}

#[derive(Debug, Clone)]
pub struct Loaded {
    inventory: Inventory,
    running: BTreeMap<String, Running>,
}

#[derive(Debug, Clone)]
pub enum Message {
    UiRequest(crate::ui_bridge::Pending),
    UiTree(crate::ui_bridge::Pending, Vec<crate::ui_bridge::Node>, bool),
    UiCaptured(
        crate::ui_bridge::Pending,
        PathBuf,
        bool,
        iced::window::Screenshot,
    ),
    UiActivationToken(crate::ui_bridge::Pending, Option<String>),
    UiCloseNow,
    SplitStart(Split),
    SplitMove(iced::Point),
    SplitEnd,
    ListWidth(f32),
    NavWidth(f32),
    Loaded(Box<Loaded>),
    Refresh,
    Page(Page),
    Select(String),
    Back,
    Field(Field, String),
    Checked(String, Box<Check>),
    CheckApp(String),
    CheckAll,
    CheckedAll(Vec<(String, Check)>),
    Requirements(String, Box<Result<Requirements, String>>),
    ShowHashes(PathBuf),
    Hashes(PathBuf, Box<Result<Hashes, String>>),
    Launch(String),
    Launched(Result<(), String>),
    OpenFolder(String),
    OpenWebsite(String),
    AskRemove(Vec<String>),
    UpdateApp(String),
    UpdateAll,
    SyncMenu(Vec<String>),
    SaveArgs,
    ResetArgs,
    AddEnv,
    RemoveEnv(usize),
    SaveEnv,
    SaveWebsite,
    SourceChoice(usize),
    Prerelease(bool),
    SaveSource,
    ChooseIcon,
    IconPicked(Result<PathBuf, String>),
    SetIconFromField,
    ResetIcon,
    Confirm,
    CloseDialog,
    Finished(Box<Result<Report, String>>),
    Cancel,
    Tick,
    OpenFiles,
    FilesPicked(Result<Vec<PathBuf>, String>),
    Inspected(PathBuf, Box<Pending>),
    Dropped(String, Vec<u8>),
    FileTransfer(String),
    DragOver(bool),
    PendingConflict(usize, Conflict),
    Integrate(Vec<usize>),
    RunPending(usize),
    DismissPending(usize),
    PendingHashes(usize),
    Verified(bool),
    Setting(SettingChange),
    ChooseFolder,
    FolderPicked(Result<PathBuf, String>),
    ImportGearLever,
    Settings,
    CloseSettings,
    About,
    ClearNotice,
    ClearOutcome,
    Copy(String),
    CloseToast(widget::ToastId),
    Key(iced::keyboard::Key, iced::keyboard::Modifiers),
    WindowChanged(iced::window::Id),
    WindowObserved(u64, iced::window::Mode, bool, iced::Size),
    Captured(iced::window::Screenshot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingChange {
    MoveOnIntegrate(bool),
    ShowExternal(bool),
    TerminalNames(bool),
    SafeMetadata(bool),
    CheckOnStartup(bool),
    PreviewMinimal(bool),
    DebugLogs(bool),
    KeepReplaced(bool),
    SyncMenus(bool),
    Theme(Theme),
}

#[derive(Debug, Default)]
pub struct Flags {
    pub open_files: Vec<PathBuf>,
    pub preview: Option<String>,
    pub capture: Option<PathBuf>,
    pub window_state: WindowState,
    pub activation_action: Option<String>,
}
impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = String;
    type Args = Vec<String>;
    fn action(&self) -> Option<&String> {
        self.activation_action.as_ref()
    }
    fn args(&self) -> Vec<&str> {
        self.open_files.iter().filter_map(|p| p.to_str()).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub title: String,
    pub ok: bool,
    pub steps: Vec<String>,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
    pub log: Option<PathBuf>,
}

pub struct RunningJob {
    pub job: Job,
    pub title: String,
    pub queue: Vec<Operation>,
    pub done: usize,
    pub total: usize,
}

pub struct App {
    core: cosmic::Core,
    flags: Flags,
    nav: nav_bar::Model,
    pub page: Page,
    paths: Paths,
    pub settings: Settings,
    pub inventory: Inventory,
    pub running: BTreeMap<String, Running>,
    pub loading: bool,
    pub query: String,
    pub selected: Option<String>,
    pub detail_open: bool,
    pub checks: BTreeMap<String, Check>,
    pub checking: BTreeSet<String>,
    pub checking_all: bool,
    pub checked_all_once: bool,
    pub requirements: BTreeMap<String, Result<Requirements, String>>,
    pub hashes: BTreeMap<PathBuf, Result<Hashes, String>>,
    // Edits for the selected app.
    pub edit_for: Option<String>,
    pub args_text: String,
    pub env_rows: Vec<(String, String)>,
    pub website_text: String,
    pub source_choice: usize,
    pub source_fields: BTreeMap<String, String>,
    pub prerelease: bool,
    pub icon_path: String,
    pub edit_error: Option<String>,
    // Add page.
    pub pending: Vec<Pending>,
    pub verified: bool,
    pub drag_over: bool,
    pub dialog: Option<Dialog>,
    pub job: Option<RunningJob>,
    pub outcome: Option<Outcome>,
    pub notice: Option<String>,
    toasts: widget::Toasts<Message>,
    window_state: WindowState,
    /// Current window width in any mode.
    width: f32,
    /// Width of the app list on wide windows; dragged with the divider.
    pub list_width: f32,
    /// While the divider is dragged: (pointer x, list width) at the start.
    split_drag: Option<(Split, Option<(f32, f32)>)>,
    /// Width of the navigation sidebar; dragged with its divider.
    pub nav_width: f32,
    window_request: u64,
    window_observed: u64,
    initial_window_mode: Option<window_state::Mode>,
    window_started: std::time::Instant,
    capture_ticks: u8,
    pub isolated: bool,
    pub ui_instance: String,
    synced_once: bool,
}

pub fn theme_of(theme: Theme) -> cosmic::Theme {
    if theme == Theme::System {
        return cosmic::theme::system_preference();
    }
    let dark = theme == Theme::Dark;
    let shared = if dark {
        cosmic::theme::system_dark()
    } else {
        cosmic::theme::system_light()
    };
    let selected = if shared.theme_type.is_dark() == dark {
        shared.cosmic().clone()
    } else {
        let builder = if dark {
            cosmic::cosmic_theme::ThemeBuilder::dark()
        } else {
            cosmic::cosmic_theme::ThemeBuilder::light()
        };
        builder
            .accent(*cosmic::theme::system_preference().cosmic().accent_color())
            .build()
    };
    cosmic::Theme::custom(Arc::new(selected))
}

fn background<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
    map: impl FnOnce(T) -> Message + Send + 'static,
) -> Task<Message> {
    cosmic::task::future(async move {
        match tokio::task::spawn_blocking(f).await {
            Ok(value) => map(value),
            Err(e) => {
                Message::Finished(Box::new(Err(format!("The background worker stopped: {e}"))))
            }
        }
    })
}

fn header_tooltip<'a>(
    button: impl Into<Element<'a, Message>>,
    label: &'a str,
) -> Element<'a, Message> {
    widget::tooltip(
        button,
        widget::text(label),
        widget::tooltip::Position::Bottom,
    )
    .gap(8)
    .into()
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    widget::container(content)
        .padding(14)
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .into()
}

fn single_line<'a>(text: widget::Text<'a, cosmic::Theme>) -> widget::Text<'a, cosmic::Theme> {
    text.width(Length::Fill)
        .wrapping(iced::advanced::text::Wrapping::None)
        .ellipsize(iced::advanced::text::Ellipsize::End(
            iced::advanced::text::EllipsizeHeightLimit::Lines(1),
        ))
}

fn wrap<'a>(text: impl Into<std::borrow::Cow<'a, str>> + 'a) -> widget::Text<'a, cosmic::Theme> {
    widget::text(text).wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
}

/// The app's icon: its file, a theme icon name, or a generic one.
pub fn app_icon<'a>(path: Option<&Path>, name: Option<&str>, size: u16) -> Element<'a, Message> {
    if let Some(p) = path.filter(|p| p.is_file()) {
        return widget::icon(icon::from_path(p.to_path_buf()))
            .size(size)
            .into();
    }
    if let Some(n) = name {
        return icon::from_name(n.to_string())
            .fallback(Some(icon::IconFallback::Names(vec![
                "application-x-executable".into(),
            ])))
            .size(size)
            .icon()
            .into();
    }
    icon::from_name("application-x-executable")
        .size(size)
        .icon()
        .into()
}

/// List rows: the selected one keeps the list-item highlight; hover adds a light tint.
fn row_style(selected: bool) -> cosmic::theme::Button {
    use cosmic::widget::button::Catalog;
    let item = cosmic::theme::Button::ListItem([10.; 4]);
    let tinted = move |focused, theme: &cosmic::Theme, alpha| {
        let mut style = theme.active(
            focused,
            selected,
            &cosmic::theme::Button::ListItem([10.; 4]),
        );
        let mut tint = iced::Color::from(theme.cosmic().background(theme.transparent).on);
        tint.a = alpha;
        style.overlay = Some(tint.into());
        style
    };
    let _ = item;
    cosmic::theme::Button::Custom {
        active: Box::new(move |focused, theme| {
            theme.active(
                focused,
                selected,
                &cosmic::theme::Button::ListItem([10.; 4]),
            )
        }),
        disabled: Box::new(|theme| theme.disabled(&cosmic::theme::Button::ListItem([10.; 4]))),
        hovered: Box::new(move |focused, theme| {
            tinted(focused, theme, if selected { 0.06 } else { 0.10 })
        }),
        pressed: Box::new(move |focused, theme| tinted(focused, theme, 0.16)),
    }
}

fn pill<'a>(text: String) -> Element<'a, Message> {
    widget::container(widget::text::caption(text))
        .padding([3, 10])
        .class(cosmic::theme::Container::Secondary)
        .into()
}

pub fn status_text(c: &Check) -> String {
    automation::status_label(c)
}

fn size_text(bytes: u64) -> String {
    let units = ["B", "kB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000. && i < units.len() - 1 {
        v /= 1000.;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

pub fn runtime_text(r: &Requirements) -> &'static str {
    match r.runtime {
        requirements::Runtime::Fuse2 => "FUSE 2 runtime (needs libfuse.so.2)",
        requirements::Runtime::StaticFuse3 => "Static runtime (FUSE 3 built in)",
        requirements::Runtime::Uruntime => "uruntime (static)",
        requirements::Runtime::Unknown => "Unknown runtime",
    }
}

pub fn dropped_paths(data: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(data)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| url::Url::parse(l).ok())
        .filter_map(|u| u.to_file_path().ok())
        .collect()
}

fn load(paths: Paths, settings: Settings) -> Loaded {
    let inventory = library::load(&paths, &settings);
    let running = library::running(&inventory.apps).unwrap_or_default();
    appimage::prune_cache(&paths);
    Loaded { inventory, running }
}

impl App {
    pub fn app(&self, id: &str) -> Option<&AppItem> {
        self.inventory.apps.iter().find(|a| a.id == id)
    }
    pub fn selected_app(&self) -> Option<&AppItem> {
        self.selected.as_deref().and_then(|id| self.app(id))
    }
    pub fn visible_apps(&self) -> Vec<&AppItem> {
        let q = self.query.to_lowercase();
        self.inventory
            .apps
            .iter()
            .filter(|a| {
                format!(
                    "{} {} {}",
                    a.name,
                    a.id,
                    a.comment.clone().unwrap_or_default()
                )
                .to_lowercase()
                .contains(&q)
            })
            .collect()
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    fn refresh(&mut self) -> Task<Message> {
        self.loading = true;
        let (paths, settings) = (self.paths.clone(), self.settings.clone());
        background(
            move || load(paths, settings),
            |l| Message::Loaded(Box::new(l)),
        )
    }
    fn toast(&mut self, text: impl Into<String>) -> Task<Message> {
        self.toasts
            .push(widget::toaster::Toast::new(text.into()))
            .map(cosmic::Action::App)
    }
    pub fn source_index(source: Option<&Source>) -> usize {
        source
            .map(|s| {
                MANAGERS
                    .iter()
                    .position(|m| m.name == s.manager().name)
                    .map_or(0, |i| i + 1)
            })
            .unwrap_or(0)
    }
    /// Load the edit fields from the selected app (once per selection).
    fn load_edits(&mut self) {
        let Some(app) = self.selected_app().cloned() else {
            return;
        };
        if self.edit_for.as_deref() == Some(app.id.as_str()) {
            return;
        }
        self.edit_for = Some(app.id.clone());
        self.args_text = gearlever_cosmic::desktop::join_command(&app.arguments);
        self.env_rows = app.env.clone();
        self.website_text = app.website.clone().unwrap_or_default();
        let custom = app
            .update_source
            .as_ref()
            .filter(|_| app.update_source_origin.is_some());
        self.source_choice = Self::source_index(custom);
        self.source_fields.clear();
        self.prerelease = false;
        if let Some(s) = custom
            && let Ok(serde_json::Value::Object(map)) = serde_json::to_value(s)
        {
            for (k, v) in map {
                match v {
                    serde_json::Value::String(t) if k != "manager" => {
                        self.source_fields.insert(k, t);
                    }
                    serde_json::Value::Bool(b) => self.prerelease = b,
                    _ => {}
                }
            }
        }
        self.icon_path.clear();
        self.edit_error = None;
    }
    fn select(&mut self, id: String) -> Task<Message> {
        self.selected = Some(id.clone());
        self.detail_open = true;
        self.edit_for = None;
        self.load_edits();
        let mut tasks = vec![];
        if let Some(app) = self.app(&id).cloned() {
            if !self.requirements.contains_key(&id) {
                let path = app.path.clone();
                let id2 = id.clone();
                tasks.push(background(
                    move || requirements::check(&path, &requirements::System::host()),
                    move |r| Message::Requirements(id2, Box::new(r)),
                ));
            }
            if !self.checks.contains_key(&id) && self.flags.capture.is_none() {
                tasks.push(self.check_app(id));
            }
        }
        Task::batch(tasks)
    }
    fn check_app(&mut self, id: String) -> Task<Message> {
        let Some(app) = self.app(&id).cloned() else {
            return Task::none();
        };
        self.checking.insert(id.clone());
        let paths = self.paths.clone();
        background(
            move || automation::check_app(&paths, &app),
            move |c| Message::Checked(id, Box::new(c)),
        )
    }
    fn check_all(&mut self) -> Task<Message> {
        if self.checking_all {
            return Task::none();
        }
        self.checking_all = true;
        self.checked_all_once = true;
        let apps = self.inventory.apps.clone();
        let paths = self.paths.clone();
        background(
            move || {
                std::thread::scope(|s| {
                    let handles: Vec<_> = apps
                        .iter()
                        .map(|a| (a.id.clone(), s.spawn(|| automation::check_app(&paths, a))))
                        .collect();
                    handles
                        .into_iter()
                        .filter_map(|(id, h)| h.join().ok().map(|c| (id, c)))
                        .collect()
                })
            },
            Message::CheckedAll,
        )
    }
    fn switch(&mut self, page: Page) -> Task<Message> {
        self.page = page;
        if let Some(pos) = Page::ALL.iter().position(|p| *p == page) {
            self.nav.activate_position(pos as u16);
        }
        if page == Page::Updates
            && !self.checked_all_once
            && !self.loading
            && self.flags.capture.is_none()
        {
            return self.check_all();
        }
        Task::none()
    }
    fn run_ops(&mut self, ops: Vec<Operation>) -> Task<Message> {
        if ops.is_empty() || self.job.is_some() {
            return Task::none();
        }
        let total = ops.len();
        let title = if total == 1 {
            ops[0].title()
        } else {
            format!("{} ({total} steps)", ops[0].title())
        };
        self.outcome = None;
        self.job = Some(RunningJob {
            job: Job::default(),
            title,
            queue: ops,
            done: 0,
            total,
        });
        self.next_op()
    }
    fn next_op(&mut self) -> Task<Message> {
        let Some(running) = &mut self.job else {
            return Task::none();
        };
        if running.queue.is_empty() {
            self.job = None;
            return self.refresh();
        }
        let op = running.queue.remove(0);
        running.title = op.title();
        let job = running.job.clone();
        let (paths, settings) = (self.paths.clone(), self.settings.clone());
        background(
            move || ops::execute(op, &paths, &settings, &job),
            |r| Message::Finished(Box::new(r)),
        )
    }
    fn plan(&self, op: &Operation) -> Vec<String> {
        op.plan(&self.paths, &self.settings)
            .unwrap_or_else(|e| vec![e])
    }
    fn change_setting(&mut self, change: SettingChange) -> Task<Message> {
        let mut s = self.settings.clone();
        match change {
            SettingChange::MoveOnIntegrate(v) => s.move_on_integrate = v,
            SettingChange::ShowExternal(v) => s.show_external = v,
            SettingChange::TerminalNames(v) => s.terminal_apps_use_executable_name = v,
            SettingChange::SafeMetadata(v) => s.safe_metadata = v,
            SettingChange::CheckOnStartup(v) => s.check_updates_on_startup = v,
            SettingChange::PreviewMinimal(v) => s.preview_minimal = v,
            SettingChange::DebugLogs(v) => s.debug_logs = v,
            SettingChange::KeepReplaced(v) => s.keep_replaced_versions_in_trash = v,
            SettingChange::SyncMenus(v) => s.sync_menu_entries = v,
            SettingChange::Theme(t) => s.theme = t,
        }
        self.save_settings(s)
    }
    fn save_settings(&mut self, s: Settings) -> Task<Message> {
        if s == self.settings {
            return Task::none();
        }
        let theme_changed = s.theme != self.settings.theme;
        let refresh = s.show_external != self.settings.show_external
            || s.appimages_folder != self.settings.appimages_folder
            || s.safe_metadata != self.settings.safe_metadata;
        match ops::execute(
            Operation::Settings(Box::new(s.clone())),
            &self.paths,
            &self.settings,
            &Job::default(),
        ) {
            Ok(r) if r.errors.is_empty() => self.settings = s,
            Ok(r) => self.notice = Some(r.errors.join("\n")),
            Err(e) => self.notice = Some(e),
        }
        let mut tasks = vec![];
        if theme_changed {
            tasks.push(cosmic::command::set_theme(theme_of(self.settings.theme)));
        }
        if refresh {
            tasks.push(self.refresh());
        }
        Task::batch(tasks)
    }
    pub fn open_files(&mut self, files: Vec<PathBuf>) -> Task<Message> {
        let mut tasks = vec![self.switch(Page::Add)];
        let mut refused = vec![];
        for file in files {
            if self.pending.iter().any(|p| p.path == file) {
                continue;
            }
            if !appimage::is_appimage(&file) {
                refused.push(
                    file.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
                continue;
            }
            self.pending.push(Pending {
                path: file.clone(),
                meta: None,
                requirements: None,
                conflict: Conflict::Ask,
                same_file: None,
                others: vec![],
                hashes: None,
            });
            let (paths, settings, inv) = (
                self.paths.clone(),
                self.settings.clone(),
                self.inventory.clone(),
            );
            tasks.push(background(
                move || {
                    let meta = appimage::inspect_cached(&paths, &file, !settings.safe_metadata);
                    let requirements =
                        requirements::check(&file, &requirements::System::host()).ok();
                    let same_file = ops::same_file(&inv, &file).map(|a| a.id);
                    let others = meta
                        .as_ref()
                        .map(|m| {
                            ops::same_app(&inv, m)
                                .into_iter()
                                .map(|a| (a.id, a.version))
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        file,
                        Pending {
                            path: PathBuf::new(),
                            meta: Some(meta),
                            requirements,
                            conflict: Conflict::Ask,
                            same_file,
                            others,
                            hashes: None,
                        },
                    )
                },
                |(f, p)| Message::Inspected(f, Box::new(p)),
            ));
        }
        if !refused.is_empty() {
            self.notice = Some(format!(
                "Not an AppImage, so not added: {}. Gear Lever COSMIC checks the file's contents, not its name.",
                refused.join(", ")
            ));
        }
        Task::batch(tasks)
    }
    fn integrate_ops(&self, indices: &[usize]) -> Result<Vec<Operation>, String> {
        let mut out = vec![];
        for &i in indices {
            let p = self
                .pending
                .get(i)
                .ok_or("That file is no longer in the list")?;
            if let Some(id) = &p.same_file {
                return Err(format!(
                    "{} is already integrated as {id}",
                    p.path.display()
                ));
            }
            let conflict = if p.others.is_empty() {
                Conflict::Ask
            } else {
                p.conflict
            };
            if conflict == Conflict::Ask && !p.others.is_empty() {
                return Err(format!(
                    "Choose Keep Both or Replace for {}",
                    p.path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            let replace = (conflict == Conflict::Replace)
                .then(|| p.others.first().and_then(|(id, _)| self.app(id).cloned()))
                .flatten();
            out.push(Operation::Integrate {
                file: p.path.clone(),
                conflict,
                replace,
                source: None,
            });
        }
        Ok(out)
    }
    fn save_window(&self) {
        if self.flags.capture.is_some() || self.isolated {
            return;
        }
        if let Err(e) = window_state::save(&self.window_state) {
            eprintln!("Cannot save window state: {e}");
        }
    }
    fn present_window(&self) -> Task<Message> {
        let Some(window) = self.core.main_window_id() else {
            return Task::none();
        };
        iced::platform_specific::shell::commands::activation::request_token(
            Some(Self::APP_ID.into()),
            Some(window),
        )
        .then(move |token| match token {
            Some(token) => {
                iced::platform_specific::shell::commands::activation::activate(window, token)
            }
            None => iced::window::request_user_attention(
                window,
                Some(iced::window::UserAttention::Informational),
            ),
        })
    }
    fn source_from_fields(&self) -> Result<Option<Source>, String> {
        if self.source_choice == 0 {
            return Ok(None);
        }
        let m = &MANAGERS[self.source_choice - 1];
        let mut pairs: Vec<(String, String)> = m
            .fields
            .iter()
            .filter(|(f, _)| *f != "allow_prereleases")
            .map(|(f, _)| {
                (
                    f.to_string(),
                    self.source_fields.get(*f).cloned().unwrap_or_default(),
                )
            })
            .collect();
        if m.fields.iter().any(|(f, _)| *f == "allow_prereleases") {
            pairs.push(("allow_prereleases".into(), self.prerelease.to_string()));
        }
        Source::from_pairs(m.name, &pairs).map(Some)
    }

    // ---------- views ----------

    fn empty<'a>(&self, title: &'a str, body: &'a str, icon_name: &'a str) -> Element<'a, Message> {
        widget::container(
            widget::column::with_capacity(3)
                .spacing(10)
                .align_x(Alignment::Center)
                .push(icon::from_name(icon_name).size(64).icon())
                .push(widget::text::title3(title))
                .push(widget::text(body).align_x(iced::alignment::Horizontal::Center)),
        )
        .center_x(Length::Fill)
        .padding([48, 16])
        .into()
    }

    fn app_row<'a>(&'a self, a: &'a AppItem) -> Element<'a, Message> {
        let mut badges = widget::row::with_capacity(4).spacing(6);
        if let Some(c) = self.checks.get(&a.id) {
            match c.status {
                Status::Available => badges = badges.push(pill("Update".into())),
                Status::SelfUpdating if c.update_known == Some(true) => {
                    badges = badges.push(pill("Update in app".into()))
                }
                _ => {}
            }
        }
        if self.running.contains_key(&a.id) {
            badges = badges.push(pill("Running".into()));
        }
        if !a.menu_outdated.is_empty() {
            badges = badges.push(pill("Menu outdated".into()));
        }
        if let Some(Ok(r)) = self.requirements.get(&a.id)
            && r.verdict == Verdict::Missing
        {
            badges = badges.push(pill("May not start".into()));
        }
        let selected = self.selected.as_deref() == Some(a.id.as_str());
        widget::button::custom(
            widget::row::with_capacity(3)
                .spacing(12)
                .align_y(Alignment::Center)
                .push(app_icon(a.icon.as_deref(), a.icon_name.as_deref(), 40))
                .push(
                    widget::column::with_capacity(3)
                        .spacing(2)
                        .width(Length::Fill)
                        .push(single_line(widget::text::heading(a.name.as_str())))
                        .push(single_line(widget::text::caption(match &a.comment {
                            Some(c) => format!("{} · {c}", a.display_version()),
                            None => a.display_version(),
                        })))
                        .push(badges),
                ),
        )
        .class(row_style(selected))
        .padding([8, 12])
        .width(Length::Fill)
        .on_press(Message::Select(a.id.clone()))
        .into()
    }

    fn apps_list(&self) -> Element<'_, Message> {
        let apps = self.visible_apps();
        let mut col = widget::column::with_capacity(apps.len() + 2).spacing(4);
        col = col.push(
            widget::search_input("Filter installed apps", &self.query)
                .id(widget::Id::new("search"))
                .on_input(|v| Message::Field(Field::Search, v))
                .on_clear(Message::Field(Field::Search, String::new())),
        );
        if self.inventory.apps.is_empty() {
            return col
                .push(self.empty(
                    "No AppImages yet",
                    "Open or drop an AppImage to add it to your app menu. Apps integrated with Gear Lever appear here too.",
                    "application-x-executable",
                ))
                .push(widget::container(widget::button::suggested("Open AppImage…").id(widget::Id::new("header.open")).on_press(Message::OpenFiles)).center_x(Length::Fill))
                .into();
        }
        if apps.is_empty() {
            col = col.push(widget::text::body("No app matches the filter."));
        }
        for a in apps {
            col = col.push(self.app_row(a));
        }
        col = col.push(widget::text::caption(format!(
            "{} apps · menu entries in {} · AppImages folder {}",
            self.inventory.apps.len(),
            self.paths.short(&self.inventory.applications_dir),
            self.paths.short(&self.inventory.folder)
        )));
        widget::scrollable(col)
            .spacing(8)
            .height(Length::Fill)
            .into()
    }

    /// Width available to the page, from the window size. (A `responsive`
    /// wrapper kept stale content after the app list changed and libcosmic
    /// panicked with "Downcast on stateless state".)
    fn content_width(&self) -> f32 {
        let nav = if self.core.nav_bar_active() && !self.core.is_condensed() {
            self.clamp_nav_width(self.nav_width) + 20.
        } else {
            0.
        };
        self.width - nav - 40.
    }
    /// The list may take what is left after a 320 px details pane.
    pub fn clamp_list_width(&self, w: f32) -> f32 {
        let max = (self.content_width() - 20. - 9. - 320.).max(260.);
        w.clamp(260., max)
    }
    /// The sidebar: NAV_MIN to 400 px, leaving the page at least 400 px.
    pub fn clamp_nav_width(&self, w: f32) -> f32 {
        let max = (self.width - 400.).clamp(NAV_MIN, 400.);
        w.clamp(NAV_MIN, max)
    }
    /// A divider line; drag it to resize the pane on its left.
    fn split_handle(&self, split: Split) -> Element<'_, Message> {
        widget::mouse_area(
            widget::container(widget::divider::vertical::default())
                .width(Length::Fixed(9.))
                .height(Length::Fill)
                .center_x(Length::Fixed(9.)),
        )
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .on_press(Message::SplitStart(split))
        .into()
    }
    fn apps_view(&self) -> Element<'_, Message> {
        {
            let wide = self.content_width() >= MIN_SPLIT;
            if wide {
                let detail: Element<'_, Message> = match self.selected_app() {
                    Some(a) => self.detail(a, false),
                    None => self.empty(
                        "Select an app",
                        "Its details, launch options and update source appear here.",
                        "view-list-symbolic",
                    ),
                };
                widget::row::with_capacity(3)
                    .spacing(6)
                    .push(
                        widget::container(self.apps_list())
                            .width(Length::Fixed(self.clamp_list_width(self.list_width))),
                    )
                    .push(self.split_handle(Split::List))
                    .push(detail)
                    .into()
            } else if self.detail_open
                && let Some(a) = self.selected_app()
            {
                self.detail(a, true)
            } else {
                self.apps_list()
            }
        }
    }

    fn banner<'a>(
        &self,
        icon_name: &'a str,
        text: String,
        action: Option<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        // The action sits under the text so the text keeps the full width
        // in the narrow details pane.
        let mut text_col = widget::column::with_capacity(2)
            .spacing(10)
            .width(Length::Fill)
            .push(wrap(text));
        if let Some(a) = action {
            text_col = text_col.push(a);
        }
        panel(
            widget::row::with_capacity(2)
                .spacing(12)
                .align_y(Alignment::Start)
                .push(icon::from_name(icon_name).size(20).icon())
                .push(text_col),
        )
    }

    fn update_section<'a>(&'a self, a: &'a AppItem) -> Element<'a, Message> {
        let busy = self.busy();
        let mut col = widget::column::with_capacity(8).spacing(10);
        let status: Element<'_, Message> = if self.checking.contains(&a.id) {
            widget::text("Checking for updates…").into()
        } else if let Some(c) = self.checks.get(&a.id) {
            let mut lines = widget::column::with_capacity(4)
                .spacing(4)
                .push(widget::text::heading(status_text(c)))
                .push(wrap(c.message.clone()));
            let mut facts = vec![];
            if let Some(s) = &c.source {
                facts.push(format!(
                    "Source: {s}{}",
                    c.source_detail
                        .as_ref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                ));
            }
            if let Some(o) = c.origin {
                facts.push(
                    match o {
                        updates::Origin::Custom => "set in this app",
                        updates::Origin::GearLever => "from Gear Lever's settings",
                        updates::Origin::Embedded => "embedded by the developer",
                        updates::Origin::SelfUpdater => "the app's own updater",
                    }
                    .into(),
                );
            }
            if let Some(by) = &c.compared_by {
                facts.push(format!("compared by {by}"));
            }
            if let Some(n) = &c.self_updater {
                facts.push(format!("the app also updates itself ({n})"));
            }
            if !facts.is_empty() {
                lines = lines.push(widget::text::caption(facts.join(" · ")));
            }
            lines.into()
        } else {
            widget::text("Not checked yet.").into()
        };
        let mut actions = widget::row::with_capacity(2).spacing(8);
        if let Some(c) = self.checks.get(&a.id)
            && c.status == Status::Available
            && c.can_install
        {
            actions = actions.push(
                widget::button::suggested(format!(
                    "Update{}",
                    c.available_version
                        .as_ref()
                        .map(|v| format!(" to {v}"))
                        .unwrap_or_default()
                ))
                .id(widget::Id::new("app.update"))
                .on_press_maybe((!busy).then(|| Message::UpdateApp(a.id.clone()))),
            );
        }
        actions = actions.push(
            widget::button::standard("Check Now")
                .id(widget::Id::new("app.check"))
                .on_press_maybe(
                    (!self.checking.contains(&a.id)).then(|| Message::CheckApp(a.id.clone())),
                ),
        );
        col = col.push(status).push(actions);
        // The source editor.
        let mut choices = widget::list_column().list_item_padding([6, 12]);
        let default_label = if a.update_info.is_some() {
            "Default: embedded update information"
        } else if a.self_updater.is_some() {
            "Default: the app's own updater"
        } else if a.update_source_origin == Some(updates::Origin::GearLever) {
            "Default: Gear Lever's saved source"
        } else {
            "Default: none"
        };
        choices = choices.add(widget::settings::item::builder(default_label).radio(
            0,
            Some(self.source_choice),
            Message::SourceChoice,
        ));
        for (i, m) in MANAGERS.iter().enumerate() {
            choices = choices.add(widget::settings::item::builder(m.label).radio(
                i + 1,
                Some(self.source_choice),
                Message::SourceChoice,
            ));
        }
        col = col
            .push(widget::text::heading("Update source"))
            .push(choices);
        if self.source_choice > 0 {
            let m = &MANAGERS[self.source_choice - 1];
            for (f, d) in m.fields {
                if *f == "allow_prereleases" {
                    col = col.push(
                        widget::list_column().add(
                            widget::settings::item::builder("Allow pre-releases")
                                .toggler(self.prerelease, Message::Prerelease),
                        ),
                    );
                } else {
                    col = col.push(
                        widget::text_input(
                            *d,
                            self.source_fields.get(*f).map(String::as_str).unwrap_or(""),
                        )
                        .label(*f)
                        .id(widget::Id::new(format!("app.source.{f}")))
                        .on_input(move |v| Message::Field(Field::Source(f.to_string()), v)),
                    );
                }
            }
        }
        if let Some(e) = &self.edit_error {
            col = col.push(wrap(e.clone()));
        }
        col = col.push(
            widget::button::standard("Save Source")
                .id(widget::Id::new("app.source.save"))
                .on_press_maybe((!busy).then_some(Message::SaveSource)),
        );
        panel(col)
    }

    fn detail<'a>(&'a self, a: &'a AppItem, narrow: bool) -> Element<'a, Message> {
        let busy = self.busy();
        let mut col = widget::column::with_capacity(16).spacing(16).max_width(720);
        if narrow {
            col = col.push(
                widget::button::text("Back")
                    .leading_icon(icon::from_name("go-previous-symbolic"))
                    .id(widget::Id::new("apps.back"))
                    .on_press(Message::Back),
            );
        }
        // Header: icon, name, pills, actions.
        let mut pills = widget::row::with_capacity(4)
            .spacing(6)
            .push(pill(a.display_version()))
            .push(pill(size_text(a.size)));
        pills = pills
            .push(pill(format!("Type {}", a.appimage_type)))
            .push(pill(a.arch.clone()));
        let running = self.running.get(&a.id);
        let launch_label = if a.terminal {
            "Launch in Terminal"
        } else {
            "Launch"
        };
        let header = widget::column::with_capacity(5)
            .spacing(8)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .push(app_icon(a.icon.as_deref(), a.icon_name.as_deref(), 96))
            .push(widget::text::title2(a.name.as_str()))
            .push_maybe(a.comment.as_ref().map(|c| widget::text::body(c.as_str())))
            .push(pills)
            .push(
                widget::row::with_capacity(2)
                    .spacing(10)
                    .push(
                        widget::button::standard(launch_label)
                            .id(widget::Id::new("app.launch"))
                            .on_press_maybe((!busy).then(|| Message::Launch(a.id.clone()))),
                    )
                    .push(
                        widget::button::destructive("Remove")
                            .id(widget::Id::new("app.remove"))
                            .on_press_maybe(
                                (!busy).then(|| Message::AskRemove(vec![a.id.clone()])),
                            ),
                    ),
            );
        col = col.push(header);
        // Banners: problems first.
        if let Some(Ok(r)) = self.requirements.get(&a.id)
            && r.verdict == Verdict::Missing
        {
            col = col.push(self.banner(
                "dialog-warning-symbolic",
                format!("This app may not start here. {}", r.problems.join(" ")),
                None,
            ));
        }
        if let Some(r) = running {
            col = col.push(self.banner(
                "media-playback-start-symbolic",
                if r.old_version {
                    format!("{} is running a version that has since been replaced. Restart it to use the new version.", a.name)
                } else {
                    format!("{} is running. Updates still install now; they take effect the next time it starts.", a.name)
                },
                None,
            ));
        }
        if !a.menu_outdated.is_empty() {
            let text = if a.menu_outdated.iter().any(|o| o == "version") {
                format!(
                    "The app menu shows version {}, but the file is {}. The AppImage was replaced or updated itself.",
                    a.menu_version.as_deref().unwrap_or("none"),
                    a.display_version()
                )
            } else {
                "The AppImage changed since its menu entry was written (it was replaced or updated itself).".into()
            };
            col = col.push(
                self.banner(
                    "view-refresh-symbolic",
                    text,
                    Some(
                        widget::button::standard("Refresh Menu Entry")
                            .id(widget::Id::new("app.sync-menu"))
                            .on_press_maybe((!busy).then(|| Message::SyncMenu(vec![a.id.clone()])))
                            .into(),
                    ),
                ),
            );
        }
        if a.arch != gearlever_cosmic::elf::host_arch() && a.arch != "unknown" {
            col = col.push(self.banner(
                "dialog-warning-symbolic",
                format!(
                    "Built for {}, but this computer is {}.",
                    a.arch,
                    gearlever_cosmic::elf::host_arch()
                ),
                None,
            ));
        }
        if a.external {
            col = col.push(
                self.banner(
                    "folder-symbolic",
                    "This app is outside the AppImages folder. You can hide such apps in Settings."
                        .into(),
                    None,
                ),
            );
        }
        if a.managed_by == ManagedBy::Other {
            col = col.push(self.banner("dialog-information-symbolic", "This menu entry was made by the app itself or another tool. Refreshing it makes Gear Lever COSMIC manage it.".into(), None));
        }
        if let Some(e) = &a.metadata_error {
            col = col.push(self.banner("dialog-information-symbolic", e.clone(), None));
        }
        // Updates.
        col = col
            .push(widget::text::title4("Updates"))
            .push(self.update_section(a));
        // Details.
        let mut info = widget::list_column().list_item_padding([8, 12]);
        info = info.add(
            widget::settings::item::builder("Location")
                .description(self.paths.short(&a.path))
                .control(
                    widget::row::with_capacity(2)
                        .spacing(4)
                        .push(
                            widget::button::icon(icon::from_name("edit-copy-symbolic"))
                                .tooltip("Copy path")
                                .id(widget::Id::new("app.copy-path"))
                                .on_press(Message::Copy(a.path.display().to_string())),
                        )
                        .push(
                            widget::button::icon(icon::from_name("folder-open-symbolic"))
                                .tooltip("Open folder")
                                .id(widget::Id::new("app.open-folder"))
                                .on_press(Message::OpenFolder(a.id.clone())),
                        ),
                ),
        );
        info = info.add(
            widget::settings::item::builder("Menu entry")
                .description(self.paths.short(&a.desktop_file))
                .control(widget::text::caption(match a.managed_by {
                    ManagedBy::GearLeverCosmic => "Gear Lever COSMIC",
                    ManagedBy::GearLever => "Gear Lever",
                    ManagedBy::Other => "Made by the app",
                })),
        );
        for o in &a.other_desktop_files {
            info = info.add(
                widget::settings::item::builder("Another menu entry starts this file")
                    .description(self.paths.short(o))
                    .control(widget::text::caption("Left as is")),
            );
        }
        info = info.add(
            widget::settings::item::builder("Version")
                .description(match (&a.version, &a.menu_version) {
                    (Some(v), Some(m)) if v != m => format!("{v} (file) · {m} (menu entry)"),
                    (Some(v), _) => format!("{v}, read from the AppImage"),
                    (None, _) => "The AppImage does not state its version".into(),
                })
                .control(widget::text::caption("")),
        );
        let req = match self.requirements.get(&a.id) {
            None => "Checking…".to_string(),
            Some(Err(e)) => format!("Not checked: {e}"),
            Some(Ok(r)) => {
                let libs = if r.program.is_some() {
                    if r.missing_libraries.is_empty() {
                        format!("all {} libraries found", r.needed_libraries.len())
                    } else {
                        format!("missing {}", r.missing_libraries.join(", "))
                    }
                } else {
                    "libraries not checked".into()
                };
                let fuse = match r.runtime {
                    requirements::Runtime::Fuse2 => {
                        if r.fuse2_library.is_some() {
                            "libfuse.so.2 installed"
                        } else {
                            "libfuse.so.2 missing"
                        }
                    }
                    _ => {
                        if r.fusermount3.is_some() || r.fusermount.is_some() {
                            "fusermount found"
                        } else {
                            "no fusermount"
                        }
                    }
                };
                format!("{} · {fuse} · {libs}", runtime_text(r))
            }
        };
        info = info.add(
            widget::settings::item::builder("System requirements")
                .description(req)
                .control(widget::text::caption(match self.requirements.get(&a.id) {
                    Some(Ok(r)) if r.verdict == Verdict::Ok => "OK",
                    Some(Ok(r)) if r.verdict == Verdict::Missing => "Missing",
                    _ => "",
                })),
        );
        let hashes: Element<'_, Message> = match self.hashes.get(&a.path) {
            Some(Ok(h)) => widget::text::monotext(format!(
                "md5 {}\nsha1 {}\nsha256 {}",
                h.md5, h.sha1, h.sha256
            ))
            .into(),
            Some(Err(e)) => widget::text::caption(e.clone()).into(),
            None => widget::button::text("Show")
                .id(widget::Id::new("app.hashes"))
                .on_press(Message::ShowHashes(a.path.clone()))
                .into(),
        };
        info = info.add(widget::settings::item::builder("Checksums").control(hashes));
        col = col.push(widget::text::title4("Details")).push(info);
        // Icon.
        let mut icon_col = widget::column::with_capacity(4).spacing(8).push(wrap(if a.icon_custom {
            "Your own icon is used. Updates and menu refreshes keep it."
        } else {
            "The icon shipped in the AppImage is used. Choose your own if you prefer; updates keep it."
        }));
        icon_col = icon_col.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .push(
                    widget::button::standard("Choose Icon…")
                        .id(widget::Id::new("app.icon.choose"))
                        .on_press_maybe((!busy).then_some(Message::ChooseIcon)),
                )
                .push_maybe(a.icon_custom.then(|| {
                    widget::button::standard("Use Official Icon")
                        .id(widget::Id::new("app.icon.reset"))
                        .on_press_maybe((!busy).then_some(Message::ResetIcon))
                })),
        );
        col = col.push(widget::text::title4("Icon")).push(panel(icon_col));
        // Launch options.
        let mut launch = widget::column::with_capacity(8).spacing(8);
        launch = launch.push(
            widget::text_input(
                "Arguments, e.g. --ozone-platform=wayland %U",
                &self.args_text,
            )
            .label("Command line arguments")
            .id(widget::Id::new("app.args"))
            .on_input(|v| Message::Field(Field::Args, v))
            .on_submit(|_| Message::SaveArgs),
        );
        launch = launch.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .push(
                    widget::button::standard("Save Arguments")
                        .id(widget::Id::new("app.args.save"))
                        .on_press_maybe(
                            (!busy
                                && self.args_text
                                    != gearlever_cosmic::desktop::join_command(&a.arguments))
                            .then_some(Message::SaveArgs),
                        ),
                )
                .push_maybe(
                    (a.arguments != a.default_arguments && !a.default_arguments.is_empty()).then(
                        || {
                            widget::button::text("Use App's Defaults")
                                .id(widget::Id::new("app.args.reset"))
                                .on_press(Message::ResetArgs)
                        },
                    ),
                ),
        );
        launch = launch.push(widget::text::heading("Environment variables"));
        for (i, (k, v)) in self.env_rows.iter().enumerate() {
            launch = launch.push(
                widget::row::with_capacity(4)
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(
                        widget::text_input("NAME", k.as_str())
                            .on_input(move |t| Message::Field(Field::EnvKey(i), t))
                            .width(Length::FillPortion(2)),
                    )
                    .push(widget::text("="))
                    .push(
                        widget::text_input("value", v.as_str())
                            .on_input(move |t| Message::Field(Field::EnvValue(i), t))
                            .width(Length::FillPortion(3)),
                    )
                    .push(
                        widget::button::icon(icon::from_name("list-remove-symbolic"))
                            .tooltip("Remove")
                            .on_press(Message::RemoveEnv(i)),
                    ),
            );
        }
        launch = launch.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .push(
                    widget::button::standard("Add Variable")
                        .leading_icon(icon::from_name("list-add-symbolic"))
                        .id(widget::Id::new("app.env.add"))
                        .on_press(Message::AddEnv),
                )
                .push(
                    widget::button::standard("Save Environment")
                        .id(widget::Id::new("app.env.save"))
                        .on_press_maybe(
                            (!busy && self.env_rows != a.env).then_some(Message::SaveEnv),
                        ),
                ),
        );
        col = col
            .push(widget::text::title4("Launch options"))
            .push(panel(launch));
        // Website.
        let site = widget::row::with_capacity(3)
            .spacing(8)
            .align_y(Alignment::End)
            .push(
                widget::text_input("https://…", &self.website_text)
                    .label("Website")
                    .id(widget::Id::new("app.website"))
                    .on_input(|v| Message::Field(Field::Website, v)),
            )
            .push(
                widget::button::standard("Save")
                    .id(widget::Id::new("app.website.save"))
                    .on_press_maybe(
                        (!busy && Some(self.website_text.as_str()) != a.website.as_deref())
                            .then_some(Message::SaveWebsite),
                    ),
            )
            .push_maybe(a.website.as_ref().map(|w| {
                widget::button::icon(icon::from_name("web-browser-symbolic"))
                    .tooltip("Open website")
                    .id(widget::Id::new("app.website.open"))
                    .on_press(Message::OpenWebsite(w.clone()))
            }));
        col = col.push(panel(site));
        col = col.push(panel(
            widget::row::with_capacity(2)
                .spacing(12)
                .align_y(Alignment::Center)
                .push(widget::column::with_capacity(2).width(Length::Fill).push(widget::text::heading("Reload metadata")).push(wrap("Rewrite the menu entry's name, version and icon from the AppImage. Useful after the app updated itself. Your arguments, environment and custom icon stay.")))
                .push(widget::button::standard("Reload").id(widget::Id::new("app.reload")).on_press_maybe((!busy).then(|| Message::SyncMenu(vec![a.id.clone()])))),
        ));
        widget::scrollable(widget::container(col).center_x(Length::Fill))
            .spacing(8)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }

    fn updates_view(&self) -> Element<'_, Message> {
        let busy = self.busy();
        let installable: Vec<&AppItem> = self
            .inventory
            .apps
            .iter()
            .filter(|a| {
                self.checks
                    .get(&a.id)
                    .is_some_and(|c| c.status == Status::Available && c.can_install)
            })
            .collect();
        let mut col = widget::column::with_capacity(8).spacing(14).max_width(860);
        col = col.push(
            widget::row::with_capacity(3)
                .spacing(10)
                .align_y(Alignment::Center)
                .push(
                    widget::column::with_capacity(2).width(Length::Fill).push(widget::text::title3(if self.checking_all {
                        "Checking for updates…".to_string()
                    } else if !self.checked_all_once {
                        "Not checked yet".to_string()
                    } else if installable.is_empty() {
                        "No installable updates found".to_string()
                    } else {
                        format!("{} updates ready", installable.len())
                    }))
                    .push(widget::text::caption("Running apps are updated in place; they switch to the new version the next time they start.")),
                )
                .push(widget::button::standard("Check for Updates").id(widget::Id::new("updates.check")).on_press_maybe((!self.checking_all).then_some(Message::CheckAll)))
                .push(widget::button::suggested(format!("Update All ({})", installable.len())).id(widget::Id::new("updates.all")).on_press_maybe((!busy && !installable.is_empty()).then_some(Message::UpdateAll))),
        );
        type Group = (&'static str, &'static str, fn(&Check) -> bool);
        let groups: [Group; 6] = [
            (
                "Ready to install",
                "Downloaded and verified here; the old file goes to the Trash.",
                |c| c.status == Status::Available && c.can_install,
            ),
            (
                "Updates in the app itself",
                "These apps have their own updater. Open them to install the update.",
                |c| {
                    (c.status == Status::SelfUpdating && c.update_known == Some(true))
                        || (c.status == Status::Available && !c.can_install)
                },
            ),
            (
                "Apps that update themselves",
                "Their updates could not be checked from here. Open the app to check.",
                |c| c.status == Status::SelfUpdating && c.update_known != Some(true),
            ),
            (
                "Check failed",
                "Nothing is known about these updates until a check succeeds.",
                |c| c.status == Status::Failed || c.status == Status::NotFound,
            ),
            (
                "No update source",
                "Add a source on the app's page to check these apps.",
                |c| c.status == Status::NoSource,
            ),
            ("Up to date", "Compared with their sources.", |c| {
                c.status == Status::UpToDate
            }),
        ];
        for (title, description, test) in groups {
            let apps: Vec<(&AppItem, &Check)> = self
                .inventory
                .apps
                .iter()
                .filter_map(|a| self.checks.get(&a.id).filter(|c| test(c)).map(|c| (a, c)))
                .collect();
            if apps.is_empty() {
                continue;
            }
            let mut list = widget::list_column().list_item_padding([8, 12]);
            for (a, c) in apps {
                let mut row = widget::row::with_capacity(4)
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(app_icon(a.icon.as_deref(), a.icon_name.as_deref(), 32))
                    .push(
                        widget::column::with_capacity(2)
                            .width(Length::Fill)
                            .push(widget::text::heading(format!(
                                "{}  {}",
                                a.name,
                                match (&a.version, &c.available_version) {
                                    (Some(v), Some(n)) if c.status == Status::Available =>
                                        format!("{v} → {n}"),
                                    (Some(v), _) => v.clone(),
                                    _ => String::new(),
                                }
                            )))
                            .push(wrap(c.message.clone()).size(12)),
                    );
                if c.status == Status::Available && c.can_install {
                    row = row.push(
                        widget::button::suggested("Update")
                            .on_press_maybe((!busy).then(|| Message::UpdateApp(a.id.clone()))),
                    );
                }
                if c.status == Status::SelfUpdating
                    || (c.status == Status::Available && !c.can_install)
                {
                    row = row.push(
                        widget::button::standard("Open App")
                            .on_press_maybe((!busy).then(|| Message::Launch(a.id.clone()))),
                    );
                }
                row = row.push(
                    widget::button::icon(icon::from_name("go-next-symbolic"))
                        .tooltip("Details")
                        .on_press(Message::Select(a.id.clone())),
                );
                list = list.add(row);
            }
            col = col
                .push(
                    widget::column::with_capacity(2)
                        .spacing(6)
                        .push(widget::text::heading(title))
                        .push(widget::text::caption(description)),
                )
                .push(list);
        }
        if self.inventory.apps.is_empty() {
            col = col.push(self.empty(
                "No AppImages",
                "Integrated apps appear here with their update status.",
                "software-update-available-symbolic",
            ));
        }
        widget::scrollable(widget::container(col).center_x(Length::Fill))
            .spacing(8)
            .height(Length::Fill)
            .into()
    }

    fn pending_card<'a>(&'a self, i: usize, p: &'a Pending) -> Element<'a, Message> {
        let busy = self.busy();
        let file = p
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut col = widget::column::with_capacity(8).spacing(8);
        let meta = match &p.meta {
            None => return panel(widget::text(format!("Reading {file}…"))),
            Some(Err(e)) => {
                return panel(
                    widget::column::with_capacity(2)
                        .push(widget::text::heading(file))
                        .push(wrap(e.clone())),
                );
            }
            Some(Ok(m)) => m,
        };
        col = col.push(
            widget::row::with_capacity(2)
                .spacing(12)
                .align_y(Alignment::Center)
                .push(app_icon(meta.icon.as_deref(), None, 56))
                .push(
                    widget::column::with_capacity(3)
                        .width(Length::Fill)
                        .push(widget::text::title4(
                            meta.name.clone().unwrap_or_else(|| file.clone()),
                        ))
                        .push_maybe(
                            meta.comment
                                .as_ref()
                                .map(|c| widget::text::body(c.as_str())),
                        )
                        .push(widget::text::caption(format!(
                            "{} · {} · Type {} · {} · {}",
                            meta.version
                                .clone()
                                .unwrap_or_else(|| "Unknown version".into()),
                            size_text(meta.fingerprint.size),
                            meta.appimage_type,
                            meta.arch,
                            self.paths.short(&p.path)
                        ))),
                ),
        );
        if let Some(r) = &p.requirements {
            col = col.push(wrap(if r.verdict == Verdict::Missing {
                format!("May not start here: {}", r.problems.join(" "))
            } else {
                format!("Requirements met: {}.", runtime_text(r))
            }));
        }
        let mut facts = vec![];
        if meta.update_info.is_some() {
            facts.push("includes update information from its developer".to_string());
        }
        if meta.self_updater.is_some() {
            facts.push("updates itself".into());
        }
        if meta.terminal {
            facts.push("runs in a terminal".into());
        }
        if !facts.is_empty() {
            col = col.push(widget::text::caption(format!(
                "This app {}.",
                facts.join(", ")
            )));
        }
        if let Some(id) = &p.same_file {
            col = col.push(wrap(format!("Already integrated as {id}.")));
        } else if !p.others.is_empty() {
            col = col.push(wrap(format!(
                "Another version is integrated: {}. Keep both, or replace it in place (its arguments, environment and icon are kept).",
                p.others.iter().map(|(id, v)| format!("{id} {}", v.clone().unwrap_or_default())).collect::<Vec<_>>().join(", ")
            )));
            col = col.push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(
                        if p.conflict == Conflict::KeepBoth {
                            widget::button::suggested("Keep Both")
                        } else {
                            widget::button::standard("Keep Both")
                        }
                        .on_press(Message::PendingConflict(i, Conflict::KeepBoth)),
                    )
                    .push(
                        if p.conflict == Conflict::Replace {
                            widget::button::suggested("Replace")
                        } else {
                            widget::button::standard("Replace")
                        }
                        .on_press(Message::PendingConflict(i, Conflict::Replace)),
                    ),
            );
        }
        match &p.hashes {
            Some(Ok(h)) => {
                col = col.push(widget::text::monotext(format!(
                    "md5 {}\nsha1 {}\nsha256 {}",
                    h.md5, h.sha1, h.sha256
                )))
            }
            Some(Err(e)) => col = col.push(widget::text::caption(e.clone())),
            None => {}
        }
        let can_add = p.same_file.is_none() && (p.others.is_empty() || p.conflict != Conflict::Ask);
        col = col.push(
            widget::flex_row(vec![
                widget::button::suggested("Add to App Menu")
                    .on_press_maybe((!busy && can_add).then(|| Message::Integrate(vec![i])))
                    .into(),
                widget::button::standard(if meta.terminal {
                    "Run in Terminal"
                } else {
                    "Run Without Adding"
                })
                .on_press_maybe((!busy).then_some(Message::RunPending(i)))
                .into(),
                widget::button::text("Checksums")
                    .on_press_maybe(p.hashes.is_none().then_some(Message::PendingHashes(i)))
                    .into(),
                widget::button::text("Remove from List")
                    .on_press(Message::DismissPending(i))
                    .into(),
            ])
            .row_spacing(8)
            .column_spacing(8),
        );
        panel(col)
    }

    fn add_view(&self) -> Element<'_, Message> {
        let mut col = widget::column::with_capacity(8).spacing(16).max_width(760);
        col = col.push(panel(
            widget::column::with_capacity(3)
                .spacing(10)
                .align_x(Alignment::Center)
                .width(Length::Fill)
                .push(icon::from_name("folder-download-symbolic").size(48).icon())
                .push(widget::text::title4("Open or drop AppImages"))
                .push(wrap(format!(
                    "Added apps are {} into {} and get a menu entry. Verify where an app comes from before you run it.",
                    if self.settings.move_on_integrate { "moved" } else { "copied" },
                    self.paths.short(&self.settings.folder(&self.paths))
                )).align_x(iced::alignment::Horizontal::Center))
                .push(widget::button::suggested("Open AppImage…").id(widget::Id::new("add.open")).on_press(Message::OpenFiles)),
        ));
        for (i, p) in self.pending.iter().enumerate() {
            col = col.push(self.pending_card(i, p));
        }
        if self.pending.len() > 1 {
            let ready: Vec<usize> = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    matches!(p.meta, Some(Ok(_)))
                        && p.same_file.is_none()
                        && (p.others.is_empty() || p.conflict != Conflict::Ask)
                })
                .map(|(i, _)| i)
                .collect();
            col = col.push(panel(
                widget::column::with_capacity(2)
                    .spacing(10)
                    .push(
                        widget::list_column().add(
                            widget::settings::item::builder(
                                "I have verified the source of these apps",
                            )
                            .checkbox(self.verified, Message::Verified),
                        ),
                    )
                    .push(
                        widget::button::suggested(format!("Add All to App Menu ({})", ready.len()))
                            .id(widget::Id::new("add.all"))
                            .on_press_maybe(
                                (self.verified && !self.busy() && !ready.is_empty())
                                    .then(|| Message::Integrate(ready.clone())),
                            ),
                    ),
            ));
        }
        widget::scrollable(widget::container(col).center_x(Length::Fill))
            .spacing(8)
            .height(Length::Fill)
            .into()
    }

    fn progress(&self, running: &RunningJob) -> Element<'_, Message> {
        let p = running
            .job
            .progress
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        let bar: Element<'_, Message> = match p.fraction {
            Some(f) => widget::progress_bar::determinate_linear(f).into(),
            None => widget::progress_bar::indeterminate_linear().into(),
        };
        panel(
            widget::column::with_capacity(3)
                .spacing(8)
                .push(
                    widget::row::with_capacity(2)
                        .align_y(Alignment::Center)
                        .push(
                            widget::text::heading(format!(
                                "{}{}",
                                running.title,
                                if running.total > 1 {
                                    format!(" · {} of {}", running.done + 1, running.total)
                                } else {
                                    String::new()
                                }
                            ))
                            .width(Length::Fill),
                        )
                        .push(
                            widget::button::standard("Cancel")
                                .id(widget::Id::new("operation.cancel"))
                                .on_press(Message::Cancel),
                        ),
                )
                .push(bar)
                .push(widget::text::caption(match p.fraction {
                    Some(f) => format!("{} · {:.0}%", p.step, f * 100.),
                    None => p.step,
                })),
        )
    }

    fn outcome_panel(&self, o: &Outcome) -> Element<'_, Message> {
        let mut col = widget::column::with_capacity(6)
            .spacing(6)
            .width(Length::Fill)
            .push(widget::text::heading(if o.ok {
                format!("{}: done", o.title)
            } else {
                format!("{}: failed", o.title)
            }));
        for e in &o.errors {
            col = col.push(wrap(e.clone()));
        }
        for n in &o.notes {
            col = col.push(wrap(n.clone()));
        }
        if let Some(l) = &o.log {
            col = col.push(widget::text::caption(format!(
                "Report: {}",
                self.paths.short(l)
            )));
        }
        panel(
            widget::row::with_capacity(2).spacing(12).push(col).push(
                widget::button::icon(icon::from_name("window-close-symbolic"))
                    .tooltip("Dismiss")
                    .id(widget::Id::new("result.dismiss"))
                    .on_press(Message::ClearOutcome),
            ),
        )
    }
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = "dev.nilltadios.GearLeverCosmic";
    fn core(&self) -> &cosmic::Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }
    fn init(core: cosmic::Core, flags: Flags) -> (Self, Task<Message>) {
        let mut nav = nav_bar::Model::default();
        for (page, name) in [
            (Page::Apps, "view-app-grid-symbolic"),
            (Page::Updates, "software-update-available-symbolic"),
            (Page::Add, "list-add-symbolic"),
        ] {
            nav.insert()
                .text(page.name())
                .icon(icon::from_name(name))
                .data(page);
        }
        nav.activate_position(0);
        let paths = Paths::discover().expect("A home directory is required");
        let (settings, notice) = match Settings::load(&paths) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e)),
        };
        let state = flags.window_state;
        let isolated = flags.capture.is_some()
            || std::env::var("GEARLEVER_COSMIC_UI_ISOLATED").is_ok_and(|v| v == "1");
        let mut app = Self {
            core,
            nav,
            page: Page::Apps,
            paths,
            settings,
            inventory: Inventory::default(),
            running: BTreeMap::new(),
            loading: true,
            query: String::new(),
            selected: None,
            detail_open: false,
            checks: BTreeMap::new(),
            checking: BTreeSet::new(),
            checking_all: false,
            checked_all_once: false,
            requirements: BTreeMap::new(),
            hashes: BTreeMap::new(),
            edit_for: None,
            args_text: String::new(),
            env_rows: vec![],
            website_text: String::new(),
            source_choice: 0,
            source_fields: BTreeMap::new(),
            prerelease: false,
            icon_path: String::new(),
            edit_error: None,
            pending: vec![],
            verified: false,
            drag_over: false,
            dialog: None,
            job: None,
            outcome: None,
            notice,
            toasts: widget::Toasts::new(Message::CloseToast),
            window_state: state,
            width: state.width as f32,
            list_width: state.list_width as f32,
            split_drag: None,
            nav_width: state.nav_width as f32,
            window_request: 0,
            window_observed: 0,
            initial_window_mode: Some(state.mode),
            window_started: std::time::Instant::now(),
            capture_ticks: 0,
            isolated,
            ui_instance: std::env::var("GEARLEVER_COSMIC_UI_INSTANCE")
                .ok()
                .filter(|n| {
                    n.starts_with(gearlever_cosmic::ui_protocol::PREFIX)
                        && zbus::names::WellKnownName::try_from(n.as_str()).is_ok()
                })
                .unwrap_or_else(gearlever_cosmic::ui_client::new_instance),
            synced_once: false,
            flags,
        };
        let title = app.set_window_title(
            "Gear Lever COSMIC".into(),
            app.core.main_window_id().unwrap(),
        );
        let load = app.refresh();
        let window = app.core.main_window_id().unwrap();
        let restore: iced::Task<Message> = window_state::restore(state.mode, window);
        let files = std::mem::take(&mut app.flags.open_files);
        let open = if files.is_empty() {
            Task::none()
        } else {
            app.open_files(files)
        };
        (
            app,
            Task::batch([title, load, restore.map(cosmic::Action::App), open]),
        )
    }
    fn nav_bar(&self) -> Option<Element<'_, cosmic::Action<Message>>> {
        if !self.core.nav_bar_active() {
            return None;
        }
        let sidebar = widget::nav_bar(&self.nav, |id| {
            cosmic::Action::Cosmic(cosmic::app::Action::NavBar(id))
        })
        .window_id_maybe(self.core.main_window_id())
        .on_surface_action(|action| cosmic::Action::Surface(action.flatten()))
        .into_container()
        .height(Length::Fill);
        Some(if self.core.is_condensed() {
            sidebar.width(Length::Shrink).into()
        } else {
            sidebar
                .width(Length::Fixed(self.clamp_nav_width(self.nav_width)))
                .into()
        })
    }
    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Message> {
        match self.nav.data::<Page>(id).copied() {
            Some(page) => self.switch(page),
            None => Task::none(),
        }
    }
    fn context_drawer(&self) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        if !self.core.window.show_context {
            return None;
        }
        let s = &self.settings;
        let set = |f: fn(bool) -> SettingChange| move |v| Message::Setting(f(v));
        let general = widget::settings::section()
            .title("General")
            .add(
                widget::settings::item::builder("AppImages folder")
                    .description(s.appimages_folder.clone())
                    .control(
                        widget::button::standard("Choose…")
                            .id(widget::Id::new("settings.folder"))
                            .on_press(Message::ChooseFolder),
                    ),
            )
            .add(
                widget::settings::item::builder("Show apps outside the folder")
                    .description("List menu entries that start an AppImage stored elsewhere")
                    .toggler(s.show_external, set(SettingChange::ShowExternal)),
            )
            .add(
                widget::settings::item::builder("Preview opened files in a compact view")
                    .toggler(s.preview_minimal, set(SettingChange::PreviewMinimal)),
            );
        let files = widget::settings::section()
            .title("Adding apps")
            .add(widget::settings::item::builder("Move into the folder").description("Saves disk space").radio(true, Some(s.move_on_integrate), |v| Message::Setting(SettingChange::MoveOnIntegrate(v))))
            .add(widget::settings::item::builder("Copy into the folder").description("Keeps the original file").radio(false, Some(s.move_on_integrate), |v| Message::Setting(SettingChange::MoveOnIntegrate(v))))
            .add(widget::settings::item::builder("Name terminal apps after their executable").description("For example \"go\" instead of \"go.appimage\"; add the folder to $PATH yourself").toggler(s.terminal_apps_use_executable_name, set(SettingChange::TerminalNames)))
            .add(widget::settings::item::builder("Use safe mechanisms to load metadata").description("Never run an AppImage to read its metadata. Off lets DwarFS images be read by running them.").toggler(s.safe_metadata, set(SettingChange::SafeMetadata)));
        let updates = widget::settings::section()
            .title("Updates")
            .add(widget::settings::item::builder("Check for updates after login").description("Sends a notification; installs nothing").toggler(s.check_updates_on_startup, set(SettingChange::CheckOnStartup)))
            .add(widget::settings::item::builder("Keep replaced versions in the Trash").description("Restore an older version from the Trash if an update misbehaves").toggler(s.keep_replaced_versions_in_trash, set(SettingChange::KeepReplaced)))
            .add(widget::settings::item::builder("Keep menu entries in sync").description("When an AppImage changed on disk, refresh the version and icon of menu entries Gear Lever COSMIC wrote. Others are only flagged.").toggler(s.sync_menu_entries, set(SettingChange::SyncMenus)));
        let theme = Some(s.theme);
        let appearance = widget::settings::section()
            .title("Appearance")
            .add(widget::settings::item::builder("Match desktop").radio(
                Theme::System,
                theme,
                |t| Message::Setting(SettingChange::Theme(t)),
            ))
            .add(
                widget::settings::item::builder("Dark").radio(Theme::Dark, theme, |t| {
                    Message::Setting(SettingChange::Theme(t))
                }),
            )
            .add(
                widget::settings::item::builder("Light").radio(Theme::Light, theme, |t| {
                    Message::Setting(SettingChange::Theme(t))
                }),
            );
        let gl = gearlever_cosmic::store::gearlever_all(&self.paths);
        let importable = self
            .inventory
            .apps
            .iter()
            .filter(|a| a.update_source_origin == Some(updates::Origin::GearLever))
            .count();
        let migrate = widget::settings::section()
            .title("Gear Lever")
            .add(widget::settings::item::builder("Import Gear Lever's update sources").description(format!("{} saved sources found; {importable} are used for apps here until you import or replace them. Gear Lever's files are only read.", gl.values().filter(|g| g.source.is_some()).count())).control(widget::button::standard("Import").id(widget::Id::new("settings.import")).on_press_maybe((importable > 0 && !self.busy()).then_some(Message::ImportGearLever))))
            .add(widget::settings::item::builder("Debug logs").description("More detailed logs on the terminal").toggler(s.debug_logs, set(SettingChange::DebugLogs)));
        let content = widget::column::with_capacity(5)
            .spacing(16)
            .push(general)
            .push(files)
            .push(updates)
            .push(appearance)
            .push(migrate);
        Some(
            cosmic::app::context_drawer::context_drawer(content, Message::CloseSettings)
                .title("Settings"),
        )
    }
    fn dbus_activation(&mut self, message: cosmic::dbus_activation::Message) -> Task<Message> {
        use cosmic::dbus_activation::Details;
        let files: Vec<PathBuf> = match message.msg {
            Details::Open { url } => url
                .into_iter()
                .filter_map(|u| u.to_file_path().ok())
                .collect(),
            Details::ActivateAction { action, args } if action == "open-files" => {
                args.into_iter().map(PathBuf::from).collect()
            }
            _ => vec![],
        };
        let mut tasks = vec![];
        if !files.is_empty() {
            tasks.push(self.open_files(files));
        }
        tasks.push(self.present_window());
        Task::batch(tasks)
    }
    fn header_start(&self) -> Vec<Element<'_, Message>> {
        vec![
            header_tooltip(
                widget::button::icon(icon::from_svg_bytes(
                    include_bytes!("../resources/dev.nilltadios.GearLeverCosmic.svg").as_slice(),
                ))
                .icon_size(24)
                .padding(4)
                .width(32)
                .height(32)
                .on_press(Message::About)
                .id(widget::Id::new("header.logo"))
                .name("Gear Lever COSMIC"),
                "Gear Lever COSMIC",
            ),
            header_tooltip(
                widget::button::icon(icon::from_name("folder-open-symbolic"))
                    .icon_size(16)
                    .padding(8)
                    .width(32)
                    .height(32)
                    .on_press(Message::OpenFiles)
                    .id(widget::Id::new("header.open"))
                    .name("Open AppImage"),
                "Open AppImage",
            ),
            header_tooltip(
                widget::button::icon(icon::from_name("view-refresh-symbolic"))
                    .icon_size(16)
                    .padding(8)
                    .width(32)
                    .height(32)
                    .on_press_maybe((!self.loading && !self.busy()).then_some(Message::Refresh))
                    .id(widget::Id::new("header.refresh"))
                    .name("Refresh"),
                "Refresh",
            ),
            // Centred in the space between the button groups, like Warehouse
            // COSMIC. The text keeps its own width (a Fill width would
            // left-align it); 4 px left padding balances the header's wider
            // gap before the end buttons.
            widget::container(
                widget::text::heading(self.page.name())
                    .wrapping(iced::advanced::text::Wrapping::None)
                    .ellipsize(iced::advanced::text::Ellipsize::End(
                        iced::advanced::text::EllipsizeHeightLimit::Lines(1),
                    )),
            )
            .padding([0, 0, 0, 4])
            .center_x(Length::Fill)
            .into(),
        ]
    }
    fn header_end(&self) -> Vec<Element<'_, Message>> {
        vec![
            header_tooltip(
                widget::button::icon(icon::from_name("preferences-system-symbolic"))
                    .icon_size(16)
                    .padding(8)
                    .width(32)
                    .height(32)
                    .name("Settings")
                    .id(widget::Id::new("header.settings"))
                    .on_press(Message::Settings),
                "Settings",
            ),
            header_tooltip(
                widget::button::icon(icon::from_name("help-about-symbolic"))
                    .on_press(Message::About)
                    .id(widget::Id::new("header.about"))
                    .name("About"),
                "About Gear Lever COSMIC",
            ),
        ]
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            crate::ui_bridge::subscription(&self.ui_instance),
            iced::event::listen_with(|event, _, id| match event {
                iced::Event::PlatformSpecific(iced::event::PlatformSpecific::Wayland(
                    iced::event::wayland::Event::Window(
                        iced::event::wayland::WindowEvent::WindowState(_),
                    ),
                )) => Some(Message::WindowChanged(id)),
                _ => None,
            }),
            iced::window::events().filter_map(|(id, event)| match event {
                iced::window::Event::Opened { .. }
                | iced::window::Event::Resized(_)
                | iced::window::Event::Focused
                | iced::window::Event::Unfocused => Some(Message::WindowChanged(id)),
                _ => None,
            }),
            if self.split_drag.is_some() {
                iced::event::listen_with(|event, _, _| match event {
                    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                        Some(Message::SplitMove(position))
                    }
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left,
                    )) => Some(Message::SplitEnd),
                    _ => None,
                })
            } else {
                Subscription::none()
            },
            if self.job.is_some() || self.flags.capture.is_some() {
                iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
            } else {
                Subscription::none()
            },
            iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    Some(Message::Key(key, modifiers))
                }
                _ => None,
            }),
        ])
    }
    fn on_escape(&mut self) -> Task<Message> {
        self.drag_over = false;
        if self.dialog.take().is_some() {
            return Task::none();
        }
        if self.core.window.show_context {
            self.set_show_context(false);
        } else if self.detail_open {
            self.detail_open = false;
        } else {
            self.query.clear();
        }
        Task::none()
    }
    fn on_close_requested(&self, _: iced::window::Id) -> Option<Message> {
        self.save_window();
        None
    }
    fn view(&self) -> Element<'_, Message> {
        let mut col = widget::column::with_capacity(6).spacing(14);
        if let Some(j) = &self.job {
            col = col.push(self.progress(j));
        }
        if let Some(o) = &self.outcome
            && (!o.ok || !o.notes.is_empty())
        {
            col = col.push(self.outcome_panel(o));
        }
        if self.drag_over {
            col = col.push(panel(widget::text::heading(
                "Drop AppImages to review them before adding them to the app menu",
            )));
        }
        if let Some(n) = &self.notice {
            col = col.push(panel(
                widget::row::with_capacity(2)
                    .spacing(12)
                    .push(wrap(n.clone()).width(Length::Fill))
                    .push(
                        widget::button::icon(icon::from_name("window-close-symbolic"))
                            .tooltip("Dismiss")
                            .id(widget::Id::new("notice.dismiss"))
                            .on_press(Message::ClearNotice),
                    ),
            ));
        }
        let page: Element<'_, Message> = if self.loading && self.inventory.apps.is_empty() {
            self.empty(
                "Loading…",
                "Reading menu entries and AppImages.",
                "view-refresh-symbolic",
            )
        } else {
            match self.page {
                Page::Apps => self.apps_view(),
                Page::Updates => self.updates_view(),
                Page::Add => self.add_view(),
            }
        };
        let content = widget::toaster(&self.toasts, {
            let page = widget::container(col.push(page))
                .id(widget::Id::new("content"))
                .height(Length::Fill);
            if self.core.nav_bar_active() && !self.core.is_condensed() {
                Element::from(
                    widget::row::with_capacity(2)
                        .push(self.split_handle(Split::Nav))
                        .push(page.padding([20, 20, 20, 11])),
                )
            } else {
                page.padding(20).into()
            }
        });
        widget::dnd_destination(
            content,
            vec!["text/uri-list".into(), FILE_TRANSFER_MIME.into()],
        )
        .on_enter(|_, _, _| Message::DragOver(true))
        .on_leave(|| Message::DragOver(false))
        .on_finish(|mime, data, _, _, _| Message::Dropped(mime, data))
        .into()
    }
    fn dialog(&self) -> Option<Element<'_, Message>> {
        let dialog = self.dialog.as_ref()?;
        let cancel = || {
            widget::button::standard("Cancel")
                .id(widget::Id::new("dialog.cancel"))
                .on_press(Message::CloseDialog)
        };
        Some(match dialog {
            Dialog::Remove(apps) => {
                let title = if apps.len() == 1 { format!("Remove {}?", apps[0].name) } else { format!("Remove {} Apps?", apps.len()) };
                let mut body = String::from("The AppImage, its menu entry and its icon move to the Trash, so you can restore them. App settings in your home folder are not touched.");
                if apps.iter().any(|a| self.running.contains_key(&a.id)) {
                    body.push_str("\n\nA running app keeps running until you close it.");
                }
                for a in apps {
                    if !a.other_desktop_files.is_empty() {
                        body.push_str(&format!("\n\n{} also has a menu entry made outside this app; it stays.", a.name));
                    }
                }
                widget::dialog()
                    .title(title)
                    .body(body)
                    .secondary_action(cancel())
                    .primary_action(widget::button::destructive("Remove").id(widget::Id::new("dialog.confirm")).on_press(Message::Confirm))
                    .into()
            }
            Dialog::Confirm(op, plan) => {
                let mut list = widget::column::with_capacity(plan.len()).spacing(4);
                for line in plan {
                    list = list.push(wrap(format!("• {line}")));
                }
                widget::dialog()
                    .title(op.title())
                    .control(list)
                    .secondary_action(cancel())
                    .primary_action(widget::button::suggested("Continue").id(widget::Id::new("dialog.confirm")).on_press(Message::Confirm))
                    .into()
            }
            Dialog::About => widget::dialog()
                .title("Gear Lever COSMIC")
                .icon(
                    widget::icon(icon::from_svg_bytes(
                        include_bytes!("../resources/dev.nilltadios.GearLeverCosmic.svg").as_slice(),
                    ))
                    .size(64),
                )
                .body(format!(
                    "Manage AppImages: add them to the app menu, keep them updated in place, and see clearly what can and cannot be updated. Inspired by Gear Lever by mijorus, rewritten in Rust with libcosmic.\n\nGPL-3.0-or-later · Version {}",
                    env!("CARGO_PKG_VERSION")
                ))
                .control(widget::text::caption("Shortcuts: Ctrl+O open · Ctrl+F filter · Ctrl+R or F5 refresh · Ctrl+, settings · Ctrl+1/2/3 pages · Escape goes back. Agents: gearlever-cosmic-cli guide"))
                .primary_action(widget::button::suggested("Close").id(widget::Id::new("dialog.cancel")).on_press(Message::CloseDialog))
                .into(),
        })
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UiRequest(r) => return self.ui_request(r),
            Message::UiTree(r, nodes, resolve) => self.ui_tree(r, nodes, resolve),
            Message::UiCaptured(r, path, overwrite, shot) => {
                self.ui_capture(r, path, overwrite, shot)
            }
            Message::UiActivationToken(r, token) => r.respond(
                gearlever_cosmic::ui_protocol::success(serde_json::json!({"token":token})),
            ),
            Message::UiCloseNow => return iced::exit(),
            Message::SplitStart(split) => self.split_drag = Some((split, None)),
            Message::SplitMove(p) => {
                if let Some((split, anchor)) = self.split_drag {
                    let current = match split {
                        Split::Nav => self.clamp_nav_width(self.nav_width),
                        Split::List => self.clamp_list_width(self.list_width),
                    };
                    match anchor {
                        None => self.split_drag = Some((split, Some((p.x, current)))),
                        Some((x0, w0)) => match split {
                            Split::Nav => self.nav_width = self.clamp_nav_width(w0 + p.x - x0),
                            Split::List => self.list_width = self.clamp_list_width(w0 + p.x - x0),
                        },
                    }
                }
            }
            Message::SplitEnd => {
                if self.split_drag.take().is_some() {
                    self.window_state.list_width = self.list_width.round() as u32;
                    self.window_state.nav_width = self.nav_width.round() as u32;
                    self.save_window();
                }
            }
            Message::ListWidth(w) => {
                self.list_width = self.clamp_list_width(w);
                self.window_state.list_width = self.list_width.round() as u32;
            }
            Message::NavWidth(w) => {
                self.nav_width = self.clamp_nav_width(w);
                self.window_state.nav_width = self.nav_width.round() as u32;
            }
            Message::Loaded(l) => {
                self.loading = false;
                self.inventory = l.inventory;
                self.running = l.running;
                self.requirements
                    .retain(|id, _| self.inventory.apps.iter().any(|a| &a.id == id));
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|id| self.app(id).is_none())
                {
                    self.selected = None;
                    self.detail_open = false;
                }
                self.edit_for = None;
                self.load_edits();
                let mut tasks = vec![];
                if let Some(p) = self.flags.preview.take() {
                    tasks.push(self.preview(p));
                }
                // Gear Lever issue #497: fix menu entries left behind by
                // self-updates, once per start (not in test instances). Only
                // entries this app wrote; Gear Lever's own entries and ones the
                // app made itself are flagged and fixed on request only.
                if !self.synced_once && self.settings.sync_menu_entries && !self.isolated {
                    self.synced_once = true;
                    let outdated: Vec<AppItem> = self
                        .inventory
                        .apps
                        .iter()
                        .filter(|a| {
                            a.menu_outdated
                                .iter()
                                .any(|o| o == "version" || o == "name")
                                && a.managed_by == ManagedBy::GearLeverCosmic
                        })
                        .cloned()
                        .collect();
                    if !outdated.is_empty() {
                        tasks.push(self.run_ops(vec![Operation::SyncMenu(outdated)]));
                    }
                }
                if self.page == Page::Updates
                    && !self.checked_all_once
                    && self.flags.capture.is_none()
                {
                    tasks.push(self.check_all());
                }
                return Task::batch(tasks);
            }
            Message::Refresh => {
                self.checks.clear();
                self.checked_all_once = false;
                self.requirements.clear();
                return self.refresh();
            }
            Message::Page(p) => return self.switch(p),
            Message::Select(id) => {
                self.page = Page::Apps;
                self.nav.activate_position(0);
                return self.select(id);
            }
            Message::Back => self.detail_open = false,
            Message::Field(f, v) => match f {
                Field::Search => self.query = v,
                Field::Args => self.args_text = v,
                Field::EnvKey(i) => {
                    if let Some(r) = self.env_rows.get_mut(i) {
                        r.0 = v;
                    }
                }
                Field::EnvValue(i) => {
                    if let Some(r) = self.env_rows.get_mut(i) {
                        r.1 = v;
                    }
                }
                Field::Website => self.website_text = v,
                Field::Source(k) => {
                    self.source_fields.insert(k, v);
                }
                Field::IconPath => self.icon_path = v,
            },
            Message::Checked(id, c) => {
                self.checking.remove(&id);
                self.checks.insert(id, *c);
            }
            Message::CheckApp(id) => return self.check_app(id),
            Message::CheckAll => return self.check_all(),
            Message::CheckedAll(list) => {
                self.checking_all = false;
                self.checks.extend(list);
            }
            Message::Requirements(id, r) => {
                self.requirements.insert(id, *r);
            }
            Message::ShowHashes(path) => {
                let p = path.clone();
                return background(
                    move || appimage::hashes(&p),
                    move |h| Message::Hashes(path, Box::new(h)),
                );
            }
            Message::Hashes(path, h) => {
                if let Some(i) = path
                    .to_str()
                    .and_then(|p| p.strip_prefix("pending:"))
                    .and_then(|i| i.parse::<usize>().ok())
                {
                    if let Some(p) = self.pending.get_mut(i) {
                        p.hashes = Some(*h);
                    }
                } else {
                    self.hashes.insert(path, *h);
                }
            }
            Message::Launch(id) => {
                if let Some(a) = self.app(&id).cloned() {
                    let t = self.toast(format!("Starting {}…", a.name));
                    return Task::batch([
                        t,
                        background(move || automation::launch_app(&a), Message::Launched),
                    ]);
                }
            }
            Message::Launched(r) => {
                if let Err(e) = r {
                    self.notice = Some(e);
                }
            }
            Message::OpenFolder(id) => {
                if let Some(a) = self.app(&id)
                    && let Err(e) = gearlever_cosmic::host::open(
                        &a.path.parent().unwrap_or(Path::new("/")).to_string_lossy(),
                    )
                {
                    self.notice = Some(e);
                }
            }
            Message::OpenWebsite(url) => {
                if let Err(e) = gearlever_cosmic::host::open(&url) {
                    self.notice = Some(e);
                }
            }
            Message::AskRemove(ids) => {
                let apps: Vec<AppItem> =
                    ids.iter().filter_map(|id| self.app(id).cloned()).collect();
                if !apps.is_empty() {
                    self.dialog = Some(Dialog::Remove(apps));
                }
            }
            Message::Confirm => match self.dialog.take() {
                Some(Dialog::Remove(apps)) => return self.run_ops(vec![Operation::Remove(apps)]),
                Some(Dialog::Confirm(op, _)) => return self.run_ops(vec![*op]),
                _ => {}
            },
            Message::CloseDialog => self.dialog = None,
            Message::UpdateApp(id) => {
                if let (Some(a), Some(c)) = (self.app(&id).cloned(), self.checks.get(&id).cloned())
                    && let (Status::Available, true, Some(d)) =
                        (&c.status, c.can_install, c.download)
                {
                    self.checks.remove(&id);
                    return self.run_ops(vec![Operation::Update {
                        app: a,
                        download: d,
                        version: c.available_version,
                    }]);
                }
            }
            Message::UpdateAll => {
                let ops: Vec<Operation> = self
                    .inventory
                    .apps
                    .iter()
                    .filter_map(|a| {
                        let c = self.checks.get(&a.id)?;
                        if c.status == Status::Available && c.can_install {
                            Some(Operation::Update {
                                app: a.clone(),
                                download: c.download.clone()?,
                                version: c.available_version.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                for op in &ops {
                    if let Operation::Update { app, .. } = op {
                        self.checks.remove(&app.id);
                    }
                }
                return self.run_ops(ops);
            }
            Message::SyncMenu(ids) => {
                let apps: Vec<AppItem> =
                    ids.iter().filter_map(|id| self.app(id).cloned()).collect();
                return self.run_ops(vec![Operation::SyncMenu(apps)]);
            }
            Message::SaveArgs => {
                if let Some(a) = self.selected_app().cloned() {
                    match gearlever_cosmic::desktop::split_command(&self.args_text) {
                        Ok(args) => {
                            self.edit_for = None;
                            return self.run_ops(vec![Operation::SetArguments(a, args)]);
                        }
                        Err(e) => self.edit_error = Some(format!("Arguments: {e}")),
                    }
                }
            }
            Message::ResetArgs => {
                if let Some(a) = self.selected_app() {
                    self.args_text = gearlever_cosmic::desktop::join_command(&a.default_arguments);
                }
            }
            Message::AddEnv => self.env_rows.push((String::new(), String::new())),
            Message::RemoveEnv(i) => {
                if i < self.env_rows.len() {
                    self.env_rows.remove(i);
                }
            }
            Message::SaveEnv => {
                if let Some(a) = self.selected_app().cloned() {
                    let rows: Vec<(String, String)> = self
                        .env_rows
                        .iter()
                        .filter(|(k, _)| !k.trim().is_empty())
                        .map(|(k, v)| (k.trim().to_string(), v.clone()))
                        .collect();
                    let mut seen = BTreeSet::new();
                    if let Some((k, _)) = rows.iter().find(|(k, _)| {
                        !gearlever_cosmic::desktop::valid_env_key(k) || !seen.insert(k.clone())
                    }) {
                        self.edit_error =
                            Some(format!("{k} is not a valid or unique variable name"));
                    } else {
                        self.edit_for = None;
                        return self.run_ops(vec![Operation::SetEnv(a, rows)]);
                    }
                }
            }
            Message::SaveWebsite => {
                if let Some(a) = self.selected_app().cloned() {
                    let w = self.website_text.trim().to_string();
                    if !w.is_empty() && !(w.starts_with("https://") || w.starts_with("http://")) {
                        self.edit_error =
                            Some("The website must start with https:// or http://".into());
                    } else {
                        self.edit_for = None;
                        return self
                            .run_ops(vec![Operation::SetWebsite(a, (!w.is_empty()).then_some(w))]);
                    }
                }
            }
            Message::SourceChoice(i) => {
                self.source_choice = i;
                self.edit_error = None;
            }
            Message::Prerelease(v) => self.prerelease = v,
            Message::SaveSource => {
                if let Some(a) = self.selected_app().cloned() {
                    match self.source_from_fields() {
                        Ok(s) => {
                            self.edit_for = None;
                            self.checks.remove(&a.id);
                            return self.run_ops(vec![Operation::SetSource(a, s)]);
                        }
                        Err(e) => self.edit_error = Some(e),
                    }
                }
            }
            Message::ChooseIcon => {
                return cosmic::task::future(async move {
                    use cosmic::dialog::file_chooser::{self, FileFilter};
                    let r = file_chooser::open::Dialog::new()
                        .title("Choose an Icon")
                        .filter(
                            FileFilter::new("Images")
                                .glob("*.png")
                                .glob("*.svg")
                                .glob("*.PNG")
                                .glob("*.SVG"),
                        )
                        .open_file()
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| {
                            r.url()
                                .to_file_path()
                                .map_err(|_| "Choose a local file".to_string())
                        });
                    Message::IconPicked(r)
                });
            }
            Message::IconPicked(r) => match r {
                Ok(p) => {
                    self.icon_path = p.display().to_string();
                    return self.update(Message::SetIconFromField);
                }
                Err(e) if !e.to_lowercase().contains("cancel") => self.notice = Some(e),
                Err(_) => {}
            },
            Message::SetIconFromField => {
                if let Some(a) = self.selected_app().cloned() {
                    let p = PathBuf::from(self.icon_path.trim());
                    match std::fs::read(&p)
                        .ok()
                        .and_then(|d| appimage::image_kind(&d))
                    {
                        Some(_) => return self.run_ops(vec![Operation::SetIcon(a, p)]),
                        None => {
                            self.notice = Some(format!("{} is not a PNG or SVG image", p.display()))
                        }
                    }
                }
            }
            Message::ResetIcon => {
                if let Some(a) = self.selected_app().cloned() {
                    return self.run_ops(vec![Operation::ResetIcon(a)]);
                }
            }
            Message::Finished(result) => {
                let title = self
                    .job
                    .as_ref()
                    .map(|j| j.title.clone())
                    .unwrap_or_default();
                let stop;
                let outcome = match *result {
                    Ok(r) => {
                        let ok = r.errors.is_empty();
                        stop = !ok || r.cancelled;
                        let mut o = self.outcome.take().filter(|o| o.ok).unwrap_or(Outcome {
                            title: title.clone(),
                            ok: true,
                            steps: vec![],
                            notes: vec![],
                            errors: vec![],
                            log: None,
                        });
                        o.ok &= ok;
                        o.steps.extend(r.steps);
                        o.notes.extend(
                            r.notes
                                .into_iter()
                                .filter(|n| !n.starts_with("Previous menu entry saved")),
                        );
                        o.errors.extend(r.errors);
                        o.log = Some(r.log);
                        o
                    }
                    Err(e) => {
                        stop = true;
                        Outcome {
                            title: title.clone(),
                            ok: false,
                            steps: vec![],
                            notes: vec![],
                            errors: vec![e],
                            log: None,
                        }
                    }
                };
                let summary = outcome
                    .steps
                    .last()
                    .cloned()
                    .unwrap_or_else(|| title.clone());
                let ok = outcome.ok;
                self.outcome = Some(outcome);
                if let Some(j) = &mut self.job {
                    j.done += 1;
                    if stop {
                        j.queue.clear();
                    }
                }
                let next = self.next_op();
                return if ok {
                    Task::batch([next, self.toast(summary)])
                } else {
                    next
                };
            }
            Message::Cancel => {
                if let Some(j) = &self.job {
                    j.job
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Message::Tick => {
                if self.flags.capture.is_some()
                    && !self.loading
                    && self.checking.is_empty()
                    && !self.checking_all
                    && self.pending.iter().all(|p| p.meta.is_some())
                {
                    self.capture_ticks += 1;
                    if self.capture_ticks == 8
                        && let Some(id) = self.core.main_window_id()
                    {
                        return iced::window::screenshot(id)
                            .map(Message::Captured)
                            .map(cosmic::Action::App);
                    }
                }
            }
            Message::Captured(shot) => {
                if let Some(path) = self.flags.capture.take() {
                    match image::save_buffer(
                        &path,
                        &shot.rgba,
                        shot.size.width,
                        shot.size.height,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => {
                            println!(
                                "Saved app render to {} ({}x{})",
                                path.display(),
                                shot.size.width,
                                shot.size.height
                            );
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                            std::process::exit(0);
                        }
                        Err(e) => {
                            eprintln!("Cannot save capture: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            Message::OpenFiles => {
                return cosmic::task::future(async move {
                    use cosmic::dialog::file_chooser::{self, FileFilter};
                    let r = file_chooser::open::Dialog::new()
                        .title("Open AppImages")
                        .filter(
                            FileFilter::new("AppImages")
                                .glob("*.AppImage")
                                .glob("*.appimage")
                                .glob("*.APPIMAGE")
                                .mimetype("application/vnd.appimage")
                                .mimetype("application/x-iso9660-appimage"),
                        )
                        .filter(FileFilter::new("All files").glob("*"))
                        .open_files()
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| {
                            r.urls()
                                .iter()
                                .map(|u| {
                                    u.to_file_path()
                                        .map_err(|_| "Choose local files".to_string())
                                })
                                .collect()
                        });
                    Message::FilesPicked(r)
                });
            }
            Message::FilesPicked(r) => match r {
                Ok(files) => return self.open_files(files),
                Err(e) if !e.to_lowercase().contains("cancel") => self.notice = Some(e),
                Err(_) => {}
            },
            Message::Inspected(file, p) => {
                if let Some(slot) = self.pending.iter_mut().find(|x| x.path == file) {
                    let path = slot.path.clone();
                    *slot = Pending { path, ..*p };
                    if slot.others.len() == 1 {
                        slot.conflict = Conflict::Ask;
                    }
                }
            }
            Message::Dropped(mime, data) => {
                self.drag_over = false;
                if mime == FILE_TRANSFER_MIME {
                    let key = String::from_utf8_lossy(&data)
                        .trim_end_matches('\0')
                        .trim()
                        .to_string();
                    if !key.is_empty() {
                        return self.update(Message::FileTransfer(key));
                    }
                } else {
                    let files = dropped_paths(&data);
                    if files.is_empty() {
                        self.notice = Some("The dropped item is not a local file.".into());
                    } else {
                        return self.open_files(files);
                    }
                }
            }
            Message::FileTransfer(key) => {
                return cosmic::command::file_transfer_receive(key).map(|r| {
                    cosmic::Action::App(Message::FilesPicked(
                        r.map(|f| f.into_iter().map(PathBuf::from).collect())
                            .map_err(|e| format!("Cannot receive the dropped files: {e}")),
                    ))
                });
            }
            Message::DragOver(v) => self.drag_over = v,
            Message::PendingConflict(i, c) => {
                if let Some(p) = self.pending.get_mut(i) {
                    p.conflict = c;
                }
            }
            Message::Integrate(indices) => match self.integrate_ops(&indices) {
                Ok(ops) => {
                    let mut sorted = indices.clone();
                    sorted.sort_unstable_by(|a, b| b.cmp(a));
                    for i in sorted {
                        if i < self.pending.len() {
                            self.pending.remove(i);
                        }
                    }
                    self.verified = false;
                    if let [
                        op @ Operation::Integrate {
                            conflict: Conflict::Replace,
                            ..
                        },
                    ] = ops.as_slice()
                    {
                        self.dialog = Some(Dialog::Confirm(Box::new(op.clone()), self.plan(op)));
                        return Task::none();
                    }
                    return self.run_ops(ops);
                }
                Err(e) => self.notice = Some(e),
            },
            Message::RunPending(i) => {
                if let Some(p) = self.pending.get(i) {
                    let path = p.path.clone();
                    let args: Vec<String> = match &p.meta {
                        Some(Ok(m)) => m
                            .default_arguments
                            .iter()
                            .filter(|a| {
                                !gearlever_cosmic::desktop::FIELD_CODES.contains(&a.as_str())
                            })
                            .cloned()
                            .collect(),
                        _ => vec![],
                    };
                    let t = self.toast(format!(
                        "Starting {}…",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    return Task::batch([
                        t,
                        background(
                            move || {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o755),
                                );
                                gearlever_cosmic::host::launch_file(&path, &args).map(|_| ())
                            },
                            Message::Launched,
                        ),
                    ]);
                }
            }
            Message::DismissPending(i) => {
                if i < self.pending.len() {
                    self.pending.remove(i);
                }
            }
            Message::PendingHashes(i) => {
                if let Some(p) = self.pending.get_mut(i) {
                    p.hashes = Some(Err("Computing…".into()));
                    let path = p.path.clone();
                    return background(
                        move || appimage::hashes(&path),
                        move |h| {
                            Message::Hashes(PathBuf::from(format!("pending:{i}")), Box::new(h))
                        },
                    );
                }
            }
            Message::Verified(v) => self.verified = v,
            Message::Setting(c) => return self.change_setting(c),
            Message::ChooseFolder => {
                return cosmic::task::future(async move {
                    let r = cosmic::dialog::file_chooser::open::Dialog::new()
                        .title("Choose the AppImages Folder")
                        .open_folder()
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| {
                            r.url()
                                .to_file_path()
                                .map_err(|_| "Choose a local folder".to_string())
                        });
                    Message::FolderPicked(r)
                });
            }
            Message::FolderPicked(r) => match r {
                Ok(dir) => {
                    let probe = dir.join(format!(
                        ".gearlever-cosmic-write-test-{}",
                        std::process::id()
                    ));
                    if std::fs::write(&probe, b"").is_ok() {
                        let _ = std::fs::remove_file(&probe);
                        let mut s = self.settings.clone();
                        s.appimages_folder = match dir.strip_prefix(&self.paths.home) {
                            Ok(rest) => format!("~/{}", rest.display()),
                            Err(_) => dir.display().to_string(),
                        };
                        return self.save_settings(s);
                    }
                    self.notice = Some(format!("{} is not writable", dir.display()));
                }
                Err(e) if !e.to_lowercase().contains("cancel") => self.notice = Some(e),
                Err(_) => {}
            },
            Message::ImportGearLever => {
                let ctx = automation::Context {
                    paths: self.paths.clone(),
                    settings: self.settings.clone(),
                };
                let req =
                    automation::Request::GearLeverImport(automation::Targets { apps: vec![] });
                if let Ok(ops) =
                    automation::prepare(&req, &ctx, &self.inventory, &automation::Policy::default())
                {
                    return self.run_ops(ops);
                }
            }
            Message::Settings => self.set_show_context(!self.core.window.show_context),
            Message::CloseSettings => self.set_show_context(false),
            Message::About => self.dialog = Some(Dialog::About),
            Message::ClearNotice => self.notice = None,
            Message::ClearOutcome => self.outcome = None,
            Message::Copy(text) => {
                let t = self.toast("Copied to clipboard");
                return Task::batch([
                    cosmic::iced::clipboard::write(text).map(cosmic::Action::App),
                    t,
                ]);
            }
            Message::CloseToast(id) => self.toasts.remove(id),
            Message::Key(key, m) => {
                use iced::keyboard::{Key, key::Named};
                if self.dialog.is_some() {
                    return Task::none();
                }
                match (key.as_ref(), m.control()) {
                    (Key::Character("o"), true) => return self.update(Message::OpenFiles),
                    (Key::Character("f"), true) => {
                        let t = self.switch(Page::Apps);
                        return Task::batch([
                            t,
                            widget::text_input::focus(widget::Id::new("search")),
                        ]);
                    }
                    (Key::Character("r"), true) | (Key::Named(Named::F5), _) => {
                        return self.update(Message::Refresh);
                    }
                    (Key::Character(","), true) => return self.update(Message::Settings),
                    (Key::Character("1"), true) => return self.switch(Page::Apps),
                    (Key::Character("2"), true) => return self.switch(Page::Updates),
                    (Key::Character("3"), true) => return self.switch(Page::Add),
                    _ => {}
                }
            }
            Message::WindowChanged(id) => {
                if self.core.main_window_id() != Some(id) {
                    return Task::none();
                }
                self.window_request += 1;
                let request = self.window_request;
                return iced::window::mode(id)
                    .then(move |mode| {
                        iced::window::is_maximized(id).then(move |max| {
                            iced::window::size(id)
                                .map(move |size| Message::WindowObserved(request, mode, max, size))
                        })
                    })
                    .map(cosmic::Action::App);
            }
            Message::WindowObserved(request, mode, maximized, size) => {
                if request < self.window_observed || mode == iced::window::Mode::Hidden {
                    return Task::none();
                }
                self.window_observed = request;
                self.width = size.width;
                let mode = if mode == iced::window::Mode::Fullscreen {
                    window_state::Mode::Fullscreen
                } else if maximized {
                    window_state::Mode::Maximized
                } else {
                    window_state::Mode::Windowed
                };
                if self.initial_window_mode.is_some_and(|e| e != mode)
                    && self.window_started.elapsed() < Duration::from_secs(1)
                {
                    if let (Some(e), Some(w)) =
                        (self.initial_window_mode, self.core.main_window_id())
                    {
                        return window_state::restore(e, w).map(cosmic::Action::App);
                    }
                    return Task::none();
                }
                self.initial_window_mode = None;
                self.window_state.observe(mode, size.width, size.height);
            }
        }
        Task::none()
    }
}

impl App {
    /// `--preview PAGE` opens a page or dialog for captures.
    fn preview(&mut self, preview: String) -> Task<Message> {
        let first = self.inventory.apps.first().map(|a| a.id.clone());
        match preview.trim_end_matches("-narrow") {
            "updates" => self.switch(Page::Updates),
            "add" => self.switch(Page::Add),
            "drop" => {
                self.drag_over = true;
                self.switch(Page::Add)
            }
            "settings" => {
                self.set_show_context(true);
                Task::none()
            }
            "about" => {
                self.dialog = Some(Dialog::About);
                Task::none()
            }
            "app" | "remove" => {
                let t = first.map(|id| self.select(id)).unwrap_or_else(Task::none);
                if preview.starts_with("remove") {
                    let apps: Vec<_> = self.selected_app().cloned().into_iter().collect();
                    self.dialog = Some(Dialog::Remove(apps));
                }
                t
            }
            _ => Task::none(),
        }
    }
}
