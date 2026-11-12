// SPDX-License-Identifier: GPL-3.0-or-later
//! The fixed set of host programs this app runs. When packaged as a Flatpak
//! they run on the host through `flatpak-spawn --host`, always with separate
//! arguments, never a shell string. See AGENTS.md ("Host programs").
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn sandboxed() -> bool {
    Path::new("/.flatpak-info").is_file()
}

/// Programs that may run on the host. Anything else is refused.
pub const HOST_PROGRAMS: [&str; 7] = [
    "gio",
    "xdg-open",
    "setsid",
    "ps",
    "update-desktop-database",
    "notify-send",
    // Only SIGTERM to an app's own processes, for Relaunch (see relaunch.rs).
    "kill",
];

pub fn command(program: &str) -> Command {
    command_for(program, sandboxed())
}

fn command_for(program: &str, sandbox: bool) -> Command {
    let mut cmd = command_raw(program, sandbox);
    for var in crate::APPIMAGE_ENV {
        cmd.env_remove(var);
    }
    cmd
}

fn command_raw(program: &str, sandbox: bool) -> Command {
    assert!(
        HOST_PROGRAMS.contains(&program),
        "{program} is not an allowed host program"
    );
    if sandbox {
        let mut cmd = Command::new("/usr/bin/flatpak-spawn");
        cmd.args(["--host", "--directory=/", "--env=LC_ALL=C"]);
        cmd.arg(program);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.env("LC_ALL", "C");
        cmd
    }
}

