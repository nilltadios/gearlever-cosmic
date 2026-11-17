// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only stand-in for an AppImage runtime: the fixture generator appends
//! a SquashFS image to this program and marks it as a type 2 AppImage. When
//! run, it stays alive (like a real app) for FIXTURE_SECONDS (default 600),
//! so tests can see it as running and update it in place.
//!
//! With FIXTURE_HELPER_DIR set to a folder (standing in for the mount), it
//! also starts a background helper the way Electron apps leave one behind:
//! a program outside the mount, with `APPIMAGE`/`APPDIR` in its environment
//! and its working folder in the "mount", that ignores SIGTERM. Quit then
//! leaves it running and Force Quit has something to stop.
use std::process::{Command, Stdio};

fn main() {
    let secs: u64 = std::env::var("FIXTURE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    if std::env::args().any(|a| a == "--appimage-extract") {
        eprintln!("fixture runtime: extraction is not implemented");
        std::process::exit(1);
    }
    eprintln!(
        "fixture app running for {secs}s: {:?}",
        std::env::args().collect::<Vec<_>>()
    );
    if let Some(dir) = std::env::var_os("FIXTURE_HELPER_DIR") {
        std::fs::create_dir_all(&dir).expect("helper folder");
        let me = std::env::current_exe().expect("own path");
        // An ignored signal stays ignored across exec, so `sleep` ignores TERM.
        let mut helper = Command::new("sh")
            .args([
                "-c",
                "trap '' TERM; exec sleep \"$1\"",
                "sh",
                &secs.to_string(),
            ])
            .current_dir(&dir)
            .env("APPIMAGE", &me)
            .env("APPDIR", &dir)
            .stdin(Stdio::null())
            .spawn()
            .expect("helper");
        eprintln!("fixture helper pid {}", helper.id());
        // Collect it when it ends, so a killed helper leaves no zombie.
        std::thread::spawn(move || helper.wait());
    }
    std::thread::sleep(std::time::Duration::from_secs(secs));
}
