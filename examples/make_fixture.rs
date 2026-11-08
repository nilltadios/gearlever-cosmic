// SPDX-License-Identifier: GPL-3.0-or-later
//! Build a disposable home folder with sample AppImages and a local update
//! server tree, for GUI/CLI tests that must not touch real apps.
//!
//! cargo run --example make_fixture -- RUNTIME OUT_DIR PORT
//!   RUNTIME  the built fixture_runtime binary
//!   OUT_DIR  new folder; gets home/ (HOME for the test) and server/ (serve
//!            with `python3 -m http.server PORT -d OUT_DIR/server -b 127.0.0.1`)
use backhand::{FilesystemWriter, NodeHeader};
use std::{fs, io::Cursor, path::Path};

const PNG_RED: &[u8] = include_bytes!("../resources/dev.nilltadios.GearLeverCosmic.png");

fn svg(color: &str, letter: &str) -> Vec<u8> {
    format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'><rect x='4' y='4' width='56' height='56' rx='14' fill='{color}'/><text x='32' y='43' font-size='30' font-family='sans-serif' text-anchor='middle' fill='white'>{letter}</text></svg>").into_bytes()
}

fn appimage(runtime: &[u8], files: &[(&str, Vec<u8>)], marker: &str, update_info: &str) -> Vec<u8> {
    let mut w = FilesystemWriter::default();
    let h = NodeHeader::new(0o755, 0, 0, 0);
    let mut dirs = std::collections::BTreeSet::new();
    for (p, _) in files {
        let mut acc = std::path::PathBuf::from("/");
        for c in Path::new(p).parent().unwrap().components().skip(1) {
            acc.push(c);
            if dirs.insert(acc.clone()) {
                w.push_dir(&acc, h).unwrap();
            }
        }
    }
    for (p, d) in files {
        w.push_file(Cursor::new(d.clone()), p, h).unwrap();
    }
    let mut img = Cursor::new(Vec::new());
    w.write(&mut img).unwrap();
    let mut out = runtime.to_vec();
    out[8..11].copy_from_slice(b"AI\x02");
    let _ = (marker, update_info);
    out.extend(img.into_inner());
    out
}

/// Incompressible bytes, so versions differ in size like real builds do.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e3779b9 ^ len as u32;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn desktop(name: &str, exec: &str, version: Option<&str>, extra: &str) -> Vec<u8> {
    let v = version
        .map(|v| format!("X-AppImage-Version={v}\n"))
        .unwrap_or_default();
    format!("[Desktop Entry]\nType=Application\nName={name}\nComment=Fixture app for Gear Lever COSMIC tests\nExec={exec} %U\nIcon={exec}\nCategories=Utility;\n{v}{extra}").into_bytes()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let runtime = fs::read(&args[1]).expect("runtime");
    let out = Path::new(&args[2]);
    let port = &args[3];
    assert!(!out.exists(), "OUT_DIR must be new");
    let home = out.join("home");
    let server = out.join("server");
    let downloads = home.join("Downloads");
    let folder = home.join("AppImages");
    for d in [
        &downloads,
        &folder,
        &home.join(".local/share/applications"),
        &server,
    ] {
        fs::create_dir_all(d).unwrap();
    }
    let base = format!("http://127.0.0.1:{port}");
    // 1. Demo Editor 1.0, update via a static URL to 1.1 on the server.
    let editor = |v: &str| {
        appimage(
            &runtime,
            &[
                (
                    "/demo-editor.desktop",
                    desktop("Demo Editor", "demo-editor", Some(v), ""),
                ),
                ("/demo-editor.svg", svg("#2f6fd8", "E")),
                (
                    "/usr/share/doc/notes.bin",
                    noise(20_000 + v.bytes().map(usize::from).sum::<usize>() * 997),
                ),
            ],
            "",
            "",
        )
    };
    fs::write(
        downloads.join("Demo_Editor-1.0-x86_64.AppImage"),
        editor("1.0"),
    )
    .unwrap();
    fs::write(
        server.join("Demo_Editor-latest-x86_64.AppImage"),
        editor("1.1"),
    )
    .unwrap();
    // 2. Sky Notes: an electron-updater app with a generic feed on the server.
    let notes = |v: &str| {
        appimage(
            &runtime,
            &[
                (
                    "/sky-notes.desktop",
                    desktop("Sky Notes", "sky-notes", Some(v), ""),
                ),
                ("/sky-notes.png", PNG_RED.to_vec()),
                (
                    "/resources/app-update.yml",
                    format!("provider: generic\nurl: {base}/sky-notes\nchannel: latest\n")
                        .into_bytes(),
                ),
            ],
            "",
            "",
        )
    };
    fs::write(downloads.join("Sky-Notes-2.0.0.AppImage"), notes("2.0.0")).unwrap();
    let newer = notes("2.1.0");
    fs::create_dir_all(server.join("sky-notes")).unwrap();
    fs::write(server.join("sky-notes/Sky-Notes-2.1.0.AppImage"), &newer).unwrap();
    use base64::Engine;
    use sha2::Digest;
    let sha = base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(&newer));
    fs::write(server.join("sky-notes/latest-linux.yml"), format!("version: 2.1.0\nfiles:\n  - url: Sky-Notes-2.1.0.AppImage\n    sha512: {sha}\n    size: {}\npath: Sky-Notes-2.1.0.AppImage\nsha512: {sha}\nreleaseDate: '2026-11-01T00:00:00.000Z'\n", newer.len())).unwrap();
    // 3. Old Tool: runtime mentions libfuse.so.2 (an AppImageKit-style runtime).
    // Overwrite the compiler note in the runtime's .comment section, so the
    // runtime mentions libfuse.so.2 like the AppImageKit runtime does.
    let mut fuse2_runtime = runtime.clone();
    let at = fuse2_runtime
        .windows(13)
        .position(|w| w == b"rustc version")
        .expect(".comment section");
    fuse2_runtime[at..at + 13].copy_from_slice(b"libfuse.so.2\0");
    let old = appimage(
        &fuse2_runtime,
        &[
            (
                "/old-tool.desktop",
                desktop("Old Tool", "old-tool", Some("0.9"), ""),
            ),
            ("/old-tool.svg", svg("#8a5a00", "O")),
        ],
        "",
        "",
    );
    fs::write(downloads.join("Old_Tool-0.9.AppImage"), old).unwrap();
    // 4. Mystery App: no version, no update information.
    fs::write(
        downloads.join("mystery.AppImage"),
        appimage(
            &runtime,
            &[
                (
                    "/mystery.desktop",
                    desktop("Mystery App", "mystery", None, ""),
                ),
                ("/mystery.svg", svg("#555555", "?")),
            ],
            "",
            "",
        ),
    )
    .unwrap();
    // 5. Term Tool: a terminal app.
    fs::write(
        downloads.join("termtool-x86_64.AppImage"),
        appimage(
            &runtime,
            &[
                (
                    "/termtool.desktop",
                    desktop("Term Tool", "termtool", Some("3.2"), "Terminal=true\n"),
                ),
                ("/termtool.svg", svg("#1d8a3a", "T")),
            ],
            "",
            "",
        ),
    )
    .unwrap();
    // 6. Not an AppImage, to test refusal.
    fs::write(
        downloads.join("fake.AppImage"),
        b"#!/bin/sh\necho not an appimage\n",
    )
    .unwrap();
    println!("{}", out.display());
}
