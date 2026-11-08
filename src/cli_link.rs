// SPDX-License-Identifier: GPL-3.0-or-later
//! `gearlever-cosmic-cli` on PATH: a symbolic link in ~/.local/bin, so people
//! and agents find the CLI with `command -v gearlever-cosmic-cli` instead of
//! having to know where the AppImage lives. It points to the Gear Lever
//! COSMIC AppImage (whose cli hook starts the CLI when it is run through a
//! link of this name) or to the native CLI binary.
use crate::{ops::Report, paths::Paths};
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const NAME: &str = "gearlever-cosmic-cli";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Status {
    /// Where the link lives: ~/.local/bin/gearlever-cosmic-cli.
    pub path: PathBuf,
    /// Something exists at that path (a link, even a broken one, or a file).
    pub exists: bool,
    /// It is a link to Gear Lever COSMIC, so this app may replace or remove it.
    pub ours: bool,
    /// Where the link points.
    pub target: Option<PathBuf>,
    /// The link's target exists.
    pub works: bool,
    /// ~/.local/bin is on this process's PATH, so the bare name resolves.
    pub on_path: bool,
}

pub fn path(paths: &Paths) -> PathBuf {
    paths.home.join(".local/bin").join(NAME)
}

fn dir_on_path(dir: &Path) -> bool {
    let want = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| d == dir || fs::canonicalize(&d).is_ok_and(|c| c == want))
    })
}

/// A link target that belongs to this app: the CLI binary or a Gear Lever
/// COSMIC AppImage (any name a user gave it that still says so).
fn is_ours(target: &Path) -> bool {
    let name = target
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    name == NAME || (name.contains("gear") && name.contains("lever") && name.contains("cosmic"))
}

pub fn status(paths: &Paths) -> Status {
    let link = path(paths);
    let meta = fs::symlink_metadata(&link).ok();
    let target = meta
        .as_ref()
        .filter(|m| m.file_type().is_symlink())
        .and_then(|_| fs::read_link(&link).ok());
    Status {
        exists: meta.is_some(),
        ours: target.as_deref().is_some_and(is_ours),
        works: target.is_some() && link.exists(),
        target,
        on_path: dir_on_path(link.parent().unwrap_or(Path::new("/"))),
        path: link,
    }
}

/// What a new link points to: the AppImage this process runs from, else the
/// native CLI next to this binary. None inside the Flatpak, whose CLI is
/// started with `flatpak run --command=gearlever-cosmic-cli`.
pub fn default_target() -> Option<PathBuf> {
    if crate::host::sandboxed() {
        return None;
    }
    if let Some(appimage) = crate::appimage_self() {
        return Some(appimage);
    }
    let cli = std::env::current_exe().ok()?.with_file_name(NAME);
    cli.is_file().then_some(cli)
}

/// Refused when something else uses the name, so nothing foreign is replaced.
fn check_free(s: &Status, paths: &Paths) -> Result<(), String> {
    if s.exists && !s.ours {
        return Err(format!(
            "{} exists and is not a link to Gear Lever COSMIC; nothing was changed",
            paths.short(&s.path)
        ));
    }
    Ok(())
}

/// Dry-run lines for turning the link on (to `target`) or off.
pub fn plan(paths: &Paths, enable: bool, target: Option<&Path>) -> Result<Vec<String>, String> {
    let s = status(paths);
    check_free(&s, paths)?;
    let short = paths.short(&s.path);
    let mut out = Vec::new();
    if enable {
        let target = target.ok_or(
            "No CLI to link: in the Flatpak use flatpak run --command=gearlever-cosmic-cli dev.nilltadios.GearLeverCosmic",
        )?;
        if s.target.as_deref() == Some(target) && s.works {
            out.push(format!(
                "Nothing changes: {short} already points to {}",
                paths.short(target)
            ));
        } else {
            out.push(format!(
                "{} the link {short} to {}",
                if s.exists { "Replace" } else { "Create" },
                paths.short(target)
            ));
        }
        if !s.on_path {
            out.push(format!(
                "Note: {} is not on PATH here; add it, or run the link by its full path",
                paths.short(s.path.parent().unwrap_or(Path::new("/")))
            ));
        }
    } else if s.exists {
        out.push(format!(
            "Move the link {short} to the Trash (the AppImage stays)"
        ));
    } else {
        out.push(format!("Nothing changes: there is no {short}"));
    }
    Ok(out)
}

