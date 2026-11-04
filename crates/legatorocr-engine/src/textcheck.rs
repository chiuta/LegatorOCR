//! Self-test of the text layer: read the finished PDF back and compare its text with the
//! text OCR produced. Catches a layer that is missing, empty or garbled by the font/encoding
//! (so that copy and search would fail), without needing any outside tool.

use std::collections::HashMap;

/// Result of reading a PDF's text layer back.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub pages: usize,
    pub pages_with_text: usize,
    /// Words OCR reported.
    pub expected: usize,
    /// Of those, how many were found again in the PDF (multiset overlap).
    pub found: usize,
}

impl Report {
    pub fn ratio(&self) -> f64 {
        if self.expected == 0 {
            1.0
        } else {
            self.found as f64 / self.expected as f64
        }
    }
    /// One line for the user; the bool is `true` when the layer looks sound (≥ 95 %).
    pub fn summary(&self) -> (bool, String) {
        let ok = self.ratio() >= 0.95;
        (
            ok,
            format!(
                "text layer check: {}/{} words from OCR found again in the PDF ({:.1} %), {}/{} pages carry text{}",
                self.found,
                self.expected,
                self.ratio() * 100.0,
                self.pages_with_text,
                self.pages,
                if ok { "" } else { " — copy/search may fail" }
            ),
        )
    }
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Compare `ocr_text` (pages separated by form feed or not) with what the PDF yields.
pub fn check(pdf: &[u8], ocr_text: &str) -> Result<Report, String> {
    let doc = legatorocr_pdf::reader::PdfDoc::open(pdf.to_vec()).map_err(|e| e.to_string())?;
    let mut have: HashMap<String, usize> = HashMap::new();
    let mut with_text = 0;
    for i in 0..doc.page_count() {
        if let Ok(t) = doc.page_text(i) {
            let toks = tokens(&t);
            if !toks.is_empty() {
                with_text += 1;
            }
            for w in toks {
                *have.entry(w).or_insert(0) += 1;
            }
        }
    }
    let want = tokens(ocr_text);
    let mut found = 0;
    for w in &want {
        if let Some(n) = have.get_mut(w) {
            if *n > 0 {
                *n -= 1;
                found += 1;
            }
        }
    }
    Ok(Report {
        pages: doc.page_count(),
        pages_with_text: with_text,
        expected: want.len(),
        found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_ignore_case_and_punctuation() {
        assert_eq!(tokens("Școala, ȘI Casa!"), vec!["școala", "și", "casa"]);
    }

    #[test]
    fn empty_expectation_is_sound() {
        let r = Report {
            pages: 1,
            pages_with_text: 0,
            expected: 0,
            found: 0,
        };
        assert!(r.summary().0);
    }

    #[test]
    fn low_overlap_is_flagged() {
        let r = Report {
            pages: 2,
            pages_with_text: 1,
            expected: 100,
            found: 40,
        };
        assert!(!r.summary().0);
    }
}
