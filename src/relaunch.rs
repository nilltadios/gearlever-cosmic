// SPDX-License-Identifier: GPL-3.0-or-later
//! Stop running AppImages: Relaunch an app whose running instance still uses
//! a file an update replaced (ask it to quit, wait, then start the new file),
//! and Quit or Force Quit an app the person chose on its page.
//!
//! An AppImage's runtime mounts the image and starts the app, which often
//! leaves the runtime's process tree (Electron apps are reparented to the
//! session manager). The app's own processes are recognized by their
//! environment, which names the AppImage (`APPIMAGE`) and its mount
//! (`APPDIR`), together with a program inside that mount. Programs the app
//! started from elsewhere, such as a terminal's shell, are left alone.
//! Quit also stops helpers the app started from a copy outside the mount
//! that still hold files in it (a background server, for example), because
//! they keep the runtime and its mount alive after the window has closed.
//!
//! Quitting uses SIGTERM, the request a desktop sends at logout. Relaunch
//! never force-kills: an app that does not quit (for example because it asks
//! to save work) is left running and not started a second time. Only Force
//! Quit, a separate confirmed action offered after a Quit that did not
//! finish, sends SIGKILL, and then only to the processes still left.
use crate::library::App;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

/// How long Quit waits for an app to exit before offering Force Quit.
pub const QUIT_WAIT: Duration = Duration::from_secs(10);

/// How long an app gets to quit before the relaunch gives up.
pub const QUIT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Part of the app, started from inside its mount.
    App,
    /// The AppImage runtime serving the mount (the file or a replaced copy).
    Runtime,
}

/// Whether a process with this executable, environment (`/proc/PID/environ`)
/// and command line (`/proc/PID/cmdline`) belongs to an instance of `appimage`.
pub fn role(appimage: &Path, exe: &Path, environ: &[u8], cmdline: &[u8]) -> Option<Role> {
    let deleted = format!("{} (deleted)", appimage.display());
    if exe == appimage || exe.to_string_lossy() == deleted {
        return Some(Role::Runtime);
    }
    let var = |name: &str| {
        environ
            .split(|b| *b == 0)
            .filter_map(|kv| std::str::from_utf8(kv).ok())
            .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
    };
    if var("APPIMAGE").map(Path::new) != Some(appimage) {
        return None;
    }
    let appdir = Path::new(var("APPDIR")?);
    if !appdir.is_absolute() || appdir.parent().is_none() {
        return None;
    }
    // The program itself, or a script inside the mount run by an
    // interpreter (`bash /tmp/.mount_x/AppRun`).
    let inside = exe.starts_with(appdir)
        || cmdline
            .split(|b| *b == 0)
            .take(2)
            .filter_map(|a| std::str::from_utf8(a).ok())
            .any(|a| Path::new(a).starts_with(appdir));
    inside.then_some(Role::App)
}

/// Whether a process outside the app's mount is a helper of that app: its
/// environment names this AppImage and its mount, and it still has a file
/// open in the mount (`open` lists its open files and working folder), so it
/// keeps the runtime alive. A shell in the app's terminal inherits the
/// environment but holds nothing in the mount, so it is not a helper.
pub fn holds_mount(appimage: &Path, environ: &[u8], open: &[PathBuf]) -> bool {
    let var = |name: &str| {
        environ
            .split(|b| *b == 0)
            .filter_map(|kv| std::str::from_utf8(kv).ok())
            .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
    };
    if var("APPIMAGE").map(Path::new) != Some(appimage) {
        return false;
    }
    let Some(appdir) = var("APPDIR").map(Path::new) else {
        return false;
    };
    // The mount is a folder of its own under a temporary folder, never `/`
    // or a top-level folder that everything lives in.
    if !appdir.is_absolute() || appdir.components().count() < 3 {
        return false;
    }
    open.iter().any(|p| p.starts_with(appdir))
}

/// A process Quit signals, with a short description for the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Proc {
    pub pid: u32,
    /// The program and its first arguments.
    pub command: String,
    pub kind: ProcKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcKind {
    /// Started from inside the mount.
    App,
    /// Started from a copy outside the mount; still holds files in it.
    Helper,
    /// The AppImage runtime serving the mount.
    Runtime,
}

