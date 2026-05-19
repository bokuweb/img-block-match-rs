#!/usr/bin/env bash
# Build wasm-bindgen packages for Node and Web targets.
#
# Output:
#   pkg-node/  -> node-side ESM bundle (load with `import { diffPng } from
#                 './pkg-node/img_block_match.js'`)
#   pkg-web/   -> browser ESM bundle  (load with dynamic import + init())
#
# Requires:
#   - rustup target add wasm32-unknown-unknown
#   - wasm-pack (https://rustwasm.github.io/wasm-pack/)

set -euo pipefail

cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown >/dev/null

echo "==> building pkg-node (target=nodejs)"
wasm-pack build --target nodejs --release --out-dir pkg-node \
  -- --no-default-features --features wasm

echo "==> building pkg-web (target=web)"
wasm-pack build --target web --release --out-dir pkg-web \
  -- --no-default-features --features wasm

echo
echo "✓ done"
ls -lh pkg-node/img_block_match_bg.wasm pkg-web/img_block_match_bg.wasm
