//! `--passages-jsonl FILE` (with `--ocr`): one JSON object per paragraph of every page, for
//! feeding the recognised text to a citation-grounded research tool. Each line says where the
//! paragraph is (source file, page, printed page label, box in output-page pixels and as
//! fractions 0-1), what it says (text, OCR language setting, mean OCR confidence) and which page
//! image it came from (SHA-256 of the image bytes), plus a stable passage id and a citation
//! string. Paragraphs are the ones `--docx` / `words_to_paragraphs` use; nothing is re-detected.
//! The schema is documented in README.md and checked by `scripts/check-passages.py`.
//! Whether any particular research tool can import this format has NOT been checked.

use crate::checksums::sha256_hex;
use legatorocr_core::{Page, Word};
use legatorocr_ocr::ParagraphBox;
use std::path::Path;
use std::sync::Mutex;

pub const SCHEMA: &str = "legatorocr.passage/1";

struct PageRec {
    source: String,
    page: usize,
    printed: Option<u32>,
    lang: String,
    w: u32,
    h: u32,
    image_sha256: String,
    image_file: Option<String>,
    image_basis: &'static str,
    paras: Vec<ParagraphBox>,
}

struct State {
    ext: String,
    jpeg_quality: u8,
    /// Output stems of the sheets when page images are written; `None` when they are not.
    stems: Option<Vec<String>>,
    pages: Vec<PageRec>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Start collecting. `ext`: extension of the page images the run writes (`png`, `tif`, `jpg`,
/// ...); `stems`: `Some(output stem per sheet)` when they are really written, `None` for
/// `--format none` (the hash is then taken over the PNG encoding the run would write).
pub fn begin(ext: &str, jpeg_quality: u8, stems: Option<Vec<String>>) {
    *STATE.lock().unwrap() = Some(State {
        ext: ext.to_ascii_lowercase(),
        jpeg_quality,
        stems,
        pages: Vec::new(),
    });
}

pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

/// Note one page. `sheet`: index of the input sheet; `page`: 1-based number among the pages kept.
pub fn record(sheet: usize, source: &Path, page: usize, pg: &Page, words: &[Word], lang: &str) {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return };
    let (bytes, basis, file) = match (&st.stems, crate::encode_page_image(pg, &st.ext, st.jpeg_quality)) {
        (Some(stems), Ok(Some(b))) => {
            let stem = stems.get(sheet).cloned().unwrap_or_default();
            let suffix = match pg.split_index {
                Some(0) => "_L",
                Some(_) => "_R",
                None => "",
            };
            (b, "file", Some(format!("{stem}{suffix}.{}", st.ext)))
        }
        _ => {
            let b = crate::encode_page_image(pg, "png", st.jpeg_quality).ok().flatten().unwrap_or_default();
            (b, "png-encoding", None)
        }
    };
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.to_string_lossy().into_owned());
    let printed = crate::pagenum::enabled()
        .then(|| legatorocr_core::quality::detect_page_number(words, pg.image.height()))
        .flatten();
    st.pages.push(PageRec {
        source: name,
        page,
        printed,
        lang: lang.to_string(),
        w: pg.image.width(),
        h: pg.image.height(),
        image_sha256: sha256_hex(&bytes),
        image_file: file,
        image_basis: basis,
        paras: legatorocr_ocr::paragraph_boxes(words),
    });
}

/// One line of the output.
#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    pub passage_id: String,
    pub source_file: String,
    pub page: usize,
    pub page_label: Option<String>,
    pub paragraph_index: usize,
    pub text: String,
    pub bbox_px: [u32; 4],
    pub bbox_frac: [f64; 4],
    pub page_width_px: u32,
    pub page_height_px: u32,
    pub language: String,
    pub mean_confidence: Option<f64>,
    pub page_image_sha256: String,
    pub page_image_file: Option<String>,
    pub page_image_basis: &'static str,
}

/// Stable id: first 16 hex digits of SHA-256 over `"<page image sha256>:x0,y0,x1,y1"`.
pub fn passage_id(page_sha: &str, b: [u32; 4]) -> String {
    sha256_hex(format!("{page_sha}:{},{},{},{}", b[0], b[1], b[2], b[3]).as_bytes())[..16].to_string()
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Page labels as the PDF would carry them with `--page-labels` (same run rule); `None` before
/// the first agreeing run and everywhere when no printed numbers were found.
fn labels(printed: &[Option<u32>]) -> Vec<Option<u32>> {
    let runs = legatorocr_core::quality::label_runs(printed);
    (0..printed.len())
        .map(|i| runs.iter().rev().find(|(at, _)| *at <= i).map(|(at, n)| n + (i - at) as u32))
        .collect()
}

/// Stop collecting and return the passages. `clean`: apply the `--clean-text` clean-up to each
/// paragraph (so the text equals what `--txt` shows for it).
pub fn finish(clean: bool) -> Vec<Passage> {
    let Some(st) = STATE.lock().unwrap().take() else { return Vec::new() };
    let labs = if crate::pagenum::enabled() {
        labels(&st.pages.iter().map(|p| p.printed).collect::<Vec<_>>())
    } else {
        vec![None; st.pages.len()]
    };
    let opts = crate::ropost::PostOptions::default();
    let mut out = Vec::new();
    for (pi, p) in st.pages.iter().enumerate() {
        let (w, h) = (p.w.max(1), p.h.max(1));
        let mut idx = 0;
        for para in &p.paras {
            let text = if clean { crate::ropost::postprocess(&para.text, &opts) } else { para.text.clone() };
            if text.trim().is_empty() {
                continue;
            }
            let b = [para.x0.min(p.w), para.y0.min(p.h), para.x1.min(p.w), para.y1.min(p.h)];
            out.push(Passage {
                passage_id: passage_id(&p.image_sha256, b),
                source_file: p.source.clone(),
                page: p.page,
                page_label: labs[pi].map(|n| n.to_string()),
                paragraph_index: idx,
                text,
                bbox_px: b,
                bbox_frac: [
                    round6(b[0] as f64 / w as f64),
                    round6(b[1] as f64 / h as f64),
                    round6(b[2] as f64 / w as f64),
                    round6(b[3] as f64 / h as f64),
                ],
                page_width_px: p.w,
                page_height_px: p.h,
                language: p.lang.clone(),
                mean_confidence: para.mean_conf.map(|c| (c as f64 * 100.0).round() / 100.0),
                page_image_sha256: p.image_sha256.clone(),
                page_image_file: p.image_file.clone(),
                page_image_basis: p.image_basis,
            });
            idx += 1;
        }
    }
    out
}

fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map_or("null".into(), js)
}

