#!/usr/bin/env bash
# Window test for Update All, Quit / Force Quit and the red Remove button, on
# a disposable home folder (never the real one), with renders of each step.
# The fixture server sends files slowly so the download can be captured.
# A running fixture app leaves a helper that ignores SIGTERM, like Electron
# apps leave a background server, so Quit leaves it and Force Quit stops it.
#
# Usage: verification/gui-quit-and-update-all.sh [BUILD_DIR]   (windows go to "Desktop 2")
# Expected: "ok:" lines, renders under the output folder, "PASS", exit 0.
set -euo pipefail
# Test windows go to a separate virtual desktop (verification/on-desktop.sh),
# never onto the desktop in use. GEARLEVER_TEST_HERE=1 opts out.
if [[ -z ${GEARLEVER_ON_TEST_DESKTOP:-} && -z ${GEARLEVER_TEST_HERE:-} ]]; then
  here=$(dirname -- "$0")
  if [[ -n ${GEARLEVER_DESKTOP_BUS:-} ]]; then exec "$here/on-desktop.sh" "$0" "$@"; fi
  exec "$here/on-desktop.sh" "$here/private-bus.sh" "$0" "$@"
fi
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
[[ -x $bin/gearlever-cosmic && -x $bin/gearlever-cosmic-cli ]] || cargo build --bins
cargo build -q --release --example fixture_runtime
out="$PWD/verification/out-quit-updateall-$(date +%Y%m%d-%H%M%S)"
port=$(( 20000 + RANDOM % 20000 ))
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out" "$port" >/dev/null
exec > >(tee "$out/gui-quit-and-update-all.log") 2>&1
H="$out/home"
mkdir -p "$H/.config/cosmic"
cp -r "$HOME"/.config/cosmic/com.system76.CosmicTheme.* "$HOME"/.config/cosmic/com.system76.CosmicTk "$H/.config/cosmic/" 2>/dev/null || true
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache"
unset HOST_XDG_DATA_HOME
cli="$bin/gearlever-cosmic-cli"
/usr/bin/python3 verification/slow-http-server.py "$port" "$out/server" 0.4 >"$out/server.log" 2>&1 &
server=$!
pids=()
instance=""
cleanup() {
  [[ -n $instance ]] && "$cli" ui --instance "$instance" close --apply >/dev/null 2>&1 || true
  kill "$server" 2>/dev/null || true
  for p in "${pids[@]}"; do kill -KILL "$p" 2>/dev/null || true; done
}
trap cleanup EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }
D="$H/Downloads"
for f in Demo_Editor-1.0-x86_64.AppImage Sky-Notes-2.0.0.AppImage Old_Tool-0.9.AppImage; do
  "$cli" run --apply --cmd "app.integrate={\"file\":\"$D/$f\"}" >/dev/null
done
"$cli" run --apply --cmd "update.set-source={\"app\":\"demo_editor\",\"source\":{\"manager\":\"StaticFileUpdater\",\"url\":\"http://127.0.0.1:$port/Demo_Editor-latest-x86_64.AppImage\"}}" >/dev/null
helper_dir="$out/mount_demo_editor/.mount_demoX"
FIXTURE_SECONDS=600 FIXTURE_HELPER_DIR="$helper_dir" "$H/AppImages/demo_editor.appimage" >"$out/app.log" 2>&1 &
app_pid=$!; pids+=("$app_pid")
for _ in $(seq 1 40); do rg -q 'helper pid' "$out/app.log" && break; sleep 0.1; done
helper_pid=$(rg -o 'helper pid (\d+)' -r '$1' "$out/app.log") || fail "fixture helper did not start"
pids+=("$helper_pid")
ok "fixture app $app_pid with a SIGTERM-ignoring helper $helper_pid"

instance=$("$cli" ui start --config-dir "$H/.config" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["instance"])')
ui() { "$cli" ui --instance "$instance" "$@"; }
state() { ui inspect | python3 -c "import json,sys; s=json.load(sys.stdin)['data']['state']; print(eval(sys.argv[1]))" "$1"; }
ui wait --timeout 30 >/dev/null
[[ $(state "s['icons_missing']") == "[]" ]] || fail "icons not found by the window: $(state "s['icons_missing']")"
ok "window started; all icons resolve"

# App page: Quit next to the running banner, Remove in red.
ui activate apps.select:demo_editor >/dev/null; ui wait --timeout 30 >/dev/null
[[ $(state "'app.quit' in [c['id'] for c in s['controls']]") == True ]] || fail "app.quit control"
ui capture "$out/1-app-running-quit-remove.png" >/dev/null
ok "app page offers Quit; render 1-app-running-quit-remove.png"

# Update All: the Updates page, the batch while it downloads, the result.
ui navigate updates >/dev/null; sleep 0.5; ui wait --timeout 60 >/dev/null
avail=$(state "sorted(r['id'] for r in s['view']['results'] if r['check']['status']=='available')")
[[ $avail == "['demo_editor', 'sky_notes']" ]] || fail "available updates: $avail"
ui capture "$out/2-updates-before-update-all.png" >/dev/null
ui activate updates.all >/dev/null 2>&1 && fail "updates.all must require --apply"
ui activate updates.all --apply >/dev/null
seen=""
for _ in $(seq 1 50); do
  op=$(state "s['operation']")
  if [[ $op != None ]]; then
    ui capture "$out/3-update-all-progress.png" >/dev/null
    seen="$op"; break
  fi
  sleep 0.1
