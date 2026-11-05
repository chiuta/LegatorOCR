//! `--epub FILE.epub` (with `--ocr`): the recognised text as a reflowable EPUB 3 book. One XHTML
//! document; headings found on the pages become the table of contents, every scanned page leaves
//! an invisible `pagebreak` marker so the book keeps its page numbers (`page-list` in the
//! navigation document). Tables become HTML tables, columns are read one after the other.
//! Not checked with EPUBCheck (not available here): the structure follows the EPUB 3.3 package
//! rules and is read back by the tests below.
use crate::docx::Block;
use crate::zipmini;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn blocks_html(
    blocks: &[Block],
    out: &mut String,
    toc: &mut Vec<(String, String)>,
    page: usize,
    n: &mut usize,
) {
    for b in blocks {
        match b {
            Block::Para(t) => out.push_str(&format!("<p>{}</p>\n", esc(t))),
            Block::Head(t) => {
                *n += 1;
                let id = format!("h{}", *n);
                out.push_str(&format!("<h2 id=\"{id}\">{}</h2>\n", esc(t)));
                toc.push((id, t.clone()));
                let _ = page;
            }
            Block::Table(rows) => {
                out.push_str("<table>\n");
                for r in rows {
                    out.push_str("<tr>");
                    for c in r {
                        out.push_str(&format!("<td>{}</td>", esc(c)));
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</table>\n");
            }
            Block::Columns(cols) => {
                for c in cols {
                    blocks_html(c, out, toc, page, n);
                }
            }
        }
    }
}

/// `pages[i]` holds the blocks of scanned page *i* (as for the Word export). `lang` is a BCP 47
/// tag ("ro"). `title` names the book.
pub fn from_blocks(pages: &[Vec<Block>], title: &str, lang: &str) -> Vec<u8> {
    let mut body = String::new();
    let mut toc: Vec<(String, String)> = Vec::new();
    let mut pagelist = String::new();
    let mut n = 0;
    for (i, p) in pages.iter().enumerate() {
        let id = format!("p{}", i + 1);
        body.push_str(&format!(
            "<span epub:type=\"pagebreak\" id=\"{id}\" role=\"doc-pagebreak\" aria-label=\"{}\"></span>\n",
            i + 1
        ));
        pagelist.push_str(&format!(
            "<li><a href=\"content.xhtml#{id}\">{}</a></li>\n",
            i + 1
        ));
        blocks_html(p, &mut body, &mut toc, i, &mut n);
    }
    let ns = "xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"";
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html {ns} lang=\"{l}\" xml:lang=\"{l}\"><head><meta charset=\"utf-8\"/><title>{t}</title>\
         <style>p{{margin:0 0 .6em;text-align:justify}}table{{border-collapse:collapse}}td{{padding:.1em .4em;border:1px solid #888}}</style></head>\
         <body>\n{body}</body></html>\n",
        l = esc(lang),
        t = esc(title)
    );
    let mut toc_items = String::new();
    for (id, t) in &toc {
        toc_items.push_str(&format!(
            "<li><a href=\"content.xhtml#{id}\">{}</a></li>\n",
            esc(t)
        ));
    }
    if toc_items.is_empty() {
        toc_items = format!("<li><a href=\"content.xhtml\">{}</a></li>\n", esc(title));
    }
    let nav = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html {ns} lang=\"{l}\" xml:lang=\"{l}\"><head><meta charset=\"utf-8\"/><title>{t}</title></head><body>\n\
         <nav epub:type=\"toc\" id=\"toc\"><h1>{t}</h1><ol>\n{toc_items}</ol></nav>\n\
         <nav epub:type=\"page-list\" hidden=\"hidden\"><ol>\n{pagelist}</ol></nav>\n</body></html>\n",
        l = esc(lang),
        t = esc(title)
    );
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bid\" xml:lang=\"{l}\">\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"bid\">urn:legatorocr:{id}</dc:identifier><dc:title>{t}</dc:title><dc:language>{l}</dc:language>\
         <dc:creator>LegatorOCR</dc:creator><meta property=\"dcterms:modified\">1980-01-01T00:00:00Z</meta>\
         <meta property=\"schema:accessMode\">textual</meta><meta property=\"schema:accessibilityFeature\">pageBreakMarkers</meta><meta property=\"schema:accessibilityFeature\">tableOfContents</meta></metadata>\
         <manifest><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/><item id=\"c\" href=\"content.xhtml\" media-type=\"application/xhtml+xml\"/></manifest>\
         <spine><itemref idref=\"c\"/></spine></package>",
        l = esc(lang),
        t = esc(title),
        id = fnv(&content)
    );
    let container = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    zipmini::write(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/content.xhtml", content.as_bytes()),
    ])
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(z: &[u8], name: &str) -> String {
        let l = zipmini::list(z).unwrap();
        let e = l.iter().find(|e| e.name == name).expect(name);
        String::from_utf8(zipmini::read(z, e).expect(name)).unwrap()
    }

    #[test]
    fn the_book_has_the_required_parts_headings_in_the_toc_and_page_markers() {
        let pages = vec![
            vec![
                Block::Head("Capitolul 1".into()),
                Block::Para("Text cu <semne> & \"ghilimele\".".into()),
            ],
            vec![
                Block::Table(vec![vec!["a".into(), "b".into()]]),
                Block::Columns(vec![
                    vec![Block::Para("st".into())],
                    vec![Block::Para("dr".into())],
                ]),
            ],
        ];
        let z = from_blocks(&pages, "Carte & Co", "ro");
        let list = zipmini::list(&z).unwrap();
        assert_eq!(list[0].name, "mimetype", "mimetype must come first");
        assert_eq!(read(&z, "mimetype"), "application/epub+zip");
        // stored, not deflated: the bytes of the first entry follow its 30-byte header + name
        assert_eq!(&z[30..38], b"mimetype");
        assert_eq!(&z[38..58], b"application/epub+zip");
        let c = read(&z, "OEBPS/content.xhtml");
        assert!(c.contains("Text cu &lt;semne&gt; &amp; &quot;ghilimele&quot;."));
        assert!(c.contains("id=\"p1\"") && c.contains("id=\"p2\""));
        assert!(c.contains("<td>a</td>") && c.contains("<p>st</p>") && c.contains("<p>dr</p>"));
        let nav = read(&z, "OEBPS/nav.xhtml");
        assert!(nav.contains("content.xhtml#h1\">Capitolul 1<"));
        assert!(nav.contains("epub:type=\"page-list\"") && nav.contains("#p2\">2<"));
        let opf = read(&z, "OEBPS/content.opf");
        assert!(
            opf.contains("<dc:title>Carte &amp; Co</dc:title>")
                && opf.contains("properties=\"nav\"")
        );
        for part in [
            "OEBPS/content.xhtml",
            "OEBPS/nav.xhtml",
            "OEBPS/content.opf",
            "META-INF/container.xml",
        ] {
            assert!(
                well_formed(&read(&z, part)),
                "{part} is not well-formed XML"
            );
        }
    }

    #[test]
    fn a_book_without_headings_still_has_a_toc_entry() {
        let z = from_blocks(&[vec![Block::Para("x".into())]], "T", "ro");
        assert!(read(&z, "OEBPS/nav.xhtml").contains("<li><a href=\"content.xhtml\">T</a></li>"));
    }

    /// Minimal well-formedness check: tags balance and entities are known.
    fn well_formed(x: &str) -> bool {
        let mut stack: Vec<String> = Vec::new();
        let b = x.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'<' {
                let j = x[i..].find('>').map(|k| i + k).unwrap_or(b.len() - 1);
                let t = &x[i + 1..j];
                if t.starts_with('?') || t.starts_with('!') || t.ends_with('/') {
                } else if let Some(n) = t.strip_prefix('/') {
                    if stack.pop().as_deref() != Some(n.trim()) {
                        return false;
                    }
                } else {
                    stack.push(t.split_whitespace().next().unwrap_or("").to_string());
                }
                i = j + 1;
            } else {
                if b[i] == b'&' {
                    let k = x[i..].find(';').unwrap_or(99);
                    if !["&amp;", "&lt;", "&gt;", "&quot;"].contains(&&x[i..i + k + 1]) {
                        return false;
                    }
                }
                i += 1;
            }
        }
        stack.is_empty()
    }
}
