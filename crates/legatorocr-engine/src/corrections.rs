//! Put a hand-corrected text (from `--proof-sheet`, pages separated by a form feed) back onto
//! the recognised words of each page, so the PDF text layer, the plain text and hOCR carry the
//! corrections while every word keeps a box.
//!
//! The corrected page is compared with the recognised words, in the order the text output lists them, by a longest
//! common subsequence of whole tokens. Between two equal tokens the differing stretch of
//! `k` old words and `m` new tokens is resolved so: the first `min(k, m)` old words take the
//! new texts one by one; surplus old words are dropped; surplus new tokens share the box of the
//! last old word in proportion to their length; with no old word in the stretch the new tokens
//! take the box of the neighbouring word (the text layer is invisible, so overlap is harmless).
//! This is a heuristic: a box is only as exact as the one-to-one pairing. A page whose corrected
//! text is empty is left as recognised, and pages beyond the corrected file are untouched.

use legatorocr_core::Word;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static PAGES: Mutex<Option<Vec<String>>> = Mutex::new(None);
static CHANGED: AtomicUsize = AtomicUsize::new(0);
static ADDED: AtomicUsize = AtomicUsize::new(0);
static REMOVED: AtomicUsize = AtomicUsize::new(0);

/// Largest `old × new` table the alignment will build (words per page squared); a bigger page
/// is left as recognised.
const MAX_CELLS: usize = 4_000_000;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Counts {
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
}

/// Load the corrected text; pages are separated by a form feed.
pub fn set(text: &str) {
    let mut pages: Vec<String> = text.split('\x0c').map(str::to_string).collect();
    // a trailing form feed after the last page is not another page
    if pages.len() > 1 && pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    *PAGES.lock().unwrap() = Some(pages);
}

pub fn enabled() -> bool {
    PAGES.lock().unwrap().is_some()
}

/// How many pages the loaded text has.
pub fn page_count() -> usize {
    PAGES.lock().unwrap().as_ref().map_or(0, Vec::len)
}

/// Totals over all pages so far.
pub fn totals() -> Counts {
    Counts {
        changed: CHANGED.load(Ordering::Relaxed),
        added: ADDED.load(Ordering::Relaxed),
        removed: REMOVED.load(Ordering::Relaxed),
    }
}

/// Apply the corrected text of page `index` (0-based) to its words.
pub fn apply(index: usize, words: Vec<Word>) -> Vec<Word> {
    let text = match PAGES.lock().unwrap().as_ref().and_then(|p| p.get(index)) {
        Some(t) => t.clone(),
        None => return words,
    };
    let (out, c) = align(words, &text);
    CHANGED.fetch_add(c.changed, Ordering::Relaxed);
    ADDED.fetch_add(c.added, Ordering::Relaxed);
    REMOVED.fetch_add(c.removed, Ordering::Relaxed);
    out
}

