#!/usr/bin/env bash
# Run a window test with its windows on a separate KDE virtual desktop, so
# they never cover or take focus from what the person is doing. A temporary
# KWin script (loaded only while COMMAND runs, nothing saved to KWin's
# settings) sends every new Gear Lever COSMIC window to that desktop, keeps the
# person on their current desktop and gives focus back to their window.
#
# Usage: verification/on-desktop.sh COMMAND [ARGS...]     Exit status: COMMAND's.
# The desktop is GEARLEVER_TEST_DESKTOP (default "Desktop 2"); it must exist,
# this script never creates or removes desktops. Needs the person's session
# bus for KWin; inside verification/private-bus.sh it uses the bus that
# private-bus.sh recorded in GEARLEVER_DESKTOP_BUS. The command itself keeps
# the private bus.
set -euo pipefail
inner_bus=${DBUS_SESSION_BUS_ADDRESS:-}
[[ $# -gt 0 ]] || { echo "usage: $0 COMMAND [ARGS...]" >&2; exit 2; }
desktop=${GEARLEVER_TEST_DESKTOP:-Desktop 2}
prefix=dev.nilltadios.gearlevercosmic
# Inside verification/private-bus.sh the session bus is private; KWin is on the desktop bus.
export DBUS_SESSION_BUS_ADDRESS=${GEARLEVER_DESKTOP_BUS:-${DBUS_SESSION_BUS_ADDRESS:-}}
qdbus=$(command -v qdbus-qt6 || command -v qdbus) || { echo "on-desktop.sh: qdbus-qt6 not found" >&2; exit 2; }
names=$(busctl --user get-property org.kde.KWin /VirtualDesktopManager org.kde.KWin.VirtualDesktopManager desktops 2>/dev/null) \
  || { echo "on-desktop.sh: KWin's virtual desktop manager is not reachable on this session bus" >&2; exit 2; }
[[ $names == *"\"$desktop\""* ]] || { echo "on-desktop.sh: no virtual desktop named \"$desktop\" ($names)" >&2; exit 2; }
current=$(busctl --user get-property org.kde.KWin /VirtualDesktopManager org.kde.KWin.VirtualDesktopManager current)

script_name="gearlever-test-desktop-$$"
script_file=$(mktemp --suffix=.js)
cat >"$script_file" <<EOF
const TARGET = ${desktop@Q};
const PREFIX = ${prefix@Q};
const ours = (w) => w && String(w.resourceClass).toLowerCase().startsWith(PREFIX);
const home = workspace.currentDesktop;
let lastOther = ours(workspace.activeWindow) ? null : workspace.activeWindow;
const target = () => workspace.desktops.find((d) => d.name === TARGET);
const stayHome = () => {
    if (workspace.currentDesktop !== home) workspace.currentDesktop = home;
    if (lastOther && workspace.activeWindow !== lastOther) workspace.activeWindow = lastOther;
};
workspace.windowAdded.connect((w) => {
    if (!ours(w) || !target()) return;
    w.desktops = [target()];
    stayHome();
});
workspace.windowActivated.connect((w) => {
    if (!w) return;
    if (!ours(w)) { lastOther = w; return; }
    // A test window asked for focus: leave it on its desktop, keep the person where they are.
    stayHome();
});
workspace.currentDesktopChanged.connect(() => {
    if (ours(workspace.activeWindow)) stayHome();
});
EOF
cleanup() {
  "$qdbus" org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "$script_name" >/dev/null 2>&1 || true
  rm -f "$script_file"
}
trap cleanup EXIT
"$qdbus" org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "$script_file" "$script_name" >/dev/null
"$qdbus" org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null
echo "on-desktop.sh: Gear Lever test windows go to \"$desktop\"; staying on desktop $current" >&2
set +e
DBUS_SESSION_BUS_ADDRESS=$inner_bus GEARLEVER_ON_TEST_DESKTOP=1 "$@"
rc=$?
set -e
now=$(busctl --user get-property org.kde.KWin /VirtualDesktopManager org.kde.KWin.VirtualDesktopManager current)
[[ $now == "$current" ]] || echo "on-desktop.sh: WARNING: the current desktop changed during the test ($current -> $now)" >&2
exit $rc