fn pids_of(list: &[Proc], kinds: &[ProcKind]) -> Vec<u32> {
    list.iter()
        .filter(|p| kinds.contains(&p.kind))
        .map(|p| p.pid)
        .collect()
}

fn command_text(cmdline: &[u8], exe: &Path) -> String {
    let args: Vec<&str> = cmdline
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .filter_map(|a| std::str::from_utf8(a).ok())
        .take(3)
        .collect();
    let text = if args.is_empty() {
        exe.display().to_string()
    } else {
        args.join(" ")
    };
    if text.chars().count() > 120 {
        format!("{}…", text.chars().take(119).collect::<String>())
    } else {
        text
    }
}

/// The open files and working folder of a process.
fn open_files(dir: &Path) -> Vec<PathBuf> {
    let mut open: Vec<PathBuf> = fs::read_dir(dir.join("fd"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_link(e.path()).ok())
        .collect();
    open.extend(fs::read_link(dir.join("cwd")));
    open
}

/// Every process of running instances of `appimage`, with helpers when
/// `helpers` is set.
fn scan(appimage: &Path, helpers: bool) -> Result<Vec<Proc>, String> {
    let own = std::process::id();
    let mut out = vec![];
    for entry in fs::read_dir("/proc").map_err(|e| format!("Cannot list processes: {e}"))? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own {
            continue;
        }
        let dir = entry.path();
        // Other users' processes are unreadable and skipped here. A program
        // run by qemu-user counts by the emulated file, not qemu.
        let Some(exe) = crate::host::process_program(&dir) else {
            continue;
        };
        let environ = fs::read(dir.join("environ")).unwrap_or_default();
        let cmdline = fs::read(dir.join("cmdline")).unwrap_or_default();
        let kind = match role(appimage, &exe, &environ, &cmdline) {
            Some(Role::App) => ProcKind::App,
            Some(Role::Runtime) => ProcKind::Runtime,
            None if helpers && holds_mount(appimage, &environ, &open_files(&dir)) => {
                ProcKind::Helper
            }
            None => continue,
        };
        out.push(Proc {
            pid,
            command: command_text(&cmdline, &exe),
            kind,
        });
    }
    Ok(out)
}

/// Processes of running instances of `appimage`: (app processes, runtimes).
pub fn processes(appimage: &Path) -> Result<(Vec<u32>, Vec<u32>), String> {
    let list = scan(appimage, false)?;
    Ok((
        pids_of(&list, &[ProcKind::App]),
        pids_of(&list, &[ProcKind::Runtime]),
    ))
}

/// The processes Quit would signal: the app's own, its helpers and the
/// runtimes, in that order.
pub fn quit_targets(appimage: &Path) -> Result<Vec<Proc>, String> {
    let mut list = scan(appimage, true)?;
    list.sort_by_key(|p| (p.kind as u8, p.pid));
    Ok(list)
}

/// What a Quit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitOutcome {
    /// Everything exited; the text is for a notice.
    Done(String),
    /// These processes are still running after the wait.
    Left(Vec<Proc>),
}

fn not_in_flatpak(app: &App) -> Result<(), String> {
    if crate::host::sandboxed() {
        return Err(format!(
            "Quitting other apps is not available in the Flatpak. Close {} yourself.",
            app.name
        ));
    }
    Ok(())
}

/// Stop the runtimes once nothing uses their mount: SIGTERM first so they
/// unmount cleanly, SIGKILL only for one still there after the wait.
fn stop_runtimes(runtimes: &[u32], force: bool) -> Result<bool, String> {
    signal(runtimes, "TERM")?;
    if wait_gone(runtimes, Duration::from_secs(5)) {
        return Ok(true);
    }
    if force {
        let left: Vec<u32> = runtimes.iter().copied().filter(|p| alive(*p)).collect();
        signal(&left, "KILL")?;
        return Ok(wait_gone(&left, Duration::from_secs(3)));
    }
    Ok(false)
}

