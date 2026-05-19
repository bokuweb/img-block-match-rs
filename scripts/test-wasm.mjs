// Smoke-test the WASM build against the bundled sample images.
//
// Usage:
//   node scripts/test-wasm.mjs
//
// Expects `pkg-node/` to be built via:
//   ./scripts/build-wasm.sh

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { PNG } from 'pngjs';
import { diffPng, diffRgba } from '../pkg-node/img_block_match.js';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');

function loadPng(name) {
  return new Uint8Array(readFileSync(resolve(root, 'assets', name)));
}

function loadRgba(name) {
  const buf = readFileSync(resolve(root, 'assets', name));
  const png = PNG.sync.read(buf);
  return { data: new Uint8Array(png.data), width: png.width, height: png.height };
}

function summary(label, result, ms) {
  console.log(`\n[${label}]  (${ms.toFixed(2)} ms)`);
  console.log(`  verdict: ${result.verdict}`);
  console.log(`  size: ${result.width}x${result.height}, block=${result.blockSize}`);
  console.log(`  removed: ${JSON.stringify(result.removed)}`);
  console.log(`  added:   ${JSON.stringify(result.added)}`);
  console.log(`  strayingRects[0]: ${JSON.stringify(result.strayingRects[0])}`);
  console.log(`  strayingRects[1]: ${JSON.stringify(result.strayingRects[1])}`);
  console.log(`  images: ${JSON.stringify(result.images)}`);
  console.log(`  matches: ${JSON.stringify(result.matches)}`);
}

// 1. PNG entry point
const t1 = process.hrtime.bigint();
const r1 = diffPng(loadPng('menu-before.png'), loadPng('menu-after.png'), {
  blockSize: 8, searchX: 16, searchY: 80, threshold: 8,
  mergeGap: 2, minBlocks: 2,
});
summary('diffPng menu', r1, Number(process.hrtime.bigint() - t1) / 1e6);

// 2. RGBA entry point — same images, decoded JS-side
const a = loadRgba('menu-before.png');
const b = loadRgba('menu-after.png');
const t2 = process.hrtime.bigint();
const r2 = diffRgba(a.data, a.width, a.height, b.data, b.width, b.height, {
  blockSize: 8, searchX: 16, searchY: 80, threshold: 8,
  mergeGap: 2, minBlocks: 2,
});
summary('diffRgba menu', r2, Number(process.hrtime.bigint() - t2) / 1e6);

// 3. PNG identical-image baseline
const t3 = process.hrtime.bigint();
const r3 = diffPng(loadPng('menu-before.png'), loadPng('menu-before.png'), {
  blockSize: 8, searchX: 16, searchY: 80, threshold: 8,
});
summary('diffPng identical', r3, Number(process.hrtime.bigint() - t3) / 1e6);

// Sanity: PNG and RGBA should agree on the menu sample.
const same =
  JSON.stringify(r1.removed) === JSON.stringify(r2.removed) &&
  JSON.stringify(r1.added) === JSON.stringify(r2.added);
console.log(`\nPNG ↔ RGBA agree on regions: ${same ? '✓' : '✗ (regressed)'}`);
process.exit(same ? 0 : 1);
