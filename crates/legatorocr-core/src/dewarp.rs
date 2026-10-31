//! Page-curl correction (placeholder until implemented below).
use image::{Rgb, RgbImage};

/// Straighten text lines that curve (book spine, bent sheet). Returns `None` when
/// the page has no measurable curvature.
pub fn dewarp(_img: &RgbImage, _dpi: u32, _fill: Rgb<u8>) -> Option<RgbImage> {
    None
}
