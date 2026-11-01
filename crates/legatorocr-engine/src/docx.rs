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
            '\t' => o.push(' '),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {} // invalid in XML 1.0
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

/// A block of a page: a flowing paragraph or a table (rows of cells).
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Para(String),
    Table(Vec<Vec<String>>),
    /// A section laid out in columns (left to right), each a list of blocks.
    Columns(Vec<Vec<Block>>),
}

/// Tables and paragraphs from the positioned words of one page. Text is cut into cells where a
/// gap exceeds 2.5 line heights; cells that sit on the same row of the page (even when the OCR
/// put them in different blocks, as Tesseract does for table columns) are put side by side, and
/// three or more consecutive rows with the same number of cells whose left edges line up become
/// a table. Everything else is flowing text in the order the OCR gave. Nothing is invented: cell
/// text is the recognised words, in order.
pub fn blocks_from_words(words: &[legatorocr_core::text::Word]) -> Vec<Block> {
    match split_columns(words) {
        Some((head, cols)) => {
            let mut out = blocks_flat(&head);
            out.push(Block::Columns(
                cols.iter().map(|c| blocks_flat(c)).collect(),
            ));
            out
        }
        None => blocks_flat(words),
    }
}

/// A page set in two columns: a gutter of at least 2.5 line heights, free of text on nearly
/// every line, between two bands that each hold a fifth or more of the words. Lines that run
/// across the gutter (a title) must all sit above the columns. Returns the words of those
/// heading lines and of each column.
fn split_columns(
    words: &[legatorocr_core::text::Word],
) -> Option<(
    Vec<legatorocr_core::text::Word>,
    Vec<Vec<legatorocr_core::text::Word>>,
)> {
    let ws: Vec<&legatorocr_core::text::Word> = words
        .iter()
        .filter(|w| !w.text.trim().is_empty() && w.w > 0 && w.h > 0)
        .collect();
    if ws.len() < 40 {
        return None;
    }
    let mut hs: Vec<f32> = ws.iter().map(|w| w.h as f32).collect();
    hs.sort_by(|a, b| a.total_cmp(b));
    let med = hs[hs.len() / 2];
    let x0 = ws.iter().map(|w| w.x as f32).fold(f32::MAX, f32::min);
    let x1 = ws
        .iter()
        .map(|w| w.x as f32 + w.w as f32)
        .fold(f32::MIN, f32::max);
    let width = x1 - x0;
    if !(width.is_finite()) || width < 12.0 * med {
        return None;
    }
    // lines (by id) and the x extent of their words
    let mut ids: Vec<u32> = ws.iter().map(|w| w.line).collect();
    ids.sort_unstable();
    ids.dedup();
    let nl = ids.len();
    if nl < 6 {
        return None;
    }
    // how many lines have a word over each probe position of the central band
    let probes = 200usize;
    let mut cover = vec![0usize; probes];
    for (p, c) in cover.iter_mut().enumerate() {
        let x = x0 + width * (0.3 + 0.4 * p as f32 / probes as f32);
        let mut hit: Vec<u32> = ws
            .iter()
            .filter(|w| (w.x as f32) <= x && x < w.x as f32 + w.w as f32)
            .map(|w| w.line)
            .collect();
        hit.sort_unstable();
        hit.dedup();
        *c = hit.len();
    }
    let limit = (nl / 8).max(1);
    // widest run of probes crossed by few lines
    let (mut best, mut cur_start, mut best_run) = (0usize, 0usize, (0usize, 0usize));
    for p in 0..=probes {
        if cover.get(p).is_some_and(|&c| c <= limit) {
            if best == 0 {
                cur_start = p;
            }
            best += 1;
            if best > best_run.1 - best_run.0 {
                best_run = (cur_start, p + 1);
            }
        } else {
            best = 0;
        }
    }
    let gutter_w = (best_run.1 - best_run.0) as f32 * 0.4 * width / probes as f32;

    if gutter_w < 2.5 * med {
        return None;
    }
    let (gl, gr) = (
        x0 + width * (0.3 + 0.4 * best_run.0 as f32 / probes as f32),
        x0 + width * (0.3 + 0.4 * best_run.1 as f32 / probes as f32),
    );
    // the run's end probes touch text: pull the gutter in by one probe on each side
    let step = 0.4 * width / probes as f32;
    let (gl, gr) = (gl + step, gr - 1.5 * step);
    let mid = (gl + gr) / 2.0;
    let crosses =
        |w: &legatorocr_core::text::Word| (w.x as f32) < gr && (w.x as f32 + w.w as f32) > gl;
    let crossing: Vec<u32> = ws.iter().filter(|w| crosses(w)).map(|w| w.line).collect();
    let center = |w: &legatorocr_core::text::Word| w.x as f32 + w.w as f32 / 2.0;
    let (mut head, mut left, mut right) = (Vec::new(), Vec::new(), Vec::new());
    let mut first_col_top = f32::MAX;
    for w in &ws {
        if !crossing.contains(&w.line) {
            first_col_top = first_col_top.min(w.y as f32);
        }
    }
    for w in &ws {
        if crossing.contains(&w.line) {
            // a heading line must lie above every column line
            if w.y as f32 + w.h as f32 > first_col_top + 0.5 * med {
                return None;
            }
            head.push((*w).clone());
        } else if center(w) < mid {
            left.push((*w).clone());
        } else {
            right.push((*w).clone());
        }
    }
    if left.len() * 5 < ws.len() || right.len() * 5 < ws.len() {
        return None;
    }
    Some((head, vec![left, right]))
}

