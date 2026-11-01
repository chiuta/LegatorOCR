//! LegatorOCR native OCR engine — pure Rust, no external program and no code from any other OCR
//! project. Pipeline: binarise (polarity, ruling removal) → X-Y cut into text lines → normalise
//! each line to 32 px high → CNN + 2×BiGRU + CTC recogniser → word boxes with confidences.
//!
//! The model file (`LNM1`) is produced by `tools/ocr-train` from synthetic text rendered with
//! freely licensed fonts; nothing of Tesseract's models or training data is used.

pub mod ctc;
pub mod layout;
pub mod lexicon;
pub mod model;

use image::GrayImage;
use legatorocr_core::{BitImage, PageImage, Word};
use rayon::prelude::*;

pub use model::{Model, ModelError, LINE_H};

#[derive(Debug, Clone)]
pub struct NativeOptions {
    /// Words below this confidence (0..100) are dropped; 0 keeps everything.
    pub min_conf: f32,
    /// Languages (`ron+eng`, `ro+en` …) whose word lists repair isolated errors; empty = off.
    pub lang: String,
}

impl Default for NativeOptions {
    fn default() -> Self {
        Self { min_conf: 0.0, lang: String::new() }
    }
}

/// The model compiled into the program, if the build included one.
pub fn embedded_model() -> Option<&'static Model> {
    use std::sync::OnceLock;
    static M: OnceLock<Option<Model>> = OnceLock::new();
    M.get_or_init(|| {
        const BYTES: &[u8] = include_bytes!("../models/multi.lnm");
        Model::from_bytes(BYTES).ok()
    })
    .as_ref()
}

/// Pages above this many pixels are analysed at reduced size (memory and time stay bounded).
const MAX_PAGE_PIXELS: u64 = 64_000_000;
/// Mean glyph probability below which a line is treated as picture/dust rather than text.
const MIN_LINE_P: f32 = 0.55;

/// Recognise a page. `dpi` only sizes the binarisation window.
pub fn recognize(page: &PageImage, dpi: u32, model: &Model, o: &NativeOptions) -> Vec<Word> {
    let (w, h) = (page.width() as u64, page.height() as u64);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    if w * h > MAX_PAGE_PIXELS {
        // shrink by an integer factor, recognise, scale the boxes back
        let k = ((w * h) as f64 / MAX_PAGE_PIXELS as f64).sqrt().ceil() as u32;
        let g = page.to_gray();
        let small = image::imageops::resize(
            &g,
            (g.width() / k).max(1),
            (g.height() / k).max(1),
            image::imageops::FilterType::Triangle,
        );
        let mut words = recognize_inner(&PageImage::Gray(small), (dpi / k).max(1), model, o);
        for wd in &mut words {
            wd.x *= k;
            wd.y *= k;
            wd.w *= k;
            wd.h *= k;
        }
        return words;
    }
    recognize_inner(page, dpi, model, o)
}

fn lexicons_for(lang: &str) -> Vec<&'static lexicon::Lexicon> {
    lexicon::codes(lang).into_iter().filter_map(lexicon::Lexicon::load).collect()
}

fn recognize_inner(page: &PageImage, dpi: u32, model: &Model, o: &NativeOptions) -> Vec<Word> {
    let prep = layout::prepare(page, dpi);
    if prep.med == 0 {
        return Vec::new();
    }
    let lines: Vec<layout::Line> = layout::find_lines(&prep.ink, prep.med)
        .iter()
        .flat_map(|l| layout::split_long(&prep.ink, l, 2000))
        .collect();
    let lexicons = lexicons_for(&o.lang);
    let ro = lexicon::codes(&o.lang).contains(&"ro");
    // `line` ids follow reading order of the *source* lines, so cut halves of one line stay apart
    let per_line: Vec<Vec<Word>> = lines
        .par_iter()
        .map(|ln| recognise_line(&prep.gray, &prep.ink, ln, model, o, &lexicons, ro))
        .collect();
    let mut words = Vec::new();
    for (i, mut ws) in per_line.into_iter().enumerate() {
        for w in &mut ws {
            w.line = i as u32;
        }
        words.append(&mut ws);
    }
    words
}

fn ink_fraction(ink: &BitImage, ln: &layout::Line) -> f32 {
    let (x1, y1) = ((ln.x + ln.w).min(ink.w), (ln.y + ln.h).min(ink.h));
    let (mut n, mut t) = (0u64, 0u64);
    for y in ln.y..y1 {
        let s = y as usize * ink.w as usize;
        for &b in &ink.data[s + ln.x as usize..s + x1 as usize] {
            n += b as u64;
        }
        t += (x1 - ln.x) as u64;
    }
    n as f32 / t.max(1) as f32
}

/// Result of the recogniser on one line, before it is turned into words.
struct Decoded {
    glyphs: Vec<ctc::Glyph>,
    scale: f32,
    off_x: u32,
}

fn decode_line(gray: &GrayImage, ink: &BitImage, ln: &layout::Line, model: &Model) -> Option<Decoded> {
    // a box that is mostly ink is light text on a dark panel: read it inverted
    let invert = ink_fraction(ink, ln) > 0.62;
    let li = layout::extract(gray, ln, LINE_H, invert)?;
    let (t, lp) = model.forward(&li.data, li.w);
    Some(Decoded { glyphs: ctc::greedy(&lp, t, model.nc, &model.charset), scale: li.scale, off_x: li.x })
}

