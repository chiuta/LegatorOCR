#![allow(clippy::needless_range_loop)] // explicit index loops read clearer in pixel code
//! CROITOR core — a clean-room scanned-page post-processing pipeline.
//!
//! Stages (same *workflow* as classic scan post-processors, independent code):
//! orientation → page split → deskew → content selection → margins → output
//! (colour / grayscale / black-and-white).

pub mod binarize;
pub mod bitimage;
pub mod content;
pub mod deskew;
pub mod orient;
pub mod pipeline;
pub mod split;
pub mod text;
pub mod transform;

pub use bitimage::BitImage;
pub use pipeline::{process_page, OutputMode, Page, PageImage, Settings, SplitMode};
pub use text::Word;

/// Axis-aligned integer rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}
