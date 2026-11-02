//! DjVu output (roadmap 6.6): each processed page is encoded as a DjVu page and the pages are
//! bundled into one multi-page document. Black-and-white pages become a lossless JB2 bilevel
//! page (`Sjbz`); grey or colour pages use the layered encoder: a JB2 foreground mask with
//! foreground colours (`FGbz`) over an IW44 wavelet background (`BG44`). The OCR words become a
//! `TXTz` text layer, so the document is searchable.
//!
//! The encoders come from the MIT-licensed `djvu-rs` crate; this module only feeds them and
//! bundles the result. The output was checked with DjVuLibre (`djvudump`, `ddjvu`): see the
//! integration test and ROADMAP 6.6.

use anyhow::{anyhow, Result};
use djvu_rs::djvu_encode::{EncodeQuality, PageEncoder};
use djvu_rs::text::{Rect, TextLayer, TextZone, TextZoneKind};
use djvu_rs::{Bitmap, Pixmap};
use legatorocr_core::{Page, PageImage, Word};
use std::sync::Mutex;

static PAGES: Mutex<Option<Vec<Vec<u8>>>> = Mutex::new(None);

/// Start collecting pages (also clears an earlier run).
pub fn begin() {
    *PAGES.lock().unwrap() = Some(Vec::new());
}

pub fn enabled() -> bool {
    PAGES.lock().unwrap().is_some()
}

fn text_layer(words: &[Word], w: u32, h: u32) -> Option<TextLayer> {
    let words: Vec<&Word> = words.iter().filter(|x| !x.text.trim().is_empty() && x.w > 0 && x.h > 0 && x.x < w && x.y < h).collect();
    if words.is_empty() {
        return None;
    }
    let rect = |x: u32, y: u32, ww: u32, hh: u32| Rect { x, y, width: ww.min(w.saturating_sub(x)), height: hh.min(h.saturating_sub(y)) };
    let mut lines: Vec<TextZone> = Vec::new();
    let mut page_text = String::new();
    let mut i = 0;
    while i < words.len() {
        let mut j = i + 1;
        while j < words.len() && words[j].line == words[i].line {
            j += 1;
        }
        let group = &words[i..j];
        let (x0, y0) = (group.iter().map(|g| g.x).min().unwrap(), group.iter().map(|g| g.y).min().unwrap());
        let (x1, y1) = (group.iter().map(|g| g.x + g.w).max().unwrap(), group.iter().map(|g| g.y + g.h).max().unwrap());
        let line_text = group.iter().map(|g| g.text.trim()).collect::<Vec<_>>().join(" ");
        let children = group
            .iter()
            .map(|g| TextZone { kind: TextZoneKind::Word, rect: rect(g.x, g.y, g.w, g.h), text: g.text.trim().to_string(), children: Vec::new() })
            .collect();
        if !page_text.is_empty() {
            page_text.push('\n');
        }
        page_text.push_str(&line_text);
        lines.push(TextZone { kind: TextZoneKind::Line, rect: rect(x0, y0, x1 - x0, y1 - y0), text: line_text, children });
        i = j;
    }
    let page = TextZone { kind: TextZoneKind::Page, rect: rect(0, 0, w, h), text: page_text.clone(), children: lines };
    Some(TextLayer { text: page_text, zones: vec![page] })
}

/// Encode one page now and keep the bytes.
pub fn record(page: &Page, words: &[Word]) -> Result<()> {
    let (w, h) = (page.image.width(), page.image.height());
    let dpi = page.dpi.clamp(1, 65_535) as u16;
    let layer = text_layer(words, w, h);
    let bytes = match &page.image {
        PageImage::Bw(b) => {
            let mut bm = Bitmap::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    if b.get(x, y) {
                        bm.set_black(x, y);
                    }
                }
            }
            let mut enc = PageEncoder::from_bitmap(&bm).with_dpi(dpi);
            if let Some(l) = layer {
                enc = enc.with_text_layer(l);
            }
            enc.encode()
        }
        other => {
            let rgb = match other {
                PageImage::Gray(g) => image::DynamicImage::ImageLuma8(g.clone()).to_rgb8(),
                PageImage::Rgb(c) => c.clone(),
                PageImage::Bw(_) => unreachable!(),
            };
            let mut pm = Pixmap::white(w, h);
            for (x, y, p) in rgb.enumerate_pixels() {
                pm.set_rgb(x, y, p[0], p[1], p[2]);
            }
            let mut enc = PageEncoder::from_pixmap(&pm).with_dpi(dpi).with_quality(EncodeQuality::Quality);
            if let Some(l) = layer {
                enc = enc.with_text_layer(l);
            }
            enc.encode()
        }
    }
    .map_err(|e| anyhow!("DjVu encoding failed: {e}"))?;
    if let Some(v) = PAGES.lock().unwrap().as_mut() {
        v.push(bytes);
    }
    Ok(())
}

/// Bundle the collected pages into one document (a single page stays a plain page file).
pub fn finish() -> Result<Option<Vec<u8>>> {
    let Some(pages) = PAGES.lock().unwrap().take() else { return Ok(None) };
    match pages.len() {
        0 => Ok(None),
        1 => Ok(pages.into_iter().next()),
        _ => {
            let refs: Vec<&[u8]> = pages.iter().map(|p| p.as_slice()).collect();
            Ok(Some(djvu_rs::djvm::merge(&refs).map_err(|e| anyhow!("DjVu bundling failed: {e}"))?))
        }
    }
}
