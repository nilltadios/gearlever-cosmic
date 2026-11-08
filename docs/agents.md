# Agent workflow

Use `gearlever-cosmic-cli` (from the AppImage: `GearLever_COSMIC-x86_64.AppImage cli`). Run `guide` first.

1. Discover with `commands --json` or `describe ID`. Do not invent fields.
2. Read the state: `app.list`, `update.check`, `app.requirements`, `file.inspect`. Check `warnings`.
3. Use the ids `app.list` returns. Read `running` before telling a person an update "worked": a running app keeps the old version until it restarts, and `running.old_version` says so.
4. Plan changes without `--apply`; read the `plan`.
5. Apply only what the person asked for. Integrating, updating and removing change their files and menu.
6. Verify: `ok`, exit code, the report's `steps`, `notes`, `errors`, then read the state again (`app.info`).

Report update results literally. `up_to_date` is the only status that means a comparison found the installed file current; say how it was compared (`compared_by`: sha1, sha256, sha512 or size — size comparisons cannot tell same-size builds apart). `no_source` means nothing was checked, `failed` means the check itself failed, `self_updating` means the app has its own updater: if `update_known` is true, tell the person to open the app to update it (or install it here when `can_install` is true).

To work in the open window instead, see [live window controls](ui-cli.md): `ui inspect` lists the page content, dialog, fields and every control with what it needs; `ui activate ID [--apply]`, `ui wait`, then inspect again.

To test without touching real apps, give the CLI a disposable home: `HOME=/tmp/x XDG_CONFIG_HOME=/tmp/x/.config XDG_DATA_HOME=/tmp/x/.local/share gearlever-cosmic-cli ...`. `verification/cli-workflow.sh` does exactly that with generated AppImages and a local update server.
