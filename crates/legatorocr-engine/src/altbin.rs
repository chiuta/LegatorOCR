//! `--binarize-by-ocr`: for black-and-white pages, read the page twice (the chosen local
//! threshold, and a single global Otsu threshold) and keep the version Tesseract is more
//! confident about (mean word confidence, weighted by word length).
//!
//! Measured on 12 real Romanian pages (see ROADMAP 1.4): Sauvola alone 5438 character errors,
//! Otsu alone 5505, the better of the two on every page (oracle) 5233, this rule 5246. It costs a
//! second clean-up and a second OCR run per page, so pages that already read with a mean
//! confidence of 93 or more are left alone.

use legatorocr_core::binarize::Threshold;
use legatorocr_core::pipeline::{process_page, OutputMode, Page, Settings};
use legatorocr_core::text::Word;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Mean word confidence weighted by word length; words without a confidence are ignored.
pub fn mean_conf(words: &[Word]) -> f32 {
    let (mut s, mut n) = (0.0f32, 0.0f32);
    for w in words.iter().filter(|w| w.conf >= 0.0) {
        let l = w.text.chars().count() as f32;
        s += w.conf * l;
        n += l;
    }
    if n == 0.0 {
        -1.0
    } else {
        s / n
    }
}

/// Keep `(pg, words)` or the Otsu alternative of the same sheet piece `seq`, whichever reads better.
pub fn choose<E>(
    pg: Page,
    words: Vec<Word>,
    src: &image::DynamicImage,
    settings: &Settings,
    seq: usize,
    ocr: impl Fn(&Page) -> Result<Vec<Word>, E>,
) -> (Page, Vec<Word>) {
    if !matches!(settings.mode, OutputMode::BlackWhite | OutputMode::Mixed | OutputMode::Hybrid)
        || pg.blank
        || words.is_empty()
    {
        return (pg, words);
    }
    let first = mean_conf(&words);
    if first >= 93.0 {
        return (pg, words);
    }
    let mut s2 = settings.clone();
    s2.threshold = if settings.threshold == Threshold::Otsu { Threshold::Sauvola } else { Threshold::Otsu };
    let Some(alt) = process_page(src, &s2).into_iter().nth(seq) else {
        return (pg, words);
    };
    match ocr(&alt) {
        Ok(w2) if mean_conf(&w2) > first => (alt, w2),
        _ => (pg, words),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, c: f32) -> Word {
        Word { text: t.into(), x: 0, y: 0, w: 10, h: 10, conf: c, line: 0 }
    }

    #[test]
    fn confidence_is_length_weighted_and_ignores_unknown() {
        let m = mean_conf(&[w("abcd", 90.0), w("a", 10.0), w("zz", -1.0)]);
        assert!((m - (90.0 * 4.0 + 10.0) / 5.0).abs() < 1e-4);
        assert_eq!(mean_conf(&[w("x", -1.0)]), -1.0);
    }
}
