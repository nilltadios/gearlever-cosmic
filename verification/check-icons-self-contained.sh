#!/usr/bin/env bash
# The window finds every icon it uses in the bundled GearLeverCosmic theme
# alone: XDG_DATA_DIRS holds only resources/ (as the AppImage's share/ would),
# no /usr/share icon themes, in a disposable home with no apps (which also
# covers the empty "No AppImages yet" page). Needs Wayland.
# Usage: verification/check-icons-self-contained.sh [BUILD_DIR]   (default target/debug)
# Expected: "PASS", exit 0. Run it as verification/private-bus.sh verification/check-icons-self-contained.sh
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
bin="$(realpath "${1:-target/debug}")"
out="$PWD/verification/out-icons-$(date +%Y%m%d-%H%M%S)"
H="$out/home"; mkdir -p "$H/.config/cosmic"
# The desktop's COSMIC theme, copied read-only, as gui-workflow.sh does.
cp -r "$HOME"/.config/cosmic/com.system76.CosmicTheme.* "$H/.config/cosmic/" 2>/dev/null || true
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache"
export XDG_DATA_DIRS="$PWD/resources"
cli="$bin/gearlever-cosmic-cli"
instance=$("$cli" ui start --config-dir "$H/.config" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["instance"])')
trap '"$cli" ui --instance "$instance" close --apply >/dev/null 2>&1 || true' EXIT
"$cli" ui --instance "$instance" wait --timeout 30 >/dev/null
r=$("$cli" ui --instance "$instance" inspect | python3 -c 'import json,sys; s=json.load(sys.stdin)["data"]["state"]; print(s["icon_theme"], s["icons_missing"])')
"$cli" ui --instance "$instance" capture "$out/icons.png" >/dev/null
[[ $r == "GearLeverCosmic []" ]] || { echo "FAIL: with only the bundled theme: $r"; exit 1; }
echo "ok: every window icon resolves from the bundled theme alone ($r); render $out/icons.png"
echo "PASS"
