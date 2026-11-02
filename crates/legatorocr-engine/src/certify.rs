//! `--certify`: read the finished PDF back and check each page against the page that was
//! meant to be written: page count, pixel size and a 96×128 thumbnail comparison. This catches
//! an encoder or reader fault (wrong size, swapped or dropped page, damaged image data), not
//! subtle loss inside the chosen compression; lossy JPEG pages are expected to differ a little.
//! Layered pages (mixed / hybrid output) are only checked for count and size.

use image::imageops::FilterType;
use legatorocr_core::Page;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);

struct Rec {
    w: u32,
    h: u32,
    thumb: Vec<u8>,
    layered: bool,
}

static RECS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());

pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if let Ok(mut r) = RECS.lock() {
        r.clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

fn thumb(img: &image::DynamicImage) -> Vec<u8> {
    img.resize_exact(96, 128, FilterType::Triangle).to_luma8().into_raw()
}

/// Note a page that is being added to the PDF (in PDF order).
pub fn record(pg: &Page) {
    if !enabled() {
        return;
    }
    let img = pg.image.to_dynamic();
    let r = Rec {
        w: img.width(),
        h: img.height(),
        thumb: thumb(&img),
        layered: pg.layers.is_some(),
    };
    if let Ok(mut v) = RECS.lock() {
        v.push(r);
    }
}

/// Outcome of the check: whether every page passed, and the lines to print.
pub fn check(pdf: &[u8]) -> (bool, Vec<String>) {
    let recs = RECS.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    verify(pdf, &recs)
}

fn verify(pdf: &[u8], recs: &[Rec]) -> (bool, Vec<String>) {
    let doc = match legatorocr_pdf::reader::PdfDoc::open(pdf.to_vec()) {
        Ok(d) => d,
        Err(e) => return (false, vec![format!("certificate: the PDF could not be read back ({e})")]),
    };
    let mut lines = Vec::new();
    let mut ok = true;
    if doc.page_count() != recs.len() {
        lines.push(format!(
            "page count: {} written, {} found when read back",
            recs.len(),
            doc.page_count()
        ));
        ok = false;
    }
    let mut worst = 0.0f64;
    for (i, r) in recs.iter().enumerate().take(doc.page_count()) {
        match doc.page_image(i) {
            Ok(p) => {
                let (w, h) = (p.image.width(), p.image.height());
                if (w, h) != (r.w, r.h) {
                    lines.push(format!("page {}: size {}×{} written, {w}×{h} read back", i + 1, r.w, r.h));
                    ok = false;
                    continue;
                }
                if r.layered {
                    continue;
                }
                let t = thumb(&p.image);
                let d = t.iter().zip(&r.thumb).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>()
                    / t.len() as f64;
                worst = worst.max(d);
                if d > 6.0 {
                    lines.push(format!("page {}: picture differs from the source page (mean difference {d:.1}/255)", i + 1));
                    ok = false;
                }
            }
            Err(e) => {
                lines.push(format!("page {}: could not be decoded ({e})", i + 1));
                ok = false;
            }
        }
    }
    lines.insert(
        0,
        format!(
            "certificate: {} page(s) read back; worst thumbnail difference {worst:.2}/255; {}",
            recs.len(),
            if ok { "all pages match" } else { "PROBLEMS FOUND" }
        ),
    );
    (ok, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use legatorocr_core::PageImage;
    use legatorocr_pdf::{Compress, PdfWriter};

    fn page(w: u32, h: u32, shade: u8) -> Page {
        let mut img = image::GrayImage::from_pixel(w, h, image::Luma([shade]));
        for x in 0..w / 2 {
            img.put_pixel(x, h / 2, image::Luma([0]));
        }
        Page {
            image: PageImage::Gray(img),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        }
    }

    fn rec_of(p: &Page) -> Rec {
        let img = p.image.to_dynamic();
        Rec { w: img.width(), h: img.height(), thumb: thumb(&img), layered: false }
    }

    fn pdf_of(pages: &[Page]) -> Vec<u8> {
        let mut w = PdfWriter::new(Compress::Flate);
        for p in pages {
            w.add_page(p, &[]);
        }
        w.finish()
    }

    #[test]
    fn matching_pages_pass_and_faults_are_caught() {
        let (a, b) = (page(100, 140, 240), page(100, 140, 120));
        let pdf = pdf_of(&[a.clone_page(), b.clone_page()]);
        let good = [rec_of(&a), rec_of(&b)];
        assert!(verify(&pdf, &good).0);
        // wrong count
        assert!(!verify(&pdf, &good[..1]).0);
        // swapped pages
        let swapped = [rec_of(&b), rec_of(&a)];
        let (ok, lines) = verify(&pdf, &swapped);
        assert!(!ok && lines.iter().any(|l| l.contains("differs")), "{lines:?}");
        // different size
        let other = [rec_of(&page(90, 140, 240)), rec_of(&b)];
        assert!(verify(&pdf, &other).1.iter().any(|l| l.contains("size")));
        // not a PDF
        assert!(!verify(b"garbage", &good).0);
    }

    trait ClonePage {
        fn clone_page(&self) -> Page;
    }
    impl ClonePage for Page {
        fn clone_page(&self) -> Page {
            Page {
                image: self.image.clone(),
                dpi: self.dpi,
                skew_deg: self.skew_deg,
                blank: self.blank,
                split_index: self.split_index,
                content_box: self.content_box,
                layers: None,
            }
        }
    }
}
