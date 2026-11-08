// SPDX-License-Identifier: GPL-3.0-or-later
//! GUI automation uses the same messages as visible controls. `ui_controls`
//! lists every control a person could click now, with a label and whether it
//! needs --apply (changes files, or starts another program).
use super::*;
use crate::ui_bridge::{Node, Pending as UiPending, Tree};
use cosmic::Application;
use gearlever_cosmic::ui_protocol::{self as protocol, Request};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Guard {
    None,
    Apply,
}
pub(super) struct Control {
    pub id: String,
    pub label: String,
    pub message: Message,
    pub guard: Guard,
}
fn c(id: impl Into<String>, label: impl Into<String>, message: Message) -> Control {
    Control {
        id: id.into(),
        label: label.into(),
        message,
        guard: Guard::None,
    }
}
fn g(id: impl Into<String>, label: impl Into<String>, message: Message) -> Control {
    Control {
        id: id.into(),
        label: label.into(),
        message,
        guard: Guard::Apply,
    }
}

impl App {
    pub(super) fn ui_fields(&self) -> Vec<(String, String)> {
        let mut f = vec![
            ("search".to_string(), "Filter installed apps".to_string()),
            (
                "nav.width".to_string(),
                format!(
                    "Sidebar width in logical pixels (now {:.0}; draggable)",
                    self.clamp_nav_width(self.nav_width)
                ),
            ),
            (
                "list.width".to_string(),
                format!(
                    "App list width in logical pixels (now {:.0}; the divider is draggable)",
                    self.clamp_list_width(self.list_width)
                ),
            ),
        ];
        if self.page == Page::Apps && self.selected_app().is_some() {
            f.push(("app.args".into(), "Command line arguments".into()));
            f.push(("app.website".into(), "Website".into()));
            f.push(("icon.path".into(), "Icon file path for app.icon.set".into()));
            for i in 0..self.env_rows.len() {
                f.push((
                    format!("app.env.{i}.key"),
                    format!("Environment variable {i} name"),
                ));
                f.push((
                    format!("app.env.{i}.value"),
                    format!("Environment variable {i} value"),
                ));
            }
            if self.source_choice > 0 {
                for (name, d) in MANAGERS[self.source_choice - 1].fields {
                    if *name != "allow_prereleases" {
                        f.push((format!("app.source.{name}"), d.to_string()));
                    }
                }
            }
        }
        f
    }
    fn field_message(&self, target: &str, text: String) -> Option<Message> {
        if target == "nav.width" {
            return text.trim().parse::<f32>().ok().map(Message::NavWidth);
        }
        if target == "list.width" {
            return text.trim().parse::<f32>().ok().map(Message::ListWidth);
        }
        let field = match target {
            "search" => Field::Search,
            "app.args" => Field::Args,
            "app.website" => Field::Website,
            "icon.path" => Field::IconPath,
            t => {
                if let Some(rest) = t.strip_prefix("app.env.") {
                    let (i, part) = rest.split_once('.')?;
                    let i: usize = i.parse().ok()?;
                    match part {
                        "key" => Field::EnvKey(i),
                        "value" => Field::EnvValue(i),
                        _ => return None,
                    }
                } else {
                    Field::Source(t.strip_prefix("app.source.")?.into())
                }
            }
        };
        self.ui_fields()
            .iter()
            .any(|(id, _)| id == target)
            .then_some(Message::Field(field, text))
    }

