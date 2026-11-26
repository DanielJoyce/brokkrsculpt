#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Build the web version into a directory a static host can serve:
#   scripts/web-build.sh [out-dir]       (default: site)
#
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli at exactly the
# version Cargo.lock names, which this script reads and checks. See
# docs/WEB.md.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-site}
target=${CARGO_TARGET_DIR:-target}

wanted=$(awk '$0=="name = \"wasm-bindgen\""{getline; gsub(/"/,"",$3); print $3}' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$have" != "$wanted" ]; then
  echo "wasm-bindgen $wanted is needed (found '${have:-none}'):" >&2
  echo "  cargo install wasm-bindgen-cli --version $wanted --locked" >&2
  exit 1
fi

cargo build --release -p brokkr-app --target wasm32-unknown-unknown
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" \
  "$target/wasm32-unknown-unknown/release/brokkrsculpt.wasm"
cp web/index.html "$out/index.html"
# A Pages site built from a branch runs Jekyll unless told not to; one
# published by the workflow does not, and the marker costs nothing.
touch "$out/.nojekyll"
echo "built $out: $(du -sh "$out" | cut -f1)"
