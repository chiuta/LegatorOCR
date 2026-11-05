//! `--compare-text A B --diff-html FILE.html`: word-by-word differences between two texts (two
//! editions of a book, an OCR output and a corrected text) as one self-contained HTML page:
//! words only in A struck out in red, words only in B underlined in green, the rest plain.
//! Case and ş/ţ against ș/ț do not count as differences. Method: patience diff (words that occur
//! exactly once in both texts anchor the alignment), the gaps between anchors by exact
//! longest-common-subsequence when small and as one replacement when large. The alignment is
//! *an* alignment, not always the shortest edit script.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Equal(usize, usize),
    Del(usize),
    Ins(usize),
}

fn key(w: &str) -> String {
    w.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'ş' => 'ș',
            'ţ' => 'ț',
            c => c,
        })
        .collect()
}

pub fn diff(a: &[String], b: &[String]) -> Vec<Op> {
    let ka: Vec<String> = a.iter().map(|w| key(w)).collect();
    let kb: Vec<String> = b.iter().map(|w| key(w)).collect();
    let mut out = Vec::new();
    rec(&ka, &kb, 0, ka.len(), 0, kb.len(), &mut out);
    out
}

fn rec(
    a: &[String],
    b: &[String],
    mut a0: usize,
    mut a1: usize,
    mut b0: usize,
    mut b1: usize,
    out: &mut Vec<Op>,
) {
    while a0 < a1 && b0 < b1 && a[a0] == b[b0] {
        out.push(Op::Equal(a0, b0));
        a0 += 1;
        b0 += 1;
    }
    let mut tail = Vec::new();
    while a0 < a1 && b0 < b1 && a[a1 - 1] == b[b1 - 1] {
        a1 -= 1;
        b1 -= 1;
        tail.push(Op::Equal(a1, b1));
    }
    if a0 == a1 {
        out.extend((b0..b1).map(Op::Ins));
    } else if b0 == b1 {
        out.extend((a0..a1).map(Op::Del));
    } else {
        let anchors = unique_anchors(a, b, a0, a1, b0, b1);
        if anchors.is_empty() {
            if (a1 - a0) * (b1 - b0) <= 4_000_000 {
                lcs(a, b, a0, a1, b0, b1, out);
            } else {
                out.extend((a0..a1).map(Op::Del));
                out.extend((b0..b1).map(Op::Ins));
            }
        } else {
            let (mut pa, mut pb) = (a0, b0);
            for (x, y) in anchors {
                rec(a, b, pa, x, pb, y, out);
                out.push(Op::Equal(x, y));
                pa = x + 1;
                pb = y + 1;
            }
            rec(a, b, pa, a1, pb, b1, out);
        }
    }
    out.extend(tail.into_iter().rev());
}

/// Words occurring exactly once in both ranges, kept in an increasing order in both (longest
/// increasing subsequence of their positions in `b`).
fn unique_anchors(
    a: &[String],
    b: &[String],
    a0: usize,
    a1: usize,
    b0: usize,
    b1: usize,
) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    let mut ca: HashMap<&str, (usize, usize)> = HashMap::new();
    for (i, w) in a[a0..a1].iter().enumerate() {
        let e = ca.entry(w.as_str()).or_insert((0, a0 + i));
        e.0 += 1;
    }
    let mut cb: HashMap<&str, (usize, usize)> = HashMap::new();
    for (i, w) in b[b0..b1].iter().enumerate() {
        let e = cb.entry(w.as_str()).or_insert((0, b0 + i));
        e.0 += 1;
    }
    let mut pairs: Vec<(usize, usize)> = ca
        .iter()
        .filter(|(_, v)| v.0 == 1)
        .filter_map(|(w, v)| cb.get(w).filter(|x| x.0 == 1).map(|x| (v.1, x.1)))
        .collect();
    pairs.sort();
    // LIS on the second coordinate
    let mut tails: Vec<usize> = Vec::new(); // index into pairs of the smallest tail of each length
    let mut prev = vec![usize::MAX; pairs.len()];
    for i in 0..pairs.len() {
        let p = tails.partition_point(|&t| pairs[t].1 < pairs[i].1);
        if p > 0 {
            prev[i] = tails[p - 1];
        }
        if p == tails.len() {
            tails.push(i);
        } else {
            tails[p] = i;
        }
    }
    let mut res = Vec::new();
    let mut k = tails.last().copied();
    while let Some(i) = k {
        res.push(pairs[i]);
        k = (prev[i] != usize::MAX).then_some(prev[i]);
    }
    res.reverse();
    res
}

fn lcs(a: &[String], b: &[String], a0: usize, a1: usize, b0: usize, b1: usize, out: &mut Vec<Op>) {
    let (n, m) = (a1 - a0, b1 - b0);
    let w = m + 1;
    let mut t = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * w + j] = if a[a0 + i] == b[b0 + j] {
                t[(i + 1) * w + j + 1] + 1
            } else {
                t[(i + 1) * w + j].max(t[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[a0 + i] == b[b0 + j] {
            out.push(Op::Equal(a0 + i, b0 + j));
            i += 1;
            j += 1;
        } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
            out.push(Op::Del(a0 + i));
            i += 1;
        } else {
            out.push(Op::Ins(b0 + j));
            j += 1;
        }
    }
    out.extend((i..n).map(|x| Op::Del(a0 + x)));
    out.extend((j..m).map(|x| Op::Ins(b0 + x)));
}

