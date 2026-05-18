use clap::{Parser, ValueEnum};
use img_block_match::{
    diff, diff_bidirectional, render_bidirectional, render_diff, BlockMatchOptions, RenderOptions,
    SearchMode,
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
        let out = render_diff(&reference, &result, [220, 50, 50], &render_opts);
        out.save(&args.output)?;
    } else {
        let bd = diff_bidirectional(&reference, &target, &opts);
        let elapsed = t.elapsed();
        let total = bd.forward.vectors.len();
        eprintln!(
            "forward (removed):  {} / {} blocks ({:.2}%)",
            bd.forward.unmatched(),
            total,
            100.0 * bd.forward.unmatched() as f64 / total.max(1) as f64
        );
        eprintln!(
            "reverse (added):    {} / {} blocks ({:.2}%)",
            bd.reverse.unmatched(),
            total,
            100.0 * bd.reverse.unmatched() as f64 / total.max(1) as f64
        );
        eprintln!("elapsed: {:?}", elapsed);
        let out = render_bidirectional(&reference, &target, &bd, &render_opts);
        out.save(&args.output)?;
    }
    eprintln!("wrote {}", args.output);
    Ok(())
}
