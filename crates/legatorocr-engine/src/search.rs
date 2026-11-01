//! Full-text search over what a digitisation project leaves behind: PDFs with a text
//! layer (LegatorOCR's own, ABBYY FineReader's, PDF24's, Word's …), plain text, hOCR/HTML,
//! Word `.docx`, and all of these inside `.zip` archives. Meant as the "find that phrase in
//! 3 000 scanned pages" step of the workflow (the job done by Agent Ransack or DocFetcher).
//!
//! Plain queries are a phrase, matched ignoring case and diacritics (`mobila` finds
//! `Mobilă`, `stiinta` finds `ştiinţă`), also when the phrase wraps across two lines.
//! `regex` matches line by line; `exact` makes both modes case- and diacritic-sensitive.

use crate::zipmini;
use anyhow::{bail, Context, Result};
use legatorocr_core::text::fold;
use regex::RegexBuilder;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub regex: bool,
    /// Case- and diacritic-sensitive.
    pub exact: bool,
    /// Stop after this many hits (0 = no limit).
    pub max_hits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// File path, or `archive.zip!member` for a file inside an archive.
    pub source: String,
    /// 1-based page number (PDF only).
    pub page: Option<usize>,
    /// 1-based line within the page / file.
    pub line: usize,
    /// The matching line, trimmed and shortened.
    pub text: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Searchable files / archive members read.
    pub files: usize,
    /// Files of a kind we do not read.
    pub skipped: usize,
    /// PDFs that opened but carry no text at all (scans that still need OCR).
    pub image_only_pdfs: usize,
    /// PDFs / archives that could not be read.
    pub unreadable: usize,
    pub hits: usize,
    /// `max_hits` was reached.
    pub truncated: bool,
}

enum Matcher {
    Plain { needle: String, exact: bool },
    Re(regex::Regex),
}

impl Matcher {
    fn new(q: &str, o: &SearchOptions) -> Result<Matcher> {
        if q.trim().is_empty() {
            bail!("the search text is empty");
        }
        if o.regex {
            let re = RegexBuilder::new(q)
                .case_insensitive(!o.exact)
                .size_limit(8 << 20)
                .build()
                .with_context(|| format!("not a valid regular expression: {q}"))?;
            return Ok(Matcher::Re(re));
        }
        let norm = q.split_whitespace().collect::<Vec<_>>().join(" ");
        let needle = if o.exact { norm } else { fold(&norm) };
        Ok(Matcher::Plain {
            needle,
            exact: o.exact,
        })
    }

    /// 1-based line numbers of `text` that match (each at most once, ascending).
    fn lines(&self, text: &str) -> Vec<usize> {
        match self {
            Matcher::Re(re) => text
                .lines()
                .enumerate()
                .filter(|(_, l)| re.is_match(l))
                .map(|(i, _)| i + 1)
                .collect(),
            Matcher::Plain { needle, exact } => {
                // One string for the whole unit, lines joined by a space, so that a phrase
                // broken over a line end is still found; remember where each line starts.
                let mut joined = String::with_capacity(text.len());
                let mut starts = Vec::new();
                for l in text.lines() {
                    starts.push(joined.len());
                    let t = l.split_whitespace().collect::<Vec<_>>().join(" ");
                    joined.push_str(&if *exact { t } else { fold(&t) });
                    joined.push(' ');
                }
                let mut out: Vec<usize> = Vec::new();
                let mut pos = 0;
                while let Some(i) = joined[pos..].find(needle.as_str()) {
                    let at = pos + i;
                    let line = starts.partition_point(|&s| s <= at); // 1-based
                    if out.last() != Some(&line) {
                        out.push(line);
                    }
                    pos = at + needle.len().max(1);
                }
                out
            }
        }
    }
}

fn shorten(l: &str) -> String {
    let t = l.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 200 {
        let mut s: String = t.chars().take(200).collect();
        s.push('…');
        s
    } else {
        t
    }
}

fn decode_entities(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        o.push_str(&rest[..i]);
        rest = &rest[i..];
        // An entity is at most a dozen bytes; look only at that window, bytewise, so a
        // multi-byte character straddling the limit cannot make the slice panic.
        let Some(j) = rest.as_bytes()[..rest.len().min(12)]
            .iter()
            .position(|&b| b == b';')
        else {
            o.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..j];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                o.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                o.push('&');
                rest = &rest[1..];
            }
        }
    }
    o.push_str(rest);
    o
}

