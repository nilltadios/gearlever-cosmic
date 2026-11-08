# Live window controls

`gearlever-cosmic-cli ui …` operates a running Gear Lever COSMIC window through the D-Bus interface the window owns (`dev.nilltadios.GearLeverCosmic.UI.<instance>`, method `Execute`). It cannot address any other application. No pointer or keyboard input is synthesized; activating a control runs the same message as clicking it.

```sh
gearlever-cosmic-cli ui start                       # present the window or start one
gearlever-cosmic-cli ui start --config-dir DIR      # isolated test instance
gearlever-cosmic-cli ui instances
gearlever-cosmic-cli ui inspect                     # page, dialog, view, fields, controls
gearlever-cosmic-cli ui activate apps.select:vscodium
gearlever-cosmic-cli ui activate app.update --apply
gearlever-cosmic-cli ui fill app.args "--ozone-platform=wayland %U"
gearlever-cosmic-cli ui open /home/me/Downloads/App.AppImage
gearlever-cosmic-cli ui wait
gearlever-cosmic-cli ui capture /abs/new.png
gearlever-cosmic-cli ui close --apply
```

Each control in `inspect` has `requires`: `none` or `apply`. Changes to files, menu entries and settings, and controls that start another program (launch, run without adding, open folder or website), need `--apply`. Removal opens a confirmation dialog; `dialog.confirm` needs `--apply`.

Pages: `apps` (list, details with updates, requirements, icon, launch options, website), `updates` (grouped by status, Update All), `add` (opened files with metadata, requirements, conflict choice). `ui open` and `ui drop` take the same path as the file chooser and drag and drop. File choosers themselves are not automated: use `ui open FILE`, `ui fill icon.path FILE` + `app.icon.set`.

The two divider lines (sidebar | page, app list | details) can be dragged with the mouse; agents set the same widths with `ui fill nav.width N` and `ui fill list.width N` (logical pixels, clamped; saved with the window size).

Test instances (`--config-dir`, or `GEARLEVER_COSMIC_UI_ISOLATED=1`) never refresh menu entries automatically and do not save window state. `verification/gui-workflow.sh` drives a full session on a disposable home folder.

Render captures without a live window: `gearlever-cosmic --preview PAGE --capture /abs/out.png` (pages: apps, app, updates, add, drop, settings, about, remove; append `-narrow` for the minimum width). The process exits after saving.
