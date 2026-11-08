#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# A launched app must be able to keep writing to stdout/stderr after the
# launcher has returned. Regression check for apps (ZCode, other Electron apps)
# that hung when started from Gear Lever COSMIC: their output went to pipes that
# closed once `gio launch` exited, so writes failed with EPIPE or SIGPIPE.
#
# Runs on a disposable home (verification/out-launch-*); never touches real apps.
# Usage: verification/check-launch-output.sh [path/to/gearlever-cosmic-cli]
set -euo pipefail
cd "$(dirname "$0")/.."
cli="${1:-target/debug/gearlever-cosmic-cli}"
[[ -x "$cli" ]] || cargo build -q --locked --bin gearlever-cosmic-cli
out="$PWD/verification/out-launch-$(date +%Y%m%d-%H%M%S)"
home="$out/home"
mkdir -p "$home/AppImages" "$home/.local/share/applications" "$home/.config"

# Stand-in "AppImage": a copy of bash with the type-2 AppImage magic in the
# ELF padding bytes 8..10 (where real AppImages carry it; the kernel ignores
# them), running a script that logs a line, waits until the launcher is long
# gone, then writes 2000 more lines plus a marker on each stream.
app="$home/AppImages/probe.appimage"
cp "$(command -v bash)" "$app"
printf 'AI\x02' | dd of="$app" bs=1 seek=8 conv=notrunc status=none
chmod +x "$app"
script="$out/probe-app.sh"
cat > "$script" <<EOF
trap '' PIPE                     # like Electron: ignore SIGPIPE, see EPIPE
echo started
sleep 1.5
for i in \$(seq 2000); do echo "log line \$i" || exit 3; done
echo "stdout still works"
echo "stderr still works" >&2
echo done > "$home/AppImages/probe.finished"
EOF
cat > "$home/.local/share/applications/probe.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Probe
TryExec=$app
Exec=$app $script
Terminal=false
X-AppImage-Name=Probe
EOF

export HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_DATA_HOME="$home/.local/share"
export GEARLEVER_COSMIC_UI_ISOLATED=1
"$cli" run --cmd 'app.launch={"app":"Probe"}' --apply > "$out/launch.json"

for _ in $(seq 50); do [[ -f "$home/AppImages/probe.finished" ]] && break; sleep 0.1; done
log="$XDG_DATA_HOME/gearlever-cosmic/launch-logs/probe.log"
fail() { echo "FAIL: $*" >&2; cat "$out/launch.json" >&2; exit 1; }
[[ -f "$home/AppImages/probe.finished" ]] || fail "the app died writing its output (no probe.finished)"
[[ -f "$log" ]] || fail "no launch log at $log"
rg -q 'stdout still works' "$log" || fail "stdout lost"
rg -q 'stderr still works' "$log" || fail "stderr lost"
echo "ok: app kept writing after the launcher exited; $(wc -l < "$log") lines in ${log#"$out"/}"
echo "PASS ($out)"
