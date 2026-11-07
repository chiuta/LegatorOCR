#![allow(clippy::needless_range_loop)] // explicit index loops read clearer in pixel code
//! LegatorOCR core — a clean-room scanned-page post-processing pipeline.
//!
//! Stages (same *workflow* as classic scan post-processors, independent code):
//! orientation → page split → deskew → content selection → margins → output
//! (colour / grayscale / black-and-white).

pub mod barcode;
pub mod binarize;
pub mod bitimage;
pub mod cleanup;
pub mod content;
pub mod deskew;
pub mod dewarp;
pub mod forms;
pub mod fusion;
pub mod layers;
pub mod orient;
pub mod perspective;
pub mod pictures;
pub mod pipeline;
pub mod quality;
pub mod sfr;
pub mod split;
pub mod text;
pub mod transform;

pub use binarize::Threshold;
pub use bitimage::BitImage;
pub use cleanup::Dropout;
pub use pipeline::{
    detect_sheet_turn, measure_page, process_page, HAlign, OutputMode, Page, PageImage, Settings,
    SplitMode, VAlign,
};
pub use text::Word;

/// Axis-aligned integer rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Optional per-stage timing hook (set by the engine for `--timings`).
pub static STAGE_HOOK: std::sync::OnceLock<fn(&'static str, std::time::Duration)> =
    std::sync::OnceLock::new();

/// Run `f`, reporting its wall time to the hook when one is installed.
#[inline]
pub fn stage<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    match STAGE_HOOK.get() {
        None => f(),
        Some(h) => {
            let t = std::time::Instant::now();
            let r = f();
            h(name, t.elapsed());
            r
        }
    }
}
