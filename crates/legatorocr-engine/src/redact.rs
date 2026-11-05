//! `--redact` (roadmap 7.12): remove personal data for hand-over. Words whose text matches a
//! pattern are painted over in the page image (the pixels are overwritten, not hidden under a
//! layer) and dropped from the text, so the text layer, .txt, .docx, hOCR and the other exports
//! no longer contain them. A page with layers is flattened to its single image so the old
//! pixels cannot survive in a background layer.
//!
//! Limits: only what OCR read can be found. A value OCR missed or misread stays visible, and a
//! value split in an unusual way may be only partly covered. Check the result by eye, and
//! test the patterns on your own documents, before sending anything out. Matching runs line by
//! line; a value that wraps onto the next line is not seen as one.

use legatorocr_core::pipeline::{Page, PageImage};
use legatorocr_core::text::Word;
use regex::Regex;
use std::sync::Mutex;

static PATTERNS: Mutex<Vec<Regex>> = Mutex::new(Vec::new());
/// What was covered per pattern, and what still looks like personal data afterwards.
static STATS: Mutex<Stats> = Mutex::new(Stats::new());

struct Stats {
    labels: Vec<String>,
    hits: Vec<usize>,
    /// (page number, kind) of leftovers that look like an address or a phone number
    suspects: Vec<(usize, &'static str)>,
    pages: usize,
}

impl Stats {
    const fn new() -> Self {
        Stats {
            labels: Vec::new(),
            hits: Vec::new(),
            suspects: Vec::new(),
            pages: 0,
        }
    }
}

pub fn set(patterns: Vec<Regex>) {
    *PATTERNS.lock().unwrap() = patterns;
}

/// Names the patterns in the closing report (same order as passed to `set`).
pub fn set_labels(labels: Vec<String>) {
    let mut st = STATS.lock().unwrap();
    st.hits = vec![0; labels.len()];
    st.labels = labels;
    st.suspects.clear();
    st.pages = 0;
}

/// Something left in the text that looks like an e-mail or web address or a phone number: a
/// word with a domain ending, or seven or more digits. OCR often reads `@` as `G`, `(a` or `©`, so
/// the address pattern then matches nothing while the address is still on the page.
fn looks_personal(word: &str) -> Option<&'static str> {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric());
    let digits = w.chars().filter(|c| c.is_ascii_digit()).count();
    let lower = w.to_lowercase();
    let tld = [
        ".com", ".ro", ".eu", ".org", ".net", ".gov", ".edu", ".info",
    ]
    .iter()
    .any(|t| lower.ends_with(t) && lower.len() > t.len() + 3);
    if tld {
        return Some("address-like word");
    }
    if digits >= 7 && digits * 2 >= w.chars().count() {
        return Some("long number");
    }
    None
}

/// Closing report for the command line: what each pattern covered and what may be left.
/// `None` when redaction was not used.
pub fn report() -> Option<String> {
    let st = STATS.lock().unwrap();
    if st.labels.is_empty() {
        return None;
    }
    let mut out = String::from("redaction:");
    for (l, h) in st.labels.iter().zip(&st.hits) {
        out.push_str(&format!(" {l}: {h} covered;"));
    }
    out.push('\n');
    for (l, h) in st.labels.iter().zip(&st.hits) {
        if *h == 0 {
            out.push_str(&format!(
                "warning: {l} matched nothing in {} page(s). OCR may have misread the value (an \"@\" is often read as \"G\" or \"(a\"): look at the pages yourself before you hand the files over.\n",
                st.pages
            ));
        }
    }
    if !st.suspects.is_empty() {
        let mut by: Vec<String> = Vec::new();
        for (pg, kind) in &st.suspects {
            let t = format!("page {pg}: {kind}");
            if !by.contains(&t) {
                by.push(t);
            }
        }
        out.push_str(&format!(
            "warning: {} word(s) left in the text still look like personal data ({}). They were NOT covered; check them by eye.\n",
            st.suspects.len(),
            by.join(", ")
        ));
    }
    Some(out)
}

/// Patterns from preset names and user regular expressions.
pub fn compile(presets: &[String], regexes: &[String]) -> anyhow::Result<Vec<Regex>> {
    use anyhow::Context;
    let mut v = Vec::new();
    for n in presets {
        v.push(Regex::new(crate::search::preset(n.trim())?)?);
    }
    for r in regexes {
        v.push(Regex::new(r).with_context(|| format!("--redact-regex {r:?}"))?);
    }
    Ok(v)
}

pub fn enabled() -> bool {
    !PATTERNS.lock().unwrap().is_empty()
}

