use crate::block_match::{BidirectionalDiff, BlockMatchResult};
use image::{Rgba, RgbaImage};

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Alpha (0..=255) used when blending the red overlay onto unmatched blocks.
    pub overlay_alpha: u8,
    /// If true, draw a small arrow on each block showing its motion vector.
    pub draw_vectors: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            overlay_alpha: 160,
            draw_vectors: false,
        }
    }
}

/// Renders a diff visualization by overlaying `color` on blocks that could
/// not be matched (i.e. genuine content differences after accounting for X/Y
/// shifts).
pub fn render_diff(
    base: &RgbaImage,
    result: &BlockMatchResult,
    color: [u8; 3],
    opts: &RenderOptions,
) -> RgbaImage {
    let mut out = base.clone();
    overlay_unmatched(&mut out, result, color, opts.overlay_alpha);
    if opts.draw_vectors {
        draw_vectors(&mut out, result);
    }
    out
}

fn overlay_unmatched(
    img: &mut RgbaImage,
    result: &BlockMatchResult,
    color: [u8; 3],
    alpha: u8,
) {
    let b = result.block_size;
    let a = alpha as u16;
    let inv = 255u16 - a;
    let (w, h) = (img.width(), img.height());
    for by in 0..result.rows {
        for bx in 0..result.cols {
            let mv = result.get(bx, by);
            if mv.matched {
                continue;
            }
            let x0 = bx * b;
            let y0 = by * b;
            for j in 0..b {
                let y = y0 + j;
                if y >= h {
                    break;
                }
                for i in 0..b {
                    let x = x0 + i;
                    if x >= w {
                        break;
                    }
                    let px = img.get_pixel_mut(x, y);
                    px.0[0] = ((color[0] as u16 * a + px.0[0] as u16 * inv) / 255) as u8;
                    px.0[1] = ((color[1] as u16 * a + px.0[1] as u16 * inv) / 255) as u8;
                    px.0[2] = ((color[2] as u16 * a + px.0[2] as u16 * inv) / 255) as u8;
                }
            }
        }
    }
}

/// Renders a side-by-side composite of the bidirectional diff: the reference
/// on the left with "removed" content overlaid in red, and the target on the
/// right with "added" content overlaid in green. A thin separator is drawn
/// between the two panels.
pub fn render_bidirectional(
    reference: &RgbaImage,
    target: &RgbaImage,
    diff: &BidirectionalDiff,
    opts: &RenderOptions,
) -> RgbaImage {
    let mut left = reference.clone();
    let mut right = target.clone();
    overlay_unmatched(&mut left, &diff.forward, [220, 50, 50], opts.overlay_alpha);
    overlay_unmatched(&mut right, &diff.reverse, [40, 180, 80], opts.overlay_alpha);
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
