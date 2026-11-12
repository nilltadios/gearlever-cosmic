#!/usr/bin/env bash
# A/B comparison with Gear Lever (Flatpak it.mijorus.gearlever).
#   Part A: both CLIs on the real installed apps, read-only.
#   Part B: both CLIs on a disposable fixture home: issue #497 (a replaced
#           file keeps showing the old version), updating a running app, and
#           update statuses for an app without a source.
# Gear Lever's HOME and XDG folders are redirected into the fixture for part B,
# so its real gearlever.conf and settings are not touched.
# Usage: verification/ab-gearlever.sh [CLI]   Output: verification/out-ab-*/
# Expected: "PASS" lines for each documented difference, exit 0.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
cli="$(realpath "${1:-target/release/gearlever-cosmic-cli}")"
gl=(flatpak run it.mijorus.gearlever)
flatpak info it.mijorus.gearlever >/dev/null || { echo "Gear Lever is not installed"; exit 1; }
out="$PWD/verification/out-ab-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"
exec > >(tee "$out/ab.log") 2>&1
ok() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; exit 1; }
echo "Gear Lever $(flatpak info it.mijorus.gearlever | sed -n 's/^ *Version: //p'), commit $(flatpak info it.mijorus.gearlever | sed -n 's/^ *Commit: //p' | cut -c1-12)"
echo "Gear Lever COSMIC $("$cli" --version)"

echo "== Part A: real installed apps (read-only)"
"${gl[@]}" --list-installed > "$out/A-gearlever-list.txt" 2>/dev/null
"${gl[@]}" --list-installed --json > "$out/A-gearlever-list.json" 2>/dev/null
"${gl[@]}" --list-updates --json > "$out/A-gearlever-updates.json" 2>/dev/null || true
"$cli" list-installed > "$out/A-ours-list.txt"
"$cli" --list-installed --json > "$out/A-ours-list.json"
"$cli" run --cmd 'update.check={}' > "$out/A-ours-check.json"
python3 - "$out" <<'PY'
import json, os, subprocess, sys
out = sys.argv[1]
gl = {a["path"]: a for a in json.load(open(f"{out}/A-gearlever-list.json"))["installed"]}
ours = {a["path"]: a for a in json.load(open(f"{out}/A-ours-list.json"))["installed"]}
print(f"apps listed: Gear Lever {len(gl)}, ours {len(ours)}")
for p in sorted(set(gl) | set(ours)):
    g, o = gl.get(p), ours.get(p)
    gv = (g['current_version'] or 'unknown') if g else '(not listed)'
    ov = (o['current_version'] or 'unknown') if o else '(not listed)'
    print(f"  {os.path.basename(p):42} GL {gv:>20}   ours {ov:>20}")
gu = json.load(open(f"{out}/A-gearlever-updates.json")) if os.path.getsize(f"{out}/A-gearlever-updates.json") else {"updates": []}
print(f"Gear Lever --list-updates: {len(gu['updates'])} updates")
checks = json.load(open(f"{out}/A-ours-check.json"))["results"][0]["data"]
print("ours update.check:", json.dumps(checks["summary"]))
for r in checks["results"]:
    print(f"  {r['name']:20} {r['check']['status']:14} {r['check']['message'][:110]}")
PY