/// Indices of words covered by a match of any pattern (matching each text line).
pub fn find(words: &[Word], pats: &[Regex]) -> Vec<usize> {
    let mut lines: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        match lines.iter_mut().find(|l| l.0 == w.line) {
            Some(l) => l.1.push(i),
            None => lines.push((w.line, vec![i])),
        }
    }
    let mut hit = vec![false; words.len()];
    for (_, idx) in &lines {
        let mut text = String::new();
        let mut spans = Vec::new();
        for &i in idx {
            if !text.is_empty() {
                text.push(' ');
            }
            let s = text.len();
            text.push_str(&words[i].text);
            spans.push((s, text.len(), i));
        }
        for (pi, re) in pats.iter().enumerate() {
            for m in re.find_iter(&text) {
                let mut any = false;
                for &(s, e, i) in &spans {
                    if s < m.end() && e > m.start() {
                        hit[i] = true;
                        any = true;
                    }
                }
                if any {
                    if let Some(h) = STATS.lock().unwrap().hits.get_mut(pi) {
                        *h += 1;
                    }
                }
            }
        }
    }
    hit.iter()
        .enumerate()
        .filter(|(_, h)| **h)
        .map(|(i, _)| i)
        .collect()
}

fn paint(img: &mut PageImage, x: u32, y: u32, w: u32, h: u32) {
    let (iw, ih) = (img.width(), img.height());
    let (x1, y1) = ((x + w).min(iw), (y + h).min(ih));
    for yy in y.min(ih)..y1 {
        for xx in x.min(iw)..x1 {
            match img {
                PageImage::Rgb(i) => i.put_pixel(xx, yy, image::Rgb([0, 0, 0])),
                PageImage::Gray(i) => i.put_pixel(xx, yy, image::Luma([0])),
                PageImage::Bw(i) => i.set(xx, yy, true),
            }
        }
    }
}

/// Cover and remove the matching words; returns how many.
pub fn apply(page: &mut Page, words: &mut Vec<Word>) -> usize {
    let pats = PATTERNS.lock().unwrap().clone();
    if pats.is_empty() {
        return 0;
    }
    let hit = find(words, &pats);
    {
        let mut st = STATS.lock().unwrap();
        st.pages += 1;
        let page_no = st.pages;
        let hs: std::collections::HashSet<usize> = hit.iter().copied().collect();
        for (i, w) in words.iter().enumerate() {
            if !hs.contains(&i) {
                if let Some(kind) = looks_personal(&w.text) {
                    st.suspects.push((page_no, kind));
                }
            }
        }
    }
    if hit.is_empty() {
        return 0;
    }
    page.layers = None;
    let pad = 2;
    for &i in &hit {
        let w = &words[i];
        paint(
            &mut page.image,
            w.x.saturating_sub(pad),
            w.y.saturating_sub(pad),
            w.w + 2 * pad,
            w.h + 2 * pad,
        );
    }
    let set: std::collections::HashSet<usize> = hit.iter().copied().collect();
    let mut i = 0;
    words.retain(|_| {
        let keep = !set.contains(&i);
        i += 1;
        keep
    });
    hit.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use legatorocr_core::bitimage::BitImage;

    fn w(t: &str, x: u32, line: u32) -> Word {
        Word {
            text: t.into(),
            x,
            y: 10,
            w: 40,
            h: 12,
            conf: 90.0,
            line,
        }
    }

    #[test]
    fn matches_across_words_of_a_line_only() {
        let words = vec![
            w("Telefon", 0, 0),
            w("0722", 50, 0),
            w("123", 100, 0),
            w("456", 150, 0),
            w("0722", 0, 1),
        ];
        let re = Regex::new(crate::search::preset("phone-ro").unwrap()).unwrap();
        // "0722 123 456" is one phone number over three words; the lone "0722" on line 1 is not
        assert_eq!(find(&words, &[re]), vec![1, 2, 3]);
    }

    #[test]
    fn pixels_and_text_are_removed() {
        let mut bw = BitImage::new(300, 60);
        // text-like ink only under the word that will be covered
        for x in 60..90 {
            for y in 12..20 {
                bw.set(x, y, false);
            }
        }
        let mut page = Page {
            image: PageImage::Bw(bw),
            dpi: 300,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        };
        let mut words = vec![w("ok", 0, 0), w("a@b.ro", 50, 0)];
        set(vec![
            Regex::new(crate::search::preset("email").unwrap()).unwrap()
        ]);
        let n = apply(&mut page, &mut words);
        set(vec![]);
        assert_eq!(n, 1);
        assert_eq!(words.len(), 1);
        let PageImage::Bw(b) = &page.image else {
            panic!()
        };
        assert!(b.get(70, 15) && b.get(48, 8) && !b.get(5, 15));
        // disabled: nothing happens
        assert_eq!(apply(&mut page, &mut words), 0);
    }

    #[test]
    fn leftovers_that_look_personal_are_recognised() {
        assert_eq!(
            looks_personal("ion.popescu(a)example.com"),
            Some("address-like word")
        );
        assert_eq!(looks_personal("0721456789"), Some("long number"));
        assert_eq!(looks_personal("Bucuresti,"), None);
        assert_eq!(looks_personal("2026"), None);
        assert_eq!(looks_personal("a.ro"), None);
    }
}
