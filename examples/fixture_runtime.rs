// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only stand-in for an AppImage runtime: the fixture generator appends
//! a SquashFS image to this program and marks it as a type 2 AppImage. When
//! run, it stays alive (like a real app) for FIXTURE_SECONDS (default 600),
//! so tests can see it as running and update it in place.
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
    std::thread::sleep(std::time::Duration::from_secs(secs));
}
