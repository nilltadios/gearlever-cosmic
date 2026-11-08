// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit controls for a running GUI. Backend commands remain headless.
use crate::ui_protocol::{self as protocol, Envelope, Request};
use clap::Subcommand;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Exact instance returned by `ui instances` or `ui start`.
    #[arg(long, global = true)]
    pub instance: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List these GUI commands with descriptions and the request schema.
    Commands,
    /// Print the JSON Schema of GUI requests.
    Schema,
    /// List running GUI instances.
    Instances,
    /// Current page, dialog, page content, text fields and every available control.
    Inspect,
    /// Only the available controls: id, label and required flags.
    Controls,
    /// Inspection plus the rendered widget tree, visible text and bounds.
    Tree,
    /// Obtain a desktop activation token for an explicit launch/activation test.
    ActivationToken,
    /// Activate the same action handler as the named GUI control.
    #[command(alias = "click")]
    Activate {
        target: String,
        #[arg(long)]
        apply: bool,
    },
    /// Show a page: apps, updates or add.
    Navigate { page: String },
    /// Open files as if chosen in Open Files; shows review without installing.
    Open {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Deliver files to the window's drag-and-drop handler as text/uri-list.
    Drop {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Type into a text field listed under `fields` in inspect.
    Fill {
        target: String,
        /// May start with a hyphen, e.g. "--verbose %U".
        #[arg(allow_hyphen_values = true)]
        text: String,
    },
    /// Theme: system, dark or light.
    Theme { theme: String },
    /// Change window mode or logical size.
    Window {
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
    },
    /// Save the window's own render to a new PNG.
    Capture {
        path: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Wait until inventory, detail reads and owned operations finish.
    Wait {
        #[arg(long, default_value_t = 15)]
        timeout: u64,
    },
    /// Present the running window, or start one; --config-dir starts an isolated test instance.
    Start {
        #[arg(long)]
        config_dir: Option<PathBuf>,
    },
    /// Close the selected GUI window (requires --apply).
    Close {
        #[arg(long)]
        apply: bool,
    },
}
const COMMANDS: [(&str, &str); 17] = [
    (
        "instances",
        "List running GUI instances; pass one with --instance when several run",
    ),
    (
        "inspect",
        "State of the visible window: page, dialog, page content (view), text fields and controls with labels and required flags",
    ),
    ("controls", "Only the controls that can be activated now"),
    (
        "tree",
        "Inspection plus rendered widgets with visible text and bounds",
    ),
    (
        "activate",
        "Activate a control id from inspect (alias: click). Changes and starting other programs need --apply",
    ),
    ("navigate", "Show apps, updates or add"),
    (
        "open",
        "Open package, repo or snapshot files for review; nothing is applied",
    ),
    (
        "drop",
        "Send files through the drag-and-drop handler; nothing is applied",
    ),
    ("fill", "Type into a text field listed under fields"),
    ("theme", "system, dark or light"),
    (
        "window",
        "Set --mode windowed|maximized|fullscreen or --width/--height",
    ),
    (
        "capture",
        "Save the window's render to a new absolute PNG path",
    ),
    ("wait", "Wait until loading, details and operations finish"),
    (
        "start",
        "Present the running GUI, or start an isolated one with --config-dir",
    ),
    (
        "activation-token",
        "Request a Wayland activation token for launch tests",
    ),
    ("close", "Close the selected window with --apply"),
    ("schema", "JSON Schema of GUI requests"),
];

pub fn connection() -> Result<zbus::Connection, String> {
    zbus::block_on(zbus::Connection::session())
        .map_err(|e| format!("Desktop D-Bus is unavailable: {e}"))
}
pub fn desktop_proxy<'a>(
    connection: &'a zbus::Connection,
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<zbus::Proxy<'a>, String> {
    zbus::block_on(zbus::Proxy::new(connection, destination, path, interface))
        .map_err(|error| error.to_string())
}
pub fn instances(connection: &zbus::Connection) -> Result<Vec<String>, String> {
    let proxy = crate::ui_client::desktop_proxy(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(|e| e.to_string())?;
    let names: Vec<String> =
        zbus::block_on(proxy.call("ListNames", &())).map_err(|e| e.to_string())?;
    let mut names: Vec<_> = names
        .into_iter()
        .filter(|name| name.starts_with(protocol::PREFIX))
        .collect();
    names.sort();
    Ok(names)
}
pub fn call(
    connection: &zbus::Connection,
    instance: &str,
    request: Request,
    apply: bool,
    allow_system: bool,
) -> Result<Value, String> {
    if !is_own_instance(instance) {
        return Err("Invalid Gear Lever COSMIC window instance".into());
    }
    let proxy =
        crate::ui_client::desktop_proxy(connection, instance, protocol::PATH, protocol::INTERFACE)
            .map_err(|e| e.to_string())?;
    let input = serde_json::to_string(&Envelope {
        request,
        apply,
        allow_system,
    })
    .map_err(|e| e.to_string())?;
    let reply: String =
        zbus::block_on(proxy.call("Execute", &(input.as_str(),))).map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
    if reply["ok"] != true {
        return Err(reply["error"]
            .as_str()
            .unwrap_or("GUI command failed")
            .into());
    }
    Ok(reply)
}
/// Only this app's own window names may be addressed; see AGENTS.md
/// ("The CLI never touches other apps").
pub fn is_own_instance(name: &str) -> bool {
    name.strip_prefix(protocol::PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('.'))
}
pub fn new_instance() -> String {
    format!(
        "{}i{}",
        protocol::PREFIX,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
pub fn selected(
    connection: &zbus::Connection,
    requested: Option<String>,
) -> Result<String, String> {
    let available = instances(connection)?;
    if let Some(instance) = requested {
        if available.contains(&instance) {
            return Ok(instance);
        }
        return Err("That GUI instance is not running; use `ui instances`".into());
    }
    match available.as_slice() {
        [instance] => Ok(instance.clone()),
        [] => Err("No Gear Lever COSMIC window is running; use `ui start`".into()),
        _ => Err(
            "Several GUI instances are running. Select one with --instance from `ui instances`"
                .into(),
        ),
    }
}
fn wait_for(
    connection: &zbus::Connection,
    instance: &str,
    timeout: u64,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let end = Instant::now() + Duration::from_secs(timeout);
    loop {
        let value = call(connection, instance, Request::Inspect, false, false)?;
        if predicate(&value["data"]["state"]) {
            return Ok(value);
        }
        if Instant::now() >= end {
            return Err("Timed out waiting for the GUI state".into());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}
pub fn run(args: Args) -> Result<Value, String> {
    match &args.command {
        Command::Commands => {
            return Ok(protocol::success(
                json!({"commands":COMMANDS.map(|(name, _)| name),
                    "descriptions":COMMANDS.iter().map(|(name, description)| json!({"command":name,"description":description})).collect::<Vec<_>>(),
                    "workflow":["ui instances","ui --instance ID inspect","ui --instance ID activate CONTROL","ui --instance ID wait","ui --instance ID inspect"],
                    "request_schema":schemars::schema_for!(Envelope)}),
            ));
        }
        Command::Schema => {
            return serde_json::to_value(schemars::schema_for!(Envelope))
                .map_err(|e| e.to_string());
        }
        _ => (),
    }
    let connection = connection()?;
    if matches!(args.command, Command::Instances) {
        return Ok(protocol::success(
            json!({"instances":instances(&connection)?}),
        ));
    }
    if let Command::Start { config_dir } = args.command {
        return start(&connection, config_dir);
    }
    let instance = selected(&connection, args.instance)?;
    let request = match args.command {
        Command::Inspect => Request::Inspect,
        Command::Tree => Request::Tree,
        Command::ActivationToken => Request::ActivationToken,
        Command::Activate { target, apply } => {
            return call(
                &connection,
                &instance,
                Request::Activate { target },
                apply,
                false,
            );
        }
        Command::Navigate { page } => Request::Activate {
            target: format!("nav.{page}"),
        },
        Command::Open { files } => Request::Open {
            files: files
                .into_iter()
                .map(std::path::absolute)
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?,
        },
        Command::Controls => {
            let mut value = call(&connection, &instance, Request::Inspect, false, false)?;
            let controls = value["data"]["state"]["controls"].take();
            return Ok(protocol::success(json!({"controls":controls})));
        }
        Command::Drop { files } => Request::Drop {
            uris: files
                .into_iter()
                .map(|path| {
                    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
                    url::Url::from_file_path(&path)
                        .map(String::from)
                        .map_err(|_| format!("Cannot express {} as a file URI", path.display()))
                })
                .collect::<Result<_, _>>()?,
        },
        Command::Fill { target, text } => Request::Fill { target, text },
        Command::Theme { theme } => {
            call(
                &connection,
                &instance,
                Request::Theme {
                    theme: theme.clone(),
                },
                false,
                false,
            )?;
            return wait_for(&connection, &instance, 10, |state| match theme.as_str() {
                "light" => state["rendered_theme_dark"] == false,
                "dark" => state["rendered_theme_dark"] == true,
                _ => state["theme"] == "system",
            });
        }
        Command::Capture { path, overwrite } => Request::Capture {
            path: std::path::absolute(path).map_err(|e| e.to_string())?,
            overwrite,
        },
        Command::Close { apply } => {
            return call(&connection, &instance, Request::Close, apply, false);
        }
        Command::Wait { timeout } => {
            return wait_for(&connection, &instance, timeout, |state| {
                state["loading"] == false && state["checking"] == false && state["busy"] == false
            });
        }
        Command::Window {
            mode,
            width,
            height,
        } => {
            if width.is_some() || height.is_some() {
                call(
                    &connection,
                    &instance,
                    Request::Window {
                        mode: Some("windowed".into()),
                        width: None,
                        height: None,
                    },
                    false,
                    false,
                )?;
                wait_for(&connection, &instance, 10, |s| {
                    s["window"]["mode"] == "windowed"
                })?;
                call(
                    &connection,
                    &instance,
                    Request::Window {
                        mode: None,
                        width,
                        height,
                    },
                    false,
                    false,
                )?;
                wait_for(&connection, &instance, 10, |s| {
                    width.is_none_or(|w| s["window"]["width"].as_u64() == Some(w.into()))
                        && height.is_none_or(|h| s["window"]["height"].as_u64() == Some(h.into()))
                })?;
            }
            if let Some(mode) = mode {
                call(
                    &connection,
                    &instance,
                    Request::Window {
                        mode: Some(mode.clone()),
                        width: None,
                        height: None,
                    },
                    false,
                    false,
                )?;
                return wait_for(&connection, &instance, 10, |s| s["window"]["mode"] == mode);
            }
            return call(&connection, &instance, Request::Inspect, false, false);
        }
        _ => return Err("Unsupported UI command".into()),
    };
    call(&connection, &instance, request, false, false)
}
fn start(connection: &zbus::Connection, config_dir: Option<PathBuf>) -> Result<Value, String> {
    let isolated = config_dir.is_some();
    if !isolated && let Some(instance) = primary_instance(connection)? {
        call(connection, &instance, Request::Present, false, false)?;
        return Ok(protocol::success(
            json!({"instance":instance,"reused":true}),
        ));
    }
    let instance = new_instance();
    let logs = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("gearlever-cosmic/ui");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let log_path = logs.join(format!(
        "{}.log",
        instance.rsplit('.').next().unwrap_or("gui")
    ));
    let log = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let mut command = if crate::host::sandboxed() {
        // Detach only the GUI process. A live launch helper would otherwise
        // keep the short-lived CLI sandbox waiting after the command returns.
        let mut command = crate::host::command("setsid");
        command.args([
            "--fork",
            "flatpak",
            "run",
            &format!("--env=GEARLEVER_COSMIC_UI_INSTANCE={instance}"),
            &format!(
                "--env=GEARLEVER_COSMIC_UI_ISOLATED={}",
                if isolated { "1" } else { "0" }
            ),
        ]);
        if let Some(config) = config_dir {
            command.args([
                "--command=env",
                "dev.nilltadios.GearLeverCosmic",
                &format!(
                    "XDG_CONFIG_HOME={}",
                    std::fs::canonicalize(config)
                        .map_err(|e| e.to_string())?
                        .display()
                ),
                "/app/bin/gearlever-cosmic",
            ]);
        } else {
            command.arg("dev.nilltadios.GearLeverCosmic");
        }
        command
    } else {
        // From an AppImage, start the AppImage itself: the binary next to this
        // CLI lives in a mount that disappears when this invocation exits.
        let program = match crate::appimage_self() {
            Some(appimage) => appimage,
            None => std::env::current_exe()
                .map_err(|e| e.to_string())?
                .with_file_name("gearlever-cosmic"),
        };
        let mut command = std::process::Command::new(program);
        for var in crate::APPIMAGE_ENV {
            command.env_remove(var);
        }
        command.env("GEARLEVER_COSMIC_UI_INSTANCE", &instance);
        command.env(
            "GEARLEVER_COSMIC_UI_ISOLATED",
            if isolated { "1" } else { "0" },
        );
        if let Some(config) = config_dir {
            command.env(
                "XDG_CONFIG_HOME",
                std::fs::canonicalize(config).map_err(|e| e.to_string())?,
            );
        }
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|e| e.to_string())?;
    let end = Instant::now() + Duration::from_secs(15);
    while !instances(connection)?.contains(&instance) {
        if !isolated && let Some(instance) = primary_instance(connection)? {
            call(connection, &instance, Request::Present, false, false)?;
            return Ok(protocol::success(
                json!({"instance":instance,"reused":true}),
            ));
        }
        if Instant::now() >= end {
            return Err(format!("GUI did not start; see {}", log_path.display()));
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(protocol::success(
        json!({"instance":instance, "log":log_path}),
    ))
}
fn primary_instance(connection: &zbus::Connection) -> Result<Option<String>, String> {
    let bus = desktop_proxy(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let primary: Result<u32, _> = zbus::block_on(bus.call(
        "GetConnectionUnixProcessID",
        &("dev.nilltadios.GearLeverCosmic",),
    ));
    let Ok(primary) = primary else {
        return Ok(None);
    };
    for instance in instances(connection)? {
        let pid: Result<u32, _> =
            zbus::block_on(bus.call("GetConnectionUnixProcessID", &(instance.as_str(),)));
        if pid.ok() == Some(primary) {
            return Ok(Some(instance));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_own_windows_can_be_addressed() {
        assert!(is_own_instance(&new_instance()));
        for other in [
            "org.freedesktop.Notifications",
            "com.system76.CosmicFiles",
            "dev.nilltadios.WarehouseCosmic.UI.i1",
            "org.kde.KWin",
            ":1.42",
            "dev.nilltadios.GearLeverCosmic",
            "dev.nilltadios.GearLeverCosmic.UI.",
            "dev.nilltadios.GearLeverCosmic.UI.i1.org.other",
        ] {
            assert!(!is_own_instance(other), "{other} must be refused");
        }
    }
}
