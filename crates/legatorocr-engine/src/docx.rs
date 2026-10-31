//! Minimal Word (`.docx`) export of recognised text: one paragraph per line, a page
//! break between pages. Plain, style-less, opens in Word, LibreOffice and Google Docs.

use crate::zipmini;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o
}

/// Split page text into paragraphs: blank lines separate paragraphs, the lines of one
/// paragraph are joined into a flowing paragraph (a line ending in `-` is joined without a space,
/// so a hyphenated word break stays visible and nothing is silently altered).
fn paragraphs(page: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in page.lines() {
        let l = line.trim();
        if l.is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() && !cur.ends_with('-') {
            cur.push(' ');
        }
        cur.push_str(l);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `pages[i]` is the text of page *i*: lines separated by `\n`, paragraphs by a blank line.
/// Text without blank lines is therefore one paragraph per page.
pub fn from_pages(pages: &[String], title: &str) -> Vec<u8> {
    let mut body = String::new();
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        for para in paragraphs(p) {
            body.push_str(&format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                esc(&para)
            ));
        }
    }
    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}\
         <w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:body></w:document>"
    );
    let ct = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
        <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
        </Types>";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
        </Relationships>";
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>LegatorOCR</dc:creator></cp:coreProperties>",
        esc(title)
    );
    zipmini::write(&[
        ("[Content_Types].xml", ct.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("docProps/core.xml", core.as_bytes()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_xml_has_the_text() {
        let z = from_pages(&["Mobilă & <ț>\nal doilea".into(), "pagina 2".into()], "T");
        let l = zipmini::list(&z).unwrap();
        let e = l.iter().find(|e| e.name == "word/document.xml").unwrap();
        let x = String::from_utf8(zipmini::read(&z, e).unwrap()).unwrap();
        assert!(x.contains("Mobilă &amp; &lt;ț&gt;"));
        assert!(x.contains("w:type=\"page\""));
        assert_eq!(x.matches("<w:p>").count(), 3); // text page, page break, text page
    }

    #[test]
    fn blank_lines_split_paragraphs_and_lines_are_joined() {
        let p = super::paragraphs("alpha\nbeta con-\ntinued\n\ngamma\n\n\ndelta");
        assert_eq!(p, vec!["alpha beta con-tinued", "gamma", "delta"]);
    }
}
