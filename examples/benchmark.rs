//! Compares single-pass vs pyramid block-matching on a synthetic 1920x1080
//! "screenshot reflow" workload.
//!
//! cargo run --release --example benchmark

use image::{Rgba, RgbaImage};
use img_block_match::{diff, diff_pyramid, BlockMatchOptions, SearchMode};
use std::time::Instant;

fn fill_rect(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
    let xe = (x + w).min(img.width());
    let ye = (y + h).min(img.height());
    for j in y..ye {
        for i in x..xe {
            img.put_pixel(i, j, Rgba(c));
        }
    }
}

fn make_pair(w: u32, h: u32, x_shift: u32, y_shift: u32) -> (RgbaImage, RgbaImage) {
    let bg = [245, 245, 248, 255];
    let card = [255, 255, 255, 255];
    let text = [110, 110, 110, 255];
    let mut a = RgbaImage::from_pixel(w, h, Rgba(bg));
    let mut b = RgbaImage::from_pixel(w, h, Rgba(bg));
    for row in 0..12 {
        let y = 40 + row * 80;
        fill_rect(&mut a, 80, y, w - 160, 60, card);
        fill_rect(&mut a, 96, y + 12, 200, 12, text);
        fill_rect(&mut a, 96, y + 32, w - 200, 6, text);
        fill_rect(
            &mut b,
            80 + x_shift,
            y + y_shift,
            (w - 160 - x_shift).min(w - 160),
            60,
            card,
        );
        fill_rect(&mut b, 96 + x_shift, y + 12 + y_shift, 200, 12, text);
        fill_rect(
            &mut b,
            96 + x_shift,
            y + 32 + y_shift,
            (w - 200 - x_shift).min(w - 200),
            6,
            text,
        );
    }
    (a, b)
}

fn run(label: &str, w: u32, h: u32, x_shift: u32, y_shift: u32, search_x: i32, search_y: i32) {
    let (a, b) = make_pair(w, h, x_shift, y_shift);
    let opts = BlockMatchOptions {
        block_size: 16,
        search_x,
        search_y,
        threshold: 8,
        mode: SearchMode::Hierarchical,
        ..Default::default()
    };

    let t = Instant::now();
    let r1 = diff(&a, &b, &opts);
    let single = t.elapsed();

    let t = Instant::now();
    let r2 = diff_pyramid(&a, &b, &opts, 4, 8);
    let pyramid = t.elapsed();

    println!(
        "\n[{}] {}x{}, search ±{}/±{}",
        label, w, h, search_x, search_y
    );
    println!(
        "  single-pass hierarchical: {:>8.2?}  ({} unmatched / {})",
        single,
        r1.unmatched(),
        r1.vectors.len()
    );
    println!(
        "  pyramid (4x + ±8):        {:>8.2?}  ({} unmatched / {})  {:.2}x",
        pyramid,
        r2.unmatched(),
        r2.vectors.len(),
        single.as_secs_f64() / pyramid.as_secs_f64()
    );
}

fn main() {
    run("1080p small search", 1920, 1080, 32, 48, 64, 96);
    run("1080p wide search", 1920, 1080, 32, 48, 200, 300);
    run("4K wide search", 3840, 2160, 32, 80, 200, 400);
}
