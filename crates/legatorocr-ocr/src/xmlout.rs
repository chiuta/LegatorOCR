//! ALTO XML (v4) and PAGE XML (PRImA, 2019-07-15 schema) writers, next to hOCR/JSON.
//!
//! Lines are the runs of words sharing a `line` id and blocks/regions are the paragraphs found
//! by the same rule as [`crate::words_to_paragraphs`], so all formats agree on the structure.
//! All coordinates are pixels of the processed page, origin top-left.

use crate::{line_boxes, paragraph_starts, xml_escape, LineBox};
use legatorocr_core::Word;

const ALTO_NS: &str = "http://www.loc.gov/standards/alto/ns-v4#";
const PAGE_NS: &str = "http://schema.primaresearch.org/PAGE/gts/pagecontent/2019-07-15";

/// Confidence as a 0..1 value, or `None` when unknown.
fn conf01(w: &Word) -> Option<f32> {
    (w.conf.is_finite() && w.conf >= 0.0).then(|| (w.conf / 100.0).clamp(0.0, 1.0))
}

/// Words whose text is empty once illegal characters are removed are skipped.
fn usable(words: &[Word]) -> Vec<Word> {
    words
        .iter()
        .filter(|w| !xml_escape(w.text.trim()).is_empty())
        .cloned()
        .collect()
}

/// Lines grouped into paragraphs.
fn blocks(words: &[Word]) -> Vec<Vec<LineBox>> {
    let lines = line_boxes(words);
    let starts = paragraph_starts(&lines);
    let mut out: Vec<Vec<LineBox>> = Vec::new();
    for (l, s) in lines.into_iter().zip(starts) {
        match out.last_mut() {
            Some(b) if !s => b.push(l),
            _ => out.push(vec![l]),
        }
    }
    out
}

fn union<'a>(lines: impl Iterator<Item = &'a LineBox> + Clone) -> (u32, u32, u32, u32) {
    (
        lines.clone().map(|l| l.x0).min().unwrap_or(0),
        lines.clone().map(|l| l.y0).min().unwrap_or(0),
        lines.clone().map(|l| l.x1).max().unwrap_or(0),
        lines.map(|l| l.y1).max().unwrap_or(0),
    )
}

/// One ALTO `<Page>` element (`page_no` starts at 1). Use [`alto_document`] to wrap pages.
/// ALTO has no resolution attribute, so `dpi` is recorded in a comment only.
pub fn alto_page(words: &[Word], width: u32, height: u32, dpi: u32, page_no: usize) -> String {
    let words = usable(words);
    let blocks = blocks(&words);
    let (px0, py0, px1, py1) = if blocks.is_empty() {
        (0, 0, width, height)
    } else {
        union(blocks.iter().flatten())
    };
    let mut o = format!(
        "    <Page ID=\"P{page_no}\" PHYSICAL_IMG_NR=\"{page_no}\" WIDTH=\"{width}\" HEIGHT=\"{height}\">\n      <!-- resolution {dpi} dpi -->\n      <PrintSpace HPOS=\"{px0}\" VPOS=\"{py0}\" WIDTH=\"{}\" HEIGHT=\"{}\">\n",
        px1.saturating_sub(px0),
        py1.saturating_sub(py0)
    );
    let mut line_no = 0;
    for (bi, b) in blocks.iter().enumerate() {
        let (x0, y0, x1, y1) = union(b.iter());
        o.push_str(&format!(
            "        <TextBlock ID=\"P{page_no}_TB{}\" HPOS=\"{x0}\" VPOS=\"{y0}\" WIDTH=\"{}\" HEIGHT=\"{}\">\n",
            bi + 1,
            x1 - x0,
            y1 - y0
        ));
        for l in b {
            line_no += 1;
            o.push_str(&format!(
                "          <TextLine ID=\"P{page_no}_TL{line_no}\" HPOS=\"{}\" VPOS=\"{}\" WIDTH=\"{}\" HEIGHT=\"{}\">\n",
                l.x0,
                l.y0,
                l.x1 - l.x0,
                l.y1 - l.y0
            ));
            let lw = &words[l.first..l.end];
            for (k, w) in lw.iter().enumerate() {
                let wc = conf01(w).map_or(String::new(), |c| format!(" WC=\"{c:.3}\""));
                o.push_str(&format!(
                    "            <String ID=\"P{page_no}_TL{line_no}_W{}\" HPOS=\"{}\" VPOS=\"{}\" WIDTH=\"{}\" HEIGHT=\"{}\"{wc} CONTENT=\"{}\"/>\n",
                    k + 1,
                    w.x,
                    w.y,
                    w.w,
                    w.h,
                    xml_escape(w.text.trim())
                ));
                if let Some(n) = lw.get(k + 1) {
                    let sx = w.x.saturating_add(w.w);
                    o.push_str(&format!(
                        "            <SP HPOS=\"{sx}\" VPOS=\"{}\" WIDTH=\"{}\" HEIGHT=\"{}\"/>\n",
                        l.y0,
                        n.x.saturating_sub(sx),
                        l.y1 - l.y0
                    ));
                }
            }
            o.push_str("          </TextLine>\n");
        }
        o.push_str("        </TextBlock>\n");
    }
    o.push_str("      </PrintSpace>\n    </Page>\n");
    o
}