fn recognise_line(
    gray: &GrayImage,
    ink: &BitImage,
    ln: &layout::Line,
    model: &Model,
    o: &NativeOptions,
    lexicons: &[&lexicon::Lexicon],
    ro: bool,
) -> Vec<Word> {
    let Some(d) = decode_line(gray, ink, ln, model) else { return Vec::new() };
    if d.glyphs.is_empty() {
        return Vec::new();
    }
    // lines the network cannot read (pictures, hatching, dust) have low mean probability
    let letters: Vec<&ctc::Glyph> = d.glyphs.iter().filter(|g| g.ch != ' ').collect();
    if letters.is_empty() {
        return Vec::new();
    }
    let mean_p = letters.iter().map(|g| g.p).sum::<f32>() / letters.len() as f32;
    if mean_p < MIN_LINE_P {
        return Vec::new();
    }
    let mut words = Vec::new();
    let mut cur: Vec<&ctc::Glyph> = Vec::new();
    let flush = |cur: &mut Vec<&ctc::Glyph>, words: &mut Vec<Word>| {
        if cur.is_empty() {
            return;
        }
        let mut text: String = cur.iter().map(|g| g.ch).collect();
        if ro {
            text = legatorocr_core::text::normalize_ro(&text);
        }
        if !lexicons.is_empty() {
            if let Some(fixed) = lexicon::repair(lexicons, &text, cur, words.is_empty()) {
                text = fixed;
            }
        }
        let conf = cur.iter().map(|g| g.p).sum::<f32>() / cur.len() as f32 * 100.0;
        // columns → recogniser pixels → page pixels. A CTC peak sits inside its glyph, so
        // widen by one column each side before tightening to the ink that is really there.
        let px0 = (cur[0].c0 as f32 - 1.0).max(0.0) * 4.0 * d.scale + d.off_x as f32;
        let px1 = (cur[cur.len() - 1].c1 as f32 + 2.0) * 4.0 * d.scale + d.off_x as f32;
        let x0 = (px0.floor().max(0.0) as u32).max(ln.x.saturating_sub(2));
        let x1 = (px1.ceil() as u32).min(ln.x + ln.w + 2).min(ink.w);
        let (mut ya, mut yb) = (u32::MAX, 0u32);
        let (mut xa, mut xb) = (u32::MAX, 0u32);
        for y in ln.y..(ln.y + ln.h).min(ink.h) {
            let s = y as usize * ink.w as usize;
            for x in x0..x1 {
                if ink.data[s + x as usize] != 0 {
                    ya = ya.min(y);
                    yb = yb.max(y + 1);
                    xa = xa.min(x);
                    xb = xb.max(x + 1);
                }
            }
        }
        let (bx, by, bw, bh) = if xa == u32::MAX {
            (x0, ln.y, x1.saturating_sub(x0).max(1), ln.h)
        } else {
            (xa, ya, xb - xa, yb - ya)
        };
        words.push(Word { text, x: bx, y: by, w: bw.max(1), h: bh.max(1), conf, line: 0 });
        cur.clear();
    };
    for g in &d.glyphs {
        if g.ch == ' ' {
            flush(&mut cur, &mut words);
        } else {
            cur.push(g);
        }
    }
    flush(&mut cur, &mut words);
    words.retain(|w| w.conf >= o.min_conf);
    words
}

/// Which way must the page be turned (clockwise, 0/90/180/270°) to stand upright?
///
/// Each of the four turns of a reduced copy is read on its longest lines; the right turn is the
/// one whose words are found in the language word lists far more often than in the others.
/// `None` when the page has too little text to decide.
pub fn detect_orientation(page: &PageImage, dpi: u32, model: &Model, lang: &str) -> Option<u16> {
    let g = page.to_gray();
    let big = g.width().max(g.height());
    let (g, dpi) = if big > 1800 {
        let k = 1800.0 / big as f32;
        let r = image::imageops::resize(
            &g,
            ((g.width() as f32 * k) as u32).max(1),
            ((g.height() as f32 * k) as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
        (r, ((dpi as f32 * k) as u32).max(1))
    } else {
        (g, dpi)
    };
    let mut lexicons = lexicons_for(lang);
    if lexicons.is_empty() {
        lexicons = lexicons_for("ro+en+fr+it+es+pt+de+hu");
    }
    let turns: [(u16, GrayImage); 4] = [
        (0, g.clone()),
        (90, image::imageops::rotate90(&g)),
        (180, image::imageops::rotate180(&g)),
        (270, image::imageops::rotate270(&g)),
    ];
    let scores: Vec<(u16, f32)> = turns
        .par_iter()
        .map(|(deg, img)| (*deg, orientation_score(img, dpi, model, &lexicons)))
        .collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, second) = (sorted[0], sorted[1]);
    (best.1 >= 12.0 && best.1 >= second.1 * 1.6).then_some(best.0)
}

/// Total length of the dictionary words read on the longest lines of one orientation.
fn orientation_score(img: &GrayImage, dpi: u32, model: &Model, lexicons: &[&lexicon::Lexicon]) -> f32 {
    let prep = layout::prepare(&PageImage::Gray(img.clone()), dpi);
    if prep.med == 0 {
        return 0.0;
    }
    let mut lines = layout::find_lines(&prep.ink, prep.med);
    lines.retain(|l| l.w >= prep.med * 6 && l.w > l.h);
    lines.sort_by_key(|l| std::cmp::Reverse(l.w));
    lines.truncate(10);
    let mut score = 0.0;
    for ln in &lines {
        let Some(d) = decode_line(&prep.gray, &prep.ink, ln, model) else { continue };
        let text: String = d.glyphs.iter().map(|g| g.ch).collect();
        for w in text.split(' ') {
            let core: String = w.chars().filter(|c| c.is_alphabetic()).collect();
            if core.chars().count() >= 3 && lexicons.iter().any(|l| l.contains(&core)) {
                score += core.chars().count() as f32;
            }
        }
    }
    score
}
