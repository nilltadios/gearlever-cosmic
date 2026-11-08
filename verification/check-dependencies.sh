#!/usr/bin/env bash
# Compare each direct crates.io dependency in Cargo.toml with the newest
# release on crates.io, and show the pinned libcosmic commit against the
# upstream main branch. Read-only. Exit 1 if a newer major/minor exists.
# Usage: verification/check-dependencies.sh
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
status=0
python3 - <<'PY' > /tmp/gl-deps.$$ 
import tomllib
c = tomllib.load(open("Cargo.toml", "rb"))
deps = {**c.get("dependencies", {}), **c.get("dev-dependencies", {})}
for k, v in deps.items():
    if isinstance(v, str): print(k, v)
    elif "version" in v: print(k, v["version"])
PY
while read -r name req; do
  latest=$(cargo search --limit 1 "$name" 2>/dev/null | sed -n "s/^$name = \"\([^\"]*\)\".*/\1/p" || true)
  locked=$(cargo tree -i "$name" --depth 0 -e normal,dev 2>/dev/null | head -1 | awk "{print \$2}" | tr -d v || true)
  mark=ok
  # semver: a "0.x" requirement covers 0.x.*; "N" covers N.*
  major_req=$(echo "$req" | cut -d. -f1); minor_req=$(echo "$req" | cut -d. -f2)
  major_new=$(echo "$latest" | cut -d. -f1); minor_new=$(echo "$latest" | cut -d. -f2)
  if [[ $major_req == 0 && $minor_req != "$minor_new" ]] || [[ $major_req != 0 && $major_req != "$major_new" ]]; then mark=NEWER; status=1; fi
  printf '%-22s req %-8s locked %-10s latest %-10s %s\n' "$name" "$req" "$locked" "$latest" "$mark"
done < /tmp/gl-deps.$$
rm -f /tmp/gl-deps.$$
pinned=$(sed -n 's/^source = "git+https:\/\/github.com\/pop-os\/libcosmic#\([0-9a-f]*\)"/\1/p' Cargo.lock | head -1)
upstream=$(git ls-remote https://github.com/pop-os/libcosmic refs/heads/master refs/heads/main | head -1 | cut -f1)
echo "libcosmic pinned ${pinned:0:12} upstream ${upstream:0:12} $([[ $pinned == "$upstream" ]] && echo ok || echo NEWER)"
[[ $pinned == "$upstream" ]] || status=1
echo "rust: $(rustc --version) (latest stable: $(curl -s https://static.rust-lang.org/dist/channel-rust-stable.toml | sed -n '/^\[pkg.rust\]/{n;p}' | cut -d'"' -f2))"
exit $status
