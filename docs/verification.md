# Verification

Run from the project root. Every script prints `ok:` lines and `PASS`, exits non-zero on failure, and keeps its fixture, logs and renders under `verification/out-*` (git-ignored). Scripts that change files only use a disposable home folder.

| Check | Command | Covers |
|---|---|---|
| Format, lint, unit tests | `cargo fmt --all --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` | ELF/SquashFS/DwarFS reading, desktop entry edits, update parsing, Gear Lever settings, integrate/replace/keep-both, self-update sync, custom icons, FUSE checks, locking, CLI parsing |
| CLI workflow | `verification/cli-workflow.sh` | Dry runs, integrating, refusing fake AppImages, honest statuses with a local server, **updating a running app in place** (it keeps running on the old file), Trash, self-update menu sync, custom icon through a replace, fuse3-only system, Gear Lever options |
| GUI workflow | `verification/gui-workflow.sh` | A real window driven only by `ui` controls: running badge, `--apply` guards, Updates page, GUI update of a running app, Add page refusal and integration, icons, arguments, environment, remove dialog, renders |
| Auto-sync guardrail | `verification/check-auto-sync.sh` | A normal window refreshes only its own outdated entries and leaves a Gear Lever entry byte for byte |
| FUSE on another system | `verification/check-requirements.sh APP_ID` | A FUSE 2 AppImage reported as missing libfuse.so.2 on a simulated fuse3-only system |
| Dependencies | `verification/check-dependencies.sh` | Crates, libcosmic and Rust against their latest releases |
| A/B with Gear Lever | `verification/ab-gearlever.sh` | Both CLIs on the real installed apps (read-only) and on a fixture home; findings in `docs/parity.md` |
| Resource use vs Gear Lever | `verification/resource-usage.sh [APPIMAGE] [RUNS]` | Start to window, memory (RSS and PSS of the whole process tree, including the AppImage mount), idle CPU, size on disk, CLI time and peak memory, on the same fixture. 2026-11-08, 3 runs, medians: start 0.48 s vs 0.73 s, PSS 73 MB vs 121 MB, CLI listing 0.22 s vs 0.37 s, 19 MB vs 26 MB plus a 1.1 GB runtime |
| Release bump | `verification/check-release-bump.sh [REF]` | Shipped files changed since tag `vVERSION` need a new version and metainfo `<release>`; run by a pre-push hook for Claude Code |
| AppImage | `packaging/build-appimage.sh && dist/*.AppImage cli --version && dist/*.AppImage --preview app --capture /abs.png` | Packaging, bundled glibc, host GPU drivers, CLI hook |

Renders must be looked at: a file existing is not proof that the layout is right. Pointer hover, native file choosers and real compositor drag and drop are not automated; check them by hand.
