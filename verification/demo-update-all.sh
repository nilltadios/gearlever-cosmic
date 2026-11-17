#!/usr/bin/env bash
# Open a Gear Lever COSMIC window on the current desktop for a person to try
# Update All: a disposable home with sample apps (never the real ones), two
# updates ready, and a local server that sends files slowly so the download
# progress is visible. The window stays open until the person closes it; the
# server then stops.
#
# Usage: verification/demo-update-all.sh [BUILD_DIR]   (default target/debug)
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
[[ -x $bin/gearlever-cosmic && -x $bin/gearlever-cosmic-cli ]] || { echo "build first: cargo build --bins" >&2; exit 2; }
[[ -x target/release/examples/fixture_runtime && -x target/release/examples/make_fixture ]] \
  || { echo "build first: cargo build --release --example fixture_runtime --example make_fixture" >&2; exit 2; }
out="$PWD/verification/out-demo-update-all-$(date +%Y%m%d-%H%M%S)"
port=$(( 20000 + RANDOM % 20000 ))
target/release/examples/make_fixture target/release/examples/fixture_runtime "$out" "$port" >/dev/null
H="$out/home"
mkdir -p "$H/.config/cosmic"
cp -r "$HOME"/.config/cosmic/com.system76.CosmicTheme.* "$HOME"/.config/cosmic/com.system76.CosmicTk "$H/.config/cosmic/" 2>/dev/null || true
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache"
unset HOST_XDG_DATA_HOME
cli="$bin/gearlever-cosmic-cli"
/usr/bin/python3 verification/slow-http-server.py "$port" "$out/server" 0.4 >"$out/server.log" 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
for f in Demo_Editor-1.0-x86_64.AppImage Sky-Notes-2.0.0.AppImage Old_Tool-0.9.AppImage; do
  "$cli" run --apply --cmd "app.integrate={\"file\":\"$H/Downloads/$f\"}" >/dev/null
done
"$cli" run --apply --cmd "update.set-source={\"app\":\"demo_editor\",\"source\":{\"manager\":\"StaticFileUpdater\",\"url\":\"http://127.0.0.1:$port/Demo_Editor-latest-x86_64.AppImage\"}}" >/dev/null
instance=$("$cli" ui start --config-dir "$H/.config" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["instance"])')
ui() { "$cli" ui --instance "$instance" "$@"; }
ui wait --timeout 30 >/dev/null
ui navigate updates >/dev/null
ui wait --timeout 60 >/dev/null
pid=$(ui inspect | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["state"]["process_id"])')
echo "Demo window open ($instance, pid $pid), fixture in $out. Close the window to stop the demo server."
while kill -0 "$pid" 2>/dev/null; do sleep 2; done
echo "Demo window closed."