/// The alignment itself (pure).
pub fn align(words: Vec<Word>, corrected: &str) -> (Vec<Word>, Counts) {
    let new_t: Vec<&str> = corrected.split_whitespace().collect();
    if new_t.is_empty() || words.is_empty() {
        return (words, Counts::default());
    }
    // the text file lists the words in the order the engine produced them (`words_to_text`), so
    // that is the order to compare in, not a re-sorted one: with another order words that moved
    // would be dropped and added again
    let order: Vec<usize> = (0..words.len()).collect();
    let n = order.len();
    let m = new_t.len();
    if n.saturating_mul(m) > MAX_CELLS {
        return (words, Counts::default());
    }
    let old_t: Vec<&str> = order.iter().map(|&i| words[i].text.as_str()).collect();
    // longest common subsequence table, suffix form
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if old_t[i] == new_t[j] {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    let mut matches: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_t[i] == new_t[j] {
            matches.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matches.push((n, m)); // sentinel: the stretch after the last match

    // replacement words per old word (by original index); None = keep as it is
    let mut repl: Vec<Option<Vec<Word>>> = vec![None; words.len()];
    let mut counts = Counts::default();
    let (mut a, mut c) = (0usize, 0usize);
    for &(mi, mj) in &matches {
        let (k, mm) = (mi - a, mj - c);
        if k > 0 || mm > 0 {
            gap(
                &words,
                &order,
                a,
                k,
                &new_t[c..c + mm],
                &mut repl,
                &mut counts,
            );
        }
        a = mi + 1;
        c = mj + 1;
    }
    let mut out = Vec::with_capacity(words.len());
    for (idx, wd) in words.into_iter().enumerate() {
        match repl[idx].take() {
            None => out.push(wd),
            Some(list) => out.extend(list),
        }
    }
    (out, counts)
}

fn with_text(w: &Word, text: &str) -> Word {
    Word {
        text: text.to_string(),
        conf: 100.0,
        ..w.clone()
    }
}

/// Resolve one stretch: old words `order[a..a+k]`, new tokens `new`.
fn gap(
    words: &[Word],
    order: &[usize],
    a: usize,
    k: usize,
    new: &[&str],
    repl: &mut [Option<Vec<Word>>],
    counts: &mut Counts,
) {
    let m = new.len();
    if k == 0 {
        // pure insertion: use the neighbouring word's box (previous, else next)
        let anchor = if a > 0 { order[a - 1] } else { order[a] };
        let list = repl[anchor].get_or_insert_with(|| vec![words[anchor].clone()]);
        for t in new {
            list.push(with_text(&words[anchor], t));
        }
        counts.added += m;
        return;
    }
    let paired = k.min(m);
    for p in 0..paired {
        let idx = order[a + p];
        let is_last_paired_with_surplus = p + 1 == paired && m > k;
        if is_last_paired_with_surplus {
            // this old word takes the remaining tokens, sharing its box by length
            let toks = &new[p..];
            let total: usize = toks.iter().map(|t| t.chars().count().max(1)).sum();
            let wd = &words[idx];
            let (mut x, mut used) = (wd.x, 0usize);
            let mut list = Vec::with_capacity(toks.len());
            for (q, t) in toks.iter().enumerate() {
                let len = t.chars().count().max(1);
                used += len;
                let end = if q + 1 == toks.len() {
                    wd.x + wd.w
                } else {
                    wd.x + (wd.w as u64 * used as u64 / total as u64) as u32
                };
                let mut nw = with_text(wd, t);
                nw.x = x;
                nw.w = end.saturating_sub(x).max(1);
                x = end;
                list.push(nw);
            }
            counts.changed += 1;
            counts.added += toks.len() - 1;
            repl[idx] = Some(list);
        } else {
            counts.changed += 1;
            repl[idx] = Some(vec![with_text(&words[idx], new[p])]);
        }
    }
    for p in paired..k {
        repl[order[a + p]] = Some(Vec::new());
        counts.removed += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: u32, line: u32) -> Word {
        Word {
            text: text.into(),
            x,
            y: 10 + line * 40,
            w: 10 * text.chars().count() as u32,
            h: 30,
            conf: 50.0,
            line,
        }
    }

    fn page(texts: &[&str]) -> Vec<Word> {
        let mut x = 0;
        texts
            .iter()
            .map(|t| {
                let w = word(t, x, 0);
                x += w.w + 10;
                w
            })
            .collect()
    }

    fn texts(w: &[Word]) -> Vec<&str> {
        w.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn same_text_changes_nothing() {
        let w = page(&["a", "bb", "ccc"]);
        let (o, c) = align(w.clone(), "a bb\nccc  ");
        assert_eq!(o, w);
        assert_eq!(c, Counts::default());
    }

    #[test]
    fn substitution_keeps_the_box_and_marks_it_checked() {
        let w = page(&["cartea", "esle", "buna"]);
        let (o, c) = align(w.clone(), "cartea este buna");
        assert_eq!(texts(&o), ["cartea", "este", "buna"]);
        assert_eq!(
            (o[1].x, o[1].y, o[1].w, o[1].h),
            (w[1].x, w[1].y, w[1].w, w[1].h)
        );
        assert_eq!(o[1].conf, 100.0);
        assert_eq!(o[0].conf, 50.0, "an untouched word keeps its confidence");
        assert_eq!(
            c,
            Counts {
                changed: 1,
                added: 0,
                removed: 0
            }
        );
    }

    #[test]
    fn deleted_word_is_dropped() {
        let (o, c) = align(page(&["un", "zgomot", "text"]), "un text");
        assert_eq!(texts(&o), ["un", "text"]);
        assert_eq!(
            c,
            Counts {
                changed: 0,
                added: 0,
                removed: 1
            }
        );
    }

    #[test]
    fn inserted_word_takes_the_neighbour_box() {
        let w = page(&["un", "text"]);
        let (o, c) = align(w.clone(), "un mic text");
        assert_eq!(texts(&o), ["un", "mic", "text"]);
        assert_eq!((o[1].x, o[1].w), (w[0].x, w[0].w));
        assert_eq!(
            c,
            Counts {
                changed: 0,
                added: 1,
                removed: 0
            }
        );
    }

    #[test]
    fn one_word_split_in_two_shares_its_box() {
        let w = page(&["unmic", "text"]);
        let (o, c) = align(w.clone(), "un mic text");
        assert_eq!(texts(&o), ["un", "mic", "text"]);
        assert_eq!(o[0].x, w[0].x);
        assert_eq!(o[0].x + o[0].w, o[1].x, "the pieces touch");
        assert_eq!(o[1].x + o[1].w, w[0].x + w[0].w, "and fill the old box");
        assert!(o[0].w < o[1].w, "shared by length: 2 letters against 3");
        assert_eq!(
            c,
            Counts {
                changed: 1,
                added: 1,
                removed: 0
            }
        );
    }

    #[test]
    fn compares_in_the_order_the_text_output_lists_the_words() {
        // vector order a, b, c although `c` sits on line 0 to the right of `a`: the text file reads
        // "a b c", so nothing may change (a sort by line and x would drop and re-add words)
        let w = vec![word("a", 0, 0), word("b", 0, 1), word("c", 50, 0)];
        let (o, c) = align(w.clone(), "a b c");
        assert_eq!(o, w);
        assert_eq!(c, Counts::default());
        let (o, c) = align(vec![word("doi", 100, 0), word("unu", 0, 0)], "doi 2");
        assert_eq!(texts(&o), ["doi", "2"]);
        assert_eq!(c.changed, 1);
    }

    #[test]
    fn empty_or_missing_corrections_leave_the_page() {
        let w = page(&["a", "b"]);
        assert_eq!(align(w.clone(), "  \n ").0, w);
        assert_eq!(align(Vec::new(), "x y").0, Vec::new());
        set("a b\x0cc d\x0c");
        assert_eq!(
            page_count(),
            2,
            "a form feed after the last page is not a page"
        );
        assert_eq!(apply(5, w.clone()), w, "a page past the file is untouched");
        *PAGES.lock().unwrap() = None;
    }
}
