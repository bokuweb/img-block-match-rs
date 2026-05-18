use clap::{Parser, ValueEnum};
use img_block_match::{
    diff, diff_bidirectional, draw_regions, render_bidirectional, render_diff, BlockMatchOptions,
    Region, RenderOptions, SearchMode,
};
use std::time::Instant;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum CliMode {
    /// Exhaustive search — globally optimal, slow for large search radii.
    Full,
    /// Hierarchical (coarse scan + logarithmic refinement) — orders of
    /// magnitude faster for wide search ranges.
    Fast,
}

impl From<CliMode> for SearchMode {
    fn from(m: CliMode) -> Self {
        match m {
            CliMode::Full => SearchMode::Full,
            CliMode::Fast => SearchMode::Hierarchical,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "img-block-match",
    about = "Block-matching image diff that tolerates X/Y content shifts."
)]
struct Args {
    /// Reference image ("expected").
    reference: String,
    /// Target image ("actual").
    target: String,
    /// Output diff image path (PNG).
    #[arg(short, long, default_value = "diff.png")]
    output: String,
    #[arg(long, default_value_t = 16)]
    block_size: u32,
    #[arg(long, default_value_t = 32)]
    search_x: i32,
    #[arg(long, default_value_t = 64)]
    search_y: i32,
    #[arg(long, default_value_t = 1)]
    step: u32,
    /// Per-channel per-pixel SAD threshold (0..255). Smaller = stricter.
    #[arg(long, default_value_t = 8)]
    threshold: u32,
    /// Draw motion vectors on top of the diff image.
    #[arg(long, default_value_t = false)]
    draw_vectors: bool,
    /// Render a single-direction overlay on the reference image only.
    /// Default is a bidirectional side-by-side composite.
    #[arg(long, default_value_t = false)]
    one_way: bool,
    /// Search strategy within the window.
    #[arg(long, value_enum, default_value_t = CliMode::Full)]
    mode: CliMode,
    /// Draw bounding boxes around clustered unmatched regions on top of the
    /// diff image.
    #[arg(long, default_value_t = false)]
    regions: bool,
    /// When merging adjacent unmatched blocks into regions, allow this many
    /// matched blocks of gap between them.
    #[arg(long, default_value_t = 1)]
    merge_gap: u32,
    /// Discard regions smaller than this many unmatched blocks (suppresses
    /// stray anti-aliasing flips).
    #[arg(long, default_value_t = 1)]
    min_blocks: u32,
    /// Emit unmatched-region bounding boxes as JSON on stdout (in addition
    /// to writing the rendered image).
    #[arg(long, default_value_t = false)]
    json: bool,
}

fn regions_to_json(label: &str, regions: &[Region]) -> String {
    let mut s = format!("    \"{}\": [", label);
    for (i, r) in regions.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "\n      {{\"x\": {}, \"y\": {}, \"width\": {}, \"height\": {}, \"blocks\": {}}}",
            r.x, r.y, r.width, r.height, r.block_count
        ));
    }
    if !regions.is_empty() {
        s.push_str("\n    ");
    }
    s.push(']');
    s
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let reference = image::open(&args.reference)?.to_rgba8();
    let target = image::open(&args.target)?.to_rgba8();

    let opts = BlockMatchOptions {
        block_size: args.block_size,
        search_x: args.search_x,
        search_y: args.search_y,
        step: args.step,
        threshold: args.threshold,
        mode: args.mode.into(),
    };
    let render_opts = RenderOptions {
        draw_vectors: args.draw_vectors,
        ..Default::default()
    };

    let t = Instant::now();
    if args.one_way {
        let result = diff(&reference, &target, &opts);
        let elapsed = t.elapsed();
        eprintln!(
            "blocks: {} total, {} unmatched ({:.2}%) in {:?}",
            result.vectors.len(),
            result.unmatched(),
            100.0 * result.unmatched() as f64 / result.vectors.len().max(1) as f64,
            elapsed,
        );
        let regions = result.unmatched_regions(args.merge_gap, args.min_blocks);
        eprintln!("regions: {}", regions.len());
        let mut out = render_diff(&reference, &result, [220, 50, 50], &render_opts);
        if args.regions {
            draw_regions(&mut out, &regions, [200, 0, 0, 255]);
        }
        out.save(&args.output)?;
        if args.json {
            println!("{{\n{}\n}}", regions_to_json("changed", &regions));
        }
    } else {
        let bd = diff_bidirectional(&reference, &target, &opts);
        let elapsed = t.elapsed();
        let total = bd.forward.vectors.len();
        let removed = bd.forward.unmatched_regions(args.merge_gap, args.min_blocks);
        let added = bd.reverse.unmatched_regions(args.merge_gap, args.min_blocks);
        eprintln!(
            "forward (removed):  {} / {} blocks ({:.2}%), {} regions",
            bd.forward.unmatched(),
            total,
            100.0 * bd.forward.unmatched() as f64 / total.max(1) as f64,
            removed.len(),
        );
        eprintln!(
            "reverse (added):    {} / {} blocks ({:.2}%), {} regions",
            bd.reverse.unmatched(),
            total,
            100.0 * bd.reverse.unmatched() as f64 / total.max(1) as f64,
            added.len(),
        );
        eprintln!("elapsed: {:?}", elapsed);
        let mut out = render_bidirectional(&reference, &target, &bd, &render_opts);
        if args.regions {
            // The right panel is offset by left.width() + 4 (gap).
            let mut right_regions = added.clone();
            let x_off = reference.width() + 4;
            for r in &mut right_regions {
                r.x += x_off;
            }
            draw_regions(&mut out, &removed, [200, 0, 0, 255]);
            draw_regions(&mut out, &right_regions, [0, 160, 60, 255]);
        }
        out.save(&args.output)?;
        if args.json {
            println!(
                "{{\n{},\n{}\n}}",
                regions_to_json("removed", &removed),
                regions_to_json("added", &added),
            );
        }
    }
    eprintln!("wrote {}", args.output);
    Ok(())
}
