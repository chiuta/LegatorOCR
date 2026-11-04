//! `--certify`: read the finished PDF back and check each page against the page that was
//! meant to be written: page count, pixel size and a 96×128 thumbnail comparison. This catches
//! an encoder or reader fault (wrong size, swapped or dropped page, damaged image data), not
//! subtle loss inside the chosen compression; lossy JPEG pages are expected to differ a little.
//! Layered pages (mixed / hybrid output) are compared as the flattened picture of their layers.

use image::imageops::FilterType;
use legatorocr_core::Page;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);

struct Rec {
    w: u32,
    h: u32,
    thumb: Vec<u8>,
    /// Boxes (x, y, w, h) of the words written to the text layer, in page pixels.
    words: Vec<[u32; 4]>,
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
pub fn record(pg: &Page, words: &[legatorocr_core::Word]) {
    if !enabled() {
        return;
    }
    // a layered page (mixed / hybrid) is compared as the flattened picture of its layers
    let img = match &pg.layers {
        Some(l) => image::DynamicImage::ImageRgb8(l.compose()),
        None => pg.image.to_dynamic(),
    };
    let r = Rec {
        w: img.width(),
        h: img.height(),
        thumb: thumb(&img),
        words: words.iter().map(|w| [w.x, w.y, w.w, w.h]).collect(),
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
                let c = check_page(&p.image, doc.page_text(i).ok().as_deref(), r);
                for l in &c.problems {
                    lines.push(format!("page {}: {l}", i + 1));
                }
                ok &= c.problems.is_empty();
                worst = worst.max(c.thumb_diff);
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


/// What the read-back of one page showed (see [`check_page`]).
#[derive(Debug, Clone, Default)]
pub struct PageCheck {
    /// Words found in the text layer divided by words written (1.0 when none were written).
    pub text_frac: f64,
    /// Text-layer boxes lying on blank paper / on ink.
    pub empty: usize,
    pub found: usize,
    /// Mean absolute difference of the 96x128 thumbnails, 0..255.
    pub thumb_diff: f64,
    /// Human-readable problems; empty when the page passes.
    pub problems: Vec<String>,
}

fn check_page(back: &image::DynamicImage, text: Option<&str>, r: &Rec) -> PageCheck {
    let mut c = PageCheck { text_frac: 1.0, ..Default::default() };
    if !r.words.is_empty() {
        let (empty, found) = words_on_ink(&back.to_luma8(), &r.words);
        let text_words = text.map_or(0, |t| t.split_whitespace().count());
        c.text_frac = (text_words as f64 / r.words.len() as f64).min(1.0);
        c.empty = empty;
        c.found = found;
        if text_words * 10 < r.words.len() * 9 {
            c.problems.push(format!("text layer holds {text_words} word(s), {} were written", r.words.len()));
        }
        if empty >= 3 && empty * 20 > found + empty {
            c.problems.push(format!(
                "{empty} of {} text-layer boxes lie on blank paper (the text layer does not match the picture)",
                found + empty
            ));
        }
    }
    let t = thumb(back);
    c.thumb_diff =
        t.iter().zip(&r.thumb).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / t.len() as f64;
    if c.thumb_diff > 6.0 {
        c.problems.push(format!(
            "picture differs from the source page (mean difference {:.1}/255)",
            c.thumb_diff
        ));
    }
    c
}

/// Write one page with its words to an in-memory PDF, read it back and run the same checks
/// `--certify` runs on the whole file. `None` when the PDF cannot be read back at all.
pub fn certify_page(pg: &Page, words: &[legatorocr_core::Word]) -> Option<PageCheck> {
    let img = match &pg.layers {
        Some(l) => image::DynamicImage::ImageRgb8(l.compose()),
        None => pg.image.to_dynamic(),
    };
    let rec = Rec {
        w: img.width(),
        h: img.height(),
        thumb: thumb(&img),
        words: words.iter().map(|w| [w.x, w.y, w.w, w.h]).collect(),
    };
    let mut wr = legatorocr_pdf::PdfWriter::new(legatorocr_pdf::Compress::Flate);
    wr.add_page(pg, words);
    let doc = legatorocr_pdf::reader::PdfDoc::open(wr.finish()).ok()?;
    let p = doc.page_image(0).ok()?;
    if (p.image.width(), p.image.height()) != (rec.w, rec.h) {
        return None;
    }
    Some(check_page(&p.image, doc.page_text(0).ok().as_deref(), &rec))
}

/// Count text-layer boxes with no ink under them (`.0`) and with ink (`.1`). A box is blank when
/// its grey range is under 40 levels: recognised words always have dark strokes on light paper
/// (or the reverse), so a box on flat paper means the layer is shifted or scaled.
fn words_on_ink(img: &image::GrayImage, words: &[[u32; 4]]) -> (usize, usize) {
    let (mut empty, mut found) = (0, 0);
    for &[x, y, w, h] in words {
        let (x1, y1) = ((x + w).min(img.width()), (y + h).min(img.height()));
        if x >= x1 || y >= y1 {
            empty += 1;
            continue;
        }
        let (mut lo, mut hi) = (255u8, 0u8);
        for yy in y..y1 {
            for xx in x..x1 {
                let v = img.get_pixel(xx, yy).0[0];
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if hi.saturating_sub(lo) < 40 {
            empty += 1;
        } else {
            found += 1;
        }
    }
    (empty, found)
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
        Rec { w: img.width(), h: img.height(), thumb: thumb(&img), words: Vec::new() }
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

    /// A mixed-mode page: black ink mask over a coloured background of the given shade.
    fn layered(w: u32, h: u32, shade: u8) -> Page {
        let mut mask = legatorocr_core::BitImage::new(w, h);
        for x in 0..w / 2 {
            mask.data[((h / 2) * w + x) as usize] = 1;
        }
        let layers = legatorocr_core::layers::Layers {
            mask,
            fg: [0, 0, 0],
            bg: Some(legatorocr_core::layers::Background {
                img: image::RgbImage::from_pixel(w, h, image::Rgb([shade, shade, shade])),
                dpi: 150,
            }),
            patches: Vec::new(),
        };
        let mut p = page(w, h, shade);
        p.image = PageImage::Rgb(layers.compose());
        p.layers = Some(layers);
        p
    }

    #[test]
    fn layered_pages_are_compared_too() {
        let (a, b) = (layered(100, 140, 240), layered(100, 140, 100));
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(&a, &[]);
        w.add_page(&b, &[]);
        let pdf = w.finish();
        let rec = |p: &Page| {
            let img = image::DynamicImage::ImageRgb8(p.layers.as_ref().unwrap().compose());
            Rec { w: img.width(), h: img.height(), thumb: thumb(&img), words: Vec::new() }
        };
        assert!(verify(&pdf, &[rec(&a), rec(&b)]).0);
        let (ok, lines) = verify(&pdf, &[rec(&b), rec(&a)]);
        assert!(!ok && lines.iter().any(|l| l.contains("differs")), "{lines:?}");
    }

    #[test]
    fn boxes_on_blank_paper_are_counted() {
        let mut img = image::GrayImage::from_pixel(200, 100, image::Luma([255]));
        for x in 20..60 {
            for y in 20..40 {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
        // one box on the ink, one beside it, one outside the page
        let (empty, found) = words_on_ink(&img, &[[10, 15, 60, 30], [100, 20, 40, 20], [500, 5, 10, 10]]);
        assert_eq!((empty, found), (2, 1));
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