pub fn apply(
    paths: &Paths,
    enable: bool,
    target: Option<&Path>,
    report: &mut Report,
) -> Result<(), String> {
    let s = status(paths);
    check_free(&s, paths)?;
    let short = paths.short(&s.path);
    if !enable {
        if s.exists {
            crate::host::trash(&s.path)?;
            report.trashed.push(s.path.clone());
            report
                .steps
                .push(format!("Moved the link {short} to the Trash"));
        } else {
            report.steps.push(format!("There is no {short}"));
        }
        return Ok(());
    }
    let target = target.ok_or("No CLI to link from inside the Flatpak")?;
    if s.target.as_deref() == Some(target) && s.works {
        report
            .steps
            .push(format!("{short} already points to {}", paths.short(target)));
        return Ok(());
    }
    let dir = s.path.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    // Make the new link beside the old one and rename it over, so the name
    // never disappears in between.
    let tmp = dir.join(format!(".{NAME}.new-{}", crate::now_nanos()));
    std::os::unix::fs::symlink(target, &tmp)
        .map_err(|e| format!("Cannot create {}: {e}", tmp.display()))?;
    if let Err(e) = fs::rename(&tmp, &s.path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Cannot create {short}: {e}"));
    }
    report.changed.push(s.path.clone());
    report.steps.push(format!(
        "{short} now starts the CLI ({})",
        paths.short(target)
    ));
    if !s.on_path {
        report.notes.push(format!(
            "{} is not on PATH here; add it, or run {} by its full path",
            paths.short(dir),
            s.path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report::default()
    }

    #[test]
    fn creates_replaces_and_removes_only_its_own_link() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::fixture(dir.path());
        let app = dir.path().join("AppImages/gear_lever_cosmic.appimage");
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, "x").unwrap();

        assert!(plan(&paths, true, Some(&app)).unwrap()[0].starts_with("Create the link"));
        apply(&paths, true, Some(&app), &mut report()).unwrap();
        let s = status(&paths);
        assert!(s.exists && s.ours && s.works);
        assert_eq!(s.target.as_deref(), Some(app.as_path()));
        assert!(plan(&paths, true, Some(&app)).unwrap()[0].starts_with("Nothing changes"));

        // A newer copy elsewhere replaces the link.
        let other = dir.path().join("GearLever_COSMIC-0.1.3-x86_64.AppImage");
        fs::write(&other, "y").unwrap();
        apply(&paths, true, Some(&other), &mut report()).unwrap();
        assert_eq!(status(&paths).target.as_deref(), Some(other.as_path()));

        // Someone else's file of that name is never touched.
        fs::remove_file(path(&paths)).unwrap();
        fs::write(path(&paths), "#!/bin/sh\n").unwrap();
        assert!(plan(&paths, true, Some(&app)).is_err());
        assert!(apply(&paths, false, None, &mut report()).is_err());
        assert_eq!(fs::read_to_string(path(&paths)).unwrap(), "#!/bin/sh\n");
    }

    #[test]
    fn recognises_its_own_targets() {
        assert!(is_ours(Path::new("/x/gear_lever_cosmic.appimage")));
        assert!(is_ours(Path::new(
            "/x/GearLever_COSMIC-0.1.3-x86_64.AppImage"
        )));
        assert!(is_ours(Path::new("/usr/bin/gearlever-cosmic-cli")));
        assert!(!is_ours(Path::new("/x/gearlever.appimage")));
        assert!(!is_ours(Path::new("/usr/bin/python3")));
    }
}
