//! OCR zones: `--ocr-zone X,Y,W,H[=LANG]` and `--ocr-ignore X,Y,W,H`.
//!
//! A zone may end in `#table`: it is read as one uniform block (Tesseract mode 6), which keeps
//! the rows of a table together instead of splitting them into columns of text.
//!
//! Rectangles are in millimetres on the processed page, origin top-left (like `--picture-zone`).
//! With zones, only the zones are recognised, one after the other in the order given (so that is
//! the reading order), each with its own language when `=LANG` is added. With only
//! ignore-rectangles the whole page is recognised and every word whose centre lies inside one is
//! dropped (a stamp, a page header, a margin note, a barcode caption).

use anyhow::{bail, Result};
use legatorocr_core::{BitImage, PageImage, Word};
use legatorocr_ocr::{OcrError, OcrOptions};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub rect: Rect,
    pub lang: Option<String>,
    /// Read as one uniform block, row by row (a table).
    pub table: bool,
}

type Config = (Vec<Zone>, Vec<Rect>);
/// Recognises one image with the given options.
pub type Inner<'a> = &'a dyn Fn(&PageImage, u32, &OcrOptions) -> Result<Vec<Word>, OcrError>;
static CFG: Mutex<Option<Config>> = Mutex::new(None);

pub fn parse_rect(s: &str) -> Result<Rect> {
    let v: Vec<f32> = s
        .split(',')
        .map(|x| x.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("{s:?}: expected four numbers X,Y,W,H in mm"))?;
    if v.len() != 4 || v.iter().any(|x| !x.is_finite() || *x < 0.0) || v[2] <= 0.0 || v[3] <= 0.0 {
        bail!("{s:?}: expected X,Y,W,H in mm, none negative, width and height above 0");
    }
    Ok(Rect {
        x: v[0],
        y: v[1],
        w: v[2],
        h: v[3],
    })
}

pub fn parse_zone(s: &str) -> Result<Zone> {
    let (s, table) = match s.trim().strip_suffix("#table") {
        Some(t) => (t.trim_end(), true),
        None => (s.trim(), false),
    };
    let (r, lang) = match s.split_once('=') {
        Some((r, l)) => (r, Some(l.trim())),
        None => (s, None),
    };
    let lang = match lang {
        None => None,
        Some(l) => {
            let c = legatorocr_ocr::canonical_lang(l);
            if !legatorocr_ocr::valid_lang(&c) {
                bail!("{s:?}: invalid language {l:?} (e.g. ron or ron+eng)");
            }
            Some(c)
        }
    };
    Ok(Zone {
        rect: parse_rect(r)?,
        lang,
        table,
    })
}

pub fn set(zones: Vec<Zone>, ignores: Vec<Rect>) {
    if let Ok(mut c) = CFG.lock() {
        *c = (!zones.is_empty() || !ignores.is_empty()).then_some((zones, ignores));
    }
}

fn config() -> Option<Config> {
    CFG.lock().ok().and_then(|c| c.clone())
}

pub fn active() -> bool {
    config().is_some()
}

/// Pixel box of a rectangle on a page, clamped to the page; `None` when nothing is left.
fn px_box(r: &Rect, dpi: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let k = dpi as f32 / 25.4;
    let x0 = ((r.x * k).round().max(0.0) as u32).min(w);
    let y0 = ((r.y * k).round().max(0.0) as u32).min(h);
    let x1 = (((r.x + r.w) * k).round().max(0.0) as u32).min(w);
    let y1 = (((r.y + r.h) * k).round().max(0.0) as u32).min(h);
    (x1 > x0 + 4 && y1 > y0 + 4).then_some((x0, y0, x1 - x0, y1 - y0))
}

fn crop(page: &PageImage, x: u32, y: u32, w: u32, h: u32) -> PageImage {
    match page {
        PageImage::Rgb(i) => PageImage::Rgb(image::imageops::crop_imm(i, x, y, w, h).to_image()),
        PageImage::Gray(i) => PageImage::Gray(image::imageops::crop_imm(i, x, y, w, h).to_image()),
        PageImage::Bw(b) => {
            let mut o = BitImage::new(w, h);
            o.paste_region(b, (x as i64, y as i64, w, h), (0, 0));
            PageImage::Bw(o)
        }
    }
}

