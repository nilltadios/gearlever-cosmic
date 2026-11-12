// SPDX-License-Identifier: GPL-3.0-or-later
//! Relaunch an app whose running instance still uses a file an update
//! replaced: ask it to quit, wait, then start the new file.
//!
//! An AppImage's runtime mounts the image and starts the app, which often
//! leaves the runtime's process tree (Electron apps are reparented to the
//! session manager). The app's own processes are recognized by their
//! environment, which names the AppImage (`APPIMAGE`) and its mount
//! (`APPDIR`), together with a program inside that mount. Programs the app
//! started from elsewhere, such as a terminal's shell, are left alone.
//!
//! Quitting uses SIGTERM, the request a desktop sends at logout. Nothing is
//! ever force-killed: an app that does not quit (for example because it asks
//! to save work) is left running and not started a second time.
use crate::library::App;
use std::{
    fs,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

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

/// Processes of running instances of `appimage`: (app processes, runtimes).
pub fn processes(appimage: &Path) -> Result<(Vec<u32>, Vec<u32>), String> {
    let own = std::process::id();
    let (mut app, mut runtime) = (vec![], vec![]);
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
        // Other users' processes are unreadable and skipped here.
        let Ok(exe) = fs::read_link(dir.join("exe")) else {
            continue;
        };
        let environ = fs::read(dir.join("environ")).unwrap_or_default();
        let cmdline = fs::read(dir.join("cmdline")).unwrap_or_default();
        match role(appimage, &exe, &environ, &cmdline) {
            Some(Role::App) => app.push(pid),
            Some(Role::Runtime) => runtime.push(pid),
            None => {}
        }
    }
    Ok((app, runtime))
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
    if pids.is_empty() {
        return Ok(());
    }
    // A process that already exited makes kill fail; that is fine.
    crate::host::command("kill")
        .args(["-s", "TERM", "--"])
        .args(pids.iter().map(u32::to_string))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
        .map_err(|e| format!("Cannot ask the app to quit: {e}"))
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
    fn a_zombie_or_missing_process_is_not_alive() {
        assert!(alive(std::process::id()));
        assert!(!alive(u32::MAX));
    }
}