done
[[ -n $seen ]] || fail "no operation seen after Update All"
ok "Update All in progress: $seen; render 3-update-all-progress.png"
ui wait --timeout 120 >/dev/null
[[ $(state "s['last_operation']['ok']") == True ]] || fail "Update All result: $(state "s['last_operation']")"
ui capture "$out/4-update-all-result.png" >/dev/null
lines=$(state "s['last_operation']['lines']")
[[ $lines == "['Demo Editor 1.0 → 1.1', 'Sky Notes 2.0.0 → 2.1.0']" ]] || fail "result lines: $lines"
[[ $(state "s['last_operation']['notes'].count('The replaced version is in the Trash')") == 1 ]] || fail "Trash note should appear once: $(state "s['last_operation']['notes']")"
[[ $(state "'updates.all' in [c['id'] for c in s['controls']]") == False ]] || fail "Update All should be gone with nothing left to install"
[[ $(state "'result.report' in [c['id'] for c in s['controls']]") == True ]] || fail "Open Report control"
ok "result: one line per app ($lines), Trash note once, Open Report, no Update All (0)"
ok "Update All finished: $(state "s['last_operation']['title']"); render 4-update-all-result.png"

# Quit: lists the processes, needs --apply; the helper ignores SIGTERM, so
# Force Quit is offered for what is left.
ui navigate apps >/dev/null; ui activate apps.select:demo_editor >/dev/null; ui wait --timeout 30 >/dev/null
ui capture "$out/5-app-old-version-relaunch-quit.png" >/dev/null
ui activate app.quit >/dev/null
[[ $(state "s['dialog']['kind']") == quit ]] || fail "quit dialog"
kinds=$(state "sorted(p['kind'] for p in s['dialog']['processes'])")
[[ $kinds == "['helper', 'runtime']" ]] || fail "quit should list the helper and the runtime: $kinds"
ui capture "$out/6-quit-dialog.png" >/dev/null
ui activate dialog.confirm >/dev/null 2>&1 && fail "quit confirm must need --apply"
ui activate dialog.confirm --apply >/dev/null
for _ in $(seq 1 80); do [[ $(state "(s['dialog'] or {}).get('kind')") == force_quit ]] && break; sleep 0.25; done
[[ $(state "s['dialog']['kind']") == force_quit ]] || fail "force quit dialog after the wait"
kill -0 "$helper_pid" || fail "the helper should still run after SIGTERM"
kill -0 "$app_pid" || fail "the runtime must not be stopped while the helper uses the mount"
ui capture "$out/7-force-quit-dialog.png" >/dev/null
ok "Quit sent SIGTERM; the helper ignored it and the runtime was left alone; Force Quit offered; renders 6, 7"
ui activate dialog.confirm >/dev/null 2>&1 && fail "force quit confirm must need --apply"
ui activate dialog.confirm --apply >/dev/null
for _ in $(seq 1 60); do kill -0 "$helper_pid" 2>/dev/null || kill -0 "$app_pid" 2>/dev/null || break; sleep 0.25; done
sleep 0.5
kill -0 "$helper_pid" 2>/dev/null && fail "the helper should be gone after Force Quit"
kill -0 "$app_pid" 2>/dev/null && fail "the runtime should be gone after Force Quit"
ui wait --timeout 30 >/dev/null
[[ $(state "s['view']['apps'][0]['running']") == None ]] || fail "still shown as running"
ui capture "$out/8-after-force-quit.png" >/dev/null
ok "Force Quit stopped the helper (SIGKILL) and the runtime; no Running badge; render 8-after-force-quit.png"

# The remove dialog, in dark and in light.
ui activate app.remove >/dev/null
ui capture "$out/9-remove-dialog-dark.png" >/dev/null
ui activate dialog.cancel >/dev/null
ui activate header.settings >/dev/null; ui activate settings.theme:light >/dev/null; ui activate settings.close >/dev/null; sleep 0.5
ui activate app.remove >/dev/null
ui capture "$out/10-remove-dialog-light.png" >/dev/null
ui activate dialog.cancel >/dev/null
ok "remove dialog renders 9 (dark) and 10 (light)"
# Renders from a window that stopped presenting (hidden desktop, screen off)
# all repeat its first frame; such a run proves the logic, not the layout.
same=$(sha256sum "$out"/*.png | awk '{print $1}' | sort | uniq -d | wc -l)
if [[ -n ${GEARLEVER_ON_TEST_DESKTOP:-} ]]; then
  echo "note: the window ran on a hidden test desktop, so its renders show only its first frame; the checks above are the result (layout needs GEARLEVER_TEST_HERE=1)"
else
  [[ $same -eq 0 ]] || fail "$same renders are identical: the window did not redraw (screen off?), so they show nothing; the checks above still passed"
  ok "every render differs"
fi
echo "PASS ($out)"
