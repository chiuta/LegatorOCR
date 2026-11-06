//! `--find-replace FILE`: the user's own corrections for recurring OCR mistakes
//! (a surname the word list does not know, a stamp, a wrongly read abbreviation).
//!
//! One rule per line, `wrong => right`; empty lines and lines starting with `#` are ignored.
//! A rule matches whole words only (letters, digits and `-` inside a word count as part of it),
//! so `Ion => Ioan` does not touch `Ionescu`. Rules run in file order on the `.txt`/`.docx` text,
//! after `--clean-text`; the PDF text layer, hOCR and JSON keep the raw words.

use std::sync::Mutex;

static RULES: Mutex<Vec<(Vec<char>, String)>> = Mutex::new(Vec::new());

/// Parse a rules file; returns the number of rules read.
pub fn parse(src: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let l = raw.trim_start_matches('\u{feff}').trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let Some((a, b)) = l.split_once("=>") else {
            return Err(format!("line {}: expected `wrong => right`", i + 1));
        };
        let (a, b) = (a.trim(), b.trim());
        if a.is_empty() {
            return Err(format!("line {}: the left side is empty", i + 1));
        }
        out.push((a.to_string(), b.to_string()));
    }
    Ok(out)
}

pub fn set_rules(rules: Vec<(String, String)>) {
    // An empty left side would match everywhere (and index out of range): drop it.
    *RULES.lock().unwrap_or_else(|e| e.into_inner()) = rules
        .into_iter()
        .filter(|(a, _)| !a.is_empty())
        .map(|(a, b)| (a.chars().collect(), b))
        .collect();
}

pub fn enabled() -> bool {
    !RULES.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

fn wordish(c: char) -> bool {
    c.is_alphanumeric() || c == '-'
}

/// Apply every rule; returns the new text and how many replacements were made.
pub fn apply(text: &str) -> (String, usize) {
    let rules = RULES.lock().unwrap_or_else(|e| e.into_inner());
    let mut cur: Vec<char> = text.chars().collect();
    let mut n = 0;
    for (from, to) in rules.iter() {
        let mut out = Vec::with_capacity(cur.len());
        let mut i = 0;
        while i < cur.len() {
            let end = i + from.len();
            let hit = end <= cur.len()
                && cur[i..end] == from[..]
                && (i == 0 || !wordish(cur[i - 1]) || !wordish(from[0]))
                && (end == cur.len() || !wordish(cur[end]) || !wordish(from[from.len() - 1]));
            if hit {
                out.extend(to.chars());
                i = end;
                n += 1;
            } else {
                out.push(cur[i]);
                i += 1;
            }
        }
        cur = out;
    }
    (cur.into_iter().collect(), n)
}

#[cfg(test)]
mod tests {
    use super::*;
    // The rules are global: tests that set them must not run at the same time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn whole_words_only_and_in_order() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert!(parse("a b").is_err());
        let r = parse("# c\n\nIon => Ioan\nteh => the\n").unwrap();
        assert_eq!(r.len(), 2);
        set_rules(r);
        let (t, n) = apply("Ion si Ionescu: teh Ion.");
        assert_eq!(t, "Ioan si Ionescu: the Ioan.");
        assert_eq!(n, 3);
        set_rules(vec![]);
        assert!(!enabled());
    }

    #[test]
    fn hostile_rules_never_panic() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // CRLF, BOM, `=>` on the right, empty right side, empty left side set directly.
        let r = parse("\u{feff}a => b\r\nx=>y=>z\r\nq =>\r\n").unwrap();
        assert_eq!(r[1], ("x".into(), "y=>z".into()));
        assert_eq!(r[2], ("q".into(), String::new()));
        assert!(parse("=> b").is_err());
        let mut rules = r;
        rules.push((String::new(), "!".into()));
        set_rules(rules);
        let (t, n) = apply("a x q é");
        assert_eq!(t, "b y=>z  é");
        assert_eq!(n, 3);
        assert_eq!(apply("").1, 0);
        set_rules(vec![]);
    }
}
