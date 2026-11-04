//! `--bookmarks-headings`: one PDF bookmark per page whose top part carries a heading, taken
//! from the OCR words. A heading line is one whose words are clearly taller than the page's
//! typical word (at least 1.4 times the median), short (at most 10 words), and in the upper
//! 40 % of the page; the highest such line wins. A heuristic with no reference to measure
//! against: pages without a recognisable heading simply get no bookmark.

use legatorocr_core::Word;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The heading of a page, if it has one.
pub fn detect(words: &[Word], page_h: u32) -> Option<String> {
    detect_sized(words, page_h).map(|d| d.0)
}

/// The heading and how many times taller its letters are than the page's typical word.
pub fn detect_sized(words: &[Word], page_h: u32) -> Option<(String, f32)> {
    detect_line(words, page_h).map(|d| (d.0, d.2))
}

/// Like [`detect_sized`], also giving the `line` id of the heading line.
pub fn detect_line(words: &[Word], page_h: u32) -> Option<(String, u32, f32)> {
    if words.len() < 4 || page_h == 0 {
        return None;
    }
    let mut hs: Vec<u32> = words.iter().map(|w| w.h).collect();
    hs.sort_unstable();
    let med = hs[hs.len() / 2].max(1);
    let mut lines: Vec<u32> = words.iter().map(|w| w.line).collect();
    lines.sort_unstable();
    lines.dedup();
    let mut best: Option<(u32, String, f32, u32)> = None;
    for l in lines {
        let ws: Vec<&Word> = words.iter().filter(|w| w.line == l).collect();
        let y = ws.iter().map(|w| w.y).min()?;
        if ws.len() > 10 || y > page_h * 2 / 5 {
            continue;
        }
        let tall = ws.iter().filter(|w| w.h * 10 >= med * 14).count();
        if tall * 4 < ws.len() * 3 {
            continue;
        }
        let mut ws = ws;
        ws.sort_by_key(|w| w.x);
        let text = ws
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let letters = text.chars().filter(|c| c.is_alphabetic()).count();
        if letters < 3 {
            continue; // a lone number or ornament is not a heading
        }
        if best.as_ref().is_none_or(|b| y < b.0) {
            let tall_h = ws.iter().map(|w| w.h).max().unwrap_or(1) as f32;
            best = Some((y, text.chars().take(120).collect(), tall_h / med as f32, l));
        }
    }
    best.map(|b| (b.1, b.3, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, x: u32, y: u32, h: u32, line: u32) -> Word {
        Word {
            text: t.into(),
            x,
            y,
            w: 40,
            h,
            conf: 90.0,
            line,
        }
    }

    fn body(n: u32, start_line: u32) -> Vec<Word> {
        (0..n)
            .map(|i| w("text", 100 + i * 60, 400 + start_line * 40, 20, start_line))
            .collect()
    }

    #[test]
    fn a_tall_short_line_near_the_top_is_the_heading() {
        let mut v = vec![w("Capitolul", 100, 100, 44, 0), w("întâi", 300, 100, 44, 0)];
        v.extend(body(8, 1));
        v.extend(body(8, 2));
        assert_eq!(detect(&v, 1600).as_deref(), Some("Capitolul întâi"));
    }

    #[test]
    fn the_size_ratio_is_reported() {
        let mut v = vec![w("Capitolul", 100, 100, 40, 0), w("întâi", 300, 100, 40, 0)];
        v.extend(body(8, 1));
        v.extend(body(8, 2));
        let (t, r) = detect_sized(&v, 1600).unwrap();
        assert_eq!(t, "Capitolul întâi");
        assert!((r - 2.0).abs() < 0.01, "{r}");
    }

    #[test]
    fn plain_pages_numbers_and_low_lines_give_nothing() {
        let mut plain = body(8, 0);
        plain.extend(body(8, 1));
        assert_eq!(detect(&plain, 1600), None);
        let mut num = vec![w("17", 700, 60, 44, 0)];
        num.extend(body(8, 1));
        assert_eq!(detect(&num, 1600), None, "a page number is not a heading");
        let mut low = vec![w("Titlu", 100, 1200, 44, 0), w("jos", 300, 1200, 44, 0)];
        low.extend(body(8, 1));
        assert_eq!(
            detect(&low, 1600),
            None,
            "a tall line low on the page is not a heading"
        );
        assert_eq!(detect(&[], 1600), None);
    }
}