/// The JSONL document (one object per line, `\n` after each). `title`: used for the `cite`
/// string `"<title> p.<label>"` (the printed label when known, else the page number); `None`
/// → the source file name without extension.
pub fn to_jsonl(ps: &[Passage], title: Option<&str>) -> String {
    let mut s = String::new();
    for p in ps {
        let t = title.map(str::to_string).unwrap_or_else(|| {
            Path::new(&p.source_file)
                .file_stem()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.source_file.clone())
        });
        let cite = format!("{t} p.{}", p.page_label.clone().unwrap_or_else(|| p.page.to_string()));
        let px = p.bbox_px;
        let fr = p.bbox_frac;
        s.push_str(&format!(
            "{{\"schema\":{},\"passage_id\":{},\"source_file\":{},\"page\":{},\"page_label\":{},\"paragraph_index\":{},\"text\":{},\"bbox_px\":[{},{},{},{}],\"bbox_frac\":[{},{},{},{}],\"page_width_px\":{},\"page_height_px\":{},\"language\":{},\"mean_confidence\":{},\"page_image_sha256\":{},\"page_image_file\":{},\"page_image_basis\":{},\"cite\":{}}}\n",
            js(SCHEMA), js(&p.passage_id), js(&p.source_file), p.page, opt(&p.page_label), p.paragraph_index, js(&p.text),
            px[0], px[1], px[2], px[3], fr[0], fr[1], fr[2], fr[3],
            p.page_width_px, p.page_height_px, js(&p.language),
            p.mean_confidence.map_or("null".into(), |c| format!("{c}")),
            js(&p.page_image_sha256), opt(&p.page_image_file), js(p.page_image_basis), js(&cite),
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(t: &str, x: u32, y: u32, line: u32) -> Word {
        Word { text: t.into(), x, y, w: 40, h: 20, conf: 80.0, line }
    }

    #[test]
    fn records_paragraphs_with_stable_ids_and_valid_json() {
        let img = legatorocr_core::PageImage::Bw(legatorocr_core::BitImage::new(300, 400));
        let pg = Page { image: img, dpi: 300, skew_deg: 0.0, blank: false, split_index: None, content_box: None, layers: None };
        begin("png", 90, None);
        let words = vec![word("Salut", 10, 10, 0), word("lume", 60, 10, 0), word("doi", 10, 300, 1)];
        record(0, Path::new("/x/carte \"1\".png"), 1, &pg, &words, "ron");
        record(1, Path::new("/x/carte \"1\".png"), 2, &pg, &words, "ron");
        let ps = finish(false);
        assert_eq!(ps.len(), 4);
        assert_eq!(ps[0].text, "Salut lume");
        assert_eq!(ps[0].bbox_px, [10, 10, 100, 30]);
        assert_eq!(ps[0].bbox_frac[2], round6(100.0 / 300.0));
        assert_eq!(ps[0].page_image_basis, "png-encoding");
        assert_eq!(ps[0].passage_id, passage_id(&ps[0].page_image_sha256, ps[0].bbox_px));
        // same image, same box: same id on both pages (ids hash page image + box)
        assert_eq!(ps[0].passage_id, ps[2].passage_id);
        assert_ne!(ps[0].passage_id, ps[1].passage_id);
        assert_eq!(ps[0].page_image_sha256.len(), 64);
        let j = to_jsonl(&ps[..1], Some("Titlu"));
        assert!(j.ends_with("}\n") && j.contains("\"source_file\":\"carte \\\"1\\\".png\"") && j.contains("\"cite\":\"Titlu p.1\""));
        assert!(!enabled());
    }

    #[test]
    fn labels_follow_the_pdf_rule() {
        let l = labels(&[None, Some(5), Some(6), Some(7), None]);
        assert_eq!(l, vec![None, Some(5), Some(6), Some(7), Some(8)]);
        assert_eq!(labels(&[None, None]), vec![None, None]);
    }
}
