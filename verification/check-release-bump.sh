#!/usr/bin/env bash
# Refuse to ship app changes under a version that is already released.
# The release workflow only publishes when Cargo.toml's version has no tag
# yet, so a push of app changes without a bump never reaches installed
# copies through their update check.
# Fails when HEAD changes shipped files (src, resources, packaging,
# Cargo.toml, Cargo.lock, docs bundled into the CLI) since tag v$VERSION,
# or when the metainfo has no <release> for $VERSION.
# Usage: verification/check-release-bump.sh [REF]   (default HEAD)
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
ref="${1:-HEAD}"
version=$(git show "$ref:Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
git fetch -q --tags origin 2>/dev/null || echo "warning: could not fetch tags; using local tags" >&2
if ! git show "$ref:resources/dev.nilltadios.GearLeverCosmic.metainfo.xml" | grep -q "<release version=\"$version\""; then
  echo "FAIL: metainfo has no <release version=\"$version\">" >&2; exit 1
fi
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  changed=$(git diff --name-only "v$version" "$ref" -- src resources packaging Cargo.toml Cargo.lock llms.txt docs)
  if [[ -n "$changed" ]]; then
    echo "FAIL: v$version is already released, but these shipped files changed since:" >&2
    echo "$changed" | sed 's/^/  /' >&2
    echo "Bump version in Cargo.toml and add a <release> to the metainfo." >&2
    exit 1
  fi
  echo "ok: no shipped changes since v$version (nothing new to release)"
else
  echo "ok: v$version is new; pushing will publish it"
fi
