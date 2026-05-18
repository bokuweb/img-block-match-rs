# img-block-match-rs

2D block-matching image diff for Rust.

Pixel-wise diffs (e.g. `pixelmatch`) flag everything below an inserted header
or beside a widened sidebar as "changed". Row-based LCS diffs (e.g.
[`lcs-image-diff-rs`](https://github.com/bokuweb/lcs-image-diff-rs)) handle
vertical reflow but miss horizontal shifts. This crate handles both axes by
treating diff like video motion estimation:

1. Split the reference image into fixed-size blocks (default 16×16).
2. For each block, search a window in the target image (`±search_x`,
   `±search_y`) for the best (lowest SAD) match.
3. Only blocks whose residual exceeds a threshold after motion compensation
   are flagged as real changes.

## Demo

### Real screenshot: navigation menu

A nav menu where `Starred` was renamed to `Favorite` and a new `Important`
entry was inserted in the second section (which shifts every following row
down by one line):

| before | after |
|:-:|:-:|
| ![menu before](assets/menu-before.png) | ![menu after](assets/menu-after.png) |

Naive pixel-wise diff (`--search-x 0 --search-y 0`):

![naive menu diff](assets/diff-menu-naive.png)

> "Starred" and every row in the lower section is flagged red — even though
> `All mail`, `Trash`, `Spam`, `Follow up` are unchanged content that just
> moved down.

Block-matching diff (`--search-y 80 --mode fast`):

![block-match menu diff](assets/diff-menu-block-match.png)

> Left panel (red, removed): just `Starred`. Right panel (green, added):
> `Favorite` and the newly inserted `Important` row. Everything else is
> correctly recognized as "same content, just shifted".

```sh
cargo run --release -- assets/menu-before.png assets/menu-after.png \
    --search-x 16 --search-y 80 --block-size 8 --threshold 8 --mode fast \
    -o assets/diff-menu-block-match.png
```

### Synthetic: 2-axis layout shift

A synthetic "web layout" pair where the header grew taller (+24px Y shift)
**and** the sidebar grew wider (+64px X shift), and one genuine change was
made (a red badge added to the middle card):

| before | after |
|:-:|:-:|
| ![before](assets/before.png) | ![after](assets/after.png) |

### Naive pixel-wise diff (no shift tolerance)

`--search-x 0 --search-y 0` (equivalent to a per-pixel comparator):

![naive diff](assets/diff-naive.png)

> **1406 / 2400 blocks (58.58 %) flagged as different** — the entire page
> below the header lights up because everything moved.

### Block-matching diff (this crate)

`--search-x 96 --search-y 96 --block-size 8` (bidirectional):

![block-matching diff](assets/diff-block-match.png)

> **0 removed + 15 / 2400 added (0.63 %) flagged** — only the new badge.

The left panel is the reference with "removed" blocks tinted red; the right
panel is the target with "added" blocks tinted green.

Reproduce locally:

```sh
cargo run --release --example generate_sample
cargo run --release -- assets/before.png assets/after.png \
    --search-x 96 --search-y 96 --block-size 8 --threshold 4 \
    -o assets/diff-block-match.png
```

## Library

```rust
use img_block_match::{
    diff_bidirectional, render_bidirectional, BlockMatchOptions, RenderOptions,
};

let reference = image::open("expected.png")?.to_rgba8();
let target = image::open("actual.png")?.to_rgba8();

let opts = BlockMatchOptions {
    block_size: 16,
    search_x: 64,
    search_y: 128,
    step: 1,
    threshold: 8,
};
let bd = diff_bidirectional(&reference, &target, &opts);
let out = render_bidirectional(&reference, &target, &bd, &RenderOptions::default());
out.save("diff.png")?;
```

`BlockMatchResult::vectors` is a `cols * rows` grid of `MotionVector { dx, dy,
cost, matched }` you can use to build your own visualization (heatmap,
overlay, vector field, ...).

> Block matching is directional. `diff(a, b)` flags blocks of `a` that have
> no match in `b` — i.e. content that disappeared. To also catch additions,
> use `diff_bidirectional` (the CLI does this by default).

## CLI

```sh
cargo run --release -- expected.png actual.png \
    --block-size 16 --search-x 64 --search-y 128 --threshold 8 \
    -o diff.png
```

- `--one-way` to render a single-direction overlay on the reference image.
- `--draw-vectors` to draw motion vectors on top.
- `--mode fast` for the hierarchical search (recommended once the search
  radius gets large).

## Search modes

| mode | strategy | when to use |
|---|---|---|
| `full` (default) | exhaustive scan of every `(dx, dy)` in the window, with `step` honored | small search radii, or when you need a globally optimal match |
| `fast` | coarse uniform grid scan (stride ≈ search/8, capped at `block_size`) + 3×3 logarithmic refinement halving down to stride 1 | large search radii, real screenshots |

Benchmark on the 480×320 demo above (8×8 blocks, 2400 blocks total, 8-core
laptop, bidirectional):

| search radius | mode | added blocks | elapsed |
|---:|---|---:|---:|
| ±96  | full | 15 | 1280 ms |
| ±96  | fast | 15 |  16 ms |
| ±200 | full | 15 | 3146 ms |
| ±200 | fast | 15 |  44 ms |

The fast mode finds the same matches up to ~70–100× faster across both
ranges. (The coarse stride is capped at `block_size` precisely so the
refinement basins overlap and no in-window match is missed for typical
inputs.)

## Tuning

- `block_size`: smaller catches finer changes but is slower and noisier on
  anti-aliased text. 8–32 is typical.
- `search_x` / `search_y`: how far content is allowed to shift. Vertical is
  usually much larger than horizontal for web screenshots.
- `step`: increase to skip candidates (faster, less accurate). Use 1 for
  exhaustive full search; 2–4 with a small block size is often a good
  trade-off.
- `threshold`: per-channel per-pixel SAD allowed inside a matched block. 0 =
  pixel-perfect match required; 4–16 tolerates anti-aliasing differences.

## License

MIT
