#!/usr/bin/env bash
# A normal (non-test) window refreshes only menu entries Gear Lever COSMIC
# wrote. A Gear Lever entry with an outdated version must stay byte for byte.
# Disposable home only. Needs Wayland. Expected: "PASS", exit 0.
# Usage: verification/check-auto-sync.sh [BUILD_DIR]   (default target/debug)
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
cargo build -q --release --example fixture_runtime
out="$PWD/verification/out-autosync-$(date +%Y%m%d-%H%M%S)"
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out" 1 >/dev/null
H="$out/home"; export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share"
unset GEARLEVER_COSMIC_UI_ISOLATED
cli="$bin/gearlever-cosmic-cli"
for f in Demo_Editor-1.0-x86_64.AppImage Old_Tool-0.9.AppImage; do "$cli" run --apply --cmd "app.integrate={\"file\":\"$H/Downloads/$f\"}" >/dev/null; done
A="$H/.local/share/applications"
# Old Tool: rewrite as a Gear Lever entry with an old version.
sed -i -e '/^X-GearLeverCosmic=/d' -e 's/^X-AppImage-Version=.*/X-AppImage-Version=0.1/' "$A/old_tool.desktop"
sha_before=$(sha256sum < "$A/old_tool.desktop")
# Demo Editor: our own entry, made outdated.
sed -i 's/^X-AppImage-Version=.*/X-AppImage-Version=0.5/' "$A/demo_editor.desktop"
"$bin/gearlever-cosmic" >"$out/gui.log" 2>&1 &
gui=$!
trap 'kill $gui 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do grep -q '^X-AppImage-Version=1.0$' "$A/demo_editor.desktop" && break; sleep 0.5; done
sleep 1
grep -q '^X-AppImage-Version=1.0$' "$A/demo_editor.desktop" || { echo "FAIL: own entry not refreshed"; exit 1; }
[[ $(sha256sum < "$A/old_tool.desktop") == "$sha_before" ]] || { echo "FAIL: the Gear Lever entry was changed"; diff <(echo) "$A/old_tool.desktop"; exit 1; }
v=$("$cli" run --cmd 'app.info={"app":"old_tool"}' | python3 -c 'import json,sys; d=json.load(sys.stdin)["results"][0]["data"]; print(d["menu_outdated"], d["managed_by"])')
[[ $v == "['version'] gear_lever" ]] || { echo "FAIL: Gear Lever entry should be flagged: $v"; exit 1; }
echo "ok: own entry refreshed to 1.0; Gear Lever entry unchanged and flagged ($v)"
echo "PASS ($out)"
