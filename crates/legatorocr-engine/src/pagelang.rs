//! OCR language per input page: `1-40=ron,41-44=eng+fra,45-=ron`. Pages are counted from 1 in
//! input order (scans, or PDF pages). Pages not named use the general `--ocr-lang`.

use anyhow::{bail, Result};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub from: usize,
    pub to: usize,
    pub lang: String,
}

static RULES: Mutex<Vec<Rule>> = Mutex::new(Vec::new());

/// Parse a specification. `a-b`, `a`, `a-` (to the end) and `-b` (from the start) are accepted.
pub fn parse(spec: &str) -> Result<Vec<Rule>> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((range, lang)) = part.split_once('=') else {
            bail!("page language {part:?}: expected PAGES=LANG, e.g. 1-40=ron");
        };
        let lang = lang.trim();
        if !legatorocr_ocr::valid_lang(&legatorocr_ocr::canonical_lang(lang)) {
            bail!("page language {part:?}: invalid language list {lang:?} (e.g. ron or ron+eng)");
        }
        let num = |s: &str| -> Result<usize> {
            s.trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or_else(|| {
                    anyhow::anyhow!("page language {part:?}: {s:?} is not a page number")
                })
        };
        let (from, to) = match range.split_once('-') {
            None => (num(range)?, num(range)?),
            Some((a, b)) => (
                if a.trim().is_empty() { 1 } else { num(a)? },
                if b.trim().is_empty() {
                    usize::MAX
                } else {
                    num(b)?
                },
            ),
        };
        if from > to {
            bail!("page language {part:?}: the range runs backwards");
        }
        out.push(Rule {
            from,
            to,
            lang: legatorocr_ocr::canonical_lang(lang),
        });
    }
    Ok(out)
}

pub fn set(rules: Vec<Rule>) {
    if let Ok(mut r) = RULES.lock() {
        *r = rules;
    }
}

/// Language for input page `page` (1-based); the last matching rule wins.
pub fn for_page(page: usize) -> Option<String> {
    RULES
        .lock()
        .ok()?
        .iter()
        .rev()
        .find(|r| (r.from..=r.to).contains(&page))
        .map(|r| r.lang.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_parse_and_last_rule_wins() {
        let r = parse("1-3=ron, 4=eng+fra, 5-=ron, 2=eng").unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!((r[2].from, r[2].to), (5, usize::MAX));
        set(r);
        assert_eq!(for_page(1).as_deref(), Some("ron"));
        assert_eq!(for_page(2).as_deref(), Some("eng"));
        assert_eq!(for_page(4).as_deref(), Some("eng+fra"));
        assert_eq!(for_page(900).as_deref(), Some("ron"));
        set(Vec::new());
        assert_eq!(for_page(1), None);
    }

    #[test]
    fn bad_specs_are_refused() {
        assert!(parse("1-3").is_err());
        assert!(parse("0=ron").is_err());
        assert!(parse("5-2=ron").is_err());
        assert!(parse("1=ro n").is_err());
        assert!(parse("").unwrap().is_empty());
    }
}
