//! WebAssembly bindings via `wasm-bindgen`.
//!
//! Build with:
//!   wasm-pack build --target web --no-default-features --features wasm
//!   wasm-pack build --target nodejs --no-default-features --features wasm
//!
//! Exposed JS entry points:
//!   - `diffPng(refBytes, tgtBytes, opts?)`    — decodes PNG/JPEG internally.
//!   - `diffRgba(refData, refW, refH, tgtData, tgtW, tgtH, opts?)` — accepts
//!     raw RGBA8 buffers (e.g. an `ImageData.data`). Useful from browser
//!     workers where the caller already has pixel data and does not want
//!     the WASM-side PNG roundtrip.
//!
//! Returned shape (camelCase):
//! ```ts
//! {
//!   width, height, blockSize,
//!   verdict: 'pass' | 'review',
//!   removed: Region[],         // bbox of content present in ref, gone in tgt
//!   added:   Region[],         // bbox of content present in tgt, gone in ref
//!   // x-img-diff-js compatibility surface:
//!   images: [{ width, height }, { width, height }],
//!   matches: DetectMatch[][],  // currently always [] (see README)
//!   strayingRects: [Rect[], Rect[]],  // mirrors [removed, added]
//! }
//! ```
//!
//! `opts` is an optional JS object whose fields override the defaults; any
//! unrecognized field is ignored.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::block_match::{
    diff_bidirectional, BidirectionalDiff, BlockMatchOptions, Region, SearchMode,
};
use image::RgbaImage;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct JsOptions {
    block_size: u32,
    search_x: i32,
    search_y: i32,
    threshold: u32,
    mode: String,
    merge_gap: u32,
    min_blocks: u32,
    smooth: bool,
}

impl Default for JsOptions {
    fn default() -> Self {
        Self {
            block_size: 8,
            search_x: 16,
            search_y: 64,
            threshold: 8,
            mode: "fast".to_string(),
            merge_gap: 2,
            min_blocks: 2,
            smooth: false,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
struct JsRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
struct JsRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    blocks: u32,
}

impl From<&Region> for JsRect {
    fn from(r: &Region) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

impl From<&Region> for JsRegion {
    fn from(r: &Region) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
            blocks: r.block_count,
        }
    }
}

#[derive(Debug, Serialize)]
struct JsSize {
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsResult {
    width: u32,
    height: u32,
    block_size: u32,
    verdict: &'static str,
    removed: Vec<JsRegion>,
    added: Vec<JsRegion>,
    // x-img-diff-js drop-in compatibility surface.
    images: [JsSize; 2],
    matches: Vec<()>, // always [] for now — see README "matches" note
    straying_rects: [Vec<JsRect>; 2],
}

fn parse_opts(opts: JsValue) -> Result<JsOptions, JsError> {
    if opts.is_undefined() || opts.is_null() {
        Ok(JsOptions::default())
    } else {
        serde_wasm_bindgen::from_value(opts).map_err(|e| JsError::new(&e.to_string()))
    }
}

fn run_diff(a: &RgbaImage, b: &RgbaImage, js_opts: &JsOptions) -> BidirectionalDiff {
    let mode = match js_opts.mode.as_str() {
        "full" => SearchMode::Full,
        _ => SearchMode::Hierarchical,
    };
    let mopts = BlockMatchOptions {
        block_size: js_opts.block_size,
        search_x: js_opts.search_x,
        search_y: js_opts.search_y,
        step: 1,
        threshold: js_opts.threshold,
        mode,
        compute_confidence: false,
    };
    let mut bd = diff_bidirectional(a, b, &mopts);
    if js_opts.smooth {
        bd.forward.smooth_matched();
        bd.reverse.smooth_matched();
    }
    bd
}

fn build_result(a: &RgbaImage, b: &RgbaImage, opts: &JsOptions) -> JsResult {
    let bd = run_diff(a, b, opts);
    let removed = bd.forward.unmatched_regions(opts.merge_gap, opts.min_blocks);
    let added = bd.reverse.unmatched_regions(opts.merge_gap, opts.min_blocks);
    let verdict = if removed.is_empty() && added.is_empty() {
        "pass"
    } else {
        "review"
    };
    JsResult {
        width: bd.forward.width,
        height: bd.forward.height,
        block_size: bd.forward.block_size,
        verdict,
        removed: removed.iter().map(JsRegion::from).collect(),
        added: added.iter().map(JsRegion::from).collect(),
        images: [
            JsSize { width: a.width(), height: a.height() },
            JsSize { width: b.width(), height: b.height() },
        ],
        matches: Vec::new(),
        straying_rects: [
            removed.iter().map(JsRect::from).collect(),
            added.iter().map(JsRect::from).collect(),
        ],
    }
}

fn rgba_from_raw(data: &[u8], w: u32, h: u32, label: &str) -> Result<RgbaImage, JsError> {
    let expected = (w as usize) * (h as usize) * 4;
    if data.len() != expected {
        return Err(JsError::new(&format!(
            "{label}: buffer length {} ≠ expected {} ({w}×{h}×4)",
            data.len(),
            expected
        )));
    }
    RgbaImage::from_raw(w, h, data.to_vec())
        .ok_or_else(|| JsError::new(&format!("{label}: failed to wrap RGBA buffer")))
}

/// Bidirectional block-matching diff over two PNG/JPEG-encoded images.
#[wasm_bindgen(js_name = diffPng)]
pub fn diff_png(reference: &[u8], target: &[u8], opts: JsValue) -> Result<JsValue, JsError> {
    let opts = parse_opts(opts)?;
    let a = image::load_from_memory(reference)
        .map_err(|e| JsError::new(&format!("decode reference: {e}")))?
        .to_rgba8();
    let b = image::load_from_memory(target)
        .map_err(|e| JsError::new(&format!("decode target: {e}")))?
        .to_rgba8();
    let out = build_result(&a, &b, &opts);
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}

/// Bidirectional block-matching diff over two raw RGBA8 buffers (e.g. the
/// `data` of an `ImageData` obtained from a `<canvas>`). Avoids the PNG
/// encode/decode roundtrip the browser worker would otherwise pay.
#[wasm_bindgen(js_name = diffRgba)]
pub fn diff_rgba(
    ref_data: &[u8],
    ref_w: u32,
    ref_h: u32,
    tgt_data: &[u8],
    tgt_w: u32,
    tgt_h: u32,
    opts: JsValue,
) -> Result<JsValue, JsError> {
    let opts = parse_opts(opts)?;
    let a = rgba_from_raw(ref_data, ref_w, ref_h, "reference")?;
    let b = rgba_from_raw(tgt_data, tgt_w, tgt_h, "target")?;
    let out = build_result(&a, &b, &opts);
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}
