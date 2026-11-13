//! `--article-query TEXT --article-crops DIR` (with `--ocr`): cuts out of every page the whole
//! newspaper or magazine article in which TEXT occurs, as one PNG per article, so a name such as
//! "Voyager" turns into the pictures of the articles that speak about it.
//!
//! The article is found from the lines OCR read, not from a layout model: the column of the hit is
//! followed up and down until a gap that is too big (more than 2.6 line heights) or a gap of one
//! line height or more before a line that is set heavier (a bold run-in heading) or taller (a
//! headline). A headline is never cut from the text under it. Limits: an article that goes on in
//! another column or on another page is cut at the column; a text that is not in the OCR (or is
//! broken by a mistake) is not found. Written and tried on newspaper pages of 1977 only.

use crate::fuzzy::fold;
use legatorocr_core::Page;
use legatorocr_ocr::ParagraphBox;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct State {
    dir: PathBuf,
    needle: String,
    written: Vec<(PathBuf, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn begin(dir: &Path, needle: &str) {
    *STATE.lock().unwrap() = Some(State {
        dir: dir.to_path_buf(),
        needle: fold(needle.trim()),
        written: Vec::new(),
    });
}

pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

/// The files written, each with the first words of its article.
pub fn finish() -> Vec<(PathBuf, String)> {
    STATE
        .lock()
        .unwrap()
        .take()
        .map(|s| s.written)
        .unwrap_or_default()
}

/// One line (or paragraph) of a page for the grouping: box and how dark its ink is.
#[derive(Clone, Copy)]
pub struct Item {
    pub b: [u32; 4],
    pub ink: f32,
}

fn height(b: &[u32; 4]) -> u32 {
    b[3].saturating_sub(b[1])
}

fn median<T: PartialOrd + Copy>(mut v: Vec<T>) -> Option<T> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

/// Box (x0, y0, x1, y1, without margin) of the article that contains item `hit`.
pub fn article_box(items: &[Item], hit: usize) -> [u32; 4] {
    let med = median(items.iter().map(|i| height(&i.b)).collect())
        .unwrap_or(1)
        .max(1) as f32;
    let h = items[hit].b;
    let hw = h[2].saturating_sub(h[0]) as f32;
    let mut col: Vec<usize> = (0..items.len())
        .filter(|&j| {
            let b = items[j].b;
            let w = b[2].saturating_sub(b[0]) as f32;
            let ov = b[2].min(h[2]) as f32 - b[0].max(h[0]) as f32;
            ov > 0.5 * w.min(hw) && w < 2.2 * hw + 400.0
        })
        .collect();
    col.sort_by_key(|&j| items[j].b[1]);
    let Some(k) = col.iter().position(|&j| j == hit) else {
        return h;
    };
    let cm = median(col.iter().map(|&j| items[j].ink).collect()).unwrap_or(0.0);
    let gap = |a: usize, b: usize| items[b].b[1] as f32 - items[a].b[3] as f32;
    let brk = |a: usize, b: usize| {
        let g = gap(a, b);
        if height(&items[a].b) as f32 >= 1.5 * med && g < 4.0 * med {
            return false; // a headline stays with its text
        }
        if g >= 2.6 * med {
            return true;
        }
        g >= 0.9 * med && (items[b].ink >= 1.3 * cm || height(&items[b].b) as f32 >= 1.5 * med)
    };
    let (mut lo, mut hi) = (k, k);
    while lo > 0 && !brk(col[lo - 1], col[lo]) {
        lo -= 1;
    }
    while hi + 1 < col.len() && !brk(col[hi], col[hi + 1]) {
        hi += 1;
    }
    let mut r = items[col[lo]].b;
    for &j in &col[lo..=hi] {
        let b = items[j].b;
        r = [
            r[0].min(b[0]),
            r[1].min(b[1]),
            r[2].max(b[2]),
            r[3].max(b[3]),
        ];
    }
    r
}

fn inside(a: &[u32; 4], b: &[u32; 4]) -> bool {
    a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3]
}

/// Indices of the paragraphs that contain `needle` (folded), also when a word is broken with a
/// hyphen at the end of a line.
fn hits(paras: &[ParagraphBox], needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    paras
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let t = fold(&p.text);
            t.contains(needle) || t.replace("-\n", "").contains(needle)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Called for every page by the export. Writes `<stem>_p<page>_<n>.png` for each article.
pub fn record(source: &Path, page: usize, pg: &Page, paras: &[ParagraphBox]) {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return };
    let hs = hits(paras, &st.needle);
    if hs.is_empty() {
        return;
    }
    let gray = pg.image.to_gray();
    let (w, h) = (gray.width(), gray.height());
    let items: Vec<Item> = paras
        .iter()
        .map(|p| {
            let (x0, y0, x1, y1) = (p.x0.min(w), p.y0.min(h), p.x1.min(w), p.y1.min(h));
            let (mut dark, mut n) = (0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    n += 1;
                    if gray.get_pixel(x, y).0[0] < 110 {
                        dark += 1;
                    }
                }
            }
            Item {
                b: [x0, y0, x1, y1],
                ink: if n == 0 { 0.0 } else { dark as f32 / n as f32 },
            }
        })
        .collect();
    let mut boxes: Vec<[u32; 4]> = Vec::new();
    for &i in &hs {
        let b = article_box(&items, i);
        if !boxes.contains(&b) {
            boxes.push(b);
        }
    }
    let all = boxes.clone();
    boxes.retain(|b| !all.iter().any(|o| o != b && inside(b, o)));
    let (base, frame) = crate::files::split_frame(source);
    let page = frame.map(|n| n as usize).unwrap_or(page);
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "page".into());
    let img = pg.image.to_dynamic();
    for (n, b) in boxes.iter().enumerate() {
        let pad = 20u32;
        let (x0, y0) = (b[0].saturating_sub(pad), b[1].saturating_sub(pad));
        let (x1, y1) = ((b[2] + pad).min(w), (b[3] + pad).min(h));
        let crop = img.crop_imm(
            x0,
            y0,
            x1.saturating_sub(x0).max(1),
            y1.saturating_sub(y0).max(1),
        );
        // two files of the same name in different folders must not overwrite each other's pictures
        let mut path = st.dir.join(format!("{stem}_p{page}_{}.png", n + 1));
        let mut again = 1;
        while st.written.iter().any(|(w, _)| *w == path) {
            again += 1;
            path = st.dir.join(format!("{stem}_p{page}_{}_{again}.png", n + 1));
        }
        if std::fs::create_dir_all(&st.dir).is_ok() && crop.save(&path).is_ok() {
            let first = paras
                .iter()
                .filter(|p| p.y0 >= b[1] && p.y1 <= b[3] && p.x0 >= b[0] && p.x1 <= b[2])
                .map(|p| p.text.replace('\n', " "))
                .next()
                .unwrap_or_default();
            st.written.push((path, first));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(x0: u32, y0: u32, x1: u32, y1: u32, ink: f32) -> Item {
        Item {
            b: [x0, y0, x1, y1],
            ink,
        }
    }

    #[test]
    fn the_headline_goes_with_its_text_and_the_next_article_is_left_out() {
        // 26 px lines, 8 px apart; a 60 px headline 80 px above the text; 150 px gap before the next.
        let mut v = vec![it(0, 0, 500, 26, 0.1), it(0, 34, 500, 60, 0.1)]; // article above
        v.push(it(0, 210, 500, 270, 0.2)); // headline
        for i in 0..5 {
            v.push(it(0, 350 + i * 34, 500, 376 + i * 34, 0.1));
        }
        v.push(it(0, 700, 500, 726, 0.1)); // next article
        let b = article_box(&v, 4);
        assert_eq!(b, [0, 210, 500, 350 + 4 * 34 + 26]);
    }

    #[test]
    fn a_bold_run_in_heading_starts_a_new_brief() {
        let v = vec![
            it(0, 0, 400, 26, 0.1),
            it(0, 34, 400, 60, 0.1),
            it(0, 90, 400, 116, 0.25), // heavy: new brief
            it(0, 124, 400, 150, 0.1),
        ];
        assert_eq!(article_box(&v, 1), [0, 0, 400, 60]);
        assert_eq!(article_box(&v, 3), [0, 90, 400, 150]);
    }

    #[test]
    fn a_neighbouring_column_is_not_followed() {
        let v = vec![
            it(0, 0, 300, 26, 0.1),
            it(400, 34, 700, 60, 0.1),
            it(0, 34, 300, 60, 0.1),
        ];
        assert_eq!(article_box(&v, 0), [0, 0, 300, 60]);
    }

    #[test]
    fn hyphen_broken_words_are_found() {
        let p = |t: &str| ParagraphBox {
            text: t.into(),
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
            mean_conf: None,
        };
        let ps = vec![p("a doua misiune „Vo-\nyager“ pleacă"), p("nimic")];
        assert_eq!(hits(&ps, "voyager"), vec![0]);
    }
}
