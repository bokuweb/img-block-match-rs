use crate::block_match::{BidirectionalDiff, BlockMatchResult, Region};
use image::{Rgba, RgbaImage};

/// Renders a heatmap where each block is tinted from green (cost 0) through
/// yellow to red (cost ≥ `max_cost`) by blending onto `base`. Unlike
/// [`render_diff`], this visualizes the magnitude of the residual for every
/// block, not just a binary "matched / unmatched" classification.
///
/// If `max_cost` is `None`, the maximum block cost in `result` is used.
pub fn render_heatmap(
    base: &RgbaImage,
    result: &BlockMatchResult,
    max_cost: Option<u64>,
    alpha: u8,
) -> RgbaImage {
    let cap = max_cost.unwrap_or_else(|| {
        result
            .vectors
            .iter()
            .map(|v| v.cost)
            .filter(|&c| c != u64::MAX)
            .max()
            .unwrap_or(1)
            .max(1)
    });
    let mut out = base.clone();
    let bs = result.block_size;
    let a = alpha as u16;
    let inv = 255u16 - a;
    let (w, h) = (out.width(), out.height());
    for by in 0..result.rows {
        for bx in 0..result.cols {
            let mv = result.get(bx, by);
            let c = if mv.cost == u64::MAX { cap } else { mv.cost.min(cap) };
            let t = (c as f32 / cap as f32).clamp(0.0, 1.0);
            let color = heat_color(t);
            let x0 = bx * bs;
            let y0 = by * bs;
            for j in 0..bs {
                let y = y0 + j;
                if y >= h {
                    break;
                }
                for i in 0..bs {
                    let x = x0 + i;
                    if x >= w {
                        break;
                    }
                    let px = out.get_pixel_mut(x, y);
                    px.0[0] = ((color[0] as u16 * a + px.0[0] as u16 * inv) / 255) as u8;
                    px.0[1] = ((color[1] as u16 * a + px.0[1] as u16 * inv) / 255) as u8;
                    px.0[2] = ((color[2] as u16 * a + px.0[2] as u16 * inv) / 255) as u8;
                }
            }
        }
    }
    out
}

/// `t` in [0, 1] mapped to a green→yellow→red gradient.
fn heat_color(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        // green → yellow
        let k = t * 2.0;
        [
            (k * 255.0) as u8,
            255,
            0,
        ]
    } else {
        // yellow → red
        let k = (t - 0.5) * 2.0;
        [
            255,
            ((1.0 - k) * 255.0) as u8,
            0,
        ]
    }
}

/// Draws axis-aligned bounding-box outlines (1-pixel stroke) around each
/// region onto `img` in place.
pub fn draw_regions(img: &mut RgbaImage, regions: &[Region], color: [u8; 4]) {
    let c = Rgba(color);
    let (iw, ih) = (img.width(), img.height());
    for r in regions {
        let x1 = r.x.min(iw.saturating_sub(1));
        let y1 = r.y.min(ih.saturating_sub(1));
        let x2 = (r.x + r.width).min(iw).saturating_sub(1);
        let y2 = (r.y + r.height).min(ih).saturating_sub(1);
        for x in x1..=x2 {
            img.put_pixel(x, y1, c);
            img.put_pixel(x, y2, c);
        }
        for y in y1..=y2 {
            img.put_pixel(x1, y, c);
            img.put_pixel(x2, y, c);
        }
    }
}

/// How unmatched regions are highlighted on top of the reference / target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightStyle {
    /// Draw only the bounding-box outline of each region (`stroke` pixels
    /// wide). Underlying content stays visible — recommended for review
    /// workflows.
    Outline { stroke: u32 },
    /// Fill the region's bounding box with the highlight color blended at
    /// ~40 % opacity. Strong visual cue while still letting the underlying
    /// content show through.
    Filled,
}

impl Default for HighlightStyle {
    fn default() -> Self {
        HighlightStyle::Outline { stroke: 2 }
    }
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// How to highlight unmatched regions.
    pub style: HighlightStyle,
    /// Passed to `BlockMatchResult::unmatched_regions` when clustering.
    pub merge_gap: u32,
    /// Passed to `BlockMatchResult::unmatched_regions` when clustering.
    pub min_blocks: u32,
    /// If true, draw a small arrow on each block showing its motion vector.
    pub draw_vectors: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            style: HighlightStyle::default(),
            merge_gap: 1,
            min_blocks: 1,
            draw_vectors: false,
        }
    }
}

/// Renders a diff visualization by clustering unmatched blocks into
/// regions and drawing each region's bounding box in `color` according to
/// `opts.style`.
pub fn render_diff(
    base: &RgbaImage,
    result: &BlockMatchResult,
    color: [u8; 3],
    opts: &RenderOptions,
) -> RgbaImage {
    let mut out = base.clone();
    let regions = result.unmatched_regions(opts.merge_gap, opts.min_blocks);
    paint_regions(&mut out, &regions, color, opts.style);
    if opts.draw_vectors {
        draw_vectors(&mut out, result);
    }
    out
}

