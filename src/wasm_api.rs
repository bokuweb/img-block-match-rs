//! WebAssembly bindings via `wasm-bindgen`.
//!
//! Build with:
//!   wasm-pack build --target web --no-default-features --features wasm
//!   wasm-pack build --target nodejs --no-default-features --features wasm
//!
//! The exposed JS surface is intentionally minimal:
//!   - `diff_png(ref, tgt, opts?) -> { width, height, removed: Region[], added: Region[] }`
//!
//! Both inputs are PNG byte arrays (Uint8Array). Returned regions carry
//! pixel-space bounding boxes ready to drive overlays or downstream
//! pixel-level diffs.
//!
//! `opts` is an optional JS object whose fields override the defaults; any
//! unrecognized field is ignored.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::block_match::{
    diff_bidirectional, BlockMatchOptions, Region, SearchMode,
};

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

#[derive(Debug, Serialize)]
struct JsRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    blocks: u32,
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
struct JsResult {
    width: u32,
    height: u32,
    block_size: u32,
    removed: Vec<JsRegion>,
    added: Vec<JsRegion>,
    /// "pass" if both removed and added are empty; otherwise "review".
    verdict: &'static str,
}

/// Bidirectional block-matching diff over two PNG-encoded images. Returns a
/// JSON-serializable object: `{ width, height, blockSize, removed, added,
/// verdict }`.
#[wasm_bindgen(js_name = diffPng)]
pub fn diff_png(reference: &[u8], target: &[u8], opts: JsValue) -> Result<JsValue, JsError> {
    let opts: JsOptions = if opts.is_undefined() || opts.is_null() {
        JsOptions::default()
    } else {
        serde_wasm_bindgen::from_value(opts).map_err(|e| JsError::new(&e.to_string()))?
    };

    let a = image::load_from_memory(reference)
        .map_err(|e| JsError::new(&format!("decode reference: {e}")))?
        .to_rgba8();
    let b = image::load_from_memory(target)
        .map_err(|e| JsError::new(&format!("decode target: {e}")))?
        .to_rgba8();

    let mode = match opts.mode.as_str() {
        "full" => SearchMode::Full,
        _ => SearchMode::Hierarchical,
    };
    let mopts = BlockMatchOptions {
        block_size: opts.block_size,
        search_x: opts.search_x,
        search_y: opts.search_y,
        step: 1,
        threshold: opts.threshold,
        mode,
        compute_confidence: false,
    };

    let mut bd = diff_bidirectional(&a, &b, &mopts);
    if opts.smooth {
        bd.forward.smooth_matched();
        bd.reverse.smooth_matched();
    }
    let removed = bd.forward.unmatched_regions(opts.merge_gap, opts.min_blocks);
    let added = bd.reverse.unmatched_regions(opts.merge_gap, opts.min_blocks);
    let verdict = if removed.is_empty() && added.is_empty() {
        "pass"
    } else {
        "review"
    };

    let out = JsResult {
        width: bd.forward.width,
        height: bd.forward.height,
        block_size: bd.forward.block_size,
        removed: removed.iter().map(JsRegion::from).collect(),
        added: added.iter().map(JsRegion::from).collect(),
        verdict,
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&e.to_string()))
}
