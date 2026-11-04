//! Plic FFF (`.fff.json`) pentru lanțul de surse „fără frontiere” (docs/FFF-ENVELOPE.md din hub).
//! LegatorOCR PRODUCE `document/ocr` (textul recunoscut pe pagini + amprenta SHA-256 a scanării);
//! TEZAUR îl consumă (`ingest --fff-in`). Fără dependențe noi: JSON scris de mână, doar escape corect.

/// Escapează un text ca literal JSON (ghilimele, backslash, caractere de control; restul rămâne UTF-8).
pub fn jstr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
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

/// ISO 8601 UTC (secunde) dintr-un moment Unix, fără chrono.
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // algoritmul civil-from-days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub struct DocMeta<'a> {
    pub title: &'a str,
    pub author: &'a str,
    pub year: &'a str,
    pub lang: &'a str,
    /// numele fișierului-sursă (scanarea), fără cale
    pub file: &'a str,
    /// SHA-256 (hex) al scanării originale
    pub sha256: &'a str,
    /// textul tuturor paginilor; paginile sunt despărțite de `\x0c` (forma motorului)
    pub text: &'a str,
    pub engine: &'a str,
    pub version: &'a str,
}

/// Plicul `document/ocr` canonic (`fff: 2`). `payload.pages[i] = {n, text}`; `source.id = sha256:<hex>`.
pub fn document_ocr(m: &DocMeta, now_secs: u64) -> String {
    let pages: Vec<String> = m
        .text
        .split('\x0c')
        .enumerate()
        .map(|(i, t)| format!("{{\"n\":{},\"text\":{}}}", i + 1, jstr(t.trim_matches('\n'))))
        .collect();
    let mut p = format!(
        "{{\"title\":{},\"lang\":{},\"file\":{},\"sha256\":{},\"engine\":{},\"pages\":[{}]",
        jstr(m.title),
        jstr(m.lang),
        jstr(m.file),
        jstr(m.sha256),
        jstr(&format!("{} {}", m.engine, m.version)),
        pages.join(",")
    );
    if !m.author.is_empty() {
        p.push_str(&format!(",\"author\":{}", jstr(m.author)));
    }
    if !m.year.is_empty() {
        p.push_str(&format!(",\"year\":{}", jstr(m.year)));
    }
    p.push('}');
    format!(
        "{{\"fff\":2,\"type\":\"document/ocr\",\"title\":{},\"payload\":{},\"source\":{{\"label\":{},\"id\":{}}},\"created\":{},\"app\":\"LegatorOCR\",\"lang\":{}}}\n",
        jstr(m.title),
        p,
        jstr(&format!("LegatorOCR · {}", m.file)),
        jstr(&format!("sha256:{}", m.sha256)),
        jstr(&iso_utc(now_secs)),
        jstr(m.lang)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_and_dates() {
        assert_eq!(jstr("a\"b\\c\n\u{1}ș"), "\"a\\\"b\\\\c\\n\\u0001ș\"");
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_793_667_568), "2026-11-03T00:59:28.000Z");
    }
    #[test]
    fn envelope_has_pages_and_hash() {
        let m = DocMeta { title: "T", author: "A. Popescu", year: "1999", lang: "ro", file: "s.png", sha256: "ab", text: "unu\n\x0cdoi", engine: "legatorocr", version: "0.2" };
        let e = document_ocr(&m, 0);
        assert!(e.contains("\"type\":\"document/ocr\""));
        assert!(e.contains("{\"n\":1,\"text\":\"unu\"},{\"n\":2,\"text\":\"doi\"}"));
        assert!(e.contains("\"id\":\"sha256:ab\""));
        assert!(e.contains("\"author\":\"A. Popescu\""));
    }
}
