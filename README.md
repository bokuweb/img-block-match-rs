# img-block-match-rs

2D block-matching image diff for Rust + WebAssembly.

Pixel-wise diffs (e.g. `pixelmatch`) flag everything below an inserted
header or beside a widened sidebar as "changed". Row-based LCS diffs (e.g.
[`lcs-image-diff-rs`](https://github.com/bokuweb/lcs-image-diff-rs)) handle
vertical reflow but miss horizontal shifts. This crate handles both axes
by treating diff like video motion estimation:

1. Split the reference image into fixed-size blocks (default 16×16).
2. For each block, search a window in the target image (`±search_x`,
   `±search_y`) for the best (lowest SAD) match.
3. Only blocks whose residual exceeds a threshold after motion
   compensation are flagged as real changes.

Outputs are emitted both as a rendered diff image and as machine-readable
bounding-box JSON, so the same library can drive a visual report and a
test runner.

---

## Demo

A nav menu where `Starred` was renamed to `Favorite` and a new `Important`
entry was inserted (which shifts every row below it down by one line):

| before | after |
|:-:|:-:|
| ![menu before](assets/menu-before.png) | ![menu after](assets/menu-after.png) |

### Naive pixel-wise diff (`--search-x 0 --search-y 0`)

![naive menu diff](assets/diff-menu-naive.png)

> `Starred` plus the entire lower section get flagged — even though
> `All mail`, `Trash`, `Spam`, `Follow up` are unchanged content that
> just moved down one row.

### Block-matching diff (`--search-y 80 --mode fast`)

![block-match menu diff](assets/diff-menu-block-match.png)

> Left panel (red, removed): just `Starred`. Right panel (green, added):
> `Favorite` and the newly inserted `Important` row. Everything else is
> correctly recognized as "same content, just shifted".

Two highlight styles — `outline` (default; content stays visible for
review) and `filled` (solid blocks, highest visibility):

| `--style outline` | `--style filled` |
|:-:|:-:|
| ![outline](assets/diff-menu-block-match.png) | ![filled](assets/diff-menu-filled.png) |

```sh
cargo run --release -- assets/menu-before.png assets/menu-after.png \
    --search-x 16 --search-y 80 --block-size 8 --threshold 8 --mode fast \
    --merge-gap 2 --min-blocks 2 \
    -o diff.png
```

A second synthetic sample with both X and Y shifts plus a real content
change (red badge) is included for stress-testing — see
[`assets/before.png`](assets/before.png) and
[`examples/generate_sample.rs`](examples/generate_sample.rs).

---

## JavaScript / WebAssembly

A `wasm-bindgen` wrapper is exposed behind the `wasm` cargo feature.
Build Node and Web bundles in one shot:

```sh
./scripts/build-wasm.sh   # outputs pkg-node/ and pkg-web/
```

```js
// node
import { diffPng } from './pkg-node/img_block_match.js';

const ref = new Uint8Array(fs.readFileSync('before.png'));
const tgt = new Uint8Array(fs.readFileSync('after.png'));

const r = diffPng(ref, tgt, {
  blockSize: 8, searchX: 16, searchY: 80, threshold: 8,
  mergeGap: 2, minBlocks: 2,
});
// {
//   verdict: 'review',
//   width, height, block_size,
//   removed: [{x, y, width, height, blocks}, ...],
//   added:   [{x, y, width, height, blocks}, ...],
// }
```

```js
// browser
import init, { diffPng } from './pkg-web/img_block_match.js';
await init();
const r = diffPng(refBytes, tgtBytes, { /* ... */ });
```

The WASM bundle is ~416 KB (post `wasm-opt`) and decodes PNG/JPEG
internally so callers only pass byte arrays. PNG decode runs inside
WASM too, so there is no JS-side image work.

Smoke test against the bundled samples:

```sh
$ node scripts/test-wasm.mjs
[menu change] menu-before.png vs menu-after.png  (7.58 ms)
  verdict: review
  removed: [{"x":80,"y":72,"width":48,"height":16,"blocks":10}]
  added:   [{"x":72,"y":72,"width":64,"height":16,"blocks":14},
            {"x":16,"y":280,"width":72,"height":16,"blocks":14}]
```

---

## Rust library

```rust
use img_block_match::{
    diff_bidirectional, render_bidirectional, BlockMatchOptions,
    RenderOptions, SearchMode,
};

let reference = image::open("expected.png")?.to_rgba8();
let target    = image::open("actual.png")?.to_rgba8();

let opts = BlockMatchOptions {
    block_size: 8,
    search_x: 16,
    search_y: 80,
    threshold: 8,
    mode: SearchMode::Hierarchical,
    ..Default::default()
};
let bd = diff_bidirectional(&reference, &target, &opts);
let out = render_bidirectional(&reference, &target, &bd, &RenderOptions::default());
out.save("diff.png")?;
```

`BlockMatchResult::vectors` is a `cols * rows` grid of
`MotionVector { dx, dy, cost, second_cost, matched }` you can use to
build custom visualizations.

> Block matching is directional. `diff(a, b)` only flags blocks of `a`
> that have no match in `b` — i.e. content that disappeared. To also
> catch additions, use `diff_bidirectional` (the CLI does this by
> default).

---

## CLI

```sh
cargo run --release -- expected.png actual.png \
    --block-size 16 --search-x 64 --search-y 128 --threshold 8 \
    --mode fast -o diff.png
```

Common flags:

| flag | default | purpose |
|---|---|---|
| `--mode full\|fast` | `full` | search strategy (use `fast` for any non-trivial search radius) |
| `--style outline\|filled` | `outline` | how regions are drawn |
| `--stroke N` | `2` | outline thickness in pixels |
| `--merge-gap N` | `1` | bridge clusters separated by N matched blocks |
| `--min-blocks N` | `1` | discard clusters smaller than this (drops AA noise) |
| `--smooth` | off | 3×3 majority filter on the `matched` flag |
| `--one-way` | off | render only the reference panel with "removed" overlay |
| `--heatmap` | off | per-block residual on a green→yellow→red gradient |
| `--json` | off | emit region bounding boxes on stdout |
| `--confidence` | off | also track runner-up SAD (slower; see below) |
| `--draw-vectors` | off | draw motion vectors on top of the diff |

---

## Region clustering

Raw per-block results are awkward to consume programmatically.
`BlockMatchResult::unmatched_regions(merge_gap, min_blocks)` groups
spatially-adjacent unmatched blocks into bounding rectangles via
8-connected flood fill. The renderer drives its highlight rectangles
from this same clustering, so what you see is what the JSON reports:

```sh
$ img-block-match assets/menu-before.png assets/menu-after.png \
    --search-y 80 --block-size 8 --mode fast \
    --merge-gap 2 --min-blocks 2 --json \
    -o diff.png
{
    "removed": [
      {"x": 80, "y": 72, "width": 48, "height": 16, "blocks": 10}
    ],
    "added": [
      {"x": 72, "y": 72, "width": 64, "height": 16, "blocks": 14},
      {"x": 16, "y": 280, "width": 72, "height": 16, "blocks": 14}
    ]
}
```

---

## Search modes & performance

| mode | strategy | when to use |
|---|---|---|
| `Full` (default) | exhaustive scan of every `(dx, dy)` in the window | small radii, or globally-optimal matching |
| `Hierarchical` (CLI `--mode fast`) | coarse uniform grid scan (stride ≈ search/8, capped at `block_size`) + 3×3 logarithmic refinement | large radii, real screenshots |

The SAD inner loop uses explicit SIMD: `_mm_sad_epu8` on x86_64 (SSE2),
`vabdq_u8` / `vaddlvq_u8` on aarch64 (NEON), with a scalar fallback.

End-to-end speedup on a 1920×1080 synthetic reflow workload (fast mode,
bidirectional):

| search ±x/±y | scalar / auto-vec | NEON intrinsics |
|---:|---:|---:|
| 64 / 96      | 255 ms |  20 ms (**~12×**) |
| 200 / 300    | 3.78 s | 183 ms (**~21×**) |
| 4K, 200 / 400 | 1.14 s |  57 ms (**~20×**) |

Real menu screenshot (360×512, bidirectional, fast mode): **~1.7 ms**.

---

## Post-processing

- **`smooth_matched()`** — 3×3 majority filter on the `matched` flag.
  Absorbs isolated anti-aliasing false positives and fills 1-block holes
  in genuine clusters. CLI: `--smooth`.
- **`render_heatmap(base, &result, max_cost, alpha)`** — tints every block
  on a green→yellow→red gradient by residual cost. Shows "how close"
  rather than a binary classification. CLI: `--heatmap`.

### Match confidence (opt-in)

Setting `BlockMatchOptions::compute_confidence = true` (CLI:
`--confidence`) also tracks the best spatially-distinct runner-up SAD
for each block:

```rust
mv.confidence()  // (second_cost - cost) / second_cost, in 0..=1
```

- `≈ 1.0` — the winning displacement is uniquely good (text, edges).
- `≈ 0.0` — many positions match equally well (flat regions, repeating
  patterns) → `(dx, dy)` is unreliable; downstream tools should discount.

Disables the early-return-on-perfect-match optimization, so expect a
2–6× slowdown.

---

## Pyramid mode (experimental)

`diff_pyramid(reference, target, opts, coarse_scale, refine_radius)`
runs the matcher at a downscaled level first, then refines each per-block
prediction at full resolution in a tiny `±refine_radius` window. With
the SIMD-accelerated single-pass matcher the resize overhead usually
dominates, so prefer plain `diff_bidirectional` with
`SearchMode::Hierarchical` and reach for the pyramid only when the
search window must cover a very large fraction of the image.

---

## Tuning

- `block_size`: smaller catches finer changes but is slower and noisier
  on anti-aliased text. 8–32 is typical.
- `search_x` / `search_y`: how far content is allowed to shift.
  Vertical is usually much larger than horizontal for web screenshots.
- `threshold`: per-channel per-pixel SAD allowed inside a matched block.
  0 = pixel-perfect; 4–16 tolerates anti-aliasing differences.
- `merge_gap` / `min_blocks`: tighten or relax clustering. `min_blocks=2`
  reliably drops AA flickers; `merge_gap=2` bridges a row of padding
  between sibling text lines.

---

## Integration sketch: visual regression pass / review

A typical screenshot-regression workflow is "PASS unless content
changed". Pixel diffs fail this when layout reflows. Block-matching
short-circuits that:

```
$ cargo run --release --example regression_check -- before.png before.png
PASS  (... decided in 6 ms)

$ cargo run --release --example regression_check -- before.png after.png
REVIEW
  removed regions: 1
  added regions:   2
    - removed 48x16 at (80, 72)
    + added   64x16 at (72, 72)
    + added   64x16 at (24, 280)
```

[examples/regression_check.rs](examples/regression_check.rs) is a 60-line
template — drop it next to a per-pixel differ and the slow pixel path
runs only inside the unmatched-region bounding boxes, never the whole
image.

---

## License

MIT