/// Wrap [`alto_page`] elements into a complete ALTO v4 document; `source_name` is the image or
/// PDF file name recorded in the description.
pub fn alto_document(pages: &[String], source_name: &str) -> String {
    let mut o = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<alto xmlns=\"{ALTO_NS}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"{ALTO_NS} https://www.loc.gov/standards/alto/v4/alto-4-2.xsd\">\n  <Description>\n    <MeasurementUnit>pixel</MeasurementUnit>\n    <sourceImageInformation>\n      <fileName>{}</fileName>\n    </sourceImageInformation>\n    <Processing ID=\"OCR_1\">\n      <processingStepDescription>optical character recognition</processingStepDescription>\n      <processingSoftware>\n        <softwareCreator>LegatorOCR</softwareCreator>\n        <softwareName>LegatorOCR</softwareName>\n        <softwareVersion>{}</softwareVersion>\n      </processingSoftware>\n    </Processing>\n  </Description>\n  <Layout>\n",
        xml_escape(source_name),
        env!("CARGO_PKG_VERSION")
    );
    for p in pages {
        o.push_str(p);
    }
    o.push_str("  </Layout>\n</alto>\n");
    o
}

fn points(x0: u32, y0: u32, x1: u32, y1: u32) -> String {
    format!("{x0},{y0} {x1},{y0} {x1},{y1} {x0},{y1}")
}

/// A complete PAGE XML document (one page; PAGE files hold a single `<Page>`) with the image
/// file name recorded as `page_{page_no}`. See [`page_xml_named`].
pub fn page_xml(words: &[Word], width: u32, height: u32, page_no: usize) -> String {
    page_xml_named(words, width, height, page_no, &format!("page_{page_no}"))
}