/// Ask every process of `app` to quit (SIGTERM, as at logout) and wait up to
/// `wait`. Processes are found again here, so a list shown earlier never
/// decides what is signalled.
pub fn quit(app: &App, wait: Duration) -> Result<QuitOutcome, String> {
    not_in_flatpak(app)?;
    let list = quit_targets(&app.path)?;
    if list.is_empty() {
        return Ok(QuitOutcome::Done(format!("{} is not running", app.name)));
    }
    let procs = pids_of(&list, &[ProcKind::App, ProcKind::Helper]);
    signal(&procs, "TERM")?;
    if !wait_gone(&procs, wait) {
        return Ok(QuitOutcome::Left(
            list.into_iter().filter(|p| alive(p.pid)).collect(),
        ));
    }
    let runtimes = pids_of(&list, &[ProcKind::Runtime]);
    if !stop_runtimes(&runtimes, false)? {
        return Ok(QuitOutcome::Left(
            list.into_iter().filter(|p| alive(p.pid)).collect(),
        ));
    }
    Ok(QuitOutcome::Done(format!("Quit {}", app.name)))
}

/// Force Quit: SIGKILL every process of `app` that is still running (found
/// again here), then stop the runtimes, which get SIGTERM first. Unsaved work
/// in those processes is lost; the GUI says so before asking.
pub fn force_quit(app: &App) -> Result<String, String> {
    not_in_flatpak(app)?;
    let list = quit_targets(&app.path)?;
    let procs = pids_of(&list, &[ProcKind::App, ProcKind::Helper]);
    signal(&procs, "KILL")?;
    let procs_gone = wait_gone(&procs, Duration::from_secs(3));
    let runtimes_gone = stop_runtimes(&pids_of(&list, &[ProcKind::Runtime]), true)?;
    if procs_gone && runtimes_gone {
        Ok(format!("Force quit {}", app.name))
    } else {
        Err(format!(
            "Some {} processes are still running after Force Quit. They may belong to another user or be stuck in the kernel.",
            app.name
        ))
    }
}

fn alive(pid: u32) -> bool {
    // A zombie has exited; only its parent has not collected it yet.
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            s.rsplit_once(") ")
                .and_then(|(_, rest)| rest.chars().next())
        })
        .is_some_and(|state| state != 'Z')
}

fn terminate(pids: &[u32]) -> Result<(), String> {
    signal(pids, "TERM")
}

/// Send TERM or KILL through the host's `kill`.
fn signal(pids: &[u32], name: &str) -> Result<(), String> {
    assert!(matches!(name, "TERM" | "KILL"));
    if pids.is_empty() {
        return Ok(());
    }
    // A process that already exited makes kill fail; that is fine.
    crate::host::command("kill")
        .args(["-s", name, "--"])
        .args(pids.iter().map(u32::to_string))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
        .map_err(|e| format!("Cannot signal the app: {e}"))
}

