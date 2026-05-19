//! Image-pyramid wrapper around the per-block matcher.
//!
//! For very large screenshots a wide search window dominates runtime even
//! in `SearchMode::Hierarchical`. The pyramid approach runs block-matching
//! at a downscaled level first to get a coarse motion estimate per block,
//! then refines at full resolution in a tiny window around each predicted
//! vector.
//!
//! Trade-off: the coarse pass uses box-averaged downsampling so it tolerates
//! sub-pixel anti-aliasing, but very thin (≤ `2 * scale` pixels) content
//! can be smoothed away entirely. For typical screenshots this is fine.

use image::{imageops::FilterType, RgbaImage};
use rayon::prelude::*;

use crate::block_match::{
    diff, BlockMatchOptions, BlockMatchResult, MotionVector, SearchMode,
};

/// Two-level pyramid diff.
///
/// - `coarse_scale`: integer downsampling factor for the coarse pass
///   (typical: 2 or 4). Must be ≥ 2; pass 1 to fall back to the single-pass
///   matcher.
/// - `refine_radius`: half-window (in full-resolution pixels) used for the
///   refinement pass around the coarse-predicted vector. Typical: `block_size`.
///
/// Other knobs (block size, threshold, mode, …) come from `opts`. The
/// coarse pass scales the search window by `1 / coarse_scale`; the
/// refinement pass uses a fixed `±refine_radius` window.
pub fn diff_pyramid(
    reference: &RgbaImage,
    target: &RgbaImage,
    opts: &BlockMatchOptions,
    coarse_scale: u32,
    refine_radius: i32,
) -> BlockMatchResult {
    if coarse_scale < 2 {
        return diff(reference, target, opts);
    }

    let s = coarse_scale;
    let coarse_block = (opts.block_size / s).max(2);
    let cw = (reference.width() / s).max(coarse_block);
    let ch = (reference.height() / s).max(coarse_block);
    let tcw = (target.width() / s).max(coarse_block);
    let tch = (target.height() / s).max(coarse_block);

    let coarse_ref = image::imageops::resize(reference, cw, ch, FilterType::Triangle);
    let coarse_tgt = image::imageops::resize(target, tcw, tch, FilterType::Triangle);

    let coarse_opts = BlockMatchOptions {
        block_size: coarse_block,
        search_x: (opts.search_x / s as i32).max(1),
        search_y: (opts.search_y / s as i32).max(1),
        step: 1,
        threshold: opts.threshold,
        mode: SearchMode::Hierarchical,
        compute_confidence: false,
    };
    let coarse = diff(&coarse_ref, &coarse_tgt, &coarse_opts);

    // Refinement pass: full-res search but only ±refine_radius around the
    // coarse-predicted vector for each block.
    let block_size = opts.block_size;
    let width = reference.width().min(target.width());
    let height = reference.height().min(target.height());
    let cols = width / block_size;
    let rows = height / block_size;
    let threshold_total =
        (block_size as u64) * (block_size as u64) * 3 * opts.threshold as u64;

    let coarse_ref = &coarse;
    let vectors: Vec<MotionVector> = (0..rows)
        .into_par_iter()
        .flat_map_iter(move |by| {
            (0..cols).map(move |bx| {
                // Map the full-res block to the closest coarse block to fetch
                // its predicted (dx, dy). Clamp to the coarse grid.
                let cbx = ((bx * block_size + block_size / 2) / s / coarse_block.max(1))
                    .min(coarse_ref.cols.saturating_sub(1));
                let cby = ((by * block_size + block_size / 2) / s / coarse_block.max(1))
                    .min(coarse_ref.rows.saturating_sub(1));
                let predicted = coarse_ref.get(cbx, cby);
                let pdx = predicted.dx * s as i32;
                let pdy = predicted.dy * s as i32;

                refine_block(
                    reference,
                    target,
                    bx * block_size,
                    by * block_size,
                    block_size,
                    pdx,
                    pdy,
                    refine_radius,
                    threshold_total,
                )
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

fn refine_block(
    reference: &RgbaImage,
    target: &RgbaImage,
    rx: u32,
    ry: u32,
    block_size: u32,
    pdx: i32,
    pdy: i32,
    radius: i32,
    threshold_total: u64,
) -> MotionVector {
    let cx = rx as i32;
    let cy = ry as i32;
    let mut best_cost = sad(reference, target, rx, ry, cx + pdx, cy + pdy, block_size, u64::MAX);
    let mut best_dx = pdx;
    let mut best_dy = pdy;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx == 0 && dy == 0 {
                continue;
            }
            let tx = cx + pdx + dx;
            let ty = cy + pdy + dy;
            let cost = sad(reference, target, rx, ry, tx, ty, block_size, best_cost);
            if cost < best_cost {
                best_cost = cost;
                best_dx = pdx + dx;
                best_dy = pdy + dy;
                if best_cost == 0 {
                    break;
                }
            }
        }
        if best_cost == 0 {
            break;
        }
    }
    MotionVector {
        dx: best_dx,
        dy: best_dy,
        cost: best_cost,
        second_cost: u64::MAX,
        matched: best_cost <= threshold_total,
    }
}

#[inline]
fn sad(
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
            sum += (r_buf[r_off] as i32 - t_buf[t_off] as i32).unsigned_abs() as u64;
            sum += (r_buf[r_off + 1] as i32 - t_buf[t_off + 1] as i32).unsigned_abs() as u64;
            sum += (r_buf[r_off + 2] as i32 - t_buf[t_off + 2] as i32).unsigned_abs() as u64;
        }
        if sum > cutoff {
            return sum;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn pyramid_recovers_large_shift() {
        // Big synthetic image with a feature shifted both axes.
        let w = 256u32;
        let h = 256u32;
        let mut a = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
        for y in 32..64 {
            for x in 32..64 {
                a.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        let mut b = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
        // Shifted +96 px in both axes.
        for y in 128..160 {
            for x in 128..160 {
                b.put_pixel(x, y, Rgba([0, 200, 0, 255]));
            }
        }
        let opts = BlockMatchOptions {
            block_size: 16,
            search_x: 128,
            search_y: 128,
            threshold: 0,
            mode: SearchMode::Hierarchical,
            ..Default::default()
        };
        let result = diff_pyramid(&a, &b, &opts, 4, 16);
        // Block (2, 2) covers the green region in `a`; its full-res
        // displacement is +96 in both axes.
        let mv = result.get(2, 2);
        assert_eq!(mv.cost, 0);
        assert_eq!(mv.dx, 96);
        assert_eq!(mv.dy, 96);
    }
}
