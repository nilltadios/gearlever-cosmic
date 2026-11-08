# Development

## Build and run

```sh
cargo run --locked --bin gearlever-cosmic
cargo run --locked --bin gearlever-cosmic-cli -- guide
cargo build --locked --no-default-features --bin gearlever-cosmic-cli   # CLI only, no libcosmic
```

Native builds need a C compiler, pkg-config, xkbcommon and Wayland headers. The GUI uses native Wayland with wgpu (Vulkan, then OpenGL) and falls back to software rendering.

## Layout

| File | Purpose |
|---|---|
| `src/elf.rs` | AppImage type, architecture, payload offset, `.upd_info` |
| `src/appimage.rs` | Image readers (SquashFS via backhand, DwarFS via dwarfs), metadata, icons, hashes, cache |
| `src/desktop.rs` | Desktop entry editing that keeps every other line; Exec parsing/quoting |
| `src/library.rs` | Installed apps (menu entries that start AppImages), running detection |
| `src/updates.rs` | Update sources, statuses, downloads with verification |
| `src/requirements.rs` | FUSE runtime and library checks |
| `src/ops.rs` | Integrate, update in place, remove to Trash, edits; lock and reports |
| `src/store.rs` | Per-app settings; read-only Gear Lever settings |
| `src/automation.rs` | The CLI: catalog, schemas, plans, Gear Lever options |
| `src/app.rs`, `src/ui_view.rs` | The window and its `ui` controls |

## AppImage

`packaging/build-appimage.sh [BIN_DIR] [OUT_DIR]` uses [quick-sharun](https://github.com/pkgforge-dev/Anylinux-AppImages) pinned to a commit and SHA-256. It bundles the binaries with their libraries (including glibc), puts the unmodified binaries back (quick-sharun rewrites `/usr/lib` and `/usr/share` strings, which would point this app's host checks into the AppImage), skips copying system fonts and the GPU driver stack, adds `packaging/cli.hook` (`AppImage cli …`), and packs a uruntime DwarFS AppImage with `gh-releases-zsync` update information, `.zsync` and `.sha256`. It needs `patchelf` and `strace` on the build machine; `packaging/container-build.sh` provides them in Ubuntu 26.04.

### Updating the AppImage tooling

Take the newest `useful-tools/quick-sharun.sh` commit from pkgforge-dev/Anylinux-AppImages, set `QS_COMMIT` and `QS_SHA` (`sha256sum quick-sharun.sh`) in `packaging/build-appimage.sh`, rebuild, check the AppImage size and run it. quick-sharun pins sharun, appimagetool and onelf with their own checksums.

## Dependencies

`verification/check-dependencies.sh` compares every direct crate with crates.io, libcosmic's pinned commit with upstream, and the toolchain with the latest Rust stable. Update with `cargo update` (and edit `Cargo.toml` for new major versions), then run the full verification. The last audit is saved as `verification/dependencies-DATE.txt`.

## Schemas

`verification/update-cli-reference.sh` regenerates `schemas/commands.json`, `schemas/request.json` and `schemas/ui-request.json` from the Rust types.

## Credits

The CLI conventions follow Warehouse COSMIC. Gear Lever's feature set, naming scheme and update-source fields are the reference for parity. The fallback icons are from [COSMIC Icons](https://github.com/pop-os/cosmic-icons) (CC-BY-SA-4.0).
