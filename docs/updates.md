# Updates

## Where updates come from

For each app the first available source is used:

1. **A custom source** set in this app (`update.set-source`, or the app page).
2. **Gear Lever's saved source** for the same file, read from `gearlever.conf` (Flatpak or native install). It is used as is until you import it (`gearlever.import`) or set another.
3. **Embedded update information** (`.upd_info` in the AppImage): `gh-releases-zsync|…` or `zsync|URL`.
4. **The app's own updater**: electron-updater (`resources/app-update.yml`). For the `github` and `generic` providers its feed (`latest-linux.yml`) is read.

Source types and fields (Gear Lever's names, so its saved settings and `--set-update-source` options carry over): `StaticFileUpdater` (url), `GithubUpdater` (repo, repo_filename, allow_prereleases), `GitlabUpdater` (repo_url, repo_filename), `CodebergUpdater` (repo, repo_filename, allow_prereleases), `ForgejoUpdater` (repo_url, repo_filename, allow_prereleases), `FTPUpdater` (url, filename). `repo_filename` is a glob; with several matches the one for this machine's architecture is chosen. Set `GITHUB_TOKEN` to raise GitHub's API rate limit.

## Statuses

| Status | Meaning |
|---|---|
| `available` | A different file exists. `can_install` says whether it is installed here. |
| `up_to_date` | Compared with the source and identical. `compared_by` says how. |
| `self_updating` | The app has its own updater. `update_known`: true (newer version seen, open the app), null (could not tell). |
| `no_source` | No source is known. Nothing was checked. |
| `not_found` | The source has no file matching the name pattern. |
| `failed` | The check failed (network, HTTP error, rate limit). Nothing is known. |

Comparison, strongest first: SHA-1 from a `.zsync` file, the SHA-256 digest GitHub publishes per asset, SHA-512 from an electron-updater feed, then the file size. Size is all a plain URL or FTP offers; it cannot tell two builds of equal size apart, and results say `compared_by: size`. Gear Lever reports "No updates available" in all of the non-`available` cases; this app never does.

## Installing an update

The new file is downloaded beside the old one (`.NAME.download-…`), its size and every published checksum are verified, and it must be an AppImage. It is then renamed over the old file in one step. The app does **not** need to be closed: a running instance keeps reading the old file (its open inode) and switches to the new version when restarted; `app.list` shows `running.old_version` meanwhile. With "Keep replaced versions in the Trash" (on by default) the old file goes to the desktop Trash, recoverable. The menu entry is refreshed (version, name, icon), keeping custom arguments, environment and a custom icon; arguments that were never customized follow the new version's defaults.

## Apps that update themselves

When an app replaces its own file (electron-updater does this), a version stored in its menu entry would go stale (Gear Lever issue #497). Versions here always come from the file itself, and the menu entries Gear Lever COSMIC writes carry no `X-AppImage-Version`, so there is nothing to keep in sync. If an app changes its name or icon in an update, the app page's Reload (or `app.sync-menu`) rewrites the menu entry from the file.

An update never closes a running app. When an instance still runs the replaced file, the app page and the update result offer **Relaunch**: after a confirmation, the app's own processes (recognized by `APPIMAGE`/`APPDIR` and a program inside the mount) get SIGTERM, as at logout. Once they have exited (within 20 seconds), the new file is started from its menu entry. Nothing is force-killed: an app that asks to save work or does not quit keeps running and is not started twice. Gear Lever COSMIC relaunching itself closes and starts the new file. Relaunch is not available in the Flatpak.

## Startup check

"Check for updates after login" writes `~/.config/autostart/dev.nilltadios.GearLeverCosmic.desktop`, which runs `gearlever-cosmic-cli fetch-updates` (from the AppImage: `AppImage cli fetch-updates`). It sends one notification naming installable updates and apps that have an update in their own updater, and installs nothing.
