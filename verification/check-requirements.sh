#!/usr/bin/env bash
# Show that a FUSE 2 AppImage is reported as not runnable on a system that has
# only FUSE 3, using a simulated system tree (the real system is not changed).
# Usage: verification/check-requirements.sh APP_ID   (default: antigravity)
# Expected: verdict "missing" with a FUSE 2 problem for the simulated system,
# and the real system's verdict printed for comparison. Exit 0 when the
# simulated verdict is "missing" for a fuse2 runtime.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
app="${1:-antigravity}"
cli=target/release/gearlever-cosmic-cli
[[ -x $cli ]] || cargo build --release --no-default-features --bin gearlever-cosmic-cli
root="$(mktemp -d)"; trap 'rm -rf -- "$root"' EXIT
mkdir -p "$root/usr/lib" "$root/usr/bin"
# Copy the real library folder listing except libfuse.so.2: empty placeholder files.
for f in /usr/lib/*.so* ; do n=$(basename "$f"); [[ $n == libfuse.so.2* ]] || : > "$root/usr/lib/$n"; done
: > "$root/usr/bin/fusermount3"
echo "== simulated system with fuse3 only"
GEARLEVER_COSMIC_SYSTEM_ROOT="$root" "$cli" run --cmd "app.requirements={\"app\":\"$app\"}" | tee "$root/sim.json" | python3 -c 'import json,sys; d=json.load(sys.stdin)["results"][0]["data"]; print(d["verdict"], d["runtime"]); [print(" -", p) for p in d["problems"]]'
echo "== this system"
"$cli" run --cmd "app.requirements={\"app\":\"$app\"}" | python3 -c 'import json,sys; d=json.load(sys.stdin)["results"][0]["data"]; print(d["verdict"], d["runtime"], "missing:", d["missing_libraries"])'
python3 -c 'import json,sys; d=json.load(open(sys.argv[1]))["results"][0]["data"]; sys.exit(0 if d["runtime"]!="fuse2" or d["verdict"]=="missing" else 1)' "$root/sim.json"