fn blocks_flat(words: &[legatorocr_core::text::Word]) -> Vec<Block> {
    struct Cell {
        l: f32,
        r: f32,
        text: String,
    }
    struct Line {
        cells: Vec<Cell>,
        top: f32,
        h: f32,
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let mut j = i + 1;
        while j < words.len() && words[j].line == words[i].line {
            j += 1;
        }
        let mut g: Vec<&legatorocr_core::text::Word> = words[i..j]
            .iter()
            .filter(|w| !w.text.trim().is_empty())
            .collect();
        i = j;
        if g.is_empty() {
            continue;
        }
        g.sort_by_key(|w| w.x);
        let top = g.iter().map(|w| w.y).min().unwrap_or(0) as f32;
        let h = g.iter().map(|w| w.h).max().unwrap_or(1).max(1) as f32;
        let mut cells: Vec<Cell> = Vec::new();
        for w in g {
            let (l, r) = (w.x as f32, w.x as f32 + w.w as f32);
            match cells.last_mut() {
                Some(c) if l - c.r <= 2.5 * h => {
                    c.r = c.r.max(r);
                    c.text.push(' ');
                    c.text.push_str(w.text.trim());
                }
                _ => cells.push(Cell {
                    l,
                    r,
                    text: w.text.trim().to_string(),
                }),
            }
        }
        lines.push(Line { cells, top, h });
    }
    if lines.is_empty() {
        return Vec::new();
    }
    let mut hs: Vec<f32> = lines.iter().map(|l| l.h).collect();
    hs.sort_by(|a, b| a.total_cmp(b));
    let med = hs[hs.len() / 2];
    let tol = 2.0 * med;

    // rows of the page: lines whose vertical centres agree, cells merged left to right
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| {
        (lines[a].top + lines[a].h / 2.0).total_cmp(&(lines[b].top + lines[b].h / 2.0))
    });
    let mut rows: Vec<(Vec<usize>, f32)> = Vec::new(); // (line indices, centre)
    for &li in &order {
        let c = lines[li].top + lines[li].h / 2.0;
        match rows.last_mut() {
            Some((m, rc)) if (c - *rc).abs() < 0.5 * med => {
                m.push(li);
            }
            _ => rows.push((vec![li], c)),
        }
    }
    // cells of every row, left to right (cells of different lines never merge)
    let row_cells: Vec<Vec<(f32, usize, usize)>> = rows
        .iter()
        .map(|(m, _)| {
            let mut v: Vec<(f32, usize, usize)> = Vec::new();
            for &li in m {
                for (ci, c) in lines[li].cells.iter().enumerate() {
                    v.push((c.l, li, ci));
                }
            }
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v
        })
        .collect();

    let mut consumed = vec![false; lines.len()];
    let mut tables: Vec<(usize, Block)> = Vec::new(); // (first line index in reading order, table)
    let mut k = 0;
    while k < rows.len() {
        let mut e = k;
        if row_cells[k].len() >= 2 {
            e = k + 1;
            while e < rows.len()
                && row_cells[e].len() == row_cells[k].len()
                && row_cells[e]
                    .iter()
                    .zip(&row_cells[k])
                    .all(|(a, b)| (a.0 - b.0).abs() <= tol)
            {
                e += 1;
            }
        }
        if e - k >= 3 {
            let mut first = usize::MAX;
            let mut table: Vec<Vec<String>> = Vec::new();
            for r in k..e {
                for &li in &rows[r].0 {
                    consumed[li] = true;
                    first = first.min(li);
                }
                table.push(
                    row_cells[r]
                        .iter()
                        .map(|&(_, li, ci)| lines[li].cells[ci].text.clone())
                        .collect(),
                );
            }
            tables.push((first, Block::Table(table)));
            k = e;
        } else {
            k += 1;
        }
    }

    let mut out: Vec<Block> = Vec::new();
    let mut para = String::new();
    let mut prev_bottom: Option<f32> = None;
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(std::mem::take(para)));
        }
    };
    for (li, l) in lines.iter().enumerate() {
        if let Some(pos) = tables.iter().position(|t| t.0 == li) {
            flush(&mut para, &mut out);
            out.push(tables.remove(pos).1);
            prev_bottom = None;
        }
        if consumed[li] {
            continue;
        }
        let text = l
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if prev_bottom.is_some_and(|b| l.top - b > 0.9 * med) {
            flush(&mut para, &mut out);
        }
        if !para.is_empty() && !para.ends_with('-') {
            para.push(' ');
        }
        para.push_str(&text);
        prev_bottom = Some(l.top + l.h);
    }
    flush(&mut para, &mut out);
    out
}