/// Text of an HTML / XML / hOCR / WordprocessingML document: tags removed, block ends and
/// OCR lines become line breaks.
fn strip_markup(s: &str) -> String {
    let mut o = String::with_capacity(s.len() / 2);
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        o.push_str(&decode_entities(&rest[..i]));
        rest = &rest[i..];
        let Some(j) = rest.find('>') else {
            rest = "";
            break;
        };
        let tag = rest[1..j].to_ascii_lowercase();
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/' && !tag.starts_with('/'))
            .next()
            .unwrap_or("");
        let brk = matches!(
            name,
            "br" | "/p" | "/div" | "/li" | "/tr" | "/h1" | "/h2" | "/h3" | "/h4" | "/w:p" | "w:br"
        ) || tag.contains("ocr_line")
            || tag.contains("ocr_caption")
            || tag.contains("ocr_par");
        if brk {
            o.push('\n');
        } else if name == "w:tab" || tag.contains("ocr_word") {
            o.push(' ');
        }
        rest = &rest[j + 1..];
    }
    o.push_str(&decode_entities(rest));
    // Drop empty lines so line numbers follow the visible text.
    o.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let u: Vec<u16> = b[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if le {
                    u16::from_le_bytes(c)
                } else {
                    u16::from_be_bytes(c)
                }
            })
            .collect();
        return String::from_utf16_lossy(&u);
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(), // Latin-1 fallback
    }
}

/// One searchable chunk: (page, text).
type Unit = (Option<usize>, String);

enum Read {
    Units(Vec<Unit>),
    /// A PDF without any text layer.
    ImageOnly,
    Unreadable,
    Unsupported,
    /// An archive: members to visit.
    Archive(Vec<(String, Vec<u8>)>),
}

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn read_units(name: &str, data: Vec<u8>) -> Read {
    match ext_of(name).as_str() {
        "pdf" => {
            let Ok(doc) = legatorocr_pdf::reader::PdfDoc::open(data) else {
                return Read::Unreadable;
            };
            let mut units = Vec::new();
            for i in 0..doc.page_count() {
                if let Ok(t) = doc.page_text(i) {
                    if !t.trim().is_empty() {
                        units.push((Some(i + 1), t));
                    }
                }
            }
            if units.is_empty() && doc.page_count() > 0 {
                Read::ImageOnly
            } else {
                Read::Units(units)
            }
        }
        "txt" | "text" | "md" | "csv" | "tsv" | "log" | "srt" | "vtt" | "lgr" | "json" => {
            Read::Units(vec![(None, decode_text(&data))])
        }
        "html" | "htm" | "xhtml" | "hocr" | "xml" => {
            Read::Units(vec![(None, strip_markup(&decode_text(&data)))])
        }
        "docx" => {
            let Some(list) = zipmini::list(&data) else {
                return Read::Unreadable;
            };
            let Some(e) = list.iter().find(|e| e.name == "word/document.xml") else {
                return Read::Unreadable;
            };
            match zipmini::read(&data, e) {
                Some(x) => Read::Units(vec![(None, strip_markup(&decode_text(&x)))]),
                None => Read::Unreadable,
            }
        }
        "zip" => {
            let Some(list) = zipmini::list(&data) else {
                return Read::Unreadable;
            };
            let mut v = Vec::new();
            let mut held = 0usize;
            for e in &list {
                if e.name.ends_with('/') || !is_searchable(&e.name) {
                    continue;
                }
                if let Some(b) = zipmini::read(&data, e) {
                    // a zip of many large text members must not be inflated into memory whole
                    held += b.len();
                    if held > 512 << 20 {
                        break;
                    }
                    v.push((e.name.clone(), b));
                }
            }
            Read::Archive(v)
        }
        _ => Read::Unsupported,
    }
}

fn is_searchable(name: &str) -> bool {
    matches!(
        ext_of(name).as_str(),
        "pdf"
            | "txt"
            | "text"
            | "md"
            | "csv"
            | "tsv"
            | "log"
            | "srt"
            | "vtt"
            | "lgr"
            | "json"
            | "html"
            | "htm"
            | "xhtml"
            | "hocr"
            | "xml"
            | "docx"
            | "zip"
    )
}

struct Ctx<'a> {
    m: &'a Matcher,
    o: &'a SearchOptions,
    sum: Summary,
    on_hit: &'a mut dyn FnMut(Hit),
}

