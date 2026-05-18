use image::RgbaImage;
use rayon::prelude::*;

/// Strategy for exploring the search window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Exhaustive search over every candidate in the window (`step` honored).
    /// O(search_x · search_y) per block — globally optimal, slow for large
    /// search radii.
    Full,
    /// Hierarchical search: a coarse uniform scan with stride
    /// `max(1, max(search_x, search_y) / 8)` covers the whole window without
    /// gaps a refinement step can't bridge, then a 3×3 logarithmic
    /// refinement halves stride down to 1 around the best coarse hit.
    /// Roughly O(64 + log(search) · 9) per block — orders of magnitude
    /// faster for wide search ranges, and avoids the local-minima trap of
    /// pure TSS on sparse content.
    Hierarchical,
}

impl Default for SearchMode {
    fn default() -> Self {
        SearchMode::Full
    }
}

#[derive(Debug, Clone)]
pub struct BlockMatchOptions {
    /// Side length of each square block in pixels.
    pub block_size: u32,
    /// Horizontal search radius (pixels). Candidate dx ∈ [-search_x, +search_x].
    pub search_x: i32,
    /// Vertical search radius (pixels). Candidate dy ∈ [-search_y, +search_y].
    pub search_y: i32,
    /// Step between candidate displacements. 1 = exhaustive full search.
    /// Only used by [`SearchMode::Full`].
    pub step: u32,
    /// Per-channel, per-pixel SAD threshold (0..=255) for a block to be
    /// considered "matched" (i.e. it shifted but did not actually change).
    pub threshold: u32,
    /// Which search strategy to use within the window.
    pub mode: SearchMode,
}

