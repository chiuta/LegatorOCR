//! `--classify RULES.txt`: tags and a target folder for a document, decided from its OCR text.
//!
//! Rules file (INI-like; `#` starts a comment):
//!
//! ```text
//! [Invoices]
//! match  = any            # any (default): one term is enough; all: every term; regex
//! terms  = factura, invoice, "total de plata"
//! tag    = invoice, finance
//! folder = Invoices
//!
//! [Contracts]
//! match   = regex
//! pattern = contract\s+nr\.?\s*\d+
//! tag     = contract
//! ```
//!
//! `any` / `all` ignore case and diacritics (`factură` matches `FACTURA`) and match whole words
//! or phrases. `regex` uses the `regex` crate's syntax, case-insensitive, on the text as written.
//! Every rule that matches contributes its tags; the first matching rule that has a `folder`
//! decides where the original goes.

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Any,
    All,
    Regex,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub mode: Mode,
    pub terms: Vec<String>,
    pub pattern: Option<Regex>,
    pub tags: Vec<String>,
    pub folder: Option<String>,
}

/// What a text matched.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub rules: Vec<String>,
    pub tags: Vec<String>,
    pub folder: Option<String>,
}

fn split_list(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in v.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                let t = cur.trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

/// A folder name that cannot leave the target directory or be empty.
fn safe_folder(v: &str) -> Option<String> {
    let t = v.trim();
    let bad = t.is_empty()
        || t == "."
        || t == ".."
        || t.contains(['/', '\\', ':', '\0'])
        || t.starts_with('.');
    (!bad).then(|| t.to_string())
}

pub fn parse(src: &str) -> Result<Vec<Rule>, String> {
    let mut rules: Vec<Rule> = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let n = i + 1;
        let l = raw.trim_start_matches('\u{feff}');
        // a comment starts at `#` outside quotes, except inside a pattern (handled by key below)
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            rules.push(Rule {
                name: name.trim().to_string(),
                mode: Mode::Any,
                terms: Vec::new(),
                pattern: None,
                tags: Vec::new(),
                folder: None,
            });
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            return Err(format!("line {n}: expected `key = value` or `[rule name]`"));
        };
        let Some(r) = rules.last_mut() else {
            return Err(format!("line {n}: a `[rule name]` line must come first"));
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let strip = |v: &str| v.split(" #").next().unwrap_or(v).trim().to_string();
        match k.as_str() {
            "match" => {
                r.mode = match strip(v).to_ascii_lowercase().as_str() {
                    "any" => Mode::Any,
                    "all" => Mode::All,
                    "regex" => Mode::Regex,
                    o => {
                        return Err(format!(
                            "line {n}: match must be any, all or regex, not `{o}`"
                        ))
                    }
                }
            }
            "terms" => r.terms = split_list(&strip(v)),
            "pattern" => {
                r.pattern = Some(
                    Regex::new(&format!("(?i){v}"))
                        .map_err(|e| format!("line {n}: bad pattern: {e}"))?,
                )
            }
            "tag" | "tags" => r.tags = split_list(&strip(v)),
            "folder" => {
                r.folder = Some(
                    safe_folder(&strip(v))
                        .ok_or_else(|| format!("line {n}: folder must be a plain name"))?,
                )
            }
            o => return Err(format!("line {n}: unknown key `{o}`")),
        }
    }
    for r in &rules {
        let ok = match r.mode {
            Mode::Regex => r.pattern.is_some(),
            _ => !r.terms.is_empty(),
        };
        if !ok {
            return Err(format!(
                "rule [{}] has nothing to match (terms / pattern)",
                r.name
            ));
        }
    }
    Ok(rules)
}

