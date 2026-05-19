//! Sketch of how a screenshot-regression tool (e.g. reg-cli) might wire
//! block-matching in front of its pixel-level differ.
//!
//! Usage:
//!   cargo run --release --example regression_check -- before.png after.png
//!
//! Decision tree:
//!   1. Run bidirectional block-matching with a reasonable shift tolerance.
//!   2. If both directions have zero unmatched regions → PASS. The content
//!      may have moved but nothing actually changed.
//!   3. Otherwise → "needs review": emit the bounding boxes of the
//!      unmatched regions (as JSON) so the downstream tool can either
//!      escalate to a per-pixel diff inside those rectangles or render a
//!      side-by-side report.
//!
//! This is much cheaper than a full per-pixel diff and side-steps the
//! "everything shifted by N pixels" failure mode of pixel diffs.

use img_block_match::{diff_bidirectional, BlockMatchOptions, Region, SearchMode};

#[derive(Debug)]
enum Verdict {
    /// Block-match says nothing meaningful changed.
    Pass,
    /// Real differences found; only these rectangles need further review.
    NeedsReview { removed: Vec<Region>, added: Vec<Region> },
}

fn check(reference: &image::RgbaImage, target: &image::RgbaImage) -> Verdict {
    let opts = BlockMatchOptions {
        block_size: 8,
        search_x: 64,
        search_y: 128,
        threshold: 8,
        mode: SearchMode::Hierarchical,
        ..Default::default()
    };
    let bd = diff_bidirectional(reference, target, &opts);
    // merge_gap=2 (≈ block padding around AA edges); min_blocks=3 drops
    // 1-2-block specks that aren't worth a reviewer's attention. Tune for
    // your tolerance — smaller min_blocks catches subtler changes at the
    // cost of more reviewer noise.
    let removed = bd.forward.unmatched_regions(2, 3);
    let added = bd.reverse.unmatched_regions(2, 3);
    if removed.is_empty() && added.is_empty() {
        Verdict::Pass
    } else {
        Verdict::NeedsReview { removed, added }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let a_path = args.next().ok_or("usage: regression_check before.png after.png")?;
    let b_path = args.next().ok_or("usage: regression_check before.png after.png")?;
    let a = image::open(&a_path)?.to_rgba8();
    let b = image::open(&b_path)?.to_rgba8();

    let t = std::time::Instant::now();
    let verdict = check(&a, &b);
    let elapsed = t.elapsed();

    match verdict {
        Verdict::Pass => {
            println!("PASS  ({} -> {}, decided in {:?})", a_path, b_path, elapsed);
            std::process::exit(0);
        }
        Verdict::NeedsReview { removed, added } => {
            println!(
                "REVIEW  ({} -> {}, decided in {:?})\n  removed regions: {}\n  added regions:   {}",
                a_path,
                b_path,
                elapsed,
                removed.len(),
                added.len()
            );
            for r in &removed {
                println!("    - removed {}x{} at ({}, {})", r.width, r.height, r.x, r.y);
            }
            for r in &added {
                println!("    + added   {}x{} at ({}, {})", r.width, r.height, r.x, r.y);
            }
            std::process::exit(1);
        }
    }
}
