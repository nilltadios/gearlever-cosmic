# Feature comparison with Gear Lever

Compared with Gear Lever 4.6.2 (it.mijorus.gearlever, commit a2917f2).

| Gear Lever | Gear Lever COSMIC |
|---|---|
| Integrate with one click, drag and drop, open with | Same (Add page, drop anywhere in the window, file arguments, D-Bus activation of the running window) |
| Move or copy into a custom folder | Same setting and naming (`name.appimage`, `name_VERSION`, `name_N`); a file outside the folder is moved or copied per the setting |
| Keep both / replace on conflict | Same; replace writes in place and keeps arguments, environment and icon |
| Multi-install with "I have verified the source" | Same |
| Launch, terminal apps not launchable | Launch through the menu entry (`gio launch`), including terminal apps |
| Remove (trash or `--delete`) | Trash only; `--delete` is refused. Menu entries made by the app itself are left and named |
| Command line arguments, environment variables, website | Same |
| Reload metadata | Same ("Reload") |
| Update sources: static URL, GitHub, GitLab, Codeberg, Forgejo, FTP, embedded zsync | Same, plus electron-updater feeds; Gear Lever's saved sources are read |
| Update blocked while the app runs ("please close it") | In-place update; the running app keeps working |
| Version from the menu entry (issue #497) | Version from the file; menu entries carry no version |
| "No updates available" for apps without a source or failed checks | Separate statuses: no source, failed, release file not found, updates itself |
| Hashes for new files | For every app and every opened file |
| Architecture warning | Same, plus FUSE 2 / FUSE 3 runtime, fusermount and missing-library checks |
| Metadata via 7zz/unsquashfs/dwarfsextract, optional appimage-extract | Built-in SquashFS and DwarFS readers; appimage-extract only when safe metadata is off |
| Background update check with notification | Same, via an autostart entry |
| Preferences (folder, move/copy, external apps, executable names, safe extractor, minimal preview, debug logs) | Same, plus "keep replaced versions in the Trash" |
| Lists only menu entries with TryExec | Also lists entries that start an AppImage through Exec (shown as made by the app); one row per file |
| CLI: --integrate --update --remove --remove-all --list-installed --list-updates --list-update-managers --set-update-source --fetch-updates | All except --remove-all (use `app.remove` with ids); plus typed commands, schemas, dry-run plans, live window controls |
| Custom icon replaced on update | Custom icons kept; "Use official icon" restores the AppImage's |

Not implemented: Gear Lever's welcome screen (the empty Apps page explains how to start), Type 1 (ISO 9660) metadata reading.

## Measured differences

`verification/ab-gearlever.sh` reproduces these against Gear Lever 4.6.2 on a disposable home folder (Gear Lever's HOME and XDG folders are redirected there too):

| Situation | Gear Lever | Gear Lever COSMIC |
|---|---|---|
| An integrated AppImage is replaced by version 1.1 (as self-updating apps do) | Lists 1.0 (issue #497) | Lists 1.1 |
| An app without any update source | "No updates available" | "No update source" (nothing was checked) |
| Updating an app while it runs | Skipped: "the application is running" | Updated in place; the app keeps running on the old file until restarted |

On real installations the script also lists, read-only, which apps each tool finds (menu entries that start an AppImage through `Exec` without `TryExec` are listed only here) and their update statuses. Its output stays on the machine (`verification/out-ab-*`, not committed).
