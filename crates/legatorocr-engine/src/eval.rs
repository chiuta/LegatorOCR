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

/// Levenshtein distance: two rows, O(n·m) time and O(m) memory for ordinary texts. For very long
/// ones (more than 4·10⁸ cells) only a diagonal band around the alignment is computed (O(n·k),
/// k grown from |n − m| + 64 up to |n − m| + 4096 until the distance fits): exact whenever the
/// true distance is within the final band, and larger than the band width otherwise (so a
/// hopeless comparison still reads as "very different").
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if (a.len() as u128) * (b.len() as u128) > 400_000_000 {
        // widen the band until the distance fits inside it (exact), up to a generous limit
        let diff = a.len().abs_diff(b.len());
        let mut k = diff + 64;
        loop {
            let d = edit_distance_banded(a, b, k);
            if d <= k || k >= diff + 4096 {
                return d;
            }
            k = (k * 4).min(diff + 4096);
        }
    }
    edit_distance_full(a, b)
}

/// Ukkonen-style banded Levenshtein: cells farther than `k` from the main diagonal are treated
/// as unreachable. Exact when the true distance is ≤ `k`; otherwise the result is > `k`.
fn edit_distance_banded<T: PartialEq>(a: &[T], b: &[T], k: usize) -> usize {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > k {
        return n.abs_diff(m);
    }
    let inf = usize::MAX / 2;
    let mut prev = vec![inf; m + 1];
    let mut cur = vec![inf; m + 1];
    for (j, p) in prev.iter_mut().enumerate().take(k.min(m) + 1) {
        *p = j;
    }
    for i in 1..=n {
        let (lo, hi) = (i.saturating_sub(k), (i + k).min(m));
        cur[lo.saturating_sub(1)..=hi].iter_mut().for_each(|c| *c = inf);
        if lo == 0 {
            cur[0] = i;
        }
        for j in lo.max(1)..=hi {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j].saturating_add(1)).min(cur[j - 1].saturating_add(1));
        }
        std::mem::swap(&mut prev, &mut cur);
        if hi < m {
            prev[hi + 1] = inf;
        }
    }
    prev[m]
}

fn edit_distance_full<T: PartialEq>(a: &[T], b: &[T]) -> usize {
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
    fn banded_distance_agrees_with_the_full_one_and_stays_fast_on_huge_texts() {
        let a: Vec<u8> = (0..3000u32).map(|i| (i * 7 % 26) as u8).collect();
        let mut b = a.clone();
        for i in (0..b.len()).step_by(97) {
            b[i] = 99; // 31 substitutions
        }
        b.insert(500, 1);
        b.remove(2000);
        let full = edit_distance_full(&a, &b);
        assert_eq!(edit_distance_banded(&a, &b, 200), full);
        // a million-character pair is compared in well under the time the full table would need
        let big: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        let mut big2 = big.clone();
        big2[1234] = 255;
        let t = std::time::Instant::now();
        assert_eq!(edit_distance(&big, &big2), 1);
        assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
    }

    #[test]
    fn distance_basics() {
        assert_eq!(edit_distance(&[1, 2, 3], &[1, 3]), 1);
        assert_eq!(edit_distance::<u8>(&[], &[]), 0);
        assert_eq!(edit_distance(b"kitten", b"sitting"), 3);
    }
}
