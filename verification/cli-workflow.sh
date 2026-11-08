#!/usr/bin/env bash
# End-to-end CLI workflow on a disposable home folder. Never touches the real
# home: HOME and XDG folders point into verification/out-cli-*/home, and the
# update server listens on 127.0.0.1 only.
#
# Usage: verification/cli-workflow.sh [CLI]   (default target/release/gearlever-cosmic-cli)
# Expected: every "ok:" line, final "PASS", exit 0. A failed check prints
# "FAIL: ..." and exits 1. The log is saved next to the fixture.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
cli="$(realpath "${1:-target/release/gearlever-cosmic-cli}")"
[[ -x $cli ]] || cargo build --release --no-default-features --bin gearlever-cosmic-cli
cargo build -q --release --example fixture_runtime
out="$PWD/verification/out-cli-$(date +%Y%m%d-%H%M%S)"
port=$(( 20000 + RANDOM % 20000 ))
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out" "$port" >/dev/null
exec > >(tee "$out/workflow.log") 2>&1
H="$out/home"
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache"
unset HOST_XDG_DATA_HOME
python3 -m http.server "$port" -d "$out/server" -b 127.0.0.1 >"$out/server.log" 2>&1 &
server=$!
pids=()
cleanup() { kill "$server" 2>/dev/null || true; for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT
sleep 0.5
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }
run() { "$cli" run "$@"; }
jq_py() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }

D="$H/Downloads"
# Plans change nothing.
run --cmd "app.integrate={\"file\":\"$D/Demo_Editor-1.0-x86_64.AppImage\"}" | jq_py 'd["results"][0]["mode"]' | grep -qx dry_run || fail "integrate without --apply must be a plan"
[[ -f $D/Demo_Editor-1.0-x86_64.AppImage && ! -e $H/AppImages/demo_editor.appimage ]] || fail "a plan changed files"
ok "dry run changes nothing"
for f in Demo_Editor-1.0-x86_64.AppImage Sky-Notes-2.0.0.AppImage Old_Tool-0.9.AppImage mystery.AppImage termtool-x86_64.AppImage; do
  run --apply --cmd "app.integrate={\"file\":\"$D/$f\"}" >/dev/null || fail "integrate $f"
done
[[ ! -e $D/Demo_Editor-1.0-x86_64.AppImage ]] || fail "move_on_integrate should move the file"
n=$(run --cmd 'app.list={}' | jq_py 'len(d["results"][0]["data"]["apps"])')
[[ $n == 5 ]] || fail "expected 5 apps, got $n"
ok "integrated 5 apps (files moved into ~/AppImages)"
"$cli" run --cmd "app.integrate={\"file\":\"$D/fake.AppImage\"}" >/dev/null 2>&1 && fail "a shell script must be refused"
ok "a non-AppImage named .AppImage is refused"
grep -q '^Exec=env DESKTOPINTEGRATION=1 '"$H"'/AppImages/demo_editor.appimage %U$' "$H/.local/share/applications/demo_editor.desktop" || fail "menu entry Exec"
ok "menu entry written: $(grep ^Exec= "$H/.local/share/applications/demo_editor.desktop")"

# Update sources and honest statuses.
run --apply --cmd "update.set-source={\"app\":\"demo_editor\",\"source\":{\"manager\":\"StaticFileUpdater\",\"url\":\"http://127.0.0.1:$port/Demo_Editor-latest-x86_64.AppImage\"}}" >/dev/null
run --cmd 'update.check={}' > "$out/check.json"
status() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print({r['app']:r['check']['status'] for r in d['results'][0]['data']['results']}[sys.argv[2]])" "$out/check.json" "$1"; }
[[ $(status demo_editor) == available ]] || fail "demo_editor should have an update: $(status demo_editor)"
[[ $(status sky_notes) == available ]] || fail "sky_notes self-updater feed should offer 2.1.0: $(status sky_notes)"
[[ $(status mystery_app) == no_source ]] || fail "mystery_app must say no_source, not up to date: $(status mystery_app)"
ok "statuses: demo_editor=available sky_notes=available (via its own feed, sha512) mystery_app=no_source"
"$cli" list-updates | tee "$out/list-updates.txt" | grep -q "Not checked or unknown" || fail "list-updates must name unchecked apps"
ok "list-updates names apps that could not be checked instead of claiming 'no updates'"

