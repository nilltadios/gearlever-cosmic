// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only: list the processes Quit would signal for an AppImage, without
//! signalling anything.
//!
//! cargo run --example quit_targets -- /path/to/app.appimage
fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: quit_targets APPIMAGE");
    match gearlever_cosmic::relaunch::quit_targets(std::path::Path::new(&path)) {
        Ok(list) => println!("{}", serde_json::to_string_pretty(&list).unwrap()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
