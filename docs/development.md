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
| `src/elf.rs` | AppImage type, architecture, payload offset (skipping a DwarFS AppImage's 128 KiB metadata SquashFS sidecar), `.upd_info` |
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

`packaging/build-appimage.sh [BIN_DIR] [OUT_DIR]` uses [quick-sharun](https://github.com/pkgforge-dev/Anylinux-AppImages) pinned to a commit and SHA-256. It bundles the binaries with their libraries (including glibc), puts the unmodified binaries back (quick-sharun rewrites `/usr/lib` and `/usr/share` strings, which would point this app's host checks into the AppImage), skips copying system fonts and the GPU driver stack, adds `packaging/cli.hook` (`AppImage cli …`) and `packaging/vulkan-icd.hook` (resolves bare Vulkan ICD SONAMEs so `cross-libc-dlopen` intercepts host drivers on newer glibc), and packs a uruntime DwarFS AppImage with `gh-releases-zsync` update information, `.zsync` and `.sha256`. It builds for the machine it runs on (`uname -m`, x86_64 or aarch64, each with its own pinned uruntime checksum), refuses binaries built for another architecture, and names the file and its zsync pattern `GearLever_COSMIC-VERSION-ARCH.AppImage`, so each architecture updates only to its own build. It runs on any glibc distribution: it checks for its tools first (`patchelf`, `strings` from binutils, `curl`, coreutils) and prints the install command for dnf, apt, pacman or zypper; it takes the library folder from the one the built binary loads libc from (`/usr/lib64`, `/usr/lib/<arch>-linux-gnu` or `/usr/lib`; set `LIB_DIR` to override); and it sets `DEPLOY_DATADIR=0`, so nothing from the build machine's `/usr/share` is bundled by name guessing (a Fedora desktop build otherwise added 149 MB of ibus dictionaries). Releases are built by `.github/workflows/appimage.yml` in Ubuntu 26.04 on GitHub's x86_64 and arm64 runners; `packaging/container-build.sh` builds the same locally with podman or docker on either architecture. GitHub's arm64 runners use 4 KB memory pages, so run the aarch64 AppImage on an Asahi Linux machine (16 KB pages) before a release that changes packaging.

### Updating the AppImage tooling

Take the newest `useful-tools/quick-sharun.sh` commit from pkgforge-dev/Anylinux-AppImages, set `QS_COMMIT` and `QS_SHA` (`sha256sum quick-sharun.sh`) in `packaging/build-appimage.sh`, rebuild, check the AppImage size and run it. quick-sharun pins sharun, appimagetool and onelf with their own checksums.

## Dependencies

`verification/check-dependencies.sh` compares every direct crate with crates.io, libcosmic's pinned commit with upstream, and the toolchain with the latest Rust stable. Update with `cargo update` (and edit `Cargo.toml` for new major versions), then run the full verification. The last audit is saved as `verification/dependencies-DATE.txt`.

## Schemas

`verification/update-cli-reference.sh` regenerates `schemas/commands.json`, `schemas/request.json` and `schemas/ui-request.json` from the Rust types.

## Credits

The CLI conventions follow Warehouse COSMIC. Gear Lever's feature set, naming scheme and update-source fields are the reference for parity. The fallback icons are from [COSMIC Icons](https://github.com/pop-os/cosmic-icons) (CC-BY-SA-4.0).