/// Alpha used for the `Filled` style — ~40 % so underlying content stays
/// legible. Outline stays fully opaque so the boundary is crisp.
const FILLED_ALPHA: u8 = 102;

#[inline]
fn blend_pixel(img: &mut RgbaImage, x: u32, y: u32, color: [u8; 3], alpha: u8) {
    let a = alpha as u16;
    let inv = 255u16 - a;
    let px = img.get_pixel_mut(x, y);
    px.0[0] = ((color[0] as u16 * a + px.0[0] as u16 * inv) / 255) as u8;
    px.0[1] = ((color[1] as u16 * a + px.0[1] as u16 * inv) / 255) as u8;
    px.0[2] = ((color[2] as u16 * a + px.0[2] as u16 * inv) / 255) as u8;
}

fn paint_regions(
    img: &mut RgbaImage,
    regions: &[Region],
    color: [u8; 3],
    style: HighlightStyle,
) {
    let (w, h) = (img.width(), img.height());
    let solid = Rgba([color[0], color[1], color[2], 255]);
    for r in regions {
        let x1 = r.x.min(w);
        let y1 = r.y.min(h);
        let x2 = (r.x + r.width).min(w);
        let y2 = (r.y + r.height).min(h);
        if x2 <= x1 || y2 <= y1 {
            continue;
        }
        match style {
            HighlightStyle::Filled => {
                for y in y1..y2 {
                    for x in x1..x2 {
                        blend_pixel(img, x, y, color, FILLED_ALPHA);
                    }
                }
            }
            HighlightStyle::Outline { stroke } => {
                let s = stroke.max(1);
                // Top + bottom bands.
                for y in y1..(y1 + s).min(y2) {
                    for x in x1..x2 {
                        img.put_pixel(x, y, solid);
                    }
                }
                for y in y2.saturating_sub(s).max(y1)..y2 {
                    for x in x1..x2 {
                        img.put_pixel(x, y, solid);
                    }
                }
                // Left + right bands (skip rows already drawn above).
                let inner_y1 = (y1 + s).min(y2);
                let inner_y2 = y2.saturating_sub(s).max(inner_y1);
                for y in inner_y1..inner_y2 {
                    for x in x1..(x1 + s).min(x2) {
                        img.put_pixel(x, y, solid);
                    }
                    for x in x2.saturating_sub(s).max(x1)..x2 {
                        img.put_pixel(x, y, solid);
                    }
                }
            }
        }
    }
}

/// Renders a side-by-side composite of the bidirectional diff: the reference
/// on the left with "removed" regions in red, and the target on the right
/// with "added" regions in green, each drawn in the configured highlight
/// style. A thin separator is drawn between the two panels.
pub fn render_bidirectional(
    reference: &RgbaImage,
    target: &RgbaImage,
    diff: &BidirectionalDiff,
    opts: &RenderOptions,
) -> RgbaImage {
    let removed = diff.forward.unmatched_regions(opts.merge_gap, opts.min_blocks);
    let added = diff.reverse.unmatched_regions(opts.merge_gap, opts.min_blocks);

    let mut left = reference.clone();
    let mut right = target.clone();
    paint_regions(&mut left, &removed, [220, 50, 50], opts.style);
    paint_regions(&mut right, &added, [40, 180, 80], opts.style);
    if opts.draw_vectors {
        draw_vectors(&mut left, &diff.forward);
        draw_vectors(&mut right, &diff.reverse);
    }

    let gap: u32 = 4;
    let w = left.width() + right.width() + gap;
    let h = left.height().max(right.height());
    let mut out = RgbaImage::from_pixel(w, h, Rgba([30, 30, 30, 255]));
    image::imageops::overlay(&mut out, &left, 0, 0);
    image::imageops::overlay(&mut out, &right, (left.width() + gap) as i64, 0);
    out
}

fn draw_vectors(img: &mut RgbaImage, result: &BlockMatchResult) {
    let b = result.block_size as i32;
    for by in 0..result.rows {
        for bx in 0..result.cols {
            let mv = result.get(bx, by);
            if mv.dx == 0 && mv.dy == 0 {
                continue;
            }
            let cx = bx as i32 * b + b / 2;
            let cy = by as i32 * b + b / 2;
            // Scale displacement down into the block.
            let scale = (b as f32 / 2.0)
                / ((mv.dx.abs().max(mv.dy.abs()).max(1)) as f32)
                .max(1.0);
            let ex = cx + (mv.dx as f32 * scale) as i32;
            let ey = cy + (mv.dy as f32 * scale) as i32;
            draw_line(img, cx, cy, ex, ey, [0, 255, 0, 255]);
        }
    }
}

fn draw_line(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        if x >= 0 && y >= 0 && x < w && y < h {
            img.put_pixel(x as u32, y as u32, image::Rgba(color));
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