impl Default for BlockMatchOptions {
    fn default() -> Self {
        Self {
            block_size: 16,
            search_x: 32,
            search_y: 64,
            step: 1,
            threshold: 8,
            mode: SearchMode::Full,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MotionVector {
    pub dx: i32,
    pub dy: i32,
    /// Total SAD over the block (sum across R/G/B channels and all pixels).
    pub cost: u64,
    /// True if `cost` is at or below the per-block threshold.
    pub matched: bool,
}

#[derive(Debug, Clone)]
pub struct BlockMatchResult {
    pub block_size: u32,
    pub cols: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    pub vectors: Vec<MotionVector>,
}

impl BlockMatchResult {
    pub fn get(&self, col: u32, row: u32) -> &MotionVector {
        &self.vectors[(row * self.cols + col) as usize]
    }

    pub fn unmatched(&self) -> usize {
        self.vectors.iter().filter(|v| !v.matched).count()
    }
}

#[inline]
fn sad_block(
    reference: &RgbaImage,
    target: &RgbaImage,
    rx: u32,
    ry: u32,
    tx: i32,
    ty: i32,
    block_size: u32,
    cutoff: u64,
) -> u64 {
    let tw = target.width() as i32;
    let th = target.height() as i32;
    let b = block_size as i32;
    if tx < 0 || ty < 0 || tx + b > tw || ty + b > th {
        return u64::MAX;
    }

    let r_buf = reference.as_raw();
    let t_buf = target.as_raw();
    let r_stride = reference.width() as usize * 4;
    let t_stride = target.width() as usize * 4;

    let mut sum: u64 = 0;
    for j in 0..block_size as usize {
        let r_row = (ry as usize + j) * r_stride + rx as usize * 4;
        let t_row = (ty as usize + j) * t_stride + tx as usize * 4;
        for i in 0..block_size as usize {
            let r_off = r_row + i * 4;
            let t_off = t_row + i * 4;
            let dr = (r_buf[r_off] as i32 - t_buf[t_off] as i32).unsigned_abs() as u64;
            let dg = (r_buf[r_off + 1] as i32 - t_buf[t_off + 1] as i32).unsigned_abs() as u64;
            let db = (r_buf[r_off + 2] as i32 - t_buf[t_off + 2] as i32).unsigned_abs() as u64;
            sum += dr + dg + db;
        }
        // Early-out: this row already exceeded the current best.
        if sum >= cutoff {
            return sum;
        }
    }
    sum
}

pub fn diff(
    reference: &RgbaImage,
    target: &RgbaImage,
    opts: &BlockMatchOptions,
) -> BlockMatchResult {
    let block_size = opts.block_size.max(1);
    let width = reference.width().min(target.width());
    let height = reference.height().min(target.height());
    let cols = width / block_size;
    let rows = height / block_size;

    let threshold_total =
        (block_size as u64) * (block_size as u64) * 3 * opts.threshold as u64;
    let step = opts.step.max(1) as i32;
    let search_x = opts.search_x.max(0);
    let search_y = opts.search_y.max(0);

    let vectors: Vec<MotionVector> = (0..rows)
        .into_par_iter()
        .flat_map_iter(|by| {
            (0..cols).map(move |bx| {
                let rx = bx * block_size;
                let ry = by * block_size;
                let cx = rx as i32;
                let cy = ry as i32;

                let (best_dx, best_dy, best_cost) = match opts.mode {
                    SearchMode::Full => search_full(
                        reference, target, rx, ry, cx, cy, block_size, search_x, search_y,
                        step,
                    ),
                    SearchMode::Hierarchical => search_hierarchical(
                        reference, target, rx, ry, cx, cy, block_size, search_x, search_y,
                    ),
                };

                MotionVector {
                    dx: best_dx,
                    dy: best_dy,
                    cost: best_cost,
                    matched: best_cost <= threshold_total,
                }
            })
        })
        .collect();

    BlockMatchResult {
        block_size,
        cols,
        rows,
        width,
        height,
        vectors,
    }
}

#[inline]
fn search_full(
    reference: &RgbaImage,
    target: &RgbaImage,
    rx: u32,
    ry: u32,
    cx: i32,
    cy: i32,
    block_size: u32,
    search_x: i32,
    search_y: i32,
    step: i32,
) -> (i32, i32, u64) {
    let mut best_cost =
        sad_block(reference, target, rx, ry, cx, cy, block_size, u64::MAX);
    let mut best_dx = 0i32;
    let mut best_dy = 0i32;
    if best_cost == 0 {
        return (0, 0, 0);
    }
    let mut dy = -search_y;
    while dy <= search_y {
        let mut dx = -search_x;
        while dx <= search_x {
            if !(dx == 0 && dy == 0) {
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, best_cost,
                );
                if cost < best_cost {
                    best_cost = cost;
                    best_dx = dx;
                    best_dy = dy;
                    if best_cost == 0 {
                        return (best_dx, best_dy, 0);
                    }
                }
            }
            dx += step;
        }
        dy += step;
    }
    (best_dx, best_dy, best_cost)
}

/// Two-phase search: a coarse uniform grid scan over the whole window
/// followed by 3×3 logarithmic refinement around the best coarse hit. The
/// coarse stride is chosen so that the subsequent halving refinement can
/// always reach any pixel inside the window.
#[inline]
fn search_hierarchical(
    reference: &RgbaImage,
    target: &RgbaImage,
    rx: u32,
    ry: u32,
    cx: i32,
    cy: i32,
    block_size: u32,
    search_x: i32,
    search_y: i32,
) -> (i32, i32, u64) {
    let mut best_cost =
        sad_block(reference, target, rx, ry, cx, cy, block_size, u64::MAX);
    let mut best_dx = 0i32;
    let mut best_dy = 0i32;
    if best_cost == 0 {
        return (0, 0, 0);
    }

    // Coarse stride: roughly 1/8 of the larger search radius, but capped at
    // `block_size` so that the refinement halving (whose total reach is
    // `coarse - 1`) always overlaps neighboring coarse points and so that
    // any block-aligned feature in the target is hit by the coarse scan.
    let coarse = (search_x.max(search_y) / 8)
        .max(1)
        .min(block_size as i32);

    // Phase 1: coarse uniform scan.
    let mut dy = -search_y;
    while dy <= search_y {
        let mut dx = -search_x;
        while dx <= search_x {
            if !(dx == 0 && dy == 0) {
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, best_cost,
                );
                if cost < best_cost {
                    best_cost = cost;
                    best_dx = dx;
                    best_dy = dy;
                    if best_cost == 0 {
                        return (best_dx, best_dy, 0);
                    }
                }
            }
            dx += coarse;
        }
        dy += coarse;
    }

    // Phase 2: halving logarithmic refinement around the best coarse hit.
    let mut stride = (coarse / 2).max(1);
    loop {
        for oy in -1..=1 {
            for ox in -1..=1 {
                if ox == 0 && oy == 0 {
                    continue;
                }
                let dx = best_dx + ox * stride;
                let dy = best_dy + oy * stride;
                if dx.abs() > search_x || dy.abs() > search_y {
                    continue;
                }
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, best_cost,
                );
                if cost < best_cost {
                    best_cost = cost;
                    best_dx = dx;
                    best_dy = dy;
                    if best_cost == 0 {
                        return (best_dx, best_dy, 0);
                    }
                }
            }
        }
        if stride == 1 {
            break;
        }
        stride /= 2;
    }
    (best_dx, best_dy, best_cost)
}

