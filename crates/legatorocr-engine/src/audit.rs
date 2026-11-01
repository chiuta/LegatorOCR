//! Append-only audit / retention log (JSON Lines): one record per file written, with its SHA-256,
//! the inputs it came from, an optional retention date and a legal-hold flag. The log only
//! *records* these facts; deleting or expiring anything is left to the archive system.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn civil(when: u64) -> (i64, u32, u32, u32, u32, u32) {
    let z = (when / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = when % 86_400;
    (
        y,
        m,
        d,
        (s / 3600) as u32,
        (s % 3600 / 60) as u32,
        (s % 60) as u32,
    )
}

fn iso(when: u64) -> String {
    let (y, m, d, h, mi, s) = civil(when);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Validate `YYYY-MM-DD` (real calendar date) and return it unchanged.
pub fn check_date(s: &str) -> Result<String> {
    let p: Vec<&str> = s.split('-').collect();
    let ok = p.len() == 3
        && p[0].len() == 4
        && p[1].len() == 2
        && p[2].len() == 2
        && p.iter().all(|x| x.bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        bail!("{s:?}: expected a date as YYYY-MM-DD");
    }
    let (y, m, d): (i64, u32, u32) = (p[0].parse()?, p[1].parse()?, p[2].parse()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => bail!("{s:?}: month must be 01-12"),
    };
    if d == 0 || d > dim {
        bail!("{s:?}: no such day");
    }
    Ok(s.to_string())
}

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
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
    o.push('"');
    o
}

#[derive(Default, Clone)]
pub struct Options {
    pub retain_until: Option<String>,
    pub legal_hold: bool,
    pub note: String,
}

/// JSONL text (one line per written file) for the given outputs.
pub fn render(
    tool: &str,
    inputs: &[PathBuf],
    written: &[PathBuf],
    when: u64,
    o: &Options,
) -> Result<String> {
    let ins = inputs
        .iter()
        .map(|p| jstr(&p.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(",");
    let mut out = String::new();
    for f in written {
        let data = std::fs::read(f).with_context(|| format!("{}: cannot read", f.display()))?;
        out.push_str(&format!(
            "{{\"time\":{},\"tool\":{},\"output\":{},\"bytes\":{},\"sha256\":{},\"inputs\":[{}],\"retain_until\":{},\"legal_hold\":{},\"note\":{}}}\n",
            jstr(&iso(when)),
            jstr(tool),
            jstr(&f.to_string_lossy()),
            data.len(),
            jstr(&crate::checksums::sha256_hex(&data)),
            ins,
            o.retain_until.as_deref().map(jstr).unwrap_or_else(|| "null".into()),
            o.legal_hold,
            jstr(&o.note),
        ));
    }
    Ok(out)
}

/// Append the records to `log` (created when missing).
pub fn append(log: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .with_context(|| format!("{}: cannot open the audit log", log.display()))?;
    f.write_all(text.as_bytes())?;
    f.sync_all().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_validated() {
        assert!(check_date("2031-02-28").is_ok());
        assert!(check_date("2028-02-29").is_ok());
        assert!(check_date("2031-02-29").is_err());
        assert!(check_date("2031-13-01").is_err());
        assert!(check_date("31-01-2031").is_err());
        assert!(check_date("2031-1-01").is_err());
    }

    #[test]
    fn records_carry_hash_hold_and_escape_names() {
        let d = std::env::temp_dir().join(format!("lo_audit_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a\"b.pdf");
        std::fs::write(&f, b"abc").unwrap();
        let o = Options {
            retain_until: Some("2040-01-01".into()),
            legal_hold: true,
            note: "x\ny".into(),
        };
        let t = render(
            "T 1",
            &[PathBuf::from("in.tif")],
            std::slice::from_ref(&f),
            0,
            &o,
        )
        .unwrap();
        assert!(t.starts_with("{\"time\":\"1970-01-01T00:00:00Z\""));
        assert!(t.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(t.contains("\"legal_hold\":true"));
        assert!(t.contains("a\\\"b.pdf"));
        assert!(t.contains("\"retain_until\":\"2040-01-01\""));
        assert_eq!(t.matches('\n').count(), 1);
        let log = d.join("audit.jsonl");
        append(&log, &t).unwrap();
        append(&log, &t).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);
        std::fs::remove_dir_all(&d).ok();
    }
}
