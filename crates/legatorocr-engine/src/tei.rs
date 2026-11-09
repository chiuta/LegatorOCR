//! TEI P5 output (roadmap 6.4, first step): one `<pb/>` per scanned page, a `<p>` per paragraph,
//! `<table>` for tables and `<div type="column">` for column sections. This is the page structure
//! the OCR found, not a scholarly edition: headings only as `<head>` for a short line clearly taller than the body text in the upper part of the page (a heuristic), no footnotes, bibliography or index markup,
//! and no check against the TEI schema was possible here (well-formedness only).

use crate::docx::{Block, GeoBlock, Rect};
use std::path::PathBuf;
use std::sync::Mutex;

static FACS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Folder where the page images of the facsimile are written (`--tei-facsimile`); `None` = off.
pub fn set_facsimile_dir(d: Option<PathBuf>) {
    *FACS_DIR.lock().unwrap_or_else(|e| e.into_inner()) = d;
}

pub fn facsimile_dir() -> Option<PathBuf> {
    FACS_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

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

/// One page of the facsimile: the image file as the TEI document should name it (a relative
/// path with `/`), its size in pixels, and the blocks with their rectangles on that image.
#[derive(Debug)]
pub struct FacsPage {
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub blocks: Vec<GeoBlock>,
}

/// Percent-encode a relative path as a URI reference (`/` stays).
fn uri_path(p: &str) -> String {
    let mut o = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

struct ZoneOut {
    id: String,
    r: Rect,
}

fn link(r: Option<Rect>, page: usize, zones: &mut Vec<ZoneOut>) -> String {
    match r {
        Some(r) => {
            let id = format!("z{page}-{}", zones.len() + 1);
            zones.push(ZoneOut { id: id.clone(), r });
            format!(" facs=\"#{id}\"")
        }
        None => String::new(),
    }
}

fn geo_blocks(
    bs: &[GeoBlock],
    depth: usize,
    page: usize,
    zones: &mut Vec<ZoneOut>,
    out: &mut String,
) {
    let pad = "  ".repeat(depth);
    for g in bs {
        match &g.block {
            Block::Para(t) => {
                if !t.trim().is_empty() {
                    let f = link(g.rect, page, zones);
                    out.push_str(&format!("{pad}<p{f}>{}</p>\n", esc(t.trim())));
                }
            }
            Block::Head(t) => {
                if !t.trim().is_empty() {
                    let f = link(g.rect, page, zones);
                    out.push_str(&format!("{pad}<head{f}>{}</head>\n", esc(t.trim())));
                }
            }
            Block::Table(rows) => {
                let f = link(g.rect, page, zones);
                out.push_str(&format!("{pad}<table{f}>\n"));
                for r in rows {
                    out.push_str(&format!("{pad}  <row>"));
                    for c in r {
                        out.push_str(&format!("<cell>{}</cell>", esc(c.trim())));
                    }
                    out.push_str("</row>\n");
                }
                out.push_str(&format!("{pad}</table>\n"));
            }
            Block::Columns(_) => {
                for c in &g.columns {
                    out.push_str(&format!("{pad}<div type=\"column\">\n"));
                    geo_blocks(c, depth + 1, page, zones, out);
                    out.push_str(&format!("{pad}</div>\n"));
                }
            }
        }
    }
}

/// Like [`render`], with a `<facsimile>` of one surface per page (the page image and one
/// rectangular zone per linked block, in pixels of that image with the origin at its top-left
/// corner) and `facs` links from the page breaks to the surfaces and from the blocks to the
/// zones. Each surface points back to its page break with `start`.
pub fn render_facs(
    pages: &[FacsPage],
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
    s.push_str("    </fileDesc>\n  </teiHeader>\n");
    let mut surfaces = String::from("  <facsimile>\n");
    let mut body = String::new();
    for (i, p) in pages.iter().enumerate() {
        let n = i + 1;
        let mut zones: Vec<ZoneOut> = Vec::new();
        let mut text = String::new();
        geo_blocks(&p.blocks, 3, n, &mut zones, &mut text);
        body.push_str(&format!(
            "      <pb n=\"{}\" xml:id=\"p{n}\" facs=\"#s{n}\"/>\n",
            labels
                .get(i)
                .copied()
                .flatten()
                .map(|v| v.to_string())
                .unwrap_or_else(|| n.to_string()),
        ));
        body.push_str(&text);
        surfaces.push_str(&format!(
            "    <surface xml:id=\"s{n}\" ulx=\"0\" uly=\"0\" lrx=\"{}\" lry=\"{}\" start=\"#p{n}\">\n      <graphic url=\"{}\"/>\n",
            p.width,
            p.height,
            esc(&uri_path(&p.image))
        ));
        for z in &zones {
            surfaces.push_str(&format!(
                "      <zone xml:id=\"{}\" ulx=\"{}\" uly=\"{}\" lrx=\"{}\" lry=\"{}\"/>\n",
                z.id,
                z.r.x,
                z.r.y,
                z.r.x.saturating_add(z.r.w),
                z.r.y.saturating_add(z.r.h)
            ));
        }
        surfaces.push_str("    </surface>\n");
    }
    surfaces.push_str("  </facsimile>\n");
    s.push_str(&surfaces);
    s.push_str("  <text>\n    <body>\n");
    s.push_str(&body);
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

    fn gp(t: &str, r: Option<Rect>) -> GeoBlock {
        GeoBlock {
            block: Block::Para(t.into()),
            rect: r,
            columns: Vec::new(),
        }
    }
    fn r(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    fn two_pages() -> Vec<FacsPage> {
        vec![
            FacsPage {
                image: "imgs/page 1.png".into(),
                width: 1000,
                height: 1400,
                blocks: vec![
                    gp("one", Some(r(100, 200, 300, 50))),
                    gp("two", Some(r(150, 220, 300, 50))),
                    gp("no box", None),
                ],
            },
            FacsPage {
                image: "imgs/page-2.png".into(),
                width: 800,
                height: 1100,
                blocks: vec![
                    gp("out", Some(r(700, 1050, 300, 200))),
                    GeoBlock {
                        block: Block::Columns(vec![]),
                        rect: None,
                        columns: vec![vec![gp("left", Some(r(10, 10, 100, 20)))]],
                    },
                ],
            },
        ]
    }

    /// Every `facs` and `start` value names an `xml:id` that exists; every id is unique.
    fn check_links(x: &str) {
        let doc = crate::xmlmini::parse(x).expect("well-formed");
        fn walk<'a>(n: &'a crate::xmlmini::Node, ids: &mut Vec<&'a str>, links: &mut Vec<&'a str>) {
            for (k, v) in &n.attrs {
                if k == "xml:id" {
                    ids.push(v);
                } else if k == "facs" || k == "start" {
                    links.push(v);
                }
            }
            for c in &n.children {
                walk(c, ids, links);
            }
        }
        let (mut ids, mut links) = (Vec::new(), Vec::new());
        walk(&doc.root, &mut ids, &mut links);
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids are unique: {ids:?}");
        for l in links {
            assert!(
                ids.contains(&l.trim_start_matches('#')),
                "unresolved {l} in {ids:?}"
            );
        }
    }

    #[test]
    fn facsimile_has_a_surface_per_page_with_pixel_extent() {
        let x = render_facs(&two_pages(), "T", Some("ro"), &[]);
        assert_eq!(x.matches("<surface ").count(), 2);
        assert!(x.contains("ulx=\"0\" uly=\"0\" lrx=\"1000\" lry=\"1400\" start=\"#p1\""));
        assert!(x.contains("lrx=\"800\" lry=\"1100\" start=\"#p2\""));
        assert!(x.contains("<graphic url=\"imgs/page%201.png\"/>"));
        check_links(&x);
        assert!(x.contains("<pb n=\"1\" xml:id=\"p1\" facs=\"#s1\"/>"));
    }

    #[test]
    fn zones_hold_the_rectangle_overlaps_and_overflow_are_kept() {
        let x = render_facs(&two_pages(), "T", None, &[]);
        assert!(
            x.contains("ulx=\"100\" uly=\"200\" lrx=\"400\" lry=\"250\""),
            "{x}"
        );
        assert!(x.contains("<p facs=\"#z1-1\">one</p>"));
        // overlapping rectangles: both zones exist
        assert!(x.contains("ulx=\"150\" uly=\"220\" lrx=\"450\" lry=\"270\""));
        // outside the surface (1000 > 800 wide): not clipped
        assert!(x.contains("ulx=\"700\" uly=\"1050\" lrx=\"1000\" lry=\"1250\""));
        // a block without a rectangle has no link
        assert!(x.contains("<p>no box</p>"));
        // columns link their blocks
        assert!(x.contains("<p facs=\"#z2-2\">left</p>"));
        assert_eq!(x.matches("<zone ").count(), 4);
    }

    #[test]
    fn page_without_ocr_has_a_surface_and_no_zones() {
        let p = vec![FacsPage {
            image: "a.png".into(),
            width: 10,
            height: 20,
            blocks: Vec::new(),
        }];
        let x = render_facs(&p, "T", None, &[Some(7)]);
        assert!(x.contains("<surface ") && !x.contains("<zone "));
        assert!(x.contains("<pb n=\"7\" xml:id=\"p1\" facs=\"#s1\"/>"));
        check_links(&x);
    }

    #[test]
    fn plain_render_is_unchanged_without_facsimile() {
        let x = render(&[vec![Block::Para("a".into())]], "T", None, &[]);
        assert!(!x.contains("facsimile") && !x.contains("facs="));
    }
}
