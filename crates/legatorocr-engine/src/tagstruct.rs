//! `--tagged-pdf`: the paragraph / heading plan for one page's structure tree. Paragraphs are
//! the OCR paragraphs (`legatorocr_ocr::paragraph_word_ranges`, the split `--passages-jsonl` and
//! hOCR use); the page's heading (the `--bookmarks-headings` detector) becomes its own element
//! `/H1`..`/H3` by how much taller its letters are than the body text. Heuristics with no
//! reference to measure against: nothing here is a conformance claim.

use legatorocr_core::Word;
use legatorocr_pdf::{StructKind, StructPara};

/// Heading level from the size ratio to the body text: 2.0 and up `H1`, 1.7 and up `H2`, else `H3`.
pub fn heading_level(ratio: f32) -> u8 {
    if ratio >= 2.0 {
        1
    } else if ratio >= 1.7 {
        2
    } else {
        3
    }
}

/// BCP 47 tag for the document language from a Tesseract language string (`ron+eng` -> `ro`,
/// the first language). Unknown three-letter codes are passed through unchanged.
pub fn bcp47(tess_lang: &str) -> String {
    let first = tess_lang.split(['+', ',']).next().unwrap_or("").trim();
    let code = match first {
        "ron" | "rum" => "ro",
        "eng" => "en",
        "fra" | "fre" => "fr",
        "deu" | "ger" => "de",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "hun" => "hu",
        "bul" => "bg",
        "rus" => "ru",
        "ukr" => "uk",
        "pol" => "pl",
        "nld" | "dut" => "nl",
        "ces" | "cze" => "cs",
        "slk" | "slo" => "sk",
        "srp" => "sr",
        "hrv" => "hr",
        "ell" | "gre" => "el",
        "tur" => "tr",
        "lat" => "la",
        other => other,
    };
    code.to_string()
}

/// Structure elements of a page, in reading order (the order of `words`).
pub fn plan(words: &[Word], page_h: u32) -> Vec<StructPara> {
    let ranges = legatorocr_ocr::paragraph_word_ranges(words);
    let head = crate::headings::detect_line(words, page_h);
    let mut out = Vec::new();
    for (a, b) in ranges {
        let Some((_, line, ratio)) = head
            .as_ref()
            .filter(|h| words[a..b].iter().any(|w| w.line == h.1))
        else {
            out.push(StructPara {
                kind: StructKind::Para,
                first: a,
                end: b,
            });
            continue;
        };
        // split the paragraph around the heading line
        let s = a + words[a..b]
            .iter()
            .position(|w| w.line == *line)
            .unwrap_or(0);
        let e = s + words[s..b].iter().take_while(|w| w.line == *line).count();
        if s > a {
            out.push(StructPara {
                kind: StructKind::Para,
                first: a,
                end: s,
            });
        }
        out.push(StructPara {
            kind: StructKind::Heading(heading_level(*ratio)),
            first: s,
            end: e,
        });
        if e < b {
            out.push(StructPara {
                kind: StructKind::Para,
                first: e,
                end: b,
            });
        }
    }
    out
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

    /// Heading line, then a 3-line paragraph, a gap, a 2-line paragraph (synthetic).
    fn page() -> Vec<Word> {
        let mut v = vec![w("Capitolul", 100, 100, 44, 0), w("întâi", 300, 100, 44, 0)];
        for (line, y) in [(1u32, 400u32), (2, 424), (3, 448), (4, 560), (5, 584)] {
            for i in 0..8 {
                v.push(w("text", 100 + i * 60, y, 20, line));
            }
        }
        v
    }

    #[test]
    fn heading_and_paragraphs_in_reading_order() {
        let v = page();
        let p = plan(&v, 1600);
        let kinds: Vec<_> = p.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![StructKind::Heading(1), StructKind::Para, StructKind::Para]
        );
        // contiguous, ordered, covering every word
        assert_eq!(p[0].first, 0);
        for k in 1..p.len() {
            assert_eq!(p[k].first, p[k - 1].end);
        }
        assert_eq!(p.last().unwrap().end, v.len());
        assert_eq!(
            p[1].end - p[1].first,
            24,
            "three lines in the first paragraph"
        );
    }

    #[test]
    fn language_tag_from_tesseract_codes() {
        assert_eq!(bcp47("ron+eng"), "ro");
        assert_eq!(bcp47("fra"), "fr");
        assert_eq!(bcp47("xyz"), "xyz");
    }

    #[test]
    fn levels_follow_the_size_ratio() {
        assert_eq!(
            (heading_level(2.4), heading_level(1.8), heading_level(1.45)),
            (1, 2, 3)
        );
    }
}
