//! 2D block-matching image diff.
//!
//! Splits the reference image into fixed-size blocks and, for each block,
//! searches a window in the target image for the best (lowest SAD) match.
//! This lets the diff tolerate content shifts in both X and Y directions
//! (e.g. a newly inserted sidebar, header, or paragraph) so that only the
//! genuinely changed regions get flagged instead of every pixel below the
//! shifted content.
//!
//! Block-matching is inherently directional: `diff(a, b)` flags blocks in `a`
//! that have no good match in `b` — i.e. things that disappeared from `a`.
//! To see additions as well, use [`diff_bidirectional`] which runs both
//! directions and returns both fields together.

pub mod block_match;
pub mod render;

pub use block_match::{
    diff, diff_bidirectional, BidirectionalDiff, BlockMatchOptions, BlockMatchResult,
    MotionVector, SearchMode,
};
pub use render::{render_bidirectional, render_diff, RenderOptions};
