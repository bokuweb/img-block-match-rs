// Smoke-test the WASM build against the bundled sample images.
//
// Usage:
//   node scripts/test-wasm.mjs
//
// Expects `pkg-node/` to be built via:
//   wasm-pack build --target nodejs --release --out-dir pkg-node \
//     -- --no-default-features --features wasm

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { diffPng } from '../pkg-node/img_block_match.js';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');

function load(name) {
  return new Uint8Array(readFileSync(resolve(root, 'assets', name)));
}

function run(label, refName, targetName, opts) {
  const ref = load(refName);
  const tgt = load(targetName);
  const t = process.hrtime.bigint();
  const result = diffPng(ref, tgt, opts);
  const ms = Number(process.hrtime.bigint() - t) / 1e6;
  console.log(`\n[${label}]  ${refName} vs ${targetName}  (${ms.toFixed(2)} ms)`);
  console.log(`  verdict: ${result.verdict}`);
  console.log(`  size: ${result.width}x${result.height}, block=${result.block_size}`);
  console.log(`  removed: ${JSON.stringify(result.removed)}`);
  console.log(`  added:   ${JSON.stringify(result.added)}`);
}

run('identical', 'menu-before.png', 'menu-before.png', {
  blockSize: 8,
  searchX: 16,
  searchY: 80,
  threshold: 8,
});

run('menu change', 'menu-before.png', 'menu-after.png', {
  blockSize: 8,
  searchX: 16,
  searchY: 80,
  threshold: 8,
  mergeGap: 2,
  minBlocks: 2,
});