/// Move a file or folder to the desktop Trash and check that it left its
/// place. Never falls back to permanent deletion.
pub fn trash(path: &Path) -> Result<(), String> {
    if sandboxed() {
        let output = command("gio")
            .args(["trash", "--"])
            .arg(path)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Cannot run host gio trash for {}: {e}", path.display()))?;
        if !output.status.success() {
            return Err(format!(
                "Moving {} to Trash failed: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    } else {
        trash::delete(path)
            .map_err(|e| format!("Moving {} to Trash failed: {e}", path.display()))?;
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Cannot verify that {} left: {e}", path.display())),
        Ok(_) => Err(format!(
            "{} still exists after moving it to Trash",
            path.display()
        )),
    }
}

fn detach(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Open a folder or URL with the desktop's default handler.
pub fn open(target: &str) -> Result<(), String> {
    let child = command("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot open {target}: {e}"))?;
    detach(child);
    Ok(())
}

/// The log a launched app writes its output to:
/// `<data>/launch-logs/<name>.log`, emptied on every launch.
///
/// The launcher (`gio`, `setsid`) hands its stdout and stderr to the app, which
/// keeps them after the launcher exits. Pipes would close as soon as this
/// process stops reading them, and the app's next write would kill it
/// (SIGPIPE) or fail with EPIPE. Electron apps ignore SIGPIPE, so a backend
/// process that logs to stdout exits and the window waits for it forever.
/// A file stays writable for as long as the app runs.
pub fn launch_log(name: &str) -> Option<PathBuf> {
    let dir = crate::paths::Paths::discover()
        .ok()?
        .data
        .join("launch-logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(format!("{name}.log")))
}

/// stdout and stderr for a launched app: the log file, or /dev/null without one.
fn launch_output(log: Option<&Path>) -> (Stdio, Stdio) {
    let file = log.and_then(|p| std::fs::File::create(p).ok());
    match file.and_then(|f| f.try_clone().ok().map(|g| (f, g))) {
        Some((out, err)) => (out.into(), err.into()),
        None => (Stdio::null(), Stdio::null()),
    }
}

/// Start an integrated app through its menu entry, the way the app menu does.
/// GIO handles Exec field codes, environment prefixes and Terminal=true.
/// The app's output goes to `log` (see [`launch_log`]), never to a pipe.
pub fn launch_desktop_file(
    desktop_file: &Path,
    log: Option<&Path>,
) -> Result<std::process::Child, String> {
    let (out, err) = launch_output(log);
    command("gio")
        .arg("launch")
        .arg(desktop_file)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", desktop_file.display()))
}

/// Start an AppImage that has no menu entry yet, detached from this process.
/// The app's output goes to `log` (see [`launch_log`]), never to a pipe.
pub fn launch_file(
    file: &Path,
    args: &[String],
    log: Option<&Path>,
) -> Result<std::process::Child, String> {
    let (out, err) = launch_output(log);
    command("setsid")
        .arg("--fork")
        .arg(file)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", file.display()))
}

/// Refresh the menu cache for the user's own applications folder. A missing
/// helper is not an error: desktops also watch the folder.
pub fn update_desktop_database(dir: &Path) -> Result<(), String> {
    match command("update-desktop-database")
        .arg("-q")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// The colour app icon as a file other programs can read (written to the
/// cache once), for notifications.
pub fn app_icon() -> Option<PathBuf> {
    const SVG: &[u8] = include_bytes!("../resources/dev.nilltadios.GearLeverCosmic.svg");
    let dir = crate::paths::Paths::discover().ok()?.data.join("icons");
    let path = dir.join("dev.nilltadios.GearLeverCosmic.svg");
    if std::fs::read(&path).ok().as_deref() != Some(SVG) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, SVG).ok()?;
    }
    Some(path)
}

/// Send a desktop notification with one "Open" action; returns whether it was
/// clicked. Used by the startup update check only.
pub fn notify(summary: &str, body: &str, action: &str) -> Result<bool, String> {
    let out = command("notify-send")
        .args(["--app-name=Gear Lever COSMIC", "--expire-time=600000"])
        .args(app_icon().map(|i| format!("--icon={}", i.display())))
        .arg(format!("--action=open={action}"))
        .arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "open")
}

/// Executable paths of running processes. Inside the Flatpak the sandbox only
/// sees its own processes, so the host's `ps` is asked. Paths of files
/// replaced while running end in " (deleted)"; that suffix is kept.
pub fn process_executables() -> Result<Vec<(u32, PathBuf)>, String> {
    if !sandboxed() {
        let mut list = Vec::new();
        for entry in std::fs::read_dir("/proc").map_err(|e| e.to_string())? {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            if let Ok(exe) = std::fs::read_link(entry.path().join("exe")) {
                list.push((pid, exe));
            }
        }
        return Ok(list);
    }
    let out = command("ps")
        .args(["-eo", "pid=,exe="])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Cannot list host processes: {e}"))?;
    if !out.status.success() {
        return Err("Cannot list host processes".into());
    }
    Ok(parse_ps(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_ps(text: &str) -> Vec<(u32, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, exe) = line.split_once(char::is_whitespace)?;
            let exe = exe.trim();
            (exe.starts_with('/')).then(|| Some((pid.parse().ok()?, PathBuf::from(exe))))?
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launched app keeps writing after its launcher exits. With the old
    /// piped stdio this grandchild was killed by SIGPIPE on its first write.
    #[test]
    fn launched_app_output_outlives_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("app.log");
        let (out, err) = launch_output(Some(&log));
        let status = Command::new("sh")
            .args(["-c", "(sleep 0.3; echo late; echo oops >&2) &"])
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .status()
            .unwrap();
        assert!(status.success());
        std::thread::sleep(std::time::Duration::from_millis(800));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("late") && text.contains("oops"), "{text:?}");
    }

    #[test]
    fn launch_output_without_a_log_is_null() {
        let (out, _) = launch_output(Some(Path::new("/nonexistent/dir/x.log")));
        let status = Command::new("sh")
            .args(["-c", "echo x"])
            .stdout(out)
            .status();
        assert!(status.unwrap().success());
    }

    #[test]
    fn host_bridge_keeps_arguments_separate() {
        let mut cmd = command_for("gio", true);
        cmd.args(["trash", "--", "/home/u/x; rm -rf ~"]);
        assert_eq!(cmd.get_program(), "/usr/bin/flatpak-spawn");
        let args: Vec<_> = cmd.get_args().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "--host",
                "--directory=/",
                "--env=LC_ALL=C",
                "gio",
                "trash",
                "--",
                "/home/u/x; rm -rf ~"
            ]
        );
    }
    #[test]
    #[should_panic]
    fn other_programs_are_refused() {
        command_for("sh", false);
    }
    #[test]
    fn ps_output_parses_paths_with_spaces_and_deleted_marks() {
        let list = parse_ps(
            "    1 /usr/lib/systemd/systemd\n  42 /home/u/My Apps/a.appimage (deleted)\n  7 -\n",
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].0, 42);
        assert_eq!(
            list[1].1,
            PathBuf::from("/home/u/My Apps/a.appimage (deleted)")
        );
    }
}
