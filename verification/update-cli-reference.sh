#!/usr/bin/env bash
# Regenerate the machine-readable CLI references from the Rust types.
# Usage: verification/update-cli-reference.sh   Check: git diff --stat schemas/
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
cargo build -q --locked --no-default-features --bin gearlever-cosmic-cli
cli=target/debug/gearlever-cosmic-cli
mkdir -p schemas
"$cli" commands --json > schemas/commands.json
"$cli" schema > schemas/request.json
"$cli" ui schema > schemas/ui-request.json
"$cli" ui commands > schemas/ui-commands.json
ls -l schemas/