/// Recognise `page` under the configured zones; `inner` recognises one image with given options.
pub fn recognize(
    page: &PageImage,
    dpi: u32,
    o: &OcrOptions,
    inner: Inner,
) -> Result<Vec<Word>, OcrError> {
    let Some((zones, ignores)) = config() else {
        return inner(page, dpi, o);
    };
    let (pw, ph) = (page.width(), page.height());
    let mut words: Vec<Word> = Vec::new();
    if zones.is_empty() {
        words = inner(page, dpi, o)?;
    } else {
        for (zi, z) in zones.iter().enumerate() {
            let Some((x, y, w, h)) = px_box(&z.rect, dpi, pw, ph) else {
                continue;
            };
            let mut opt = o.clone();
            if let Some(l) = &z.lang {
                opt.lang = l.clone();
            }
            if z.table {
                opt.psm = 6;
            }
            for mut wd in inner(&crop(page, x, y, w, h), dpi, &opt)? {
                wd.x += x;
                wd.y += y;
                // lines of different zones must never merge
                wd.line = wd.line.saturating_add(zi as u32 * 100_000);
                words.push(wd);
            }
        }
    }
    let boxes: Vec<_> = ignores
        .iter()
        .filter_map(|r| px_box(r, dpi, pw, ph))
        .collect();
    words.retain(|w| {
        let (cx, cy) = (w.x + w.w / 2, w.y + w.h / 2);
        !boxes
            .iter()
            .any(|&(x, y, bw, bh)| cx >= x && cx < x + bw && cy >= y && cy < y + bh)
    });
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as M;

    // the configuration is global: tests that set it run one at a time
    static LOCK: M<()> = M::new(());

    fn word(t: &str, x: u32, y: u32) -> Word {
        Word {
            text: t.into(),
            x,
            y,
            w: 20,
            h: 10,
            conf: 90.0,
            line: 1,
        }
    }

    fn page() -> PageImage {
        PageImage::Gray(image::GrayImage::from_pixel(600, 800, image::Luma([255])))
    }

    #[test]
    fn specs_parse_and_bad_ones_are_refused() {
        assert_eq!(
            parse_rect("10, 20,30,40").unwrap(),
            Rect {
                x: 10.0,
                y: 20.0,
                w: 30.0,
                h: 40.0
            }
        );
        assert!(parse_rect("1,2,3").is_err());
        assert!(parse_rect("1,2,0,4").is_err());
        assert!(parse_rect("1,-2,3,4").is_err());
        let z = parse_zone("5,5,50,50=ron+eng").unwrap();
        assert_eq!(z.lang.as_deref(), Some("ron+eng"));
        assert!(parse_zone("5,5,50,50=not a lang!").is_err());
        assert_eq!(parse_zone("5,5,50,50").unwrap().lang, None);
        let t = parse_zone("5,5,50,50=ron#table").unwrap();
        assert!(t.table && t.lang.as_deref() == Some("ron"));
        assert!(parse_zone("5,5,50,50 #table").unwrap().table);
        assert!(!parse_zone("5,5,50,50").unwrap().table);
    }

    #[test]
    fn a_table_zone_is_read_as_one_uniform_block() {
        let _g = LOCK.lock().unwrap();
        set(vec![parse_zone("0,0,100,100#table").unwrap()], vec![]);
        let psm = std::cell::Cell::new(0);
        let inner = |_: &PageImage, _d: u32, o: &OcrOptions| {
            psm.set(o.psm);
            Ok(vec![])
        };
        recognize(&page(), 25, &OcrOptions::default(), &inner).unwrap();
        set(vec![], vec![]);
        assert_eq!(psm.get(), 6);
    }

    #[test]
    fn zones_are_read_in_order_with_their_language_and_offset() {
        let _g = LOCK.lock().unwrap();
        // dpi 25 is about 0.984 px per mm
        set(
            vec![
                parse_zone("100,200,300,100=fra").unwrap(),
                parse_zone("10,10,200,100").unwrap(),
            ],
            vec![],
        );
        let seen = std::cell::RefCell::new(Vec::new());
        let inner = |p: &PageImage, _d: u32, o: &OcrOptions| {
            seen.borrow_mut()
                .push((p.width(), p.height(), o.lang.clone()));
            Ok(vec![word("a", 5, 6)])
        };
        let base = OcrOptions {
            lang: "ron".into(),
            ..OcrOptions::default()
        };
        let w = recognize(&page(), 25, &base, &inner).unwrap();
        set(vec![], vec![]);
        assert_eq!(
            *seen.borrow(),
            vec![(296, 98, "fra".into()), (197, 98, "ron".into())]
        );
        // dpi 25 ≈ 0.984 px/mm: offsets are the zone corners in pixels
        assert_eq!((w[0].x, w[0].y), (5 + 98, 6 + 197));
        assert_eq!((w[1].x, w[1].y), (5 + 10, 6 + 10));
        assert!(w[0].line < w[1].line, "lines of two zones stay apart");
    }

    #[test]
    fn ignored_areas_drop_the_words_centred_in_them() {
        let _g = LOCK.lock().unwrap();
        set(vec![], vec![parse_rect("0,0,100,30").unwrap()]);
        let inner = |_: &PageImage, _: u32, _: &OcrOptions| {
            Ok(vec![word("header", 10, 5), word("body", 10, 300)])
        };
        let w = recognize(&page(), 25, &OcrOptions::default(), &inner).unwrap();
        set(vec![], vec![]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].text, "body");
        // nothing configured: plain pass-through
        let w = recognize(&page(), 25, &OcrOptions::default(), &inner).unwrap();
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn a_black_and_white_page_is_cropped_too() {
        let mut b = BitImage::new(100, 100);
        b.set(60, 70, true);
        let PageImage::Bw(c) = crop(&PageImage::Bw(b), 50, 60, 30, 30) else {
            panic!()
        };
        assert_eq!((c.w, c.h), (30, 30));
        assert!(c.get(10, 10) && !c.get(0, 0));
        assert!(px_box(&parse_rect("0,0,2,2").unwrap(), 25, 600, 800).is_none());
    }
}
