//! `--form-fields` (roadmap 6.8): turn the ruled boxes, check boxes and underlines of a scanned
//! paper form into fillable PDF fields (AcroForm text and check-box widgets) laid over the scan.

use legatorocr_core::forms::{self, Kind};
use legatorocr_core::pipeline::{Page, PageImage};
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Fields of the page as `(is_checkbox, [x0, y0, x1, y1])` in PDF points.
pub fn for_page(pg: &Page) -> Vec<(bool, [f32; 4])> {
    let bw = match &pg.image {
        PageImage::Bw(b) => b.clone(),
        PageImage::Gray(g) => legatorocr_core::binarize::sauvola(g, 41, 0.3),
        PageImage::Rgb(i) => {
            legatorocr_core::binarize::sauvola(&legatorocr_core::transform::gray_of(i), 41, 0.3)
        }
    };
    let dpi = pg.dpi.max(1) as f32;
    let hpt = bw.h as f32 * 72.0 / dpi;
    forms::detect(&bw, pg.dpi.max(1))
        .into_iter()
        .map(|f| {
            let x0 = f.x as f32 * 72.0 / dpi;
            let x1 = (f.x + f.w) as f32 * 72.0 / dpi;
            let y1 = hpt - f.y as f32 * 72.0 / dpi;
            let y0 = hpt - (f.y + f.h) as f32 * 72.0 / dpi;
            (f.kind == Kind::Check, [x0, y0, x1, y1])
        })
        .collect()
}