/// A complete PAGE XML document: one `TextRegion` per paragraph, in reading order
/// (`ReadingOrder`), each with `TextLine`s, `Word`s, rectangular polygon `Coords` and
/// `TextEquiv`/`Unicode` text. Word `TextEquiv@conf` is 0..1 and omitted when unknown.
pub fn page_xml_named(
    words: &[Word],
    width: u32,
    height: u32,
    page_no: usize,
    image_name: &str,
) -> String {
    let words = usable(words);
    let blocks = blocks(&words);
    let stamp = iso_now();
    let mut o = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PcGts xmlns=\"{PAGE_NS}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"{PAGE_NS} http://schema.primaresearch.org/PAGE/gts/pagecontent/2019-07-15/pagecontent.xsd\">\n  <Metadata>\n    <Creator>LegatorOCR</Creator>\n    <Created>{stamp}</Created>\n    <LastChange>{stamp}</LastChange>\n  </Metadata>\n  <Page imageFilename=\"{}\" imageWidth=\"{width}\" imageHeight=\"{height}\">\n",
        xml_escape(image_name)
    );
    if !blocks.is_empty() {
        o.push_str(&format!(
            "    <ReadingOrder>\n      <OrderedGroup id=\"ro{page_no}\">\n"
        ));
        for i in 0..blocks.len() {
            o.push_str(&format!(
                "        <RegionRefIndexed index=\"{i}\" regionRef=\"P{page_no}_r{}\"/>\n",
                i + 1
            ));
        }
        o.push_str("      </OrderedGroup>\n    </ReadingOrder>\n");
    }
    let mut line_no = 0;
    for (bi, b) in blocks.iter().enumerate() {
        let (x0, y0, x1, y1) = union(b.iter());
        o.push_str(&format!(
            "    <TextRegion id=\"P{page_no}_r{}\">\n      <Coords points=\"{}\"/>\n",
            bi + 1,
            points(x0, y0, x1, y1)
        ));
        let mut region_text: Vec<String> = Vec::new();
        for l in b {
            line_no += 1;
            let lw = &words[l.first..l.end];
            let text = lw
                .iter()
                .map(|w| xml_escape(w.text.trim()))
                .collect::<Vec<_>>()
                .join(" ");
            o.push_str(&format!(
                "      <TextLine id=\"P{page_no}_l{line_no}\">\n        <Coords points=\"{}\"/>\n",
                points(l.x0, l.y0, l.x1, l.y1)
            ));
            for (k, w) in lw.iter().enumerate() {
                let c = conf01(w).map_or(String::new(), |c| format!(" conf=\"{c:.3}\""));
                o.push_str(&format!(
                    "        <Word id=\"P{page_no}_l{line_no}_w{}\">\n          <Coords points=\"{}\"/>\n          <TextEquiv{c}><Unicode>{}</Unicode></TextEquiv>\n        </Word>\n",
                    k + 1,
                    points(w.x, w.y, w.x.saturating_add(w.w), w.y.saturating_add(w.h)),
                    xml_escape(w.text.trim())
                ));
            }
            o.push_str(&format!(
                "        <TextEquiv><Unicode>{text}</Unicode></TextEquiv>\n      </TextLine>\n"
            ));
            region_text.push(text);
        }
        o.push_str(&format!(
            "      <TextEquiv><Unicode>{}</Unicode></TextEquiv>\n    </TextRegion>\n",
            region_text.join("\n")
        ));
    }
    o.push_str("  </Page>\n</PcGts>\n");
    o
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SS` (xsd:dateTime), without external crates.
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    iso_from_unix(secs)
}

fn iso_from_unix(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn w(text: &str, x: u32, y: u32, wd: u32, h: u32, conf: f32, line: u32) -> Word {
        Word {
            text: text.into(),
            x,
            y,
            w: wd,
            h,
            conf,
            line,
        }
    }

    fn bad_ref(v: &str) -> bool {
        let t = v
            .replace("&amp;", "")
            .replace("&lt;", "")
            .replace("&gt;", "")
            .replace("&quot;", "")
            .replace("&#39;", "");
        t.contains('&') || t.contains('<')
    }

    /// Minimal stack-based well-formedness check; returns element counts by name.
    fn check(xml: &str) -> Result<HashMap<String, usize>, String> {
        let b: Vec<char> = xml.chars().collect();
        let (mut i, mut stack) = (0, Vec::<String>::new());
        let mut counts = HashMap::new();
        let mut root_done = false;
        while i < b.len() {
            if b[i] != '<' {
                if b[i] == '>' {
                    return Err("stray >".into());
                }
                if b[i] == '&' {
                    let rest: String = b[i..].iter().take(8).collect();
                    let ok = ["&amp;", "&lt;", "&gt;", "&quot;", "&apos;", "&#39;"]
                        .iter()
                        .any(|e| rest.starts_with(e));
                    if !ok {
                        return Err(format!("bad entity near {rest}"));
                    }
                }
                let c = b[i] as u32;
                if (c < 0x20 && c != 9 && c != 10 && c != 13) || c == 0xFFFE || c == 0xFFFF {
                    return Err(format!("illegal char {c:#x}"));
                }
                if stack.is_empty() && !b[i].is_whitespace() {
                    return Err("text outside root".into());
                }
                i += 1;
                continue;
            }
            let rest: String = b[i..].iter().collect();
            if rest.starts_with("<?") {
                i += rest[..rest.find("?>").ok_or("unterminated PI")? + 2]
                    .chars()
                    .count();
                continue;
            }
            if rest.starts_with("<!--") {
                let e = rest.find("-->").ok_or("unterminated comment")?;
                if rest[4..e].contains("--") {
                    return Err("-- in comment".into());
                }
                i += rest[..e + 3].chars().count();
                continue;
            }
            let (mut j, mut q) = (i + 1, None);
            loop {
                let c = *b.get(j).ok_or("unterminated tag")?;
                match q {
                    Some(qc) if c == qc => q = None,
                    Some(_) => {}
                    None if c == '"' || c == '\'' => q = Some(c),
                    None if c == '>' => break,
                    None => {}
                }
                j += 1;
            }
            let tag: String = b[i + 1..j].iter().collect();
            i = j + 1;
            if let Some(name) = tag.strip_prefix('/') {
                let open = stack.pop().ok_or("close with empty stack")?;
                if open != name.trim() {
                    return Err(format!("mismatch {open} vs {name}"));
                }
                if stack.is_empty() {
                    root_done = true;
                }
            } else {
                if root_done {
                    return Err("second root".into());
                }
                let selfclose = tag.ends_with('/');
                let body = tag.trim_end_matches('/');
                let name: String = body.chars().take_while(|c| !c.is_whitespace()).collect();
                let mut seen = HashSet::new();
                let mut s = body[name.len()..].trim_start();
                while !s.is_empty() {
                    let eq = s.find('=').ok_or("attr without =")?;
                    let an = s[..eq].trim().to_string();
                    if !seen.insert(an.clone()) {
                        return Err(format!("duplicate attr {an}"));
                    }
                    let v = s[eq + 1..].trim_start();
                    let qc = v
                        .chars()
                        .next()
                        .filter(|c| *c == '"' || *c == '\'')
                        .ok_or("unquoted attr")?;
                    let end = v[1..].find(qc).ok_or("unterminated attr")? + 1;
                    let val = &v[1..end];
                    if bad_ref(val) {
                        return Err(format!("bad attr value {val}"));
                    }
                    s = v[end + 1..].trim_start();
                }
                *counts.entry(name.clone()).or_insert(0) += 1;
                if selfclose {
                    if stack.is_empty() {
                        root_done = true;
                    }
                } else {
                    stack.push(name);
                }
            }
        }
        if !stack.is_empty() || !root_done {
            return Err("unclosed elements".into());
        }
        Ok(counts)
    }

    fn sample() -> Vec<Word> {
        vec![
            w("Șoarecele", 10, 10, 90, 20, 96.0, 0),
            w("țărănesc", 110, 10, 80, 20, 40.0, 0),
            w("a&b<c>", 10, 40, 60, 20, 100.0, 1),
            w("\"x\"'y'", 80, 40, 50, 20, -1.0, 1),
            // far below: new paragraph
            w("Nou", 10, 200, 40, 20, 88.0, 2),
        ]
    }

    #[test]
    fn checker_rejects_bad_xml() {
        assert!(check("<a><b></a></b>").is_err());
        assert!(check("<a x=\"&\"/>").is_err());
        assert!(check("<a>&bogus;</a>").is_err());
        assert!(check("<a/><b/>").is_err());
        assert!(check("<a x='1' x='2'/>").is_err());
        assert!(check("<a>\u{1}</a>").is_err());
        assert!(check("<a>ok &amp; fine</a>").is_ok());
    }

    #[test]
    fn alto_structure() {
        let p = alto_page(&sample(), 600, 800, 300, 2);
        let doc = alto_document(&[p], "scan & co.pdf");
        let c = check(&doc).expect("well-formed");
        assert_eq!(c["alto"], 1);
        assert_eq!(c["Page"], 1);
        assert_eq!(c["TextBlock"], 2);
        assert_eq!(c["TextLine"], 3);
        assert_eq!(c["String"], 5);
        assert_eq!(c["SP"], 2);
        assert!(doc.contains("<MeasurementUnit>pixel</MeasurementUnit>"));
        assert!(doc.contains("scan &amp; co.pdf"));
        assert!(doc.contains("CONTENT=\"a&amp;b&lt;c&gt;\""));
        assert!(doc.contains("CONTENT=\"&quot;x&quot;&#39;y&#39;\""));
        assert!(doc.contains("CONTENT=\"Șoarecele\""));
        assert!(doc.contains("CONTENT=\"țărănesc\""));
        // SP fills the gap 100..110
        assert!(doc.contains("<SP HPOS=\"100\" VPOS=\"10\" WIDTH=\"10\" HEIGHT=\"20\"/>"));
        // unknown confidence has no WC; the others are within 0..1
        assert_eq!(doc.matches(" WC=\"").count(), 4);
        for part in doc.split(" WC=\"").skip(1) {
            let v: f32 = part.split('"').next().unwrap().parse().unwrap();
            assert!((0.0..=1.0).contains(&v));
        }
        assert!(doc.contains("WC=\"0.960\""));
    }

    #[test]
    fn alto_empty_page() {
        let doc = alto_document(&[alto_page(&[], 600, 800, 300, 1)], "e.png");
        let c = check(&doc).unwrap();
        assert_eq!(c["Page"], 1);
        assert_eq!(c["PrintSpace"], 1);
        assert!(!c.contains_key("TextBlock") && !c.contains_key("String"));
        check(&alto_document(&[], "")).unwrap();
    }

    #[test]
    fn illegal_chars_removed() {
        let ws = vec![
            w("a\u{1}b\u{0}c", 0, 0, 10, 10, 50.0, 0),
            w("\u{8}\u{FFFF}", 20, 0, 10, 10, 50.0, 0),
        ];
        let doc = alto_document(&[alto_page(&ws, 100, 100, 72, 1)], "x");
        let c = check(&doc).unwrap();
        assert_eq!(c["String"], 1); // the all-illegal word is dropped
        assert!(doc.contains("CONTENT=\"abc\""));
        let px = page_xml(&ws, 100, 100, 1);
        check(&px).unwrap();
        assert!(px.contains("<Unicode>abc</Unicode>"));
    }

    #[test]
    fn page_structure() {
        let doc = page_xml(&sample(), 600, 800, 1);
        let c = check(&doc).expect("well-formed");
        assert_eq!(c["PcGts"], 1);
        assert_eq!(c["TextRegion"], 2);
        assert_eq!(c["RegionRefIndexed"], 2);
        assert_eq!(c["TextLine"], 3);
        assert_eq!(c["Word"], 5);
        assert_eq!(c["Coords"], 10); // 2 regions + 3 lines + 5 words
        assert_eq!(c["TextEquiv"], 10); // 5 words + 3 lines + 2 regions
        assert!(doc.contains("<Coords points=\"10,10 100,10 100,30 10,30\"/>"));
        assert!(doc.contains("<Unicode>Șoarecele</Unicode>"));
        assert!(doc.contains("<Unicode>Șoarecele țărănesc</Unicode>"));
        assert!(doc.contains(
            "<Unicode>Șoarecele țărănesc\na&amp;b&lt;c&gt; &quot;x&quot;&#39;y&#39;</Unicode>"
        ));
        assert_eq!(doc.matches(" conf=\"").count(), 4);
        for part in doc.split(" conf=\"").skip(1) {
            let v: f32 = part.split('"').next().unwrap().parse().unwrap();
            assert!((0.0..=1.0).contains(&v));
        }
        assert!(doc.contains("index=\"0\" regionRef=\"P1_r1\""));
        assert!(doc.contains("id=\"P1_r2\""));
        assert!(doc.contains("imageFilename=\"page_1\""));
        let created = doc
            .split("<Created>")
            .nth(1)
            .unwrap()
            .split('<')
            .next()
            .unwrap();
        assert_eq!(created.len(), 19);
    }

    #[test]
    fn page_empty() {
        let doc = page_xml(&[], 10, 20, 3);
        let c = check(&doc).unwrap();
        assert_eq!(c["Page"], 1);
        assert!(!c.contains_key("TextRegion") && !c.contains_key("ReadingOrder"));
        assert!(doc.contains("imageWidth=\"10\" imageHeight=\"20\""));
    }

    #[test]
    fn named_image_is_escaped() {
        let doc = page_xml_named(&[], 1, 1, 1, "a\"b&c.png");
        check(&doc).unwrap();
        assert!(doc.contains("imageFilename=\"a&quot;b&amp;c.png\""));
    }

    #[test]
    fn date_algorithm() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00");
        assert_eq!(iso_from_unix(1_709_210_096), "2024-02-29T12:34:56");
        assert_eq!(iso_from_unix(1_793_577_600), "2026-11-02T00:00:00");
    }
}
