#!/usr/bin/env bash
# Run a test command on its own D-Bus session, so its windows cannot reach a
# Gear Lever COSMIC the user has open, then stop the helpers (portals,
# ksecretd) that the bus activated, which otherwise outlive the test and keep
# its output pipe open.
# Usage: verification/private-bus.sh COMMAND [ARGS...]   Exit status: COMMAND's.
set -uo pipefail
addr_file=$(mktemp)
dbus-run-session -- bash -c 'printf %s "$DBUS_SESSION_BUS_ADDRESS" > "$0"; exec "$@"' "$addr_file" "$@"
rc=$?
addr=$(cat "$addr_file"); rm -f "$addr_file"
if [[ $addr == unix:path=/tmp/* ]]; then
  for env in /proc/[0-9]*/environ; do
    [[ -r $env ]] || continue
    { tr "\0" "\n" < "$env"; } 2>/dev/null | rg -qxF "DBUS_SESSION_BUS_ADDRESS=$addr" || continue
    pid=${env#/proc/}; pid=${pid%/environ}
    [[ $pid == "$$" ]] || kill -TERM "$pid" 2>/dev/null || true
  done
fi
exit $rc
