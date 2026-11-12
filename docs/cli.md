# Command line

`gearlever-cosmic-cli` does one invocation's work and exits. It needs no window and no display, and calls the same Rust backend as the desktop app. From an AppImage, prefix every command with the AppImage's path and `cli` (such as `~/AppImages/gear_lever_cosmic.appimage cli`; `guide` and `--help` print the exact command), or put it on PATH with `run --apply --cmd 'cli.link={"enabled":true}'` (a link `~/.local/bin/gearlever-cosmic-cli` to the AppImage; Gear Lever COSMIC also creates it when it adds itself to the app menu, and the Settings page has a switch for it).

## Scope

The CLI manages AppImages as files and menu entries: it integrates them (copies or moves the file into the AppImages folder, extracts the icon, writes `~/.local/share/applications/NAME.desktop`), updates them, edits their launch options and icon, and moves them to the Trash. It reads Gear Lever's settings (`gearlever.conf`) but never writes them. It starts another program only with `--apply` (`app.launch`, `app.open-folder`). The `ui` commands talk only to Gear Lever COSMIC windows (D-Bus names starting with `dev.nilltadios.GearLeverCosmic.UI.`). Host programs are a fixed list: `gio` (launch, trash), `xdg-open`, `setsid`, `ps`, `update-desktop-database`, `notify-send`.

## Discover and read

```sh
gearlever-cosmic-cli guide
gearlever-cosmic-cli commands
gearlever-cosmic-cli commands --json
gearlever-cosmic-cli describe app.update
gearlever-cosmic-cli schema
gearlever-cosmic-cli inspect
gearlever-cosmic-cli run --cmd 'app.list={}'
gearlever-cosmic-cli run --cmd 'update.check={"apps":[]}'
gearlever-cosmic-cli run --cmd 'file.inspect={"file":"/home/me/Downloads/App.AppImage","hashes":true}'
gearlever-cosmic-cli run --cmd 'app.requirements={"app":"vscodium"}'
```

An app is named by its id (the menu entry's file name without `.desktop`, from `app.list`), its AppImage path, its menu entry path, or its exact name. Ambiguous names are refused with the matching ids. Unknown commands, fields or wrong types are errors; schemas and examples come from the same Rust types that parse requests.

`app.list` returns, per app, `version` (read from the AppImage file now), `running` (`pids`, `old_version` when a replaced file still runs), the update source and its origin, `managed_by` (`gear_lever_cosmic`, `gear_lever`, `other`), `flatpak` (the app id when the same app is also installed as a Flatpak, matched by window class or name, so the menu shows it twice), and `standard_path` when the file in the AppImages folder still carries a version or other non-standard name; `app.rename-file` renames it (with its menu entry and icons) to that path, also while the app runs. Its `summary` names the folder and how many menu entries were scanned, so an empty list is distinguishable from a scan that found nothing.

## Plan and apply

```sh
gearlever-cosmic-cli run --cmd 'app.integrate={"file":"/home/me/Downloads/App.AppImage"}'
gearlever-cosmic-cli run --apply --cmd 'app.integrate={"file":"/home/me/Downloads/App.AppImage"}'
```

Without `--apply` every change returns `mode: "dry_run"` with a `plan` of effects naming exact paths. With `--apply` it returns `mode: "applied"` and the operation reports: steps, notes (for example "still running the previous version"), errors, `changed` and `trashed` paths, and the durable report file in `~/.local/share/gearlever-cosmic/operations/`.

`app.integrate` refuses when another version of the app is integrated unless `conflict` is `keep_both` or `replace` (exit 2, code `conflict`, with the integrated versions in `details`). It refuses a file that is already integrated (`already_integrated`) and any file that is not an AppImage by content, whatever its name.

`app.update` with named apps fails with code `no_update` (exit 3) and the check in `details` when there is nothing installable; with `all: true` it updates what can be updated and leaves the rest.

## Gear Lever's options

`--integrate FILE [--keep-both|--replace] [--update-url URL]`, `--update APP|--all`, `--remove APP`, `--list-installed [--json]`, `--list-updates [--json] [--all-statuses]`, `--list-update-managers`, `--set-update-source APP --manager NAME KEY=VALUE... | --unset`, `--fetch-updates`. APP is an id, name, AppImage path or menu entry, as in `run`; an unknown APP lists the known ids. The dash-less forms (`integrate FILE`) work too. Changes print a plan unless `-y`/`--yes`/`--apply` is given. `--remove --delete` is refused: removal always uses the Trash. `--update --force` is accepted and ignored, since running apps are always updated in place. The JSON documents keep Gear Lever's `schema_version: 1` fields and add `status`, `message` and `can_install`. `--list-updates` without `--all-statuses` lists installable updates and then names the apps that could not be checked, instead of saying "No updates available".

## Scripts

Repeated `--cmd` values run in order. `--script FILE` accepts `{"schema_version":1,"commands":[...]}`, a JSON array, or text with one `command={json}` or JSON request per line (`#` comments allowed). Everything is parsed before the first command runs; execution stops at the first failure and earlier applied steps stay applied.

## Output and exit codes

`run` and `inspect` print one envelope: `{"schema":"gearlever-cosmic/1","ok":…,"results":[…],"error":…}`. Diagnostics also go to stderr.

| Exit | Meaning |
|---|---|
| 0 | Everything succeeded, or every plan was valid |
| 1 | Backend or I/O failure, including a partly applied operation |
| 2 | Invalid request, conflict, already integrated, not an AppImage |
| 3 | Missing or ambiguous app, or no installable update |
| 4 | A path outside the granted automation roots |
| 5 | Cancelled (Ctrl+C); completed steps stay applied |
| 6 | Another operation holds the lock |

## File access roots

`--automation-read-root DIR` and `--automation-write-root DIR` confine paths given in requests (files to integrate, icons, report files). If either is given, both permissions become explicit. The AppImages folder, menu entries and this app's own data are not restricted by them.

## Recovery

Removed AppImages, menu entries and icons, replaced versions and trashed custom icons are in the desktop Trash. Every rewritten menu entry is first copied to `~/.local/share/gearlever-cosmic/menu-entry-backups/`. Settings and `apps.json` keep their last five versions as `NAME.bak-*`.