/// A pair of [`BlockMatchResult`]s, one for each direction. `forward` flags
/// blocks of the reference image that disappeared from the target ("removed"
/// content). `reverse` flags blocks of the target image that did not exist
/// in the reference ("added" content).
#[derive(Debug, Clone)]
pub struct BidirectionalDiff {
    pub forward: BlockMatchResult,
    pub reverse: BlockMatchResult,
}

/// Runs `diff` in both directions in parallel. Use this when you want to
/// visualize additions as well as removals.
pub fn diff_bidirectional(
    reference: &RgbaImage,
    target: &RgbaImage,
    opts: &BlockMatchOptions,
) -> BidirectionalDiff {
    let (forward, reverse) = rayon::join(
        || diff(reference, target, opts),
        || diff(target, reference, opts),
    );
    BidirectionalDiff { forward, reverse }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn identical_images_have_zero_cost_and_all_matched() {
        let img = solid(64, 64, [200, 100, 50, 255]);
        let result = diff(&img, &img, &BlockMatchOptions::default());
        assert!(result.vectors.iter().all(|v| v.matched && v.cost == 0));
    }

    #[test]
    fn vertical_shift_is_recovered() {
        // Reference: a horizontal red stripe at y = 16..32.
        let mut a = solid(64, 64, [255, 255, 255, 255]);
        for y in 16..32 {
            for x in 0..64 {
                a.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        // Target: same stripe shifted down by 16 px (y = 32..48).
        let mut b = solid(64, 64, [255, 255, 255, 255]);
        for y in 32..48 {
            for x in 0..64 {
                b.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let opts = BlockMatchOptions {
            block_size: 16,
            search_x: 0,
            search_y: 32,
            step: 1,
            threshold: 0,
            mode: SearchMode::Full,
        };
        let result = diff(&a, &b, &opts);
        // The block originally containing the stripe (row 1) should find it
        // at dy = +16 with zero residual.
        let mv = result.get(0, 1);
        assert_eq!(mv.dy, 16);
        assert_eq!(mv.cost, 0);
        assert!(mv.matched);
    }

    #[test]
    fn hierarchical_search_recovers_diagonal_shift() {
        let mut a = solid(96, 96, [255, 255, 255, 255]);
        for y in 16..32 {
            for x in 16..32 {
                a.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        // Shift +32 in x and +48 in y.
        let mut b = solid(96, 96, [255, 255, 255, 255]);
        for y in 64..80 {
            for x in 48..64 {
                b.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        let opts = BlockMatchOptions {
            block_size: 16,
            search_x: 64,
            search_y: 64,
            step: 1,
            threshold: 0,
            mode: SearchMode::Hierarchical,
        };
        let result = diff(&a, &b, &opts);
        // The block at (1, 1) in `a` is the green square; it should find its
        // match at (3, 4) in `b` — i.e. dx = +32, dy = +48.
        let mv = result.get(1, 1);
        assert_eq!(mv.dx, 32);
        assert_eq!(mv.dy, 48);
        assert_eq!(mv.cost, 0);
    }

    #[test]
    fn horizontal_shift_is_recovered() {
        let mut a = solid(64, 64, [255, 255, 255, 255]);
        for y in 0..64 {
            for x in 16..32 {
                a.put_pixel(x, y, Rgba([0, 0, 255, 255]));
            }
        }
        let mut b = solid(64, 64, [255, 255, 255, 255]);
        for y in 0..64 {
            for x in 32..48 {
                b.put_pixel(x, y, Rgba([0, 0, 255, 255]));
            }
        }
        let opts = BlockMatchOptions {
            block_size: 16,
            search_x: 32,
            search_y: 0,
            step: 1,
            threshold: 0,
            mode: SearchMode::Full,
        };
        let result = diff(&a, &b, &opts);
        let mv = result.get(1, 0);
        assert_eq!(mv.dx, 16);
        assert_eq!(mv.cost, 0);
        assert!(mv.matched);
    }
}
