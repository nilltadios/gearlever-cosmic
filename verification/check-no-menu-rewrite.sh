#!/usr/bin/env bash
# Opening a normal (non-test) window never rewrites menu entries: versions are
# read from the files, so a stale X-AppImage-Version (ours or Gear Lever's)
# stays byte for byte and nothing is flagged as outdated.
# Disposable home only. Needs Wayland. Expected: "PASS", exit 0.
# Usage: verification/check-no-menu-rewrite.sh [BUILD_DIR]   (default target/debug)
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
cargo build -q --release --example fixture_runtime
out="$PWD/verification/out-nomenurewrite-$(date +%Y%m%d-%H%M%S)"
cargo run -q --release --example make_fixture -- target/release/examples/fixture_runtime "$out" 1 >/dev/null
H="$out/home"; export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share"
unset GEARLEVER_COSMIC_UI_ISOLATED
cli="$bin/gearlever-cosmic-cli"
for f in Demo_Editor-1.0-x86_64.AppImage Old_Tool-0.9.AppImage; do "$cli" run --apply --cmd "app.integrate={\"file\":\"$H/Downloads/$f\"}" >/dev/null; done
A="$H/.local/share/applications"
grep -q '^X-AppImage-Version=' "$A/demo_editor.desktop" && { echo "FAIL: a new menu entry carries a version"; exit 1; }
# Old Tool: a Gear Lever entry with an old version. Demo Editor: ours, with a stale one.
sed -i -e '/^X-GearLeverCosmic=/d' -e '/^X-AppImage-Name=/a X-AppImage-Version=0.1' "$A/old_tool.desktop"
sed -i '/^X-GearLeverCosmic=/a X-AppImage-Version=0.5' "$A/demo_editor.desktop"
sha_old=$(sha256sum < "$A/old_tool.desktop"); sha_own=$(sha256sum < "$A/demo_editor.desktop")
"$bin/gearlever-cosmic" >"$out/gui.log" 2>&1 &
gui=$!
trap 'kill $gui 2>/dev/null || true' EXIT
# Wait until the window has loaded its inventory, then give it time to act.
for _ in $(seq 1 60); do "$cli" ui inspect 2>/dev/null | python3 -c 'import json,sys; sys.exit(json.load(sys.stdin)["data"]["state"]["loading"])' 2>/dev/null && break; sleep 0.5; done
sleep 3
m=$("$cli" ui inspect | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["state"]["icons_missing"])')
[[ $m == "[]" ]] || { echo "FAIL: the window found no icons for $m"; exit 1; }
[[ $(sha256sum < "$A/old_tool.desktop") == "$sha_old" ]] || { echo "FAIL: the Gear Lever entry was changed"; exit 1; }
[[ $(sha256sum < "$A/demo_editor.desktop") == "$sha_own" ]] || { echo "FAIL: our entry was changed"; exit 1; }
v=$("$cli" run --cmd 'app.info={"app":"demo_editor"}' | python3 -c 'import json,sys; d=json.load(sys.stdin)["results"][0]["data"]; print(d["version"], "menu_outdated" in d)')
[[ $v == "1.0 False" ]] || { echo "FAIL: expected version 1.0 from the file and no outdated flag: $v"; exit 1; }
echo "ok: both entries unchanged after the window loaded; version $v read from the file"
echo "PASS ($out)"
