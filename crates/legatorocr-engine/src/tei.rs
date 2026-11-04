//! TEI P5 output (roadmap 6.4, first step): one `<pb/>` per scanned page, a `<p>` per paragraph,
//! `<table>` for tables and `<div type="column">` for column sections. This is the page structure
//! the OCR found, not a scholarly edition: headings only as `<head>` for a short line clearly taller than the body text in the upper part of the page (a heuristic), no footnotes, bibliography or index markup,
//! and no check against the TEI schema was possible here (well-formedness only).

use crate::docx::Block;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn blocks(bs: &[Block], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    for b in bs {
        match b {
            Block::Para(t) => {
                if !t.trim().is_empty() {
                    out.push_str(&format!("{pad}<p>{}</p>\n", esc(t.trim())));
                }
            }
            Block::Head(t) => {
                if !t.trim().is_empty() {
                    out.push_str(&format!("{pad}<head>{}</head>\n", esc(t.trim())));
                }
            }
            Block::Table(rows) => {
                out.push_str(&format!("{pad}<table>\n"));
                for r in rows {
                    out.push_str(&format!("{pad}  <row>"));
                    for c in r {
                        out.push_str(&format!("<cell>{}</cell>", esc(c.trim())));
                    }
                    out.push_str("</row>\n");
                }
                out.push_str(&format!("{pad}</table>\n"));
            }
            Block::Columns(cols) => {
                for c in cols {
                    out.push_str(&format!("{pad}<div type=\"column\">\n"));
                    blocks(c, depth + 1, out);
                    out.push_str(&format!("{pad}</div>\n"));
                }
            }
        }
    }
}

/// `lang`: BCP 47 code for `xml:lang` (e.g. `ro`); `labels`: printed page numbers if known.
pub fn render(
    pages: &[Vec<Block>],
    title: &str,
    lang: Option<&str>,
    labels: &[Option<u32>],
) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TEI xmlns=\"http://www.tei-c.org/ns/1.0\"",
    );
    if let Some(l) = lang {
        s.push_str(&format!(" xml:lang=\"{}\"", esc(l)));
    }
    s.push_str(">\n  <teiHeader>\n    <fileDesc>\n");
    s.push_str(&format!(
        "      <titleStmt><title>{}</title></titleStmt>\n",
        esc(title)
    ));
    s.push_str("      <publicationStmt><p>Text recognised by LegatorOCR; unproofread.</p></publicationStmt>\n");
    s.push_str("      <sourceDesc><p>Born-digital from scanned page images.</p></sourceDesc>\n");
    s.push_str("    </fileDesc>\n  </teiHeader>\n  <text>\n    <body>\n");
    for (i, p) in pages.iter().enumerate() {
        s.push_str(&format!(
            "      <pb n=\"{}\" xml:id=\"p{}\"/>\n",
            labels
                .get(i)
                .copied()
                .flatten()
                .map(|n| n.to_string())
                .unwrap_or_else(|| (i + 1).to_string()),
            i + 1
        ));
        blocks(p, 3, &mut s);
    }
    s.push_str("    </body>\n  </text>\n</TEI>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_and_escaping() {
        let pages = vec![
            vec![
                Block::Para("A & B <c>".into()),
                Block::Table(vec![vec!["1".into(), "2".into()]]),
            ],
            vec![Block::Columns(vec![
                vec![Block::Para("left".into())],
                vec![Block::Para("right".into())],
            ])],
        ];
        let x = render(&pages, "Carte \"x\"", Some("ro"), &[None, Some(7)]);
        assert!(x.contains("<p>A &amp; B &lt;c&gt;</p>") && x.contains("<cell>2</cell>"));
        assert!(
            x.contains("<pb n=\"1\" xml:id=\"p1\"/>") && x.contains("<pb n=\"7\" xml:id=\"p2\"/>")
        );
        assert_eq!(x.matches("<div type=\"column\">").count(), 2);
        assert!(x.contains("xml:lang=\"ro\"") && x.contains("&quot;x&quot;"));
        // balanced tags: every open has a close
        assert_eq!(x.matches("<p>").count(), x.matches("</p>").count());
    }
}
