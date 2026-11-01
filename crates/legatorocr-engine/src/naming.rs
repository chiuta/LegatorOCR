//! Output names from a template, so archives get orderly names and folders by themselves:
//! `--pdf "archive/{year}/{month}/{date}_{name}.pdf"`.
//!
//! Tokens: `{date}` (2026-11-01), `{year}`, `{month}`, `{day}` (UTC), `{name}` (the first
//! input's name without extension, or its folder's name). Anything else in braces is an error,
//! so a typo does not end up in a file name.

use anyhow::{bail, Result};

fn civil_ymd(when: u64) -> (i64, u32, u32) {
    // days since 1970-01-01 → civil date (Howard Hinnant's algorithm)
    let z = (when / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Keep a value usable as one path component on every system.
fn safe(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() {
        "scan".into()
    } else {
        t
    }
}

/// Expand the tokens of `template`. A template without braces is returned unchanged.
pub fn expand(template: &str, name: &str, when: u64) -> Result<String> {
    if !template.contains(['{', '}']) {
        return Ok(template.to_string());
    }
    let (y, m, d) = civil_ymd(when);
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        if rest.as_bytes()[i] == b'}' {
            bail!("name template {template:?}: unmatched '}}'");
        }
        let Some(j) = rest[i..].find('}') else {
            bail!("name template {template:?}: missing '}}'");
        };
        let tok = &rest[i + 1..i + j];
        match tok {
            "date" => out.push_str(&format!("{y:04}-{m:02}-{d:02}")),
            "year" => out.push_str(&format!("{y:04}")),
            "month" => out.push_str(&format!("{m:02}")),
            "day" => out.push_str(&format!("{d:02}")),
            "name" => out.push_str(&safe(name)),
            _ => bail!(
                "name template {template:?}: unknown {{{tok}}} (use date, year, month, day, name)"
            ),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Names a scanner or camera gives (`Scan0001`, `IMG_4821`, `DSC01234`, `Scanned Document`):
/// they say nothing about the content and are worth renaming.
pub fn is_generic(stem: &str) -> bool {
    let s = stem.trim().to_lowercase();
    let letters: String = s
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == ' ')
        .collect();
    let tail = &s[letters.len()..];
    let digits_only = !tail.is_empty()
        && tail
            .chars()
            .all(|c| c.is_ascii_digit() || c == '_' || c == '-');
    let known = [
        "scan",
        "img",
        "dsc",
        "dscn",
        "image",
        "document",
        "doc",
        "scanned document",
        "photo",
        "pxl",
        "p",
        "page",
        "scanned page",
        "untitled",
        "new document",
    ];
    let l = letters.trim().trim_end_matches(['_', '-']).to_string();
    (digits_only && known.contains(&l.as_str())) || known.contains(&s.as_str()) && s.len() > 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_expand_and_typos_are_errors() {
        // 2026-11-01 12:00 UTC
        let t = 1_793_534_400;
        assert_eq!(
            expand("a/{year}/{month}/{date}_{name}.pdf", "Carte: 1/2", t).unwrap(),
            "a/2026/11/2026-11-01_Carte_ 1_2.pdf"
        );
        assert_eq!(expand("plain.pdf", "x", t).unwrap(), "plain.pdf");
        assert!(expand("{dat}.pdf", "x", t).is_err());
        assert!(expand("{date.pdf", "x", t).is_err());
        assert!(expand("date}.pdf", "x", t).is_err());
        assert_eq!(expand("{name}", "..", t).unwrap(), "scan");
        // leap day
        assert_eq!(expand("{date}", "x", 1_709_208_000).unwrap(), "2024-02-29");
    }

    #[test]
    fn generic_scanner_names_are_recognised() {
        for g in [
            "Scan0001",
            "IMG_4821",
            "DSC01234",
            "scan_001",
            "Scanned Document",
            "Untitled",
        ] {
            assert!(is_generic(g), "{g}");
        }
        for ok in [
            "Contract_Popescu_2026",
            "factura-123-furnizor",
            "Carte",
            "2026-11-01_raport",
        ] {
            assert!(!is_generic(ok), "{ok}");
        }
    }
}