# Update while the app is running.
FIXTURE_SECONDS=120 "$H/AppImages/demo_editor.appimage" >/dev/null 2>&1 &
pids+=($!); app_pid=$!
sleep 0.5
running=$(run --cmd 'app.list={}' | jq_py '[a["running"] for a in d["results"][0]["data"]["apps"] if a["id"]=="demo_editor"][0]')
[[ $running == *"$app_pid"* ]] || fail "running pid not detected: $running"
ok "running instance detected (pid $app_pid)"
run --apply --cmd 'app.update={"apps":["demo_editor"]}' > "$out/update.json" || fail "update failed: $(cat "$out/update.json")"
grep -q "still running the previous version" "$out/update.json" || fail "update should note the running instance"
kill -0 "$app_pid" || fail "the running app was stopped by the update"
[[ "$(readlink /proc/$app_pid/exe)" == *"(deleted)" ]] || fail "running instance should still use the old file"
v=$(run --cmd 'app.info={"app":"demo_editor"}' | jq_py 'd["results"][0]["data"]["version"], d["results"][0]["data"]["menu_version"], d["results"][0]["data"]["running"]["old_version"]')
[[ $v == "('1.1', '1.1', True)" ]] || fail "after update expected 1.1 in file and menu, old version running: $v"
ok "updated to 1.1 in place while running; the app kept running on the old file; menu shows 1.1"
ls "$H/.local/share/Trash/files" | grep -q "demo_editor.appimage.replaced" || fail "replaced version should be in the Trash"
ok "replaced version kept in the Trash"

# Self-update simulation: the file changes under the menu entry (issue #497).
cp "$out/server/sky-notes/Sky-Notes-2.1.0.AppImage" "$H/AppImages/.tmp" && mv "$H/AppImages/.tmp" "$H/AppImages/sky_notes.appimage"
v=$(run --cmd 'app.info={"app":"sky_notes"}' | jq_py 'd["results"][0]["data"]["version"], d["results"][0]["data"]["menu_version"], d["results"][0]["data"]["menu_outdated"]')
[[ $v == "('2.1.0', '2.0.0', ['version', 'file'])" ]] || fail "self-update should be visible: $v"
run --apply --cmd 'app.sync-menu={}' >/dev/null
grep -q '^X-AppImage-Version=2.1.0$' "$H/.local/share/applications/sky_notes.desktop" || fail "sync-menu should write 2.1.0"
ok "a self-updated file shows its real version (2.1.0, menu said 2.0.0) and sync-menu fixes the menu entry"

# Custom icon kept across an update.
printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="gold"/></svg>' > "$H/my.svg"
run --apply --cmd "app.set-icon={\"app\":\"old_tool\",\"file\":\"$H/my.svg\"}" >/dev/null
cp "$D/../AppImages/old_tool.appimage" "$H/old-copy.AppImage"
run --apply --cmd "app.integrate={\"file\":\"$H/old-copy.AppImage\",\"conflict\":\"replace\"}" >/dev/null 2>&1 || true
grep -q "^Icon=$H/AppImages/.icons/old_tool-custom.svg$" "$H/.local/share/applications/old_tool.desktop" || fail "custom icon lost"
run --apply --cmd 'app.reset-icon={"app":"old_tool"}' >/dev/null
grep -q "^Icon=$H/AppImages/.icons/old_tool.svg$" "$H/.local/share/applications/old_tool.desktop" || fail "official icon not restored"
ok "custom icon kept through a replace; official icon restored on request"

# Requirements on a simulated fuse3-only system.
root="$out/sys"; mkdir -p "$root/usr/lib" "$root/usr/bin"; : > "$root/usr/bin/fusermount3"
v=$(GEARLEVER_COSMIC_SYSTEM_ROOT="$root" run --cmd 'app.requirements={"app":"old_tool"}' | jq_py 'd["results"][0]["data"]["verdict"], d["results"][0]["data"]["runtime"]')
[[ $v == "('missing', 'fuse2')" ]] || fail "fuse2 app on fuse3-only system: $v"
ok "a FUSE 2 AppImage is reported as missing libfuse.so.2 on a fuse3-only system"

# Gear Lever compatible flags.
"$cli" --list-installed --json | jq_py 'd["schema_version"], len(d["installed"])' | grep -qx "(1, 5)" || fail "--list-installed --json"
"$cli" --remove "$H/AppImages/mystery_app.appimage" | grep -q "Nothing was changed" || fail "--remove without -y must be a plan"
"$cli" --remove "$H/AppImages/mystery_app.appimage" -y >/dev/null
[[ ! -e $H/AppImages/mystery_app.appimage && ! -e $H/.local/share/applications/mystery_app.desktop ]] || fail "remove -y"
ls "$H/.local/share/Trash/files" | grep -q '^mystery_app.appimage$' || fail "removed file should be in the Trash"
ok "Gear Lever flags work; --remove -y moved the app to the Trash"
echo "PASS ($out)"
