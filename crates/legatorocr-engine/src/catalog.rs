//! `--catalog`: a table of the PDFs a run wrote (title, file, size, SHA-256) as HTML or CSV, ready
//! to publish next to the files so a reader can check a download, and `--dublin-core`: one
//! Dublin Core XML record per PDF for library catalogues.

use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file: String,
    pub title: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Rows for the given files (hash and size are read from disk). `title` is used for every file
/// when given, else the file name without its extension.
pub fn rows(files: &[PathBuf], title: Option<&str>) -> Result<Vec<Row>> {
    let mut v = Vec::new();
    for f in files {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = f
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        v.push(Row {
            file: name,
            title: title.map(str::to_string).unwrap_or(stem),
            sha256: crate::checksums::file_hash(f)?,
            bytes: std::fs::metadata(f)?.len(),
        });
    }
    Ok(v)
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn csv_field(s: &str) -> String {
    // quote when needed, and defuse spreadsheet formulas (=, +, -, @ at the start)
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn csv(rows: &[Row]) -> String {
    let mut o = String::from("title,file,bytes,sha256\r\n");
    for r in rows {
        o.push_str(&format!(
            "{},{},{},{}\r\n",
            csv_field(&r.title),
            csv_field(&r.file),
            r.bytes,
            r.sha256
        ));
    }
    o
}

pub fn html(rows: &[Row], heading: &str) -> String {
    let mut o = String::new();
    o.push_str("<!doctype html>\n<html><head><meta charset=\"utf-8\">");
    o.push_str(&format!("<title>{}</title>", esc(heading)));
    o.push_str(
        "<style>body{font:15px/1.5 system-ui,sans-serif;margin:2rem auto;max-width:60rem;padding:0 1rem}\
table{border-collapse:collapse;width:100%}th,td{border-bottom:1px solid #8884;padding:.4rem .6rem;text-align:left;vertical-align:top}\
code{font-size:.8rem;word-break:break-all}</style></head><body>",
    );
    o.push_str(&format!("<h1>{}</h1>", esc(heading)));
    o.push_str("<p>Check a downloaded file: its SHA-256 must equal the value below (<code>sha256sum FILE</code>, or <code>Get-FileHash FILE</code> on Windows).</p>");
    o.push_str("<table><thead><tr><th>Title</th><th>File</th><th>Size</th><th>SHA-256</th></tr></thead><tbody>");
    for r in rows {
        o.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{} bytes</td><td><code>{}</code></td></tr>",
            esc(&r.title),
            esc(&r.file),
            r.bytes,
            r.sha256
        ));
    }
    o.push_str("</tbody></table></body></html>\n");
    o
}

/// Optional descriptive fields of a Dublin Core record.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub rights: Option<String>,
    pub language: Option<String>,
}

/// One `oai_dc` record for a PDF.
pub fn dublin_core(r: &Row, m: &Meta) -> String {
    let mut o = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<oai_dc:dc xmlns:oai_dc=\"http://www.openarchives.org/OAI/2.0/oai_dc/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n",
    );
    let mut add = |tag: &str, v: &str| {
        if !v.trim().is_empty() {
            o.push_str(&format!("  <dc:{tag}>{}</dc:{tag}>\n", esc(v.trim())));
        }
    };
    add("title", &r.title);
    if let Some(a) = &m.author {
        add("creator", a);
    }
    if let Some(s) = &m.subject {
        add("subject", s);
    }
    if let Some(k) = &m.keywords {
        for kw in k.split([',', ';']) {
            add("subject", kw);
        }
    }
    if let Some(l) = &m.language {
        add("language", l);
    }
    if let Some(x) = &m.rights {
        add("rights", x);
    }
    add("format", "application/pdf");
    add("identifier", &format!("sha256:{}", r.sha256));
    add("source", &r.file);
    o.push_str("</oai_dc:dc>\n");
    o
}

/// Write `<pdf>.dc.xml` beside `pdf`; returns its path.
pub fn write_dublin_core(pdf: &Path, r: &Row, m: &Meta) -> Result<PathBuf> {
    let mut p = pdf.as_os_str().to_owned();
    p.push(".dc.xml");
    let p = PathBuf::from(p);
    crate::write_atomic(&p, dublin_core(r, m).as_bytes())?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        Row {
            file: "carte <1>.pdf".into(),
            title: "Brașov, \"vechi\" & nou".into(),
            sha256: "ab".repeat(32),
            bytes: 1234,
        }
    }

    #[test]
    fn html_escapes_and_lists_hash() {
        let h = html(&[row()], "Catalog <test>");
        assert!(h.contains("Catalog &lt;test&gt;"));
        assert!(h.contains("carte &lt;1&gt;.pdf"));
        assert!(h.contains("Brașov, &quot;vechi&quot; &amp; nou"));
        assert!(h.contains(&"ab".repeat(32)));
        assert!(!h.contains("<1>"));
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let mut r = row();
        r.title = "=SUM(A1)".into();
        let c = csv(&[r, row()]);
        assert!(c.starts_with("title,file,bytes,sha256\r\n"));
        assert!(c.contains("'=SUM(A1),"));
        assert!(c.contains("\"Brașov, \"\"vechi\"\" & nou\""));
    }

    #[test]
    fn dublin_core_has_escaped_fields_and_hash() {
        let m = Meta {
            author: Some("Ion & Maria".into()),
            keywords: Some("istorie; Brașov".into()),
            language: Some("ro".into()),
            ..Meta::default()
        };
        let x = dublin_core(&row(), &m);
        assert!(x.contains("<dc:creator>Ion &amp; Maria</dc:creator>"));
        assert!(x.contains("<dc:subject>istorie</dc:subject>"));
        assert!(x.contains("<dc:subject>Brașov</dc:subject>"));
        assert!(x.contains("<dc:language>ro</dc:language>"));
        assert!(x.contains(&format!("sha256:{}", "ab".repeat(32))));
        assert!(x.ends_with("</oai_dc:dc>\n"));
    }

    #[test]
    fn rows_read_hash_and_size_from_disk() {
        let d = std::env::temp_dir().join(format!("lgr-cat-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.pdf");
        std::fs::write(&f, b"abc").unwrap();
        let r = rows(&[f.clone()], None).unwrap();
        assert_eq!(r[0].bytes, 3);
        assert_eq!(r[0].title, "x");
        assert_eq!(
            r[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