pub fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(|w| w.to_string()).collect()
}

pub struct Summary {
    pub same: usize,
    pub only_a: usize,
    pub only_b: usize,
}

pub fn summary(ops: &[Op]) -> Summary {
    let mut s = Summary {
        same: 0,
        only_a: 0,
        only_b: 0,
    };
    for o in ops {
        match o {
            Op::Equal(..) => s.same += 1,
            Op::Del(_) => s.only_a += 1,
            Op::Ins(_) => s.only_b += 1,
        }
    }
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Self-contained HTML page.
pub fn html(a: &[String], b: &[String], ops: &[Op], name_a: &str, name_b: &str) -> String {
    let s = summary(ops);
    let mut body = String::new();
    for o in ops {
        match *o {
            Op::Equal(_, j) => body.push_str(&format!("{} ", esc(&b[j]))),
            Op::Del(i) => body.push_str(&format!("<del>{}</del> ", esc(&a[i]))),
            Op::Ins(j) => body.push_str(&format!("<ins>{}</ins> ", esc(&b[j]))),
        }
    }
    format!(
        "<!doctype html><html lang=\"ro\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Diferențe</title>\
         <style>:root{{--bg:#fff;--fg:#222;--d:#b3261e;--i:#1b7a3a}}@media(prefers-color-scheme:dark){{:root{{--bg:#1c1c1c;--fg:#ddd;--d:#ff8a80;--i:#6fdc8c}}}}\
         body{{font:16px/1.6 Georgia,serif;max-width:46em;margin:1em auto;padding:0 1em;background:var(--bg);color:var(--fg)}}del{{color:var(--d);text-decoration:line-through}}ins{{color:var(--i);text-decoration:underline}}header{{font:14px system-ui,sans-serif;margin-bottom:1em}}</style></head><body>\
         <header><b>{}</b> (<del>șters</del>) față de <b>{}</b> (<ins>adăugat</ins>): {} cuvinte la fel, {} doar în prima, {} doar în a doua.</header><p>{}</p></body></html>\n",
        esc(name_a),
        esc(name_b),
        s.same,
        s.only_a,
        s.only_b,
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(a: &str, b: &str) -> (Vec<String>, Vec<String>, Vec<Op>) {
        let (x, y) = (words(a), words(b));
        let o = diff(&x, &y);
        (x, y, o)
    }

    #[test]
    fn identical_texts_have_no_difference_and_case_and_cedilla_do_not_count() {
        let (_, _, o) = d("Tot ce ştim Ţara", "tot ce știm țara");
        let s = summary(&o);
        assert_eq!((s.same, s.only_a, s.only_b), (4, 0, 0));
    }

    #[test]
    fn a_changed_word_is_one_deletion_and_one_insertion() {
        let (_, _, o) = d("a b c d", "a x c d");
        let s = summary(&o);
        assert_eq!((s.same, s.only_a, s.only_b), (3, 1, 1));
    }

    #[test]
    fn alignment_is_consistent_every_word_appears_once_in_order() {
        let (a, b, o) = d(
            "sînt pînă cînd merg acasă sînt",
            "sunt până când merg acasă azi sunt",
        );
        let (mut ia, mut ib) = (0, 0);
        for op in o {
            match op {
                Op::Equal(i, j) => {
                    assert_eq!((i, j), (ia, ib));
                    ia += 1;
                    ib += 1;
                }
                Op::Del(i) => {
                    assert_eq!(i, ia);
                    ia += 1;
                }
                Op::Ins(j) => {
                    assert_eq!(j, ib);
                    ib += 1;
                }
            }
        }
        assert_eq!((ia, ib), (a.len(), b.len()));
    }

    #[test]
    fn a_long_text_with_few_edits_is_fast_and_finds_them() {
        let a: Vec<String> = (0..200_000).map(|i| format!("w{}", i % 5000)).collect();
        let mut b = a.clone();
        b[1000] = "xx".into();
        b.remove(50_000);
        b.insert(150_000, "yy".into());
        let t = std::time::Instant::now();
        let o = diff(&a, &b);
        let s = summary(&o);
        assert!(t.elapsed().as_secs() < 20, "too slow: {:?}", t.elapsed());
        // repeated words make anchors rare; the alignment must still account for every word
        assert_eq!(s.same + s.only_a, a.len());
        assert_eq!(s.same + s.only_b, b.len());
    }

    #[test]
    fn html_is_escaped_and_marks_both_sides() {
        let (a, b, o) = d("<x> a", "a & b");
        let h = html(&a, &b, &o, "A<1>", "B");
        assert!(
            h.contains("<del>&lt;x&gt;</del>")
                && h.contains("<ins>b</ins>")
                && h.contains("A&lt;1&gt;")
        );
        assert!(!h.contains("http"));
    }
}
