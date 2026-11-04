//! `--provenance`: a small JSON record stored in the PDF (Info key `/LegatorOCRProvenance`):
//! which program and version made it, when, from which input files (SHA-256) and with which
//! command-line options. Passwords never go in. It documents what was done; it is not a
//! signature, so anyone who can edit the PDF can edit it.

use std::path::PathBuf;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Options safe to record: password flags and their values are removed.
pub fn clean_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a.starts_with("--") && a.to_ascii_lowercase().contains("password") {
            skip = !a.contains('=');
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// The record as JSON. Inputs that cannot be hashed (PDF pages `file.pdf#3`, unreadable) are
/// listed without a hash.
pub fn build(inputs: &[PathBuf], args: &[String], when: u64) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for p in inputs {
        let base = crate::files::split_frame(p).0;
        if !seen.insert(base.clone()) {
            continue;
        }
        let name = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let hash = crate::checksums::file_hash(&base).ok();
        files.push(match hash {
            Some(h) => format!("{{\"name\":\"{}\",\"sha256\":\"{h}\"}}", esc(&name)),
            None => format!("{{\"name\":\"{}\"}}", esc(&name)),
        });
    }
    let opts: Vec<String> = clean_args(args).iter().map(|a| format!("\"{}\"", esc(a))).collect();
    format!(
        "{{\"software\":\"LegatorOCR {}\",\"unix_time\":{when},\"inputs\":[{}],\"options\":[{}]}}",
        env!("CARGO_PKG_VERSION"),
        files.join(","),
        opts.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_dropped_and_hashes_recorded() {
        let a: Vec<String> = ["x.png", "--pdf-password", "secret", "--ocr", "--package-password-file=/p", "--lang", "ron"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = clean_args(&a);
        assert_eq!(c, vec!["x.png", "--ocr", "--lang", "ron"]);
        let f = std::env::temp_dir().join(format!("lgr_prov_{}.txt", std::process::id()));
        std::fs::write(&f, "abc").unwrap();
        let j = build(&[f.clone(), f.clone()], &a, 123);
        assert!(j.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"), "{j}");
        assert_eq!(j.matches("sha256").count(), 1, "same file listed once");
        assert!(!j.contains("secret") && j.contains("\"unix_time\":123"));
        let _ = std::fs::remove_file(f);
    }
}
