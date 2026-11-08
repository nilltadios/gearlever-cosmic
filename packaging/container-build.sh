#!/usr/bin/env bash
# Build release binaries and the AppImage inside the Ubuntu 26.04 container
# (podman or docker). Output: dist/GearLever_COSMIC-VERSION-x86_64.AppImage,
# .zsync and .sha256. Cargo downloads and build artifacts are cached in
# packaging/cache. Nothing is installed on the host.
# Usage: [OUT=dir] packaging/container-build.sh   (default OUT=dist)
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
engine=$(command -v podman || command -v docker)
image=localhost/gearlever-cosmic-build:26.04
"$engine" build -q -t "$image" -f packaging/Containerfile packaging >/dev/null
mkdir -p packaging/cache/cargo/registry packaging/cache/cargo/git packaging/cache/target "${OUT:-dist}"
"$engine" run --rm \
  -v "$PWD:/src:Z" \
  -v "$PWD/packaging/cache/cargo/registry:/root/.cargo/registry:Z" \
  -v "$PWD/packaging/cache/cargo/git:/root/.cargo/git:Z" \
  -e CARGO_TARGET_DIR=/src/packaging/cache/target \
  -e GITHUB_REPOSITORY="${GITHUB_REPOSITORY:-}" -e OUT="${OUT:-dist}" \
  "$image" bash -c 'set -e; cargo build --locked --release --bins; packaging/build-appimage.sh packaging/cache/target/release "$OUT"'
[[ "$engine" == *podman ]] || chown -R "$(id -u):$(id -g)" dist packaging/cache 2>/dev/null || true
ls -l "${OUT:-dist}"/
