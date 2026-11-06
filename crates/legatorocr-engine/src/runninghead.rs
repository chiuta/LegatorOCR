//! Running heads, running feet and printed page numbers in the recognised text of a batch of
//! pages: the first and the last non-empty line of a page that look the same (digits ignored,
//! case ignored) on at least half of the pages, and at least four pages, are page furniture, not
//! text. `strip` removes them from text whose pages are separated by form feeds and tells which
//! lines it took out (nothing is hidden: the CLI lists them).

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// `--drop-running-heads`
pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

fn key(line: &str) -> String {
    let k: String = line
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .collect();
    // a page number with a dot or a dash around it is still just a number
    k.trim_matches(|c: char| !c.is_alphanumeric() && c != '#')
        .to_string()
}

/// Text of the pages without their running heads/feet, and the removed lines (page, text).
pub fn strip(text: &str) -> (String, Vec<(usize, String)>) {
    let pages: Vec<&str> = text.split('\x0c').collect();
    let n = pages.iter().filter(|p| !p.trim().is_empty()).count();
    if n < 4 {
        return (text.to_string(), Vec::new());
    }
    // candidate lines: first and last non-empty line of each page, not longer than 70 characters
    let edge = |p: &str| -> (Option<usize>, Option<usize>) {
        let lines: Vec<&str> = p.split('\n').collect();
        let first = lines.iter().position(|l| !l.trim().is_empty());
        let last = lines.iter().rposition(|l| !l.trim().is_empty());
        (first, last)
    };
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in &pages {
        let lines: Vec<&str> = p.split('\n').collect();
        let (f, l) = edge(p);
        let mut seen = std::collections::HashSet::new();
        for i in [f, l].into_iter().flatten() {
            let t = lines[i].trim();
            if t.chars().count() <= 70 {
                let k = key(t);
                if !k.is_empty() && seen.insert(k.clone()) {
                    *count.entry(k).or_insert(0) += 1;
                }
            }
        }
    }
    let need = n.div_ceil(2).max(4);
    let furniture: std::collections::HashSet<&String> = count
        .iter()
        .filter(|&(_, &c)| c >= need)
        .map(|(k, _)| k)
        .collect();
    if furniture.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let mut removed = Vec::new();
    let mut out_pages: Vec<String> = Vec::with_capacity(pages.len());
    for (pi, p) in pages.iter().enumerate() {
        let mut lines: Vec<&str> = p.split('\n').collect();
        let (f, l) = edge(p);
        let mut drop = std::collections::BTreeSet::new();
        for i in [f, l].into_iter().flatten() {
            let t = lines[i].trim();
            if t.chars().count() <= 70 && furniture.contains(&key(t)) {
                drop.insert(i);
            }
        }
        for &i in drop.iter().rev() {
            removed.push((pi + 1, lines[i].trim().to_string()));
            lines.remove(i);
        }
        out_pages.push(lines.join("\n"));
    }
    removed.sort();
    (out_pages.join("\x0c"), removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(pages: usize) -> String {
        (1..=pages)
            .map(|i| {
                format!(
                    "TEHNOLOGIA MOBILEI\nText al paginii {i}, cu cuvinte.\nAlt rând de text.\n{}",
                    20 + i
                )
            })
            .collect::<Vec<_>>()
            .join("\x0c")
    }

    #[test]
    fn heads_and_page_numbers_that_repeat_are_removed() {
        let (t, gone) = strip(&book(6));
        assert!(!t.contains("TEHNOLOGIA"));
        assert!(!t.contains("\n21"));
        assert!(t.contains("Text al paginii 3"));
        assert_eq!(gone.len(), 12); // head and number on six pages
        assert_eq!(gone[0].0, 1);
    }

    #[test]
    fn a_short_batch_is_left_alone() {
        let b = book(3);
        let (t, gone) = strip(&b);
        assert_eq!(t, b);
        assert!(gone.is_empty());
    }

    #[test]
    fn a_line_that_is_only_sometimes_repeated_stays() {
        let mut pages: Vec<String> = (1..=8)
            .map(|i| format!("Capitol {i}\nText {i}\nfinal {i}x"))
            .collect();
        pages[2] = "Capitol 3\nText\nCapitol".into();
        let (_, gone) = strip(&pages.join("\x0c"));
        assert!(gone.iter().all(|(_, t)| !t.starts_with("Text")));
    }
}
