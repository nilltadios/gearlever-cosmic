# Gear Lever COSMIC

Add AppImages to your app menu, keep them updated, and see clearly what can and cannot be updated. Inspired by [Gear Lever](https://github.com/mijorus/gearlever), rewritten in Rust with libcosmic, and usable by scripts and AI agents through a command-line tool.

## Install

Download `GearLever_COSMIC-VERSION-x86_64.AppImage` from the [latest release](https://github.com/nilltadios/gearlever-cosmic/releases/latest), make it executable and open it. It runs on any Linux distribution (built the [Anylinux](https://github.com/pkgforge-dev/Anylinux-AppImages) way: it carries its own libraries, needs no FUSE 2, and uses your system's GPU drivers).

To add it to your app menu, open the AppImage with itself: drop it on its own window, or open it from the Add page. After that it updates like any other app it manages; the release carries `.zsync` update information.

## What it does

- **Integrate:** move or copy AppImages into a folder (default `~/AppImages`), extract their icons and add menu entries. Drag and drop, multi-file adding, "keep both" or "replace" when another version exists.
- **Update in place:** updates download beside the old file, are verified, and replace it in one step. **Running apps keep working** and switch to the new version when they restart. The replaced version goes to the Trash. Sources: GitHub, GitLab, Codeberg, Forgejo/Gitea, static URLs, FTP, update information embedded by developers, and electron-updater feeds. Gear Lever's saved sources are picked up automatically.
- **Honest statuses:** "Up to date" only after a real comparison. Apps without a source, failed checks and apps that **update themselves** are labelled as such. If an app has its own updater and a newer version exists, it says so and tells you to open the app (or installs it here when the app's feed allows).
- **Real versions:** names and versions are read from the AppImage itself, so a file that was replaced or updated itself never shows a stale version ([Gear Lever #497](https://github.com/mijorus/gearlever/issues/497)).
- **Will it start?** Checks whether the AppImage's runtime needs FUSE 2 (`libfuse.so.2`) or only `fusermount`, and which libraries its program needs that are neither bundled nor installed.
- **Launch options:** command line arguments, environment variables, website, and **your own icon**, which updates keep; "Use Official Icon" brings back the AppImage's.
- **Pure Rust:** SquashFS and DwarFS images are read in-process; nothing is executed to read metadata.

Nothing is deleted permanently: removed AppImages, menu entries and icons go to the desktop Trash, and rewritten menu entries are backed up first.

## Command line and AI agents

```sh
GearLever_COSMIC-x86_64.AppImage cli guide
GearLever_COSMIC-x86_64.AppImage cli run --cmd 'update.check={}'
GearLever_COSMIC-x86_64.AppImage cli --list-updates --all-statuses
```

`guide` is the starting point. Changes are dry-run plans until `--apply`. Gear Lever's own options (`--integrate`, `--update`, `--list-updates --json`, …) work too. The open window can be inspected and operated with `cli ui …`. See the [CLI guide](docs/cli.md), the [agent workflow](docs/agents.md), [live window controls](docs/ui-cli.md) and [how updates work](docs/updates.md).

## Build from source

```sh
cargo run --locked --bin gearlever-cosmic
packaging/build-appimage.sh            # AppImage from target/release, on any glibc distro (needs patchelf, binutils)
packaging/container-build.sh           # the same inside Ubuntu 26.04 (podman or docker)
```

See [development](docs/development.md) and [verification](docs/verification.md).

## More

- [Feature comparison with Gear Lever](docs/parity.md)

## License

GPL-3.0-or-later. An independent project, not maintained by the Gear Lever developers. See [NOTICE](NOTICE).
