# Gear Lever COSMIC

A Rust/libcosmic AppImage manager, distributed as x86_64 and aarch64 AppImages built with Anylinux tooling (quick-sharun + uruntime, DwarFS). Rust 1.100.0 and every crate, including libcosmic's Git revision, are pinned in `rust-toolchain.toml` and `Cargo.lock`.

## Launch and inspect

```sh
cargo build --bins
target/debug/gearlever-cosmic --preview PAGE --capture /absolute/new.png   # render and exit
target/debug/gearlever-cosmic-cli guide
dist/GearLever_COSMIC-*-$(uname -m).AppImage cli commands --json
```

When opening the app for someone to use, leave that window open. Test windows use `ui start --config-dir DIR` (isolated) or `--capture`, and are closed afterwards. Never interrupt a window or operation you did not start.

## Never act on real apps in tests

Tests that change anything run on a disposable home: `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` pointing into `verification/out-*`, with AppImages from `examples/make_fixture.rs` and a 127.0.0.1 update server (`verification/cli-workflow.sh`, `gui-workflow.sh`, `check-no-menu-rewrite.sh`). Copy the desktop's COSMIC theme files into a fixture to render with the real accent; never write `~/.config/cosmic`. The user's real AppImages may only be read (`list-installed`, `update.check`, `app.requirements`, capture instances). Anything that may start the GUI against the real home must set `GEARLEVER_COSMIC_UI_ISOLATED=1` (the AppImage build does).

## The app never touches other apps' windows

- `ui` commands talk only to D-Bus names starting with `dev.nilltadios.GearLeverCosmic.UI.` and only to this app's `Execute` method (`ui_client::call` checks).
- No synthesized input, no desktop screenshots. Captures are the app's own render.
- Host programs are the fixed list in `host::HOST_PROGRAMS` with separate arguments (`gio`, `xdg-open`, `setsid`, `ps`, `update-desktop-database`, `notify-send`, `kill`). Never accept a program name or shell string from input.
- Managing AppImages as files and menu entries is the app's purpose. Every change is a plan without `--apply`; starting another program (launch, open folder) also needs `--apply`.
- Menu entries are rewritten only by an explicit operation (add, update, Reload, icon, arguments), never automatically. They carry no version; versions come from the files.
- The app stops another program only through Relaunch, Quit and Force Quit (`relaunch.rs`), each after a confirmation, and only that AppImage's own processes: programs inside its mount, helpers outside it whose environment names the AppImage and that still hold files in the mount, and its runtimes. Relaunch and Quit send SIGTERM via `kill`; a process that does not quit is left running, and Relaunch does not start the app twice. SIGKILL is sent only by Force Quit, which the GUI offers after a Quit left processes running, says unsaved work is lost, and re-lists the processes at confirm time. Runtimes get SIGTERM before any SIGKILL, so the mount is unmounted cleanly. Not in the Flatpak, never for Gear Lever itself.

## Honesty rules for updates

Never report "no update" without a comparison: keep `no_source`, `failed`, `not_found` and `self_updating` distinct, and say how files were compared. Updates replace files in place and never require closing the app; say when a running instance still uses the old version. Removal uses the Trash; there is no permanent delete.

## Verification

Run before reporting a change: `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `verification/cli-workflow.sh`, and for GUI changes `verification/gui-workflow.sh` plus looking at the renders. Window tests (`gui-workflow.sh`, `gui-quit-and-update-all.sh`, `check-icons-self-contained.sh`) start themselves through `verification/on-desktop.sh` and `private-bus.sh`: their windows go to the KDE virtual desktop "Desktop 2" and never appear on the desktop in use. There the window does not redraw, so `ui capture` renders show only its first frame; the `ok:` checks are the result. Do not create, remove or rename virtual desktops, and never start another compositor (it writes the person's `~/.config/kwinrc`). Renders for layout need the person's go-ahead to run on their desktop (`GEARLEVER_TEST_HERE=1`). A render with blank icons is a broken test setup, not a result: `ui inspect` must report `icons_missing: []` (source-tree builds find `resources/icons` themselves). For packaging changes build the AppImage and run it (`cli --version`, a `--capture` render). See `docs/verification.md`.

## Releases

Every push to `main` builds the AppImage; a GitHub release is created when the version in `Cargo.toml` has no release yet. Before pushing a change that ships, bump `version` in `Cargo.toml` and add a `<release>` to `resources/dev.nilltadios.GearLeverCosmic.metainfo.xml`. `verification/check-release-bump.sh` fails when shipped files changed under an already released version, and the project's Claude Code hook (`.claude/settings.json`) runs it before every `git push`. Commits carry no AI attribution or co-author trailers. Pushing and publishing need the person's explicit approval.