    pub(super) fn ui_controls(&self) -> Vec<Control> {
        let busy = self.busy();
        let mut list = vec![];
        if busy {
            list.push(c(
                "operation.cancel",
                "Cancel the running operation",
                Message::Cancel,
            ));
        }
        if let Some(d) = &self.dialog {
            list.push(c("dialog.cancel", "Cancel or close", Message::CloseDialog));
            match d {
                Dialog::Remove(apps) => list.push(g(
                    "dialog.confirm",
                    format!(
                        "Remove {} (to the Trash)",
                        apps.iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Message::Confirm,
                )),
                Dialog::Confirm(op, _) => {
                    list.push(g("dialog.confirm", op.title(), Message::Confirm))
                }
                Dialog::About => {}
            }
            return list;
        }
        list.push(c("header.logo", "About Gear Lever COSMIC", Message::About));
        list.push(c("header.about", "About Gear Lever COSMIC", Message::About));
        list.push(c("header.settings", "Settings", Message::Settings));
        list.push(c(
            "header.open",
            "Open AppImage… (file chooser; use `ui open FILE` instead)",
            Message::OpenFiles,
        ));
        if !self.loading && !busy {
            list.push(c("header.refresh", "Refresh", Message::Refresh));
        }
        for p in Page::ALL {
            list.push(c(format!("nav.{}", p.slug()), p.name(), Message::Page(p)));
        }
        if self.notice.is_some() {
            list.push(c(
                "notice.dismiss",
                "Dismiss the message",
                Message::ClearNotice,
            ));
        }
        if self.outcome.is_some() {
            list.push(c(
                "result.dismiss",
                "Dismiss the last result",
                Message::ClearOutcome,
            ));
        }
        if self.core.window.show_context {
            self.settings_controls(&mut list);
        }
        match self.page {
            Page::Apps => self.app_controls(&mut list, busy),
            Page::Updates => {
                if !self.checking_all {
                    list.push(c(
                        "updates.check",
                        "Check all apps for updates",
                        Message::CheckAll,
                    ));
                }
                let n = self
                    .inventory
                    .apps
                    .iter()
                    .filter(|a| {
                        self.checks
                            .get(&a.id)
                            .is_some_and(|c| c.status == Status::Available && c.can_install)
                    })
                    .count();
                if n > 0 && !busy {
                    list.push(g(
                        "updates.all",
                        format!("Update all ({n})"),
                        Message::UpdateAll,
                    ));
                }
                for a in &self.inventory.apps {
                    if let Some(ch) = self.checks.get(&a.id) {
                        if ch.status == Status::Available && ch.can_install && !busy {
                            list.push(g(
                                format!("updates.{}.update", a.id),
                                format!("Update {}", a.name),
                                Message::UpdateApp(a.id.clone()),
                            ));
                        }
                        if (ch.status == Status::SelfUpdating
                            || (ch.status == Status::Available && !ch.can_install))
                            && !busy
                        {
                            list.push(g(
                                format!("updates.{}.launch", a.id),
                                format!("Open {} so it updates itself", a.name),
                                Message::Launch(a.id.clone()),
                            ));
                        }
                        list.push(c(
                            format!("updates.{}.details", a.id),
                            format!("Details of {}", a.name),
                            Message::Select(a.id.clone()),
                        ));
                    }
                }
            }
            Page::Add => {
                list.push(c(
                    "add.open",
                    "Open AppImage… (file chooser; use `ui open FILE` instead)",
                    Message::OpenFiles,
                ));
                for (i, p) in self.pending.iter().enumerate() {
                    let name = p
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    list.push(c(
                        format!("add.{i}.dismiss"),
                        format!("Remove {name} from the list"),
                        Message::DismissPending(i),
                    ));
                    if !matches!(p.meta, Some(Ok(_))) {
                        continue;
                    }
                    if p.hashes.is_none() {
                        list.push(c(
                            format!("add.{i}.hashes"),
                            format!("Checksums of {name}"),
                            Message::PendingHashes(i),
                        ));
                    }
                    if !p.others.is_empty() && p.same_file.is_none() {
                        list.push(c(
                            format!("add.{i}.keep-both"),
                            format!("Keep both versions of {name}"),
                            Message::PendingConflict(i, Conflict::KeepBoth),
                        ));
                        list.push(c(
                            format!("add.{i}.replace"),
                            format!("Replace the integrated version with {name}"),
                            Message::PendingConflict(i, Conflict::Replace),
                        ));
                    }
                    if !busy {
                        list.push(g(
                            format!("add.{i}.run"),
                            format!("Run {name} without adding it (starts it)"),
                            Message::RunPending(i),
                        ));
                        if p.same_file.is_none()
                            && (p.others.is_empty() || p.conflict != Conflict::Ask)
                        {
                            list.push(g(
                                format!("add.{i}.integrate"),
                                format!("Add {name} to the app menu"),
                                Message::Integrate(vec![i]),
                            ));
                        }
                    }
                }
                if self.pending.len() > 1 {
                    list.push(c(
                        "add.verified",
                        if self.verified {
                            "Uncheck \"I have verified the source\""
                        } else {
                            "Check \"I have verified the source of these apps\""
                        },
                        Message::Verified(!self.verified),
                    ));
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
                    if self.verified && !busy && !ready.is_empty() {
                        list.push(g(
                            "add.all",
                            format!("Add all to the app menu ({})", ready.len()),
                            Message::Integrate(ready),
                        ));
                    }
                }
            }
        }
        list
    }

    fn app_controls(&self, list: &mut Vec<Control>, busy: bool) {
        for a in self.visible_apps() {
            list.push(c(
                format!("apps.select:{}", a.id),
                format!("Show {}", a.name),
                Message::Select(a.id.clone()),
            ));
        }
        let Some(a) = self.selected_app() else { return };
        if self.detail_open {
            list.push(c(
                "apps.back",
                "Back to the list (narrow windows)",
                Message::Back,
            ));
        }
        let id = a.id.clone();
        list.push(c(
            "app.copy-path",
            "Copy the AppImage path",
            Message::Copy(a.path.display().to_string()),
        ));
        list.push(g(
            "app.open-folder",
            "Open the AppImage's folder in the file manager",
            Message::OpenFolder(id.clone()),
        ));
        if !self.hashes.contains_key(&a.path) {
            list.push(c(
                "app.hashes",
                "Show checksums",
                Message::ShowHashes(a.path.clone()),
            ));
        }
        if !self.checking.contains(&id) {
            list.push(c(
                "app.check",
                format!("Check {} for updates", a.name),
                Message::CheckApp(id.clone()),
            ));
        }
        if let Some(w) = &a.website {
            list.push(g(
                "app.website.open",
                format!("Open {w}"),
                Message::OpenWebsite(w.clone()),
            ));
        }
        for (i, m) in std::iter::once("Default")
            .chain(MANAGERS.iter().map(|m| m.label))
            .enumerate()
        {
            list.push(c(
                format!("app.source:{i}"),
                format!("Update source: {m}"),
                Message::SourceChoice(i),
            ));
        }
        if self.source_choice > 0
            && MANAGERS[self.source_choice - 1]
                .fields
                .iter()
                .any(|(f, _)| *f == "allow_prereleases")
        {
            list.push(c(
                "app.prerelease",
                if self.prerelease {
                    "Disallow pre-releases"
                } else {
                    "Allow pre-releases"
                },
                Message::Prerelease(!self.prerelease),
            ));
        }
        list.push(c(
            "app.env.add",
            "Add an environment variable row",
            Message::AddEnv,
        ));
        for i in 0..self.env_rows.len() {
            list.push(c(
                format!("app.env.{i}.remove"),
                format!("Remove environment row {i}"),
                Message::RemoveEnv(i),
            ));
        }
        if a.arguments != a.default_arguments && !a.default_arguments.is_empty() {
            list.push(c(
                "app.args.reset",
                "Put the app's default arguments in the field",
                Message::ResetArgs,
            ));
        }
        if busy {
            return;
        }
        list.push(g(
            "app.launch",
            format!("Launch {}", a.name),
            Message::Launch(id.clone()),
        ));
        list.push(c(
            "app.remove",
            format!("Remove {} (asks first)", a.name),
            Message::AskRemove(vec![id.clone()]),
        ));
        if let Some(ch) = self.checks.get(&id)
            && ch.status == Status::Available
            && ch.can_install
        {
            list.push(g(
                "app.update",
                format!(
                    "Update {}{}",
                    a.name,
                    ch.available_version
                        .as_ref()
                        .map(|v| format!(" to {v}"))
                        .unwrap_or_default()
                ),
                Message::UpdateApp(id.clone()),
            ));
        }
        if !a.menu_outdated.is_empty() {
            list.push(g(
                "app.sync-menu",
                "Refresh the outdated menu entry",
                Message::SyncMenu(vec![id.clone()]),
            ));
        }
        list.push(g(
            "app.reload",
            "Reload metadata into the menu entry",
            Message::SyncMenu(vec![id.clone()]),
        ));
        list.push(g(
            "app.args.save",
            "Save the arguments field",
            Message::SaveArgs,
        ));
        list.push(g(
            "app.env.save",
            "Save the environment rows",
            Message::SaveEnv,
        ));
        list.push(g(
            "app.website.save",
            "Save the website field",
            Message::SaveWebsite,
        ));
        list.push(g(
            "app.source.save",
            "Save the selected update source",
            Message::SaveSource,
        ));
        list.push(c(
            "app.icon.choose",
            "Choose an icon… (file chooser; use fill icon.path + app.icon.set instead)",
            Message::ChooseIcon,
        ));
        list.push(g(
            "app.icon.set",
            "Use the image in the icon.path field as the icon",
            Message::SetIconFromField,
        ));
        if let Some(t) = &a.standard_path {
            list.push(g(
                "app.rename-file",
                format!(
                    "Rename the file to its standard name {}",
                    t.file_name().unwrap_or_default().to_string_lossy()
                ),
                Message::RenameFile(id.clone()),
            ));
        }
        if a.icon_custom {
            list.push(g(
                "app.icon.reset",
                "Use the official icon again",
                Message::ResetIcon,
            ));
        }
    }

    fn settings_controls(&self, list: &mut Vec<Control>) {
        let s = &self.settings;
        type Toggle = (&'static str, &'static str, bool, fn(bool) -> SettingChange);
        let toggles: [Toggle; 8] = [
            (
                "settings.show-external",
                "Show apps outside the folder",
                s.show_external,
                SettingChange::ShowExternal,
            ),
            (
                "settings.preview-minimal",
                "Compact preview",
                s.preview_minimal,
                SettingChange::PreviewMinimal,
            ),
            (
                "settings.terminal-names",
                "Name terminal apps after their executable",
                s.terminal_apps_use_executable_name,
                SettingChange::TerminalNames,
            ),
            (
                "settings.safe-metadata",
                "Use safe mechanisms to load metadata",
                s.safe_metadata,
                SettingChange::SafeMetadata,
            ),
            (
                "settings.check-on-startup",
                "Check for updates after login",
                s.check_updates_on_startup,
                SettingChange::CheckOnStartup,
            ),
            (
                "settings.keep-replaced",
                "Keep replaced versions in the Trash",
                s.keep_replaced_versions_in_trash,
                SettingChange::KeepReplaced,
            ),
            (
                "settings.sync-menus",
                "Keep menu entries in sync",
                s.sync_menu_entries,
                SettingChange::SyncMenus,
            ),
            (
                "settings.debug-logs",
                "Debug logs",
                s.debug_logs,
                SettingChange::DebugLogs,
            ),
        ];
        for (id, label, on, f) in toggles {
            list.push(g(
                id,
                format!("{label}: turn {}", if on { "off" } else { "on" }),
                Message::Setting(f(!on)),
            ));
        }
        list.push(g(
            "settings.move",
            "Move files into the folder when adding",
            Message::Setting(SettingChange::MoveOnIntegrate(true)),
        ));
        list.push(g(
            "settings.copy",
            "Copy files into the folder when adding",
            Message::Setting(SettingChange::MoveOnIntegrate(false)),
        ));
        for (name, t) in [
            ("system", Theme::System),
            ("dark", Theme::Dark),
            ("light", Theme::Light),
        ] {
            list.push(c(
                format!("settings.theme:{name}"),
                format!("Theme: {name}"),
                Message::Setting(SettingChange::Theme(t)),
            ));
        }
        list.push(c(
            "settings.folder",
            "Choose the AppImages folder (file chooser)",
            Message::ChooseFolder,
        ));
        if !self.busy()
            && self
                .inventory
                .apps
                .iter()
                .any(|a| a.update_source_origin == Some(updates::Origin::GearLever))
        {
            list.push(g(
                "settings.import",
                "Import Gear Lever's update sources",
                Message::ImportGearLever,
            ));
        }
        list.push(c(
            "settings.close",
            "Close Settings",
            Message::CloseSettings,
        ));
    }

    fn ui_page(&self) -> Value {
        match self.page {
            Page::Apps => json!({
                "apps": self.visible_apps().iter().map(|a| json!({"id":a.id,"name":a.name,"version":a.version,"menu_version":a.menu_version,"menu_outdated":a.menu_outdated,
                    "running":self.running.get(&a.id),"update":self.checks.get(&a.id).map(|c| json!({"status":c.status,"label":status_text(c)})),
                    "requirements":self.requirements.get(&a.id).and_then(|r| r.as_ref().ok()).map(|r| &r.verdict)})).collect::<Vec<_>>(),
                "selected": self.selected_app().map(|a| json!({
                    "app": a, "check": self.checks.get(&a.id), "checking": self.checking.contains(&a.id),
                    "requirements": self.requirements.get(&a.id).map(|r| match r { Ok(r) => json!(r), Err(e) => json!({"error":e}) }),
                    "hashes": self.hashes.get(&a.path).and_then(|h| h.as_ref().ok()),
                    "edits": {"arguments":self.args_text,"env":self.env_rows,"website":self.website_text,
                        "source_choice": if self.source_choice == 0 { "Default".to_string() } else { MANAGERS[self.source_choice - 1].name.to_string() },
                        "source_fields":self.source_fields,"allow_prereleases":self.prerelease,"icon_path":self.icon_path,"error":self.edit_error}})),
                "detail_open": self.detail_open,
                "list_width": self.clamp_list_width(self.list_width),
                "nav_width": self.clamp_nav_width(self.nav_width),
                "split_layout": self.content_width() >= MIN_SPLIT,
            }),
            Page::Updates => json!({"checking":self.checking_all,"checked":self.checked_all_once,
                "results": self.inventory.apps.iter().map(|a| json!({"id":a.id,"name":a.name,"version":a.version,"check":self.checks.get(&a.id)})).collect::<Vec<_>>()}),
            Page::Add => {
                json!({"verified":self.verified,"pending": self.pending.iter().map(|p| json!({
                "path":p.path,"loading":p.meta.is_none(),
                "metadata": p.meta.as_ref().map(|m| match m { Ok(m) => json!({"name":m.name,"version":m.version,"arch":m.arch,"type":m.appimage_type,"update_info":m.update_info,"self_updater":m.self_updater,"terminal":m.terminal}), Err(e) => json!({"error":e}) }),
                "requirements":p.requirements,"already_integrated":p.same_file,"other_versions":p.others,
                "conflict":p.conflict,"hashes":p.hashes.as_ref().and_then(|h| h.as_ref().ok())})).collect::<Vec<_>>()})
            }
        }
    }

    pub(super) fn ui_state(&self) -> Value {
        let controls = self.ui_controls();
        let operation = self.job.as_ref().map(|j| {
            let p = j.job.progress.lock().map(|p| p.clone()).unwrap_or_default();
            json!({"title":j.title,"step":p.step,"percent":p.fraction.map(|f| (f * 100.).round()),"done":j.done,"total":j.total})
        });
        json!({"instance":self.ui_instance,"process_id":std::process::id(),"page":self.page.name(),"page_id":self.page.slug(),
            "loading":self.loading,"busy":self.busy(),"checking":self.checking_all || !self.checking.is_empty(),
            "settings_open":self.core.window.show_context,"settings":self.settings,"rendered_theme_dark":cosmic::theme::active().theme_type.is_dark(),
            "dialog":self.dialog.as_ref().map(|d| match d {
                Dialog::Remove(apps) => json!({"kind":"remove","apps":apps.iter().map(|a| json!({"id":a.id,"name":a.name,"path":a.path})).collect::<Vec<_>>(),
                    "explanation":"The AppImage, its menu entry and its icon move to the Trash."}),
                Dialog::Confirm(op, plan) => json!({"kind":"confirm","title":op.title(),"plan":plan}),
                Dialog::About => json!({"kind":"about"}),
            }),
            "operation":operation,
            "last_operation":self.outcome.as_ref().map(|o| json!({"title":o.title,"ok":o.ok,"steps":o.steps,"notes":o.notes,"errors":o.errors,"report":o.log})),
            "notice":self.notice,"drag_over":self.drag_over,"search":self.query,"window":self.window_state,"sidebar_visible":self.core.nav_bar_active(),
            "view":self.ui_page(),
            "fields":self.ui_fields().iter().map(|(id, label)| json!({"id":id,"label":label})).collect::<Vec<_>>(),
            "controls":controls.iter().map(|c| json!({"id":c.id,"label":c.label,"requires":if c.guard == Guard::Apply { "apply" } else { "none" }})).collect::<Vec<_>>()})
    }

    pub(super) fn ui_activate(&self, target: &str, apply: bool) -> Result<Message, String> {
        let control = self
            .ui_controls()
            .into_iter()
            .find(|c| c.id == target)
            .ok_or_else(|| format!("Control {target:?} is not available in the current view; run `ui inspect` for the list"))?;
        if control.guard == Guard::Apply && !apply {
            return Err(format!("{} requires --apply", control.label));
        }
        Ok(control.message)
    }

    pub(super) fn ui_request(&mut self, pending: UiPending) -> Task<Message> {
        let failure = |message: String| {
            pending.respond(protocol::error(message));
            Task::none()
        };
        match pending.envelope.request.clone() {
            Request::Present => {
                pending.respond(protocol::success(json!({"state":self.ui_state()})));
                self.present_window()
            }
            Request::ActivationToken => {
                let Some(window) = self.core.main_window_id() else {
                    return failure("No main window".into());
                };
                iced::platform_specific::shell::commands::activation::request_token(
                    Some(Self::APP_ID.into()),
                    Some(window),
                )
                .map(move |token| {
                    cosmic::Action::App(Message::UiActivationToken(pending.clone(), token))
                })
            }
            Request::Inspect | Request::Tree => iced::advanced::widget::operate(Tree {
                target: None,
                nodes: vec![],
            })
            .map(move |nodes| cosmic::Action::App(Message::UiTree(pending.clone(), nodes, false))),
            Request::Resolve { target } => iced::advanced::widget::operate(Tree {
                target: Some(widget::Id::new(target)),
                nodes: vec![],
            })
            .map(move |nodes| cosmic::Action::App(Message::UiTree(pending.clone(), nodes, true))),
            Request::Activate { target } => {
                if target == "escape" {
                    let t = self.on_escape();
                    pending.respond(protocol::success(json!({"state":self.ui_state()})));
                    return t;
                }
                match self.ui_activate(&target, pending.envelope.apply) {
                    Ok(m) => {
                        let t = self.update(m);
                        pending.respond(protocol::success(json!({"state":self.ui_state()})));
                        t
                    }
                    Err(e) => failure(e),
                }
            }
            Request::Open { files } => {
                if files.is_empty() || files.iter().any(|p| !p.is_absolute() || !p.is_file()) {
                    return failure("Give existing absolute file paths".into());
                }
                let t = self.open_files(files);
                pending.respond(protocol::success(json!({"state":self.ui_state()})));
                t
            }
            Request::Drop { uris } => {
                let t = self.update(Message::Dropped(
                    "text/uri-list".into(),
                    uris.join("\r\n").into_bytes(),
                ));
                pending.respond(protocol::success(json!({"state":self.ui_state()})));
                t
            }
            Request::Fill { target, text } => match self.field_message(&target, text) {
                Some(m) => {
                    let t = self.update(m);
                    pending.respond(protocol::success(json!({"state":self.ui_state()})));
                    t
                }
                None => failure(format!(
                    "Unknown text field {target}; fields now: {}",
                    self.ui_fields()
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            },
            Request::Theme { theme } => {
                let t = match theme.as_str() {
                    "system" | "desktop" => Theme::System,
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    _ => return failure("Use system, dark or light".into()),
                };
                let task = self.update(Message::Setting(SettingChange::Theme(t)));
                pending.respond(protocol::success(json!({"state":self.ui_state()})));
                task
            }
            Request::Capture { path, overwrite } => {
                if !path.is_absolute() || (!overwrite && path.exists()) {
                    return failure(
                        "Capture needs a new absolute output path, or --overwrite".into(),
                    );
                }
                let Some(window) = self.core.main_window_id() else {
                    return failure("No main window".into());
                };
                iced::window::screenshot(window).map(move |shot| {
                    cosmic::Action::App(Message::UiCaptured(
                        pending.clone(),
                        path.clone(),
                        overwrite,
                        shot,
                    ))
                })
            }
            Request::Window {
                width,
                height,
                mode,
            } => {
                self.initial_window_mode = None;
                let Some(window) = self.core.main_window_id() else {
                    return failure("No main window".into());
                };
                let task: iced::Task<Message> = if let Some(mode) = mode {
                    match mode.as_str() {
                        "windowed" => iced::Task::batch([
                            iced::window::set_mode(window, iced::window::Mode::Windowed),
                            iced::window::maximize(window, false),
                        ]),
                        "maximized" => iced::Task::batch([
                            iced::window::set_mode(window, iced::window::Mode::Windowed),
                            iced::window::maximize(window, true),
                        ]),
                        "fullscreen" => {
                            iced::window::set_mode(window, iced::window::Mode::Fullscreen)
                        }
                        _ => return failure("Use windowed, maximized or fullscreen".into()),
                    }
                } else {
                    use crate::window_state::{MIN_HEIGHT, MIN_WIDTH};
                    let w = width.unwrap_or(self.window_state.width);
                    let h = height.unwrap_or(self.window_state.height);
                    if !(MIN_WIDTH..=16384).contains(&w) || !(MIN_HEIGHT..=16384).contains(&h) {
                        return failure(format!(
                            "Window size must be at least {MIN_WIDTH}x{MIN_HEIGHT}"
                        ));
                    }
                    iced::window::resize(window, iced::Size::new(w as f32, h as f32))
                };
                pending.respond(protocol::success(
                    json!({"accepted":true,"state":self.ui_state()}),
                ));
                task.map(cosmic::Action::App)
            }
            Request::Close => {
                if !pending.envelope.apply {
                    return failure("Closing the window requires --apply".into());
                }
                if self.busy() {
                    return failure("Wait for or cancel the running operation first".into());
                }
                self.save_window();
                pending.respond(protocol::success(
                    json!({"closing":true,"instance":self.ui_instance}),
                ));
                cosmic::task::future(async {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    Message::UiCloseNow
                })
            }
        }
    }

    pub(super) fn ui_tree(&self, pending: UiPending, nodes: Vec<Node>, resolve: bool) {
        if resolve {
            match nodes.first() {
                Some(n) => pending.respond(protocol::success(json!({"bounds":n.bounds}))),
                None => pending.respond(protocol::error(
                    "The target has no rendered widget in this view",
                )),
            }
        } else {
            pending.respond(protocol::success(
                json!({"state":self.ui_state(),"tree":nodes}),
            ));
        }
    }

    pub(super) fn ui_capture(
        &self,
        pending: UiPending,
        path: PathBuf,
        overwrite: bool,
        shot: iced::window::Screenshot,
    ) {
        let result = (|| -> Result<(), String> {
            let image =
                image::RgbaImage::from_raw(shot.size.width, shot.size.height, shot.rgba.to_vec())
                    .ok_or("Invalid renderer screenshot")?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true);
            if overwrite {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options.open(&path).map_err(|e| e.to_string())?;
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut file, image::ImageFormat::Png)
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(()) => pending.respond(protocol::success(
                json!({"path":path,"width":shot.size.width,"height":shot.size.height}),
            )),
            Err(e) => pending.respond(protocol::error(e)),
        }
    }
}
