#!/usr/bin/env bash
# PreToolUse hook (Bash): before a `git push` in this repo, run
# verification/check-release-bump.sh and block the push if it fails.
input=$(cat)
cmd=$(printf '%s' "$input" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')
[[ "$cmd" =~ (^|[[:space:];&|])git([[:space:]]+-[^[:space:]]+)*[[:space:]]+push([[:space:]]|$) ]] || exit 0
root="$(cd -- "$(dirname -- "$0")/../.." && pwd)"
if ! out=$("$root/verification/check-release-bump.sh" 2>&1); then
  echo "$out" >&2
  exit 2
fi
exit 0
