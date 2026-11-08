#!/usr/bin/env bash
# Drive a real Gear Lever COSMIC window through its `ui` controls on a
# disposable home folder (never the real one). Needs a Wayland session.
# The desktop's COSMIC theme files are copied (read-only) into the fixture so
# renders use the real accent; ~/.config/cosmic itself is never written.
#
# Usage: verification/gui-workflow.sh [BUILD_DIR]   (default target/debug)
# Expected: "ok:" lines, renders under the output folder, "PASS", exit 0.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
[[ -x $bin/gearlever-cosmic && -x $bin/gearlever-cosmic-cli ]] || cargo build --bins
cargo build -q --release --example fixture_runtime
out="$PWD/verification/out-gui-$(date +%Y%m%d-%H%M%S)"
port=$(( 20000 + RANDOM % 20000 ))
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out" "$port" >/dev/null
exec > >(tee "$out/gui-workflow.log") 2>&1
H="$out/home"
mkdir -p "$H/.config/cosmic"
cp -r "$HOME"/.config/cosmic/com.system76.CosmicTheme.* "$HOME"/.config/cosmic/com.system76.CosmicTk "$H/.config/cosmic/" 2>/dev/null || true
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache"
unset HOST_XDG_DATA_HOME
cli="$bin/gearlever-cosmic-cli"
python3 -m http.server "$port" -d "$out/server" -b 127.0.0.1 >"$out/server.log" 2>&1 &
server=$!
pids=()
instance=""
cleanup() {
  [[ -n $instance ]] && "$cli" ui --instance "$instance" close --apply >/dev/null 2>&1 || true
  kill "$server" 2>/dev/null || true
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
}
trap cleanup EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }
D="$H/Downloads"
# Setup through the CLI: two apps integrated, a source for Demo Editor.
for f in Demo_Editor-1.0-x86_64.AppImage Old_Tool-0.9.AppImage; do "$cli" run --apply --cmd "app.integrate={\"file\":\"$D/$f\"}" >/dev/null; done
"$cli" run --apply --cmd "update.set-source={\"app\":\"demo_editor\",\"source\":{\"manager\":\"StaticFileUpdater\",\"url\":\"http://127.0.0.1:$port/Demo_Editor-latest-x86_64.AppImage\"}}" >/dev/null
FIXTURE_SECONDS=300 "$H/AppImages/demo_editor.appimage" >/dev/null 2>&1 &
pids+=($!); app_pid=$!

instance=$("$cli" ui start --config-dir "$H/.config" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["instance"])')
ui() { "$cli" ui --instance "$instance" "$@"; }
state() { ui inspect | python3 -c "import json,sys; s=json.load(sys.stdin)['data']['state']; print(eval(sys.argv[1]))" "$1"; }
ui wait --timeout 30 >/dev/null
ok "window started ($instance)"
[[ $(state "[a['id'] for a in s['view']['apps']]") == "['demo_editor', 'old_tool']" ]] || fail "apps listed"
ui activate apps.select:demo_editor >/dev/null; ui wait --timeout 30 >/dev/null
[[ $(state "s['view']['apps'][0]['running'] is not None") == True ]] || fail "running badge"
ui capture "$out/1-app-running.png" >/dev/null
ok "details page shows the app as running; render 1-app-running.png"

# Guarded controls refuse without --apply.
ui activate app.update 2>/dev/null && fail "app.update must require --apply"
ok "app.update refuses without --apply"
ui navigate updates >/dev/null; sleep 0.5; ui wait --timeout 60 >/dev/null
[[ $(state "{r['id']: r['check']['status'] for r in s['view']['results']}") == "{'demo_editor': 'available', 'old_tool': 'no_source'}" ]] || fail "update statuses: $(state "s['view']['results']")"
ui capture "$out/2-updates.png" >/dev/null
ok "Updates page: demo_editor available, old_tool no_source; render 2-updates.png"
ui activate updates.demo_editor.update --apply >/dev/null; sleep 0.5; ui wait --timeout 60 >/dev/null
notes=$(state "s['last_operation']['notes']")
[[ $notes == *"still running the previous version"* ]] || fail "update notes: $notes"
kill -0 "$app_pid" || fail "the running app was stopped"
[[ "$(readlink /proc/$app_pid/exe)" == *"(deleted)" ]] || fail "running app should keep the old file"
ui capture "$out/3-after-update.png" >/dev/null
ok "updated from the GUI while running; app kept running; note shown; render 3-after-update.png"