fn blocks_xml(blocks: &[Block]) -> String {
    let mut body = String::new();
    for b in blocks {
        match b {
            Block::Para(t) => body.push_str(&format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                esc(t)
            )),
            Block::Columns(cols) => {
                // the single-column part so far ends here; the columns form a continuous
                // section of their own, closed by the paragraph that carries its properties
                body.push_str(
                    "<w:p><w:pPr><w:sectPr><w:type w:val=\"continuous\"/>\
                     <w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:pPr></w:p>",
                );
                for (k, c) in cols.iter().enumerate() {
                    if k > 0 {
                        body.push_str("<w:p><w:r><w:br w:type=\"column\"/></w:r></w:p>");
                    }
                    body.push_str(&blocks_xml(c));
                }
                body.push_str(&format!(
                    "<w:p><w:pPr><w:sectPr><w:type w:val=\"continuous\"/>\
                     <w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
                     <w:cols w:num=\"{}\" w:space=\"567\"/></w:sectPr></w:pPr></w:p>",
                    cols.len().clamp(1, 8)
                ));
            }
            Block::Table(rows) => {
                body.push_str(
                    "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>\
                     <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"808080\"/>\
                     </w:tblBorders></w:tblPr>",
                );
                for r in rows {
                    body.push_str("<w:tr>");
                    for c in r {
                        body.push_str(&format!(
                            "<w:tc><w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>",
                            esc(c)
                        ));
                    }
                    body.push_str("</w:tr>");
                }
                // a table must be followed by a paragraph before the next block or the section
                body.push_str("</w:tbl><w:p/>");
            }
        }
    }
    body
}

/// `pages[i]` is the text of page *i*: lines separated by `\n`, paragraphs by a blank line.
/// Text without blank lines is therefore one paragraph per page.
pub fn from_pages(pages: &[String], title: &str) -> Vec<u8> {
    let blocks: Vec<Vec<Block>> = pages
        .iter()
        .map(|p| paragraphs(p).into_iter().map(Block::Para).collect())
        .collect();
    from_blocks(&blocks, title)
}

