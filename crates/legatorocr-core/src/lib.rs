#![allow(clippy::needless_range_loop)] // explicit index loops read clearer in pixel code
//! LegatorOCR core — an independently implemented scanned-page post-processing pipeline.
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

#[cfg(test)]
mod gutter_tests;

pub use binarize::Threshold;
pub use bitimage::BitImage;
pub use cleanup::Dropout;
pub use pipeline::{
    detect_page_keystone, detect_sheet_turn, measure_page, process_page, HAlign, OutputMode, Page,
    PageImage, Settings, SplitMode, VAlign,
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

/// Named paper sizes in millimetres (width, height; portrait), shared by the window and the
/// command line. ISO 216 A and B series, plus the North American sizes.
pub const PAPER_SIZES: &[(&str, f32, f32)] = &[
    ("A2", 420.0, 594.0),
    ("A3", 297.0, 420.0),
    ("A4", 210.0, 297.0),
    ("A5", 148.0, 210.0),
    ("A6", 105.0, 148.0),
    ("B3", 353.0, 500.0),
    ("B4", 250.0, 353.0),
    ("B5", 176.0, 250.0),
    ("B6", 125.0, 176.0),
    ("Letter", 215.9, 279.4),
    ("Legal", 215.9, 355.6),
    ("Tabloid", 279.4, 431.8),
];

/// Size in millimetres of a paper name from [`PAPER_SIZES`] (case does not matter).
pub fn paper_size_mm(name: &str) -> Option<(f32, f32)> {
    PAPER_SIZES
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name.trim()))
        .map(|&(_, w, h)| (w, h))
}

#[cfg(test)]
mod paper_tests {
    use super::*;

    #[test]
    fn paper_names_resolve() {
        assert_eq!(paper_size_mm("a3"), Some((297.0, 420.0)));
        assert_eq!(paper_size_mm(" B5 "), Some((176.0, 250.0)));
        assert_eq!(paper_size_mm("nope"), None);
    }

    #[test]
    fn a_series_halves() {
        // every A size folded in half gives the next one (its height is the next one's width)
        for w in PAPER_SIZES
            .windows(2)
            .filter(|w| w[0].0.starts_with('A') && w[1].0.starts_with('A'))
        {
            assert!((w[0].2 / 2.0 - w[1].1).abs() < 1.5, "{} {}", w[0].0, w[1].0);
        }
    }
}