echo "== Part B: fixture home"
cargo build -q --release --example fixture_runtime
port=$(( 20000 + RANDOM % 20000 ))
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out/fx" "$port" >/dev/null
H="$out/fx/home"
python3 -m http.server "$port" -d "$out/fx/server" -b 127.0.0.1 >"$out/server.log" 2>&1 &
server=$!; pids=()
trap 'kill $server 2>/dev/null || true; for p in "${pids[@]}"; do kill $p 2>/dev/null || true; done' EXIT
glh=("${gl[0]}" "${gl[1]}" --env=HOME="$H" --env=XDG_CONFIG_HOME="$H/.var/gl/config" --env=XDG_DATA_HOME="$H/.var/gl/data" --env=XDG_CACHE_HOME="$H/.var/gl/cache" "${gl[2]}")
ours() { HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" "$cli" "$@"; }
sleep 0.5
# Gear Lever integrates Demo Editor 1.0 (it moves the file into ~/AppImages).
"${glh[@]}" --integrate "$H/Downloads/Demo_Editor-1.0-x86_64.AppImage" -y > "$out/B-gl-integrate.txt" 2>&1
cat "$out/B-gl-integrate.txt" | tail -2
app=$(ls "$H"/AppImages/*.appimage | head -1)
[[ -f $app ]] || fail "Gear Lever did not integrate the fixture"
echo "Gear Lever integrated: $app"
# The app replaces its own file with 1.1 (what self-updating apps do).
cp "$out/fx/server/Demo_Editor-latest-x86_64.AppImage" "$app.new" && mv "$app.new" "$app"
"${glh[@]}" --list-installed > "$out/B-gl-after-selfupdate.txt" 2>/dev/null
ours --list-installed > "$out/B-ours-after-selfupdate.txt"
cat "$out/B-gl-after-selfupdate.txt" "$out/B-ours-after-selfupdate.txt"
grep -q '\[1.0\]' "$out/B-gl-after-selfupdate.txt" && grep -q '\[1.1\] (menu shows 1.0)' "$out/B-ours-after-selfupdate.txt" \
  && ok "issue #497: after the file became 1.1, Gear Lever still lists 1.0; Gear Lever COSMIC lists 1.1 and flags the menu entry" \
  || fail "issue #497 comparison"
# No update source: honest status.
"${glh[@]}" --list-updates > "$out/B-gl-updates-nosource.txt" 2>&1 || true
ours --list-updates > "$out/B-ours-updates-nosource.txt" || true
cat "$out/B-gl-updates-nosource.txt"; cat "$out/B-ours-updates-nosource.txt"
grep -q "No updates available" "$out/B-gl-updates-nosource.txt" && grep -q "No update source" "$out/B-ours-updates-nosource.txt" \
  && ok "app without any update source: Gear Lever says 'No updates available', Gear Lever COSMIC says 'No update source'" \
  || echo "NOTE: no-source wording differs from the expectation; see the files"
# Put 1.0 back, give both the same static source, start the app, update while running.
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out/fx2" "$port" >/dev/null
cp "$out/fx2/home/Downloads/Demo_Editor-1.0-x86_64.AppImage" "$app.new" && chmod 755 "$app.new" && mv "$app.new" "$app"
url="http://127.0.0.1:$port/Demo_Editor-latest-x86_64.AppImage"
"${glh[@]}" --set-update-source "$app" --manager StaticFileUpdater "url=$url" > "$out/B-gl-setsource.txt" 2>&1 || true
ours --set-update-source "$app" --manager StaticFileUpdater "url=$url" -y >/dev/null
FIXTURE_SECONDS=120 "$app" >/dev/null 2>&1 &
pids+=($!); app_pid=$!
sleep 1
kill -0 "$app_pid" && [[ "$(readlink /proc/$app_pid/exe)" == "$app" ]] || fail "the fixture app is not running; the comparison would be meaningless"
echo "fixture app running: pid $app_pid"
"${glh[@]}" --update "$app" -y 2>&1 | tr '\r' '\n' | grep -v '^Status' > "$out/B-gl-update-running.txt" || true
cat "$out/B-gl-update-running.txt"
grep -q "skipped because the application is running" "$out/B-gl-update-running.txt" \
  && ok "Gear Lever refuses to update the running app (skipped because the application is running)" \
  || echo "NOTE: Gear Lever did not print the running-app refusal; see B-gl-update-running.txt"
ours --update "$app" -y > "$out/B-ours-update-running.txt" 2>&1
cat "$out/B-ours-update-running.txt"
kill -0 "$app_pid" && [[ "$(readlink /proc/$app_pid/exe)" == *"(deleted)" ]] \
  && grep -q "Updated Demo Editor to 1.1" "$out/B-ours-update-running.txt" \
  && ok "Gear Lever COSMIC updated the running app in place; it kept running on the old file" \
  || fail "in-place update of the running app"
echo "Output: $out"
