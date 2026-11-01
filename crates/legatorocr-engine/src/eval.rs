//! Character and word error rates for judging OCR output against a reference text.
//!
//! Both texts are normalised first (Unicode composed, whitespace collapsed to single
//! spaces, `ş`/`ţ` mapped to `ș`/`ț`), so only real recognition errors are counted.
//! CER = edit distance over characters ÷ reference length; WER likewise over words.

use legatorocr_core::text::normalize_ro;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Character error rate, 0.0 = identical (may exceed 1.0).
    pub cer: f64,
    /// Word error rate.
    pub wer: f64,
    pub ref_chars: usize,
    pub ref_words: usize,
}

fn norm(s: &str) -> String {
    normalize_ro(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Levenshtein distance, two rows, O(n·m) time and O(m) memory.
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn rates(reference: &str, hypothesis: &str) -> Rates {
    let (r, h) = (norm(reference), norm(hypothesis));
    let (rc, hc): (Vec<char>, Vec<char>) = (r.chars().collect(), h.chars().collect());
    let (rw, hw): (Vec<&str>, Vec<&str>) = (r.split(' ').collect(), h.split(' ').collect());
    let (rw, hw) = if r.is_empty() { (vec![], hw) } else { (rw, hw) };
    let hw = if h.is_empty() { vec![] } else { hw };
    let div = |d: usize, n: usize| {
        if n == 0 {
            if d == 0 {
                0.0
            } else {
                1.0
            }
        } else {
            d as f64 / n as f64
        }
    };
    Rates {
        cer: div(edit_distance(&rc, &hc), rc.len()),
        wer: div(edit_distance(&rw, &hw), rw.len()),
        ref_chars: rc.len(),
        ref_words: rw.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_and_normalised_texts_score_zero() {
        let r = rates(
            "Știința  lemnului\nși a mobilei",
            "știința lemnului și a mobilei".replace('ș', "ş").as_str(),
        );
        // Different case counts as an error; whitespace and cedilla do not.
        assert!(r.cer > 0.0 && r.cer < 0.1);
        let z = rates("Mobilă ştiinţă", "Mobilă știință");
        assert_eq!((z.cer, z.wer), (0.0, 0.0));
    }

    #[test]
    fn counts_substitutions_insertions_deletions() {
        let r = rates("abcd", "abxd");
        assert!((r.cer - 0.25).abs() < 1e-9);
        let r = rates("un doi trei patru", "un doi trei");
        assert!((r.wer - 0.25).abs() < 1e-9);
        assert_eq!(rates("", "").cer, 0.0);
        assert_eq!(rates("", "x").cer, 1.0);
        assert_eq!(rates("abc", "").cer, 1.0);
    }

    #[test]
    fn distance_basics() {
        assert_eq!(edit_distance(&[1, 2, 3], &[1, 3]), 1);
        assert_eq!(edit_distance::<u8>(&[], &[]), 0);
        assert_eq!(edit_distance(b"kitten", b"sitting"), 3);
    }
}