/// Word-boundary containment on folded text: `hay` and `needle` are already folded.
fn has_phrase(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(p) = hay[from..].find(needle) {
        let a = from + p;
        let b = a + needle.len();
        let before = hay[..a]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = hay[b..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = a + hay[a..].chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

pub fn classify(rules: &[Rule], text: &str) -> Outcome {
    let folded = crate::fuzzy::fold(text);
    let mut out = Outcome::default();
    for r in rules {
        let hit = match r.mode {
            Mode::Regex => r.pattern.as_ref().is_some_and(|p| p.is_match(text)),
            Mode::Any => r
                .terms
                .iter()
                .any(|t| has_phrase(&folded, &crate::fuzzy::fold(t))),
            Mode::All => r
                .terms
                .iter()
                .all(|t| has_phrase(&folded, &crate::fuzzy::fold(t))),
        };
        if !hit {
            continue;
        }
        out.rules.push(r.name.clone());
        for t in &r.tags {
            if !out.tags.contains(t) {
                out.tags.push(t.clone());
            }
        }
        if out.folder.is_none() {
            out.folder = r.folder.clone();
        }
    }
    out
}

pub fn to_json(file: &str, o: &Outcome) -> String {
    let list = |v: &[String]| {
        v.iter()
            .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",")
    };
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{\"file\":\"{}\",\"rules\":[{}],\"tags\":[{}],\"folder\":{}}}\n",
        esc(file),
        list(&o.rules),
        list(&o.tags),
        o.folder
            .as_deref()
            .map_or("null".into(), |f| format!("\"{}\"", esc(f)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = "# demo\n[Facturi]\nmatch = any\nterms = factura, invoice, \"total de plata\"\ntag = factura, financiar\nfolder = Facturi\n\n[Contract]\nmatch = regex\npattern = contract\\s+nr\\.?\\s*\\d+\ntag = contract\nfolder = Contracte\n\n[Both]\nmatch = all\nterms = ion, popescu\ntag = ion\n";

    #[test]
    fn any_ignores_case_and_diacritics_and_needs_whole_words() {
        let r = parse(RULES).unwrap();
        let o = classify(&r, "FACTURĂ fiscală nr. 12 — Total de plată: 100 lei");
        assert_eq!(o.rules, ["Facturi"]);
        assert_eq!(o.tags, ["factura", "financiar"]);
        assert_eq!(o.folder.as_deref(), Some("Facturi"));
        // part of a longer word does not count
        assert!(classify(&r, "facturare lunară").rules.is_empty());
    }

    #[test]
    fn all_needs_every_term_and_regex_matches_the_written_text() {
        let r = parse(RULES).unwrap();
        assert!(classify(&r, "Ion Ionescu").rules.is_empty());
        let o = classify(&r, "Ion Popescu, Contract nr. 45/2020");
        assert_eq!(o.rules, ["Contract", "Both"]);
        assert_eq!(o.tags, ["contract", "ion"]);
        // the first rule with a folder decides
        assert_eq!(o.folder.as_deref(), Some("Contracte"));
    }

    #[test]
    fn bad_rules_are_refused_with_a_line_number() {
        assert!(parse("tag = x\n").unwrap_err().contains("line 1"));
        assert!(parse("[a]\nmatch = maybe\n")
            .unwrap_err()
            .contains("line 2"));
        assert!(parse("[a]\nmatch = regex\npattern = (\n")
            .unwrap_err()
            .contains("line 3"));
        assert!(parse("[a]\nterms = x\nfolder = ../x\n")
            .unwrap_err()
            .contains("line 3"));
        assert!(parse("[a]\n").unwrap_err().contains("nothing to match"));
        assert!(parse("[a]\nterms = x\nwhat = 1\n")
            .unwrap_err()
            .contains("unknown key"));
    }

    #[test]
    fn json_escapes_and_phrases_do_not_overrun_multibyte_text() {
        let o = Outcome {
            rules: vec!["a\"b".into()],
            tags: vec!["t".into()],
            folder: None,
        };
        assert_eq!(
            to_json("x\\y.pdf", &o),
            "{\"file\":\"x\\\\y.pdf\",\"rules\":[\"a\\\"b\"],\"tags\":[\"t\"],\"folder\":null}\n"
        );
        assert!(has_phrase("țară ță ta", "ta"));
        assert!(!has_phrase("tata", "ta"));
    }
}