impl Ctx<'_> {
    fn done(&self) -> bool {
        self.sum.truncated
    }

    fn visit(&mut self, source: &str, name: &str, data: Vec<u8>, depth: u32) {
        if self.done() {
            return;
        }
        match read_units(name, data) {
            Read::Unsupported => self.sum.skipped += 1,
            Read::Unreadable => self.sum.unreadable += 1,
            Read::ImageOnly => {
                self.sum.files += 1;
                self.sum.image_only_pdfs += 1;
            }
            Read::Archive(members) => {
                self.sum.files += 1;
                if depth < 2 {
                    for (n, b) in members {
                        self.visit(&format!("{source}!{n}"), &n, b, depth + 1);
                    }
                }
            }
            Read::Units(units) => {
                self.sum.files += 1;
                for (page, text) in units {
                    let lines: Vec<&str> = text.lines().collect();
                    for ln in self.m.lines(&text) {
                        if self.o.max_hits > 0 && self.sum.hits >= self.o.max_hits {
                            self.sum.truncated = true;
                            return;
                        }
                        self.sum.hits += 1;
                        (self.on_hit)(Hit {
                            source: source.to_string(),
                            page,
                            line: ln,
                            text: shorten(lines.get(ln - 1).copied().unwrap_or("")),
                        });
                    }
                }
            }
        }
    }

    fn walk(&mut self, p: &Path, depth: u32) {
        if self.done() || depth > 40 {
            return;
        }
        let Ok(md) = std::fs::symlink_metadata(p) else {
            self.sum.unreadable += 1;
            return;
        };
        if md.file_type().is_symlink() {
            return;
        }
        if md.is_dir() {
            let Ok(rd) = std::fs::read_dir(p) else {
                self.sum.unreadable += 1;
                return;
            };
            let mut v: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            v.sort();
            for c in v {
                self.walk(&c, depth + 1);
            }
        } else if md.is_file() {
            let name = p.to_string_lossy().into_owned();
            if matches!(
                crate::archive::kind(p),
                Some(
                    crate::archive::Kind::Tar
                        | crate::archive::Kind::TarGz
                        | crate::archive::Kind::External
                )
            ) {
                // .tar/.tgz built in; .7z/.rar/… through an installed extractor.
                match crate::archive::extract(p, &|n| is_searchable(n)) {
                    Ok(members) => {
                        self.sum.files += 1;
                        for (m, f) in members {
                            if let Ok(d) = std::fs::read(&f) {
                                self.visit(&format!("{name}!{m}"), &m, d, 1);
                            }
                            let _ = std::fs::remove_file(&f);
                        }
                    }
                    Err(e) => {
                        eprintln!("{name}: {e:#}");
                        self.sum.unreadable += 1;
                    }
                }
            } else if !is_searchable(&name) {
                self.sum.skipped += 1;
            } else if md.len() > 1 << 30 {
                self.sum.unreadable += 1;
            } else if let Ok(d) = std::fs::read(p) {
                self.visit(&name, &name, d, 0);
            } else {
                self.sum.unreadable += 1;
            }
        }
    }
}

