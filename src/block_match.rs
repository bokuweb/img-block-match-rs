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
    /// When true, the search also tracks the best spatially-distinct
    /// runner-up SAD so [`MotionVector::confidence`] is meaningful. Comes
    /// at the cost of disabling the early-return-on-perfect-match
    /// optimization, so prefer the default `false` for visualization-only
    /// workflows.
    pub compute_confidence: bool,
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
            compute_confidence: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MotionVector {
    pub dx: i32,
    pub dy: i32,
    /// Total SAD over the block (sum across R/G/B channels and all pixels).
    pub cost: u64,
    /// Best SAD among candidates whose displacement differs from
    /// `(dx, dy)` by more than the block size (i.e. the runner-up at a
    /// spatially distinct location). `u64::MAX` if no such candidate was
    /// evaluated. Use [`confidence`] to turn this into a 0..=1 score.
    pub second_cost: u64,
    /// True if `cost` is at or below the per-block threshold.
    pub matched: bool,
}

impl MotionVector {
    /// 0.0 = the best match is no better than other positions in the search
    /// window (flat / repeating content — vector is unreliable);
    /// 1.0 = the best match is strictly the only good one (high confidence).
    pub fn confidence(&self) -> f32 {
        if self.second_cost == u64::MAX {
            return 1.0;
        }
        if self.second_cost == 0 {
            return 0.0;
        }
        let c = self.cost as f64;
        let s = self.second_cost as f64;
        ((s - c) / s).clamp(0.0, 1.0) as f32
    }
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

/// Pixel-space axis-aligned bounding box of a cluster of unmatched blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// How many block-grid cells make up this cluster.
    pub block_count: u32,
}

impl BlockMatchResult {
    pub fn get(&self, col: u32, row: u32) -> &MotionVector {
        &self.vectors[(row * self.cols + col) as usize]
    }

    pub fn unmatched(&self) -> usize {
        self.vectors.iter().filter(|v| !v.matched).count()
    }

    /// Applies a 3×3 majority filter on the `matched` flag: each block is
    /// re-classified as matched iff the majority of its 3×3 neighborhood
    /// (including itself) is matched. Useful to suppress isolated false
    /// positives caused by anti-aliasing flips, and to fill 1-block holes
    /// inside a genuine diff region. Leaves `cost`/`dx`/`dy` unchanged.
    pub fn smooth_matched(&mut self) {
        let cols = self.cols as i32;
        let rows = self.rows as i32;
        let original: Vec<bool> = self.vectors.iter().map(|v| v.matched).collect();
        for y in 0..rows {
            for x in 0..cols {
                let mut matched_count = 0i32;
                let mut total = 0i32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let nx = x + dx;
                        let ny = y + dy;
                        if nx < 0 || nx >= cols || ny < 0 || ny >= rows {
                            continue;
                        }
                        total += 1;
                        if original[(ny * cols + nx) as usize] {
                            matched_count += 1;
                        }
                    }
                }
                let i = (y * cols + x) as usize;
                self.vectors[i].matched = matched_count * 2 > total;
            }
        }
    }

    /// Groups spatially adjacent unmatched blocks into bounding rectangles
    /// using 8-connected flood fill (orthogonal + diagonal neighbors). An
    /// optional `merge_gap` dilation merges clusters separated by up to that
    /// many matched blocks, and the final bounding box spans the min/max
    /// `(col, row)` of every original unmatched block in the cluster.
    ///
    /// `min_blocks` discards tiny clusters (helps suppress isolated
    /// false-positives on anti-aliased edges).
    pub fn unmatched_regions(&self, merge_gap: u32, min_blocks: u32) -> Vec<Region> {
        let cols = self.cols as i32;
        let rows = self.rows as i32;
        let bs = self.block_size;
        let mut mask = vec![false; (cols * rows) as usize];
        for (i, v) in self.vectors.iter().enumerate() {
            if !v.matched {
                mask[i] = true;
            }
        }

        // Dilate the unmatched mask by `merge_gap` blocks so that nearby
        // clusters get merged into one rectangle.
        let mask = dilate(&mask, cols as u32, rows as u32, merge_gap);

        let mut visited = vec![false; mask.len()];
        let mut regions = Vec::new();
        for y in 0..rows {
            for x in 0..cols {
                let idx = (y * cols + x) as usize;
                if !mask[idx] || visited[idx] {
                    continue;
                }
                // BFS for the connected component.
                let mut stack = vec![(x, y)];
                let mut min_x = i32::MAX;
                let mut max_x = i32::MIN;
                let mut min_y = i32::MAX;
                let mut max_y = i32::MIN;
                let mut count = 0u32;
                while let Some((cx, cy)) = stack.pop() {
                    let i = (cy * cols + cx) as usize;
                    if visited[i] || !mask[i] {
                        continue;
                    }
                    visited[i] = true;
                    // The dilation halo is used only for connectivity; the
                    // bounding box and block count come from the original
                    // unmatched mask.
                    if !self.vectors[i].matched {
                        count += 1;
                        if cx < min_x { min_x = cx; }
                        if cx > max_x { max_x = cx; }
                        if cy < min_y { min_y = cy; }
                        if cy > max_y { max_y = cy; }
                    }
                    for dy in -1..=1i32 {
                        for dx in -1..=1i32 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let nx = cx + dx;
                            let ny = cy + dy;
                            if nx >= 0 && nx < cols && ny >= 0 && ny < rows {
                                stack.push((nx, ny));
                            }
                        }
                    }
                }
                if count < min_blocks.max(1) {
                    continue;
                }
                regions.push(Region {
                    x: min_x as u32 * bs,
                    y: min_y as u32 * bs,
                    width: (max_x - min_x + 1) as u32 * bs,
                    height: (max_y - min_y + 1) as u32 * bs,
                    block_count: count,
                });
            }
        }
        regions.sort_by_key(|r| (r.y, r.x));
        regions
    }
}

