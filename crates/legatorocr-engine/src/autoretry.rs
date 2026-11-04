//! `--auto-retry`: a page whose read-back looks doubtful is processed again with a short list of
//! alternatives, and the alternative with the best objective score is kept.
//!
//! The score deliberately does NOT use Tesseract's own confidence (ROADMAP 4.6: it predicts
//! character errors poorly). It uses what `--certify` reads back from a one-page PDF: the share
//! of text-layer words found, the text-layer boxes lying on blank paper (words read where there is
//! no ink) and the thumbnail comparison, plus one measure of ours: the share of the page's dark
//! pixels that fall inside a text-layer box (ink nobody read). None of this sees a misspelt letter,
//! so it can only separate alternatives that lose or invent whole words. ROADMAP 4.24 holds the
//! measurement.

use crate::certify::{certify_page, PageCheck};
use legatorocr_core::binarize::Threshold;
use legatorocr_core::pipeline::{process_page, OutputMode, Page, Settings};
use legatorocr_core::text::Word;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
/// Coverage under this value flags a page (stored as per-mille).
static BELOW: AtomicU32 = AtomicU32::new(800);
/// An alternative must beat the first reading by this much to replace it.
pub const MARGIN: f64 = 0.02;
static LOG: Mutex<Vec<(usize, usize, String)>> = Mutex::new(Vec::new());

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if let Ok(mut l) = LOG.lock() {
        l.clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Pages whose ink coverage is under `v` (0..1; above 1 retries every page) are retried.
pub fn set_below(v: f64) {
    BELOW.store((v * 1000.0).round().clamp(0.0, 100_000.0) as u32, Ordering::Relaxed);
}

fn below() -> f64 {
    BELOW.load(Ordering::Relaxed) as f64 / 1000.0
}

/// Objective measures of one reading of a page.
#[derive(Debug, Clone)]
pub struct Score {
    pub check: PageCheck,
    /// Share of dark pixels lying inside a text-layer box.
    pub coverage: f64,
}

impl Score {
    /// Higher is better. A failed certificate check costs one point.
    pub fn value(&self) -> f64 {
        let n = (self.check.empty + self.check.found).max(1) as f64;
        self.coverage - 2.0 * self.check.empty as f64 / n - (1.0 - self.check.text_frac)
            - if self.check.problems.is_empty() { 0.0 } else { 1.0 }
    }

    pub fn doubtful(&self, below: f64) -> bool {
        !self.check.problems.is_empty() || self.coverage < below
    }
}

fn otsu(hist: &[u32; 256], total: u64) -> u8 {
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut t) = (0f64, 0f64, -1f64, 0u8);
    for (i, &c) in hist.iter().enumerate() {
        wb += c as f64;
        if wb == 0.0 {
            continue;
        }
        let wf = total as f64 - wb;
        if wf == 0.0 {
            break;
        }
        sb += i as f64 * c as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let v = wb * wf * (mb - mf) * (mb - mf);
        if v > best {
            best = v;
            t = i as u8;
        }
    }
    t
}