/// Search `roots` (files and/or folders, folders recursively) for `query`, calling `on_hit`
/// for every match in path order.
pub fn search(
    roots: &[std::path::PathBuf],
    query: &str,
    o: &SearchOptions,
    on_hit: &mut dyn FnMut(Hit),
) -> Result<Summary> {
    let m = Matcher::new(query, o)?;
    let mut c = Ctx {
        m: &m,
        o,
        sum: Summary::default(),
        on_hit,
    };
    for r in roots {
        if !r.exists() {
            bail!("{}: no such file or folder", r.display());
        }
        c.walk(r, 0);
    }
    Ok(c.sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, q: &str, o: &SearchOptions) -> (Vec<Hit>, Summary) {
        let mut hits = Vec::new();
        let s = search(&[dir.to_path_buf()], q, o, &mut |h| hits.push(h)).unwrap();
        (hits, s)
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lgr-search-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn plain_phrase_ignores_case_and_diacritics_and_line_breaks() {
        let d = tmp("plain");
        std::fs::write(
            d.join("a.txt"),
            "Tehnologia Producerii\nMOBILEI din lemn\nștiința lemnului",
        )
        .unwrap();
        let o = SearchOptions::default();
        let (h, s) = run(&d, "producerii mobilei", &o);
        assert_eq!(h.len(), 1, "{h:?}");
        assert_eq!(h[0].line, 1);
        assert_eq!(s.files, 1);
        assert_eq!(run(&d, "STIINTA", &o).0[0].line, 3);
        assert!(run(
            &d,
            "stiinta",
            &SearchOptions {
                exact: true,
                ..o.clone()
            }
        )
        .0
        .is_empty());
        assert_eq!(
            run(&d, "știința", &SearchOptions { exact: true, ..o })
                .0
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn regex_zip_docx_hocr_and_limits() {
        let d = tmp("mixed");
        let docx = crate::docx::from_pages(&["Raport anual\n\nBuget 2026".into()], "t");
        let z = zipmini::write(&[
            ("in/n.txt", "cod RO-1234 aici".as_bytes()),
            ("in/r.docx", &docx),
            ("in/skip.bin", b"RO-9999"),
        ]);
        std::fs::write(d.join("arh.zip"), z).unwrap();
        std::fs::write(
            d.join("p.hocr"),
            "<div class='ocr_page'><span class='ocr_line'><span class='ocr_word'>Cod</span> <span class='ocr_word'>RO-5555</span></span></div>",
        )
        .unwrap();
        let o = SearchOptions {
            regex: true,
            ..Default::default()
        };
        let (h, s) = run(&d, r"RO-\d{4}", &o);
        assert_eq!(h.len(), 2, "{h:?}");
        assert!(h.iter().any(|x| x.source.ends_with("arh.zip!in/n.txt")));
        assert!(h.iter().any(|x| x.source.ends_with("p.hocr")));
        assert_eq!(s.files, 4); // archive, its two members, hOCR
        let (h, _) = run(&d, "buget", &SearchOptions::default());
        assert_eq!(h[0].line, 2);
        assert!(h[0].source.ends_with("arh.zip!in/r.docx"));
        let (h, s) = run(
            &d,
            "RO",
            &SearchOptions {
                regex: true,
                max_hits: 1,
                ..Default::default()
            },
        );
        assert_eq!((h.len(), s.truncated), (1, true));
        assert!(search(std::slice::from_ref(&d), "(", &o, &mut |_| {}).is_err());
        assert!(search(
            std::slice::from_ref(&d),
            "  ",
            &SearchOptions::default(),
            &mut |_| {}
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pdf_text_layer_is_searched_and_scans_are_reported() {
        use legatorocr_core::{text::Word, Page, PageImage};
        use legatorocr_pdf::{Compress, PdfWriter};
        let d = tmp("pdf");
        let pg = || Page {
            image: PageImage::Rgb(image::RgbImage::new(200, 100)),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        };
        let w = |t: &str, y: u32, line: u32| Word {
            text: t.into(),
            x: 10,
            y,
            w: 80,
            h: 18,
            conf: 90.0,
            line,
        };
        let mut a = PdfWriter::new(Compress::Flate);
        a.add_page(&pg(), &[w("Argeș", 5, 0)]);
        a.add_page(&pg(), &[w("Ornamente", 5, 0), w("populare", 40, 1)]);
        std::fs::write(d.join("carte.pdf"), a.finish()).unwrap();
        let mut b = PdfWriter::new(Compress::Flate);
        b.add_page(&pg(), &[]);
        std::fs::write(d.join("scan.pdf"), b.finish()).unwrap();
        let (h, s) = run(&d, "arges", &SearchOptions::default());
        assert_eq!(h.len(), 1, "{h:?}");
        assert_eq!((h[0].page, h[0].line), (Some(1), 1));
        assert_eq!(
            run(&d, "ornamente populare", &SearchOptions::default()).0[0].page,
            Some(2)
        );
        assert_eq!(s.image_only_pdfs, 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tgz_members_are_searched() {
        if !std::process::Command::new("tar")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return;
        }
        let d = tmp("tgz");
        std::fs::create_dir_all(d.join("in")).unwrap();
        std::fs::write(d.join("in/n.txt"), "Cartea de bucate").unwrap();
        assert!(std::process::Command::new("tar")
            .current_dir(&d)
            .args(["-czf", "a.tgz", "in"])
            .status()
            .unwrap()
            .success());
        std::fs::remove_dir_all(d.join("in")).unwrap();
        let (h, s) = run(&d, "BUCATE", &SearchOptions::default());
        assert_eq!(h.len(), 1, "{h:?}");
        assert!(h[0].source.ends_with("a.tgz!in/n.txt"));
        assert_eq!(s.files, 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn entities_and_garbage() {
        assert_eq!(
            decode_entities("a &amp; b &#537; &#x21B; &bogus; &"),
            "a & b ș ț &bogus; &"
        );
        assert_eq!(
            strip_markup("<p>Un&nbsp;rând</p><p>doi<br/>trei</p>"),
            "Un rând\ndoi\ntrei"
        );
        assert!(matches!(
            read_units("x.pdf", b"nu e pdf".to_vec()),
            Read::Unreadable
        ));
        assert!(matches!(read_units("x.zip", vec![1; 50]), Read::Unreadable));
    }
}

#[cfg(test)]
mod entity_boundary_tests {
    #[test]
    fn entity_window_ending_inside_a_multibyte_char_does_not_panic() {
        // '&' followed by bytes so that byte 12 falls inside a 2-byte character
        let s = format!("&{}\u{81}\u{81}tail", "a".repeat(10));
        let _ = super::decode_entities(&s);
        assert_eq!(super::decode_entities("a &amp; b"), "a & b");
    }
}