fn wait_gone(pids: &[u32], timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if !pids.iter().any(|p| alive(*p)) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Ask a running app to quit and start it again from its (new) file.
/// Returns what happened, for a notice.
pub fn relaunch(app: &App, timeout: Duration) -> Result<String, String> {
    if crate::host::sandboxed() {
        return Err(format!(
            "Relaunching is not available in the Flatpak. Close {} and start it again.",
            app.name
        ));
    }
    let (procs, runtimes) = processes(&app.path)?;
    if !procs.is_empty() {
        terminate(&procs)?;
        if !wait_gone(&procs, timeout) {
            return Err(format!(
                "{} did not quit within {} seconds, so it was not started again. It may be asking to save your work. Close it yourself, then press Launch.",
                app.name,
                timeout.as_secs()
            ));
        }
    }
    // The runtimes only served the old mount; with the app gone they would
    // exit on their own, but would hold the replaced file until then.
    terminate(&runtimes)?;
    wait_gone(&runtimes, Duration::from_secs(5));
    crate::automation::launch_app(app)?;
    Ok(if procs.is_empty() && runtimes.is_empty() {
        format!("Started {}", app.name)
    } else {
        format!("Restarted {} with the new version", app.name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "/home/u/AppImages/claude.appimage";

    fn env(vars: &[&str]) -> Vec<u8> {
        vars.join("\0").into_bytes()
    }
    fn role_of(exe: &str, environ: &[&str], cmdline: &[&str]) -> Option<Role> {
        role(
            Path::new(APP),
            Path::new(exe),
            &env(environ),
            cmdline.join("\0").as_bytes(),
        )
    }

    #[test]
    fn the_runtime_of_the_file_or_a_replaced_copy() {
        assert_eq!(role_of(APP, &[], &[APP]), Some(Role::Runtime));
        assert_eq!(
            role_of(&format!("{APP} (deleted)"), &[], &[APP]),
            Some(Role::Runtime)
        );
    }

    #[test]
    fn app_processes_live_in_the_mount() {
        let e = [
            "HOME=/home/u",
            &format!("APPIMAGE={APP}"),
            "APPDIR=/tmp/.mount_claudeX",
        ];
        assert_eq!(
            role_of(
                "/tmp/.mount_claudeX/usr/lib/claude/claude",
                &e,
                &["/tmp/.mount_claudeX/usr/lib/claude/claude"]
            ),
            Some(Role::App)
        );
        // The AppRun script, run by bash from the host.
        assert_eq!(
            role_of("/usr/bin/bash", &e, &["bash", "/tmp/.mount_claudeX/AppRun"]),
            Some(Role::App)
        );
    }

    #[test]
    fn programs_the_app_started_elsewhere_are_left_alone() {
        let e = [
            &format!("APPIMAGE={APP}") as &str,
            "APPDIR=/tmp/.mount_claudeX",
        ];
        // A shell in the app's terminal inherits the environment.
        assert_eq!(role_of("/usr/bin/bash", &e, &["/bin/bash", "-l"]), None);
        // Another AppImage's processes.
        let other = [
            "APPIMAGE=/home/u/AppImages/zed.appimage",
            "APPDIR=/tmp/.mount_zedY",
        ];
        assert_eq!(
            role_of("/tmp/.mount_zedY/zed", &other, &["/tmp/.mount_zedY/zed"]),
            None
        );
        // A suspicious APPDIR never matches everything.
        let root = [&format!("APPIMAGE={APP}") as &str, "APPDIR=/"];
        assert_eq!(role_of("/usr/bin/firefox", &root, &["firefox"]), None);
        assert_eq!(role_of("/usr/bin/x", &[], &["x"]), None);
    }

    #[test]
    fn quit_also_finds_helpers_that_hold_the_mount() {
        let e = env(&[
            "HOME=/home/u",
            &format!("APPIMAGE={APP}"),
            "APPDIR=/tmp/.mount_claudeX",
        ]);
        let app = Path::new(APP);
        // A background server copied out of the image (opencode-cli serve):
        // not part of the app for Relaunch, but it keeps the mount busy.
        let server = "/home/u/.config/app/cli/opencode-cli";
        assert_eq!(role_of(server, &[], &[server, "serve"]), None);
        let held = [
            PathBuf::from("/home/u"),
            PathBuf::from("/tmp/.mount_claudeX/resources/app.asar"),
        ];
        assert!(holds_mount(app, &e, &held));
        // A shell in the app's terminal has the environment but no files
        // in the mount.
        let shell = [PathBuf::from("/dev/pts/3"), PathBuf::from("/home/u")];
        assert!(!holds_mount(app, &e, &shell));
        // Another AppImage's helper, and a suspicious APPDIR.
        let other = env(&[
            "APPIMAGE=/home/u/AppImages/zed.appimage",
            "APPDIR=/tmp/.mount_claudeX",
        ]);
        assert!(!holds_mount(app, &other, &held));
        for bad in ["APPDIR=/", "APPDIR=/tmp", "APPDIR=tmp/.mount_claudeX"] {
            let e = env(&[&format!("APPIMAGE={APP}"), bad]);
            assert!(!holds_mount(app, &e, &[PathBuf::from("/tmp/x")]), "{bad}");
        }
    }

    #[test]
    fn command_text_is_short() {
        assert_eq!(
            command_text(b"/a/b\0serve\0--service\0--extra\0", Path::new("/a/b")),
            "/a/b serve --service"
        );
        assert_eq!(command_text(b"", Path::new("/a/b")), "/a/b");
        assert!(command_text(&[b'x'; 300], Path::new("/x")).chars().count() <= 120);
    }

    #[test]
    fn a_zombie_or_missing_process_is_not_alive() {
        assert!(alive(std::process::id()));
        assert!(!alive(u32::MAX));
    }
}
