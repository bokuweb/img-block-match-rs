//! Generates a synthetic "before / after" pair that simulates a web layout
//! whose content shifted in BOTH X and Y directions (wider sidebar + taller
//! header), plus one genuine content change (an accent badge added to the
//! middle card).
//!
//! Run with:  cargo run --release --example generate_sample

use image::{Rgba, RgbaImage};

fn fill_rect(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
    let xe = (x + w).min(img.width());
    let ye = (y + h).min(img.height());
    for j in y..ye {
        for i in x..xe {
            img.put_pixel(i, j, Rgba(c));
        }
    }
}

fn draw_cards(img: &mut RgbaImage, x: u32, top: u32, card_w: u32) {
    let card = [255, 255, 255, 255];
    let text = [110, 110, 110, 255];
    let muted = [180, 180, 180, 255];
    for k in 0..3 {
        let y = top + k * 80;
        fill_rect(img, x, y, card_w, 64, card);
        fill_rect(img, x + 16, y + 12, 96, 14, text);
        fill_rect(img, x + 16, y + 36, (card_w as i32 - 80).max(40) as u32, 8, muted);
        fill_rect(img, x + 16, y + 50, (card_w as i32 - 140).max(20) as u32, 8, muted);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let w = 480u32;
    let h = 320u32;
    let bg = [245, 245, 248, 255];
    let header = [60, 100, 180, 255];
    let sidebar = [225, 228, 235, 255];
    let accent = [220, 70, 70, 255];

    // BEFORE: 24px header, 80px-wide sidebar.
    let mut before = RgbaImage::from_pixel(w, h, Rgba(bg));
    fill_rect(&mut before, 0, 0, w, 24, header);
    fill_rect(&mut before, 0, 24, 80, h - 24, sidebar);
    draw_cards(&mut before, 96, 40, 368);

    // AFTER: 48px header (Y shift +24) AND 144px sidebar (X shift +64).
    // Plus a real content change: an accent badge added to the middle card.
    let mut after = RgbaImage::from_pixel(w, h, Rgba(bg));
    fill_rect(&mut after, 0, 0, w, 48, header);
    fill_rect(&mut after, 0, 48, 144, h - 48, sidebar);
    draw_cards(&mut after, 160, 64, 304);
    // genuine diff: red badge on the middle card
    fill_rect(&mut after, 160 + 304 - 56, 64 + 80 + 12, 40, 16, accent);

    std::fs::create_dir_all("assets")?;
    before.save("assets/before.png")?;
    after.save("assets/after.png")?;
    println!("wrote assets/before.png");
    println!("wrote assets/after.png");
    Ok(())
}