/// Share of the page's dark pixels (at or below the Otsu level) that lie inside a word box.
/// A page with no dark pixels counts as fully covered.
pub fn ink_coverage(img: &image::GrayImage, words: &[Word]) -> f64 {
    let mut hist = [0u32; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let t = otsu(&hist, img.width() as u64 * img.height() as u64);
    let (w, h) = img.dimensions();
    let mut inside = vec![false; (w * h) as usize];
    for wd in words {
        for y in wd.y..(wd.y + wd.h).min(h) {
            for x in wd.x..(wd.x + wd.w).min(w) {
                inside[(y * w + x) as usize] = true;
            }
        }
    }
    let (mut ink, mut hit) = (0u64, 0u64);
    for (i, p) in img.pixels().enumerate() {
        if p.0[0] <= t {
            ink += 1;
            hit += inside[i] as u64;
        }
    }
    if ink == 0 {
        1.0
    } else {
        hit as f64 / ink as f64
    }
}

pub fn score(pg: &Page, words: &[Word]) -> Option<Score> {
    let check = certify_page(pg, words)?;
    let luma = match &pg.layers {
        Some(l) => image::DynamicImage::ImageRgb8(l.compose()).to_luma8(),
        None => pg.image.to_dynamic().to_luma8(),
    };
    Some(Score { check, coverage: ink_coverage(&luma, words) })
}

/// One thing to try: a name for the report, changed settings, and the dpi given to the OCR.
pub struct Alt {
    pub name: String,
    pub settings: Settings,
    pub ocr_dpi: Option<u32>,
}

/// The short list: other binarisers (black-and-white modes only), deskew flipped, OCR at a
/// different resolution. NICK, Niblack and Bradley are left out: ROADMAP 1.4 measured them far worse.
pub fn alternatives(s: &Settings, dpi: u32) -> Vec<Alt> {
    let mut v = Vec::new();
    if matches!(s.mode, OutputMode::BlackWhite | OutputMode::Mixed | OutputMode::Hybrid) {
        for (t, n) in [(Threshold::Otsu, "Otsu"), (Threshold::Wolf, "Wolf"), (Threshold::Sauvola, "Sauvola")] {
            if s.threshold != t {
                v.push(Alt { name: format!("binariser {n}"), settings: Settings { threshold: t, ..s.clone() }, ocr_dpi: None });
            }
        }
    }
    v.push(Alt {
        name: format!("deskew {}", if s.deskew { "off" } else { "on" }),
        settings: Settings { deskew: !s.deskew, manual_skew_deg: None, ..s.clone() },
        ocr_dpi: None,
    });
    let od = (dpi * 4 / 3).clamp(150, 600);
    if od != dpi {
        v.push(Alt { name: format!("OCR at {od} dpi"), settings: s.clone(), ocr_dpi: Some(od) });
    }
    v
}

/// Keep `(pg, words)` or the best alternative of sheet piece `seq`. `ocr(page, dpi)` reads a page.
/// The outcome is logged under `(index, seq)` for [`report`].
pub fn refine<E>(
    index: usize,
    seq: usize,
    pg: Page,
    words: Vec<Word>,
    src: &image::DynamicImage,
    settings: &Settings,
    ocr: impl Fn(&Page, u32) -> Result<Vec<Word>, E>,
) -> (Page, Vec<Word>) {
    if pg.blank || words.is_empty() {
        return (pg, words);
    }
    let Some(first) = score(&pg, &words) else {
        return (pg, words);
    };
    if !first.doubtful(below()) {
        return (pg, words);
    }
    let base = first.value();
    let (mut best_pg, mut best_words, mut best_val, mut best_name) = (pg, words, base, "first reading".to_string());
    let mut tried = vec![format!("first reading {base:.3} (ink covered {:.0}%)", 100.0 * first.coverage)];
    let dpi = best_pg.dpi;
    for alt in alternatives(settings, dpi) {
        let (cand, w2) = match alt.ocr_dpi {
            // same picture, other resolution hint: only the reading changes
            Some(d) => match ocr(&best_pg, d) {
                Ok(w) => (None, w),
                Err(_) => continue,
            },
            None => {
                let Some(p) = process_page(src, &alt.settings).into_iter().nth(seq) else { continue };
                match ocr(&p, p.dpi) {
                    Ok(w) => (Some(p), w),
                    Err(_) => continue,
                }
            }
        };
        let sc = match &cand {
            Some(p) => score(p, &w2),
            None => score(&best_pg, &w2),
        };
        let Some(sc) = sc else { continue };
        tried.push(format!("{} {:.3} (ink covered {:.0}%)", alt.name, sc.value(), 100.0 * sc.coverage));
        if sc.value() > best_val + MARGIN {
            if let Some(p) = cand {
                best_pg = p;
            }
            best_words = w2;
            best_val = sc.value();
            best_name = alt.name;
        }
    }
    if let Ok(mut l) = LOG.lock() {
        l.push((index, seq, format!("retried; kept: {best_name}; scores: {}", tried.join(", "))));
    }
    (best_pg, best_words)
}

/// Note for page `(index, seq)` when it was retried (used by `--explain`).
pub fn note(index: usize, seq: usize) -> Option<String> {
    LOG.lock().ok()?.iter().find(|(i, s, _)| (*i, *s) == (index, seq)).map(|(_, _, t)| t.clone())
}

/// A summary line and one line per retried page, in page order.
pub fn report() -> Vec<String> {
    let mut l = LOG.lock().map(|l| l.clone()).unwrap_or_default();
    l.sort_by_key(|(i, s, _)| (*i, *s));
    let changed = l.iter().filter(|(_, _, t)| !t.contains("kept: first reading")).count();
    let mut out = vec![format!("auto-retry: {} page(s) retried, {changed} changed", l.len())];
    for (i, s, t) in &l {
        let seq = if *s > 0 { format!(" [{}]", s + 1) } else { String::new() };
        out.push(format!("auto-retry: sheet {}{seq}: {t}", i + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(x: u32, y: u32, wd: u32, h: u32) -> Word {
        Word { text: "ab".into(), x, y, w: wd, h, conf: 90.0, line: 0 }
    }

    fn sheet() -> image::GrayImage {
        let mut g = image::GrayImage::from_pixel(100, 100, image::Luma([240]));
        for y in (10..20).chain(50..60) {
            for x in 10..60 {
                g.put_pixel(x, y, image::Luma([10]));
            }
        }
        g
    }

    #[test]
    fn coverage_counts_ink_inside_boxes() {
        let g = sheet();
        assert!((ink_coverage(&g, &[w(10, 10, 50, 10), w(10, 50, 50, 10)]) - 1.0).abs() < 1e-9);
        let half = ink_coverage(&g, &[w(10, 10, 50, 10)]);
        assert!((half - 0.5).abs() < 1e-9, "{half}");
        assert_eq!(ink_coverage(&image::GrayImage::from_pixel(10, 10, image::Luma([200])), &[]), 1.0);
    }

    #[test]
    fn score_prefers_covered_ink_and_punishes_boxes_on_paper() {
        let good = Score { check: PageCheck { text_frac: 1.0, found: 10, ..Default::default() }, coverage: 0.95 };
        let missing = Score { check: PageCheck { text_frac: 1.0, found: 10, ..Default::default() }, coverage: 0.5 };
        let invented = Score { check: PageCheck { text_frac: 1.0, found: 6, empty: 4, ..Default::default() }, coverage: 0.95 };
        assert!(good.value() > missing.value() && good.value() > invented.value());
        assert!(missing.doubtful(0.8) && !good.doubtful(0.8) && good.doubtful(1.01));
    }

    #[test]
    fn alternatives_depend_on_the_mode() {
        let mut s = Settings { mode: OutputMode::Color, ..Settings::default() };
        let a = alternatives(&s, 300);
        assert!(a.iter().all(|x| !x.name.starts_with("binariser")));
        assert!(a.iter().any(|x| x.name.starts_with("deskew")) && a.iter().any(|x| x.ocr_dpi == Some(400)));
        s.mode = OutputMode::BlackWhite;
        let b = alternatives(&s, 300);
        assert_eq!(b.iter().filter(|x| x.name.starts_with("binariser")).count(), 2);
        assert!(!b.iter().any(|x| x.name == "binariser Sauvola"));
    }

    #[test]
    fn a_page_with_unread_ink_is_retried_and_the_better_reading_kept() {
        set_enabled(true);
        let src = image::DynamicImage::ImageLuma8(sheet());
        let s = Settings { mode: OutputMode::Gray, ..Settings::default() };
        let pg = process_page(&src, &s).into_iter().next().unwrap();
        let (iw, ih) = (pg.image.width(), pg.image.height());
        let full = vec![w(0, 0, iw, ih)];
        let n = std::cell::Cell::new(0);
        // the first reading covers nothing; every alternative reads the whole sheet
        let (_, words) = refine(0, 0, pg, vec![w(0, 0, 3, 3)], &src, &s, |_, _| -> Result<Vec<Word>, ()> {
            n.set(n.get() + 1);
            Ok(full.clone())
        });
        assert!(n.get() > 0, "a page with uncovered ink must be retried");
        assert_eq!(words, full);
        assert!(note(0, 0).unwrap().contains("kept: "));
        assert!(!note(0, 0).unwrap().contains("kept: first reading"));
        set_enabled(false);
    }
}