# Add page: open a file like the file chooser/drop would.
ui open "$D/Sky-Notes-2.0.0.AppImage" "$D/fake.AppImage" >/dev/null; sleep 0.5; ui wait --timeout 30 >/dev/null
[[ $(state "s['page_id']") == add ]] || fail "open should show the Add page"
[[ $(state "len(s['view']['pending'])") == 1 ]] || fail "only the real AppImage should be pending"
[[ $(state "s['notice'] is not None and 'fake.AppImage' in s['notice']") == True ]] || fail "the fake file should be refused with a message"
ui capture "$out/4-add.png" >/dev/null
ui activate add.0.integrate --apply >/dev/null; sleep 0.5; ui wait --timeout 30 >/dev/null
[[ -f $H/AppImages/sky_notes.appimage ]] || fail "integrate from the Add page"
ok "Add page refused the fake file, integrated Sky Notes; render 4-add.png"

# Custom icon through the GUI fields.
printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><circle cx="4" cy="4" r="4" fill="purple"/></svg>' > "$H/mine.svg"
ui navigate apps >/dev/null; ui activate apps.select:old_tool >/dev/null; ui wait >/dev/null
ui fill icon.path "$H/mine.svg" >/dev/null
ui activate app.icon.set --apply >/dev/null; sleep 0.3; ui wait >/dev/null
grep -q "old_tool-custom.svg" "$H/.local/share/applications/old_tool.desktop" || fail "custom icon"
ui activate app.icon.reset --apply >/dev/null; sleep 0.3; ui wait >/dev/null
grep -q "^Icon=$H/AppImages/.icons/old_tool.svg$" "$H/.local/share/applications/old_tool.desktop" || fail "official icon"
ok "custom icon set and reset through GUI controls"

# Arguments and environment.
ui fill app.args "--verbose %U" >/dev/null
ui activate app.args.save --apply >/dev/null; sleep 0.3; ui wait >/dev/null
grep -q -- "old_tool.appimage --verbose %U$" "$H/.local/share/applications/old_tool.desktop" || fail "arguments"
ui activate app.env.add >/dev/null
n=$(state "len(s['view']['selected']['edits']['env']) - 1")
ui fill "app.env.$n.key" GDK_BACKEND >/dev/null; ui fill "app.env.$n.value" wayland >/dev/null
ui activate app.env.save --apply >/dev/null; sleep 0.3; ui wait >/dev/null
grep -q "^Exec=env DESKTOPINTEGRATION=1 GDK_BACKEND=wayland " "$H/.local/share/applications/old_tool.desktop" || fail "environment"
ok "arguments and environment saved to the menu entry"

# Remove asks first and goes to the Trash.
ui activate app.remove >/dev/null
[[ $(state "s['dialog']['kind']") == remove ]] || fail "remove dialog"
ui capture "$out/5-remove-dialog.png" >/dev/null
ui activate dialog.confirm 2>/dev/null && fail "confirm must need --apply"
ui activate dialog.confirm --apply >/dev/null; sleep 0.3; ui wait >/dev/null
[[ ! -e $H/AppImages/old_tool.appimage ]] && ls "$H/.local/share/Trash/files" | grep -q '^old_tool.appimage$' || fail "remove to Trash"
ok "remove asked first, needed --apply, and moved files to the Trash; render 5-remove-dialog.png"

ui activate header.settings >/dev/null
ui capture "$out/6-settings.png" >/dev/null
# The CLI-on-PATH switch: needs --apply, creates the link, and turns it off again.
ui activate settings.cli-link 2>/dev/null && fail "settings.cli-link must require --apply"
ui activate settings.cli-link --apply >/dev/null; sleep 0.3; ui wait >/dev/null
[[ -L $H/.local/bin/gearlever-cosmic-cli ]] && "$H/.local/bin/gearlever-cosmic-cli" --version >/dev/null || fail "settings.cli-link on"
ui activate settings.cli-link --apply >/dev/null; sleep 0.3; ui wait >/dev/null
[[ ! -L $H/.local/bin/gearlever-cosmic-cli ]] || fail "settings.cli-link off"
ok "the Settings switch puts gearlever-cosmic-cli on PATH and takes it off"
ui activate settings.close >/dev/null
# Narrow layout: a separate capture instance at the minimum width (live
# resizes depend on the compositor). It exits after saving its render.
"$bin/gearlever-cosmic" --preview app-narrow --capture "$out/7-narrow.png" >/dev/null
ok "settings and narrow renders saved"
echo "PASS ($out)"
