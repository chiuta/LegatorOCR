//! `--quality-report`: advice gathered while a batch is processed, printed at the end.
//!
//! Measures each source scan (sharpness, perceptual hash, text cut at the border, resolution
//! if the page were A4) with `legatorocr_core::quality`. Every threshold is a heuristic that
//! has not been tuned on real scans; the report says "may", not "is".

use legatorocr_core::quality as q;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    sharp: f32,
    hash: u64,
    thumb: Vec<u8>,
    cut: bool,
    dpi: f32,
    /// The paper is clearly tinted (cream, yellow): red well above blue (15 or more).
    tinted: bool,
}

static DATA: Mutex<BTreeMap<usize, Entry>> = Mutex::new(BTreeMap::new());

/// Turn collection on or off (and clear what was collected).
pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if let Ok(mut d) = DATA.lock() {
        d.clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Measure one source image; `index` is the sheet's position in the batch.
pub fn record(index: usize, path: &Path, img: &image::DynamicImage) {
    if !enabled() {
        return;
    }
    let g = img.to_luma8();
    let (w, h) = g.dimensions();
    let a4 = if w >= h {
        (297.0, 210.0)
    } else {
        (210.0, 297.0)
    };
    // a colour photo of a page: its paper tone, from the bright majority of the picture
    let tinted = if matches!(
        img,
        image::DynamicImage::ImageRgb8(_)
            | image::DynamicImage::ImageRgba8(_)
            | image::DynamicImage::ImageRgb16(_)
    ) {
        let p = legatorocr_core::transform::estimate_background(&img.to_rgb8()).0;
        p[0] as i32 - p[2] as i32 >= 15 && p[1] >= 150
    } else {
        false
    };
    let e = Entry {
        path: path.to_path_buf(),
        tinted,
        sharp: q::sharpness(&g),
        hash: q::perceptual_hash(&g),
        thumb: q::thumbnail(&g),
        cut: q::edge_text_cut(&g),
        // the resolution the file declares (a scanned PDF, a JPEG header) wins over the guess that
        // the page is A4: a newspaper sheet or a poster is not
        dpi: crate::files::detect_dpi(path)
            .map(|d| d as f32)
            .unwrap_or_else(|| q::effective_dpi(w, h, a4)),
    };
    if let Ok(mut d) = DATA.lock() {
        d.insert(index, e);
    }
}

/// Human-readable alerts, one per line; empty when nothing stands out.
pub fn report() -> Vec<String> {
    let d = match DATA.lock() {
        Ok(d) => d.clone(),
        Err(_) => return Vec::new(),
    };
    let rows: Vec<&Entry> = d.values().collect();
    let name = |e: &Entry| {
        e.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    let sharp: Vec<f32> = rows.iter().map(|e| e.sharp).collect();
    for i in q::blurred_pages(&sharp) {
        out.push(format!("{}: may be out of focus or blurred", name(rows[i])));
    }
    for e in &rows {
        if e.cut {
            out.push(format!("{}: text may be cut off at the page edge", name(e)));
        }
    }
    let hashes: Vec<u64> = rows.iter().map(|e| e.hash).collect();
    for (a, b) in q::near_duplicates(&hashes, 6) {
        // the hash is only a pre-filter: text pages share margins and columns, so confirm
        if q::thumb_correlation(&rows[a].thumb, &rows[b].thumb) < 0.9 {
            continue;
        }
        out.push(format!(
            "{} and {} look like duplicates",
            name(rows[a]),
            name(rows[b])
        ));
    }
    // One resolution note per kind, not per page.
    let mut low = 0usize;
    let mut high = 0usize;
    for e in &rows {
        match q::dpi_advice(e.dpi) {
            q::DpiAdvice::TooLow => low += 1,
            q::DpiAdvice::ExcessForText => high += 1,
            q::DpiAdvice::Good => {}
        }
    }
    if low > 0 {
        out.push(format!(
            "{low} page(s) have under about 200 dpi if the page is A4: OCR may misread characters; rescan at 300 dpi or more"
        ));
    }
    if high > 0 {
        out.push(format!(
            "{high} page(s) have over about 400 dpi if the page is A4: plain text gains nothing, files and processing time grow"
        ));
    }
    let tinted = rows.iter().filter(|e| e.tinted).count();
    if tinted > 0 {
        out.push(format!(
            "{tinted} page(s) have tinted paper (cream or yellow): colour output keeps it; \
             --remove-background makes the paper white and evens the lighting out"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, Luma};

    fn text_like(w: u32, h: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| {
            if (60..h - 60).contains(&y)
                && (80..w - 80).contains(&x)
                && (y / 14) % 2 == 0
                && (x / 5) % 3 != 0
            {
                Luma([20])
            } else {
                Luma([240])
            }
        })
    }

    // One test: the collector is global state.
    #[test]
    fn flags_duplicates_low_dpi_and_stays_silent_when_off() {
        enable(false);
        record(
            0,
            Path::new("a.png"),
            &DynamicImage::ImageLuma8(text_like(300, 400)),
        );
        assert!(report().is_empty());
        enable(true);
        record(
            0,
            Path::new("a.png"),
            &DynamicImage::ImageLuma8(text_like(300, 400)),
        );
        record(
            1,
            Path::new("b.png"),
            &DynamicImage::ImageLuma8(text_like(300, 400)),
        );
        let r = report();
        assert!(
            r.iter()
                .any(|l| l.contains("a.png and b.png look like duplicates")),
            "{r:?}"
        );
        // 300 px across 210 mm is about 36 dpi
        assert!(r.iter().any(|l| l.contains("under about 200 dpi")), "{r:?}");
        // a cream page is reported, a neutral one is not
        assert!(!r.iter().any(|l| l.contains("tinted paper")), "{r:?}");
        record(
            2,
            Path::new("c.png"),
            &DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                300,
                400,
                image::Rgb([240, 226, 208]),
            )),
        );
        let r = report();
        assert!(
            r.iter().any(|l| l.contains("1 page(s) have tinted paper")),
            "{r:?}"
        );
        enable(false);
        assert!(report().is_empty());
    }
}