/// One entry per page; a page break separates them.
pub fn from_blocks(pages: &[Vec<Block>], title: &str) -> Vec<u8> {
    let mut body = String::new();
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        body.push_str(&blocks_xml(p));
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

    fn w(text: &str, x: u32, y: u32, wd: u32, line: u32) -> legatorocr_core::text::Word {
        legatorocr_core::text::Word {
            text: text.into(),
            x,
            y,
            w: wd,
            h: 20,
            conf: 90.0,
            line,
        }
    }

    #[test]
    fn aligned_columns_become_a_table_and_prose_stays_prose() {
        let mut v = vec![w("Intro", 100, 100, 60, 0), w("text", 170, 100, 50, 0)];
        for (r, (a, b, c)) in [
            ("Nume", "Preț", "Stoc"),
            ("Mere", "3", "10"),
            ("Pere", "5", "7"),
        ]
        .iter()
        .enumerate()
        {
            let y = 200 + r as u32 * 30;
            v.push(w(a, 100, y, 80, 1 + r as u32));
            v.push(w(b, 400, y, 40, 1 + r as u32));
            v.push(w(c, 700, y, 40, 1 + r as u32));
        }
        v.push(w("Sfârșit", 100, 400, 80, 9));
        let b = blocks_from_words(&v);
        assert_eq!(b.len(), 3, "{b:?}");
        assert_eq!(b[0], Block::Para("Intro text".into()));
        let Block::Table(rows) = &b[1] else {
            panic!("{b:?}")
        };
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["Mere", "3", "10"]);
        assert_eq!(b[2], Block::Para("Sfârșit".into()));
        let doc = from_blocks(&[b], "t");
        let text = String::from_utf8_lossy(&doc).into_owned();
        assert!(doc.starts_with(b"PK"));
        assert!(text.len() > 100);
    }

    #[test]
    fn two_column_page_becomes_a_columns_section_and_one_column_does_not() {
        let mut v = vec![w("Titlu", 100, 40, 900, 0)];
        let mut id = 1;
        for r in 0..14u32 {
            for (k, x0) in [100u32, 700].iter().enumerate() {
                for c in 0..4u32 {
                    v.push(w(
                        &format!("c{k}r{r}w{c}"),
                        x0 + c * 130,
                        120 + r * 30,
                        110,
                        id,
                    ));
                }
                id += 1;
            }
        }
        let b = blocks_from_words(&v);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(matches!(&b[0], Block::Para(t) if t == "Titlu"));
        let Block::Columns(cols) = &b[1] else {
            panic!("{b:?}")
        };
        assert_eq!(cols.len(), 2);
        let text = |c: &Vec<Block>| match &c[0] {
            Block::Para(t) => t.clone(),
            other => format!("{other:?}"),
        };
        assert!(text(&cols[0]).starts_with("c0r0w0"), "{}", text(&cols[0]));
        assert!(text(&cols[1]).starts_with("c1r0w0"));
        let docx = from_blocks(&[b], "t");
        let xml = zipmini_text(&docx);
        assert!(xml.contains("w:cols w:num=\"2\""));
        assert!(xml.contains("w:type=\"column\""));
        // a single wide column of text is left alone
        let one: Vec<_> = (0..60u32)
            .map(|i| {
                w(
                    &format!("t{i}"),
                    100 + (i % 10) * 90,
                    100 + (i / 10) * 30,
                    80,
                    i / 10,
                )
            })
            .collect();
        assert!(blocks_from_words(&one)
            .iter()
            .all(|b| !matches!(b, Block::Columns(_))));
    }

    fn zipmini_text(zip: &[u8]) -> String {
        let list = crate::zipmini::list(zip).expect("zip");
        let e = list
            .iter()
            .find(|e| e.name == "word/document.xml")
            .expect("entry");
        String::from_utf8_lossy(&crate::zipmini::read(zip, e).expect("read")).into_owned()
    }

    #[test]
    fn two_line_columns_and_empty_input_are_not_tables() {
        assert!(blocks_from_words(&[]).is_empty());
        let v = vec![
            w("a", 100, 100, 20, 0),
            w("b", 500, 100, 20, 0),
            w("c", 100, 130, 20, 1),
            w("d", 500, 130, 20, 1),
        ];
        assert!(blocks_from_words(&v)
            .iter()
            .all(|b| matches!(b, Block::Para(_))));
        // hostile boxes must not overflow
        let h = vec![
            w("x", u32::MAX, u32::MAX, u32::MAX, 0),
            w("y", 0, 0, u32::MAX, 0),
        ];
        let _ = blocks_from_words(&h);
    }

    #[test]
    fn blank_lines_split_paragraphs_and_lines_are_joined() {
        let p = super::paragraphs("alpha\nbeta con-\ntinued\n\ngamma\n\n\ndelta");
        assert_eq!(p, vec!["alpha beta con-tinued", "gamma", "delta"]);
    }
}