fn dilate(mask: &[bool], cols: u32, rows: u32, radius: u32) -> Vec<bool> {
    if radius == 0 {
        return mask.to_vec();
    }
    let r = radius as i32;
    let mut out = vec![false; mask.len()];
    for y in 0..rows as i32 {
        for x in 0..cols as i32 {
            if !mask[(y * cols as i32 + x) as usize] {
                continue;
            }
            for dy in -r..=r {
                let ny = y + dy;
                if ny < 0 || ny >= rows as i32 { continue; }
                for dx in -r..=r {
                    let nx = x + dx;
                    if nx < 0 || nx >= cols as i32 { continue; }
                    out[(ny * cols as i32 + nx) as usize] = true;
                }
            }
        }
    }
    out
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
        // Early-out: this row strictly exceeds the cutoff. Strict so that
        // ties (equal-cost candidates at spatially distant locations) still
        // run to completion and reach the second-best tracker.
        if sum > cutoff {
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

                let (best_dx, best_dy, best_cost, second_cost) = match opts.mode {
                    SearchMode::Full => search_full(
                        reference, target, rx, ry, cx, cy, block_size, search_x, search_y,
                        step, opts.compute_confidence,
                    ),
                    SearchMode::Hierarchical => search_hierarchical(
                        reference, target, rx, ry, cx, cy, block_size, search_x, search_y,
                        opts.compute_confidence,
                    ),
                };

                MotionVector {
                    dx: best_dx,
                    dy: best_dy,
                    cost: best_cost,
                    second_cost,
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
    compute_confidence: bool,
) -> (i32, i32, u64, u64) {
    let mut tracker = SecondBestTracker::new(block_size as i32);
    let initial = sad_block(reference, target, rx, ry, cx, cy, block_size, u64::MAX);
    tracker.consider(0, 0, initial);
    if initial == 0 && !compute_confidence {
        return (0, 0, 0, u64::MAX);
    }
    let mut dy = -search_y;
    while dy <= search_y {
        let mut dx = -search_x;
        while dx <= search_x {
            if !(dx == 0 && dy == 0) {
                let cutoff = tracker.second_cost().max(tracker.best_cost);
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, cutoff,
                );
                tracker.consider(dx, dy, cost);
                if !compute_confidence && tracker.best_cost == 0 {
                    return (tracker.best_dx, tracker.best_dy, 0, u64::MAX);
                }
            }
            dx += step;
        }
        dy += step;
    }
    (
        tracker.best_dx,
        tracker.best_dy,
        tracker.best_cost,
        tracker.second_cost(),
    )
}

/// Tracks the best and the spatially-separated runner-up SAD seen so far.
/// "Spatially separated" means displacement differs from the current best by
/// more than `block_size` pixels, so we don't double-count near-duplicates
/// from the immediate neighborhood.
struct SecondBestTracker {
    sep: i32,
    best_cost: u64,
    best_dx: i32,
    best_dy: i32,
    runner_cost: u64,
    runner_dx: i32,
    runner_dy: i32,
}

impl SecondBestTracker {
    fn new(sep: i32) -> Self {
        Self {
            sep,
            best_cost: u64::MAX,
            best_dx: 0,
            best_dy: 0,
            runner_cost: u64::MAX,
            runner_dx: 0,
            runner_dy: 0,
        }
    }

    fn second_cost(&self) -> u64 {
        self.runner_cost
    }

    #[inline]
    fn far_from(&self, dx: i32, dy: i32, ax: i32, ay: i32) -> bool {
        (dx - ax).abs() > self.sep || (dy - ay).abs() > self.sep
    }

    fn consider(&mut self, dx: i32, dy: i32, cost: u64) {
        if cost == u64::MAX {
            return;
        }
        if cost < self.best_cost {
            // New best — if it's spatially separated from the previous best,
            // demote the previous best to runner-up; otherwise keep the
            // existing runner if it stays far from the new best.
            if self.best_cost != u64::MAX && self.far_from(dx, dy, self.best_dx, self.best_dy) {
                self.runner_cost = self.best_cost;
                self.runner_dx = self.best_dx;
                self.runner_dy = self.best_dy;
            } else if self.runner_cost != u64::MAX
                && !self.far_from(dx, dy, self.runner_dx, self.runner_dy)
            {
                // Existing runner is now too close to the new best — drop it.
                self.runner_cost = u64::MAX;
            }
            self.best_cost = cost;
            self.best_dx = dx;
            self.best_dy = dy;
        } else if cost < self.runner_cost && self.far_from(dx, dy, self.best_dx, self.best_dy) {
            self.runner_cost = cost;
            self.runner_dx = dx;
            self.runner_dy = dy;
        }
    }
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
    compute_confidence: bool,
) -> (i32, i32, u64, u64) {
    let mut tracker = SecondBestTracker::new(block_size as i32);
    let initial = sad_block(reference, target, rx, ry, cx, cy, block_size, u64::MAX);
    tracker.consider(0, 0, initial);
    if initial == 0 && !compute_confidence {
        return (0, 0, 0, u64::MAX);
    }

    let coarse = (search_x.max(search_y) / 8)
        .max(1)
        .min(block_size as i32);

    let mut dy = -search_y;
    while dy <= search_y {
        let mut dx = -search_x;
        while dx <= search_x {
            if !(dx == 0 && dy == 0) {
                let cutoff = tracker.second_cost().max(tracker.best_cost);
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, cutoff,
                );
                tracker.consider(dx, dy, cost);
                if !compute_confidence && tracker.best_cost == 0 {
                    return (tracker.best_dx, tracker.best_dy, 0, u64::MAX);
                }
            }
            dx += coarse;
        }
        dy += coarse;
    }

    let mut stride = (coarse / 2).max(1);
    loop {
        for oy in -1..=1 {
            for ox in -1..=1 {
                if ox == 0 && oy == 0 {
                    continue;
                }
                let dx = tracker.best_dx + ox * stride;
                let dy = tracker.best_dy + oy * stride;
                if dx.abs() > search_x || dy.abs() > search_y {
                    continue;
                }
                let cutoff = tracker.second_cost().max(tracker.best_cost);
                let cost = sad_block(
                    reference, target, rx, ry, cx + dx, cy + dy, block_size, cutoff,
                );
                tracker.consider(dx, dy, cost);
                if !compute_confidence && tracker.best_cost == 0 {
                    return (tracker.best_dx, tracker.best_dy, 0, u64::MAX);
                }
            }
        }
        if stride == 1 {
            break;
        }
        stride /= 2;
    }
    (
        tracker.best_dx,
        tracker.best_dy,
        tracker.best_cost,
        tracker.second_cost(),
    )
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
    fn smooth_matched_removes_isolated_unmatched() {
        let img = solid(64, 64, [255, 255, 255, 255]);
        let opts = BlockMatchOptions {
            block_size: 8,
            ..Default::default()
        };
        let mut result = diff(&img, &img, &opts);
        // One isolated unmatched block in the middle of a matched field.
        let i = (3 * result.cols + 3) as usize;
        result.vectors[i].matched = false;
        assert_eq!(result.unmatched(), 1);
        result.smooth_matched();
        assert_eq!(result.unmatched(), 0, "isolated unmatched should be smoothed out");
    }

    #[test]
    fn smooth_matched_keeps_genuine_cluster() {
        let img = solid(64, 64, [255, 255, 255, 255]);
        let opts = BlockMatchOptions {
            block_size: 8,
            ..Default::default()
        };
        let mut result = diff(&img, &img, &opts);
        // A 3x3 unmatched cluster.
        for y in 2..5 {
            for x in 2..5 {
                let i = (y * result.cols + x) as usize;
                result.vectors[i].matched = false;
            }
        }
        assert_eq!(result.unmatched(), 9);
        result.smooth_matched();
        // The center stays unmatched (5 of 9 neighbors are unmatched).
        let center = (3 * result.cols + 3) as usize;
        assert!(!result.vectors[center].matched);
    }

    #[test]
    fn confidence_is_low_when_runner_is_close() {
        // A block of pure gray in `a` matched against `b` that contains
        // many gray patches of the same shade → many tied near-zero SADs at
        // spatially separated locations → confidence collapses to 0.
        let mut a = solid(64, 64, [255, 255, 255, 255]);
        for y in 16..24 {
            for x in 16..24 {
                a.put_pixel(x, y, Rgba([180, 180, 180, 255]));
            }
        }
        let mut b = solid(64, 64, [255, 255, 255, 255]);
        // Drop the same gray patch at TWO distinct locations in `b`.
        for &(ox, oy) in &[(8u32, 8u32), (40, 40)] {
            for y in 0..8 {
                for x in 0..8 {
                    b.put_pixel(ox + x, oy + y, Rgba([180, 180, 180, 255]));
                }
            }
        }
        let opts = BlockMatchOptions {
            block_size: 8,
            search_x: 32,
            search_y: 32,
            mode: SearchMode::Full,
            compute_confidence: true,
            ..Default::default()
        };
        let result = diff(&a, &b, &opts);
        let mv = result.get(2, 2);
        assert_eq!(mv.cost, 0);
        // Two equally-good matches exist far apart, so the runner-up cost
        // also hits 0 → confidence is 0.
        assert_eq!(mv.confidence(), 0.0);
    }

    #[test]
    fn confidence_is_high_on_unique_content() {
        // A single bright square on white background: only one position
        // in the target produces a low SAD, so confidence should be high.
        let mut a = solid(64, 64, [255, 255, 255, 255]);
        for y in 16..24 {
            for x in 16..24 {
                a.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        let mut b = solid(64, 64, [255, 255, 255, 255]);
        for y in 24..32 {
            for x in 32..40 {
                b.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        let opts = BlockMatchOptions {
            block_size: 8,
            search_x: 24,
            search_y: 16,
            mode: SearchMode::Full,
            compute_confidence: true,
            ..Default::default()
        };
        let result = diff(&a, &b, &opts);
        // Block at (2, 2) in `a` is the green square; should find unique
        // match at (+16, +8).
        let mv = result.get(2, 2);
        assert_eq!(mv.cost, 0);
        assert!(mv.confidence() > 0.5, "expected high confidence, got {}", mv.confidence());
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
            threshold: 0,
            mode: SearchMode::Full,
            ..Default::default()
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
    fn unmatched_regions_cluster_adjacent_blocks() {
        // Build a 4x4 grid of "matched" blocks then forcibly mark a few
        // unmatched. block_size=8 => image 32x32, grid 4x4.
        let img = solid(32, 32, [255, 255, 255, 255]);
        let mut result = diff(&img, &img, &BlockMatchOptions::default());
        // Force two clusters: a 2x2 at top-left and a single block at (3,3).
        // (Using default block_size=16 actually gives a 2x2 grid, so use
        // explicit options.)
        let opts = BlockMatchOptions {
            block_size: 8,
            ..Default::default()
        };
        result = diff(&img, &img, &opts);
        for &(cx, cy) in &[(0u32, 0u32), (1, 0), (0, 1), (1, 1), (3, 3)] {
            let i = (cy * result.cols + cx) as usize;
            result.vectors[i].matched = false;
        }
        let regions = result.unmatched_regions(0, 1);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0], Region { x: 0, y: 0, width: 16, height: 16, block_count: 4 });
        assert_eq!(regions[1], Region { x: 24, y: 24, width: 8, height: 8, block_count: 1 });
    }

    #[test]
    fn unmatched_regions_merge_with_gap() {
        let img = solid(80, 16, [255, 255, 255, 255]);
        let opts = BlockMatchOptions {
            block_size: 8,
            ..Default::default()
        };
        let mut result = diff(&img, &img, &opts);
        // Two unmatched blocks separated by one matched block in the middle.
        for &(cx, cy) in &[(0u32, 0u32), (2, 0)] {
            let i = (cy * result.cols + cx) as usize;
            result.vectors[i].matched = false;
        }
        // merge_gap=0 -> two regions.
        assert_eq!(result.unmatched_regions(0, 1).len(), 2);
        // merge_gap=1 -> dilation bridges the gap -> one region spanning both.
        let merged = result.unmatched_regions(1, 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].x, 0);
        assert_eq!(merged[0].width, 24);
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
            threshold: 0,
            mode: SearchMode::Hierarchical,
            ..Default::default()
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
            threshold: 0,
            mode: SearchMode::Full,
            ..Default::default()
        };
        let result = diff(&a, &b, &opts);
        let mv = result.get(1, 0);
        assert_eq!(mv.dx, 16);
        assert_eq!(mv.cost, 0);
        assert!(mv.matched);
    }
}
