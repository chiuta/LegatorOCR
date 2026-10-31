//! LegatorOCR native OCR engine — pure Rust, no external program and no code from any other OCR
//! project. Pipeline: binarise → X-Y cut into text lines → normalise each line to 32 px high →
//! CNN + 2×BiGRU + CTC recogniser → word boxes with confidences.
//!
//! The model file (`LNM1`) is produced by `tools/ocr-train` from synthetic text rendered with
//! freely licensed fonts; nothing of Tesseract's models or training data is used.

pub mod ctc;
pub mod layout;
pub mod lexicon;
pub mod model;

use legatorocr_core::{PageImage, Word};
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

/// Recognise a page. `dpi` is only used to size the binarisation window.
pub fn recognize(page: &PageImage, dpi: u32, model: &Model, o: &NativeOptions) -> Vec<Word> {
    let (gray, ink) = layout::to_ink(page, dpi);
    let med = layout::median_component_height(&ink);
    if med == 0 {
        return Vec::new();
    }
    let lines = layout::find_lines(&ink, med);
    let per_line: Vec<Vec<Word>> = lines
        .par_iter()
        .map(|ln| recognise_line(&gray, &ink, ln, model, o))
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

fn recognise_line(
    gray: &image::GrayImage,
    ink: &legatorocr_core::BitImage,
    ln: &layout::Line,
    model: &Model,
    o: &NativeOptions,
) -> Vec<Word> {
    let Some(li) = layout::extract(gray, ln, LINE_H) else { return Vec::new() };
    let (t, lp) = model.forward(&li.data, li.w);
    let glyphs = ctc::greedy(&lp, t, model.nc, &model.charset);
    let lexicons: Vec<&lexicon::Lexicon> =
        lexicon::codes(&o.lang).into_iter().filter_map(lexicon::Lexicon::load).collect();
    let mut words = Vec::new();
    let mut cur: Vec<&ctc::Glyph> = Vec::new();
    let flush = |cur: &mut Vec<&ctc::Glyph>, words: &mut Vec<Word>| {
        if cur.is_empty() {
            return;
        }
        let mut text: String = cur.iter().map(|g| g.ch).collect();
        for lx in &lexicons {
            if lx.contains(&text) {
                break;
            }
            if let Some(fixed) = lx.repair(&text, cur) {
                text = fixed;
                break;
            }
        }
        let conf = cur.iter().map(|g| g.p).sum::<f32>() / cur.len() as f32 * 100.0;
        // columns → recogniser pixels → page pixels, then tighten to the ink actually there
        let px0 = cur[0].c0 as f32 * 4.0 * li.scale + li.x as f32;
        let px1 = (cur[cur.len() - 1].c1 as f32 + 1.0) * 4.0 * li.scale + li.x as f32;
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
    for g in &glyphs {
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
