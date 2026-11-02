//! Tables found on the pages (see `docx::blocks_from_words`) as an Excel workbook (.xlsx) or
//! an HTML file. One worksheet per table, named `p<page>-t<n>`. Cell text is exactly the
//! recognised words; only plain integers and decimals with a dot are stored as numbers (so
//! `007`, `1.234,56` and dates stay text, as printed).

use crate::docx::Block;

/// A table with the page (1-based) it came from.
pub struct Found {
    pub page: usize,
    pub n: usize,
    pub rows: Vec<Vec<String>>,
}

fn collect(blocks: &[Block], page: usize, out: &mut Vec<Found>) {
    for b in blocks {
        match b {
            Block::Table(rows) => {
                let n = out.iter().filter(|f| f.page == page).count() + 1;
                out.push(Found { page, n, rows: rows.clone() });
            }
            Block::Columns(cols) => {
                for c in cols {
                    collect(c, page, out);
                }
            }
            Block::Para(_) => {}
        }
    }
}

/// All tables of all pages, in page order.
pub fn find(pages: &[Vec<Block>]) -> Vec<Found> {
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        collect(p, i + 1, &mut out);
    }
    out
}

fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn is_plain_number(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = t.split_once('.').map_or((t, None), |(a, b)| (a, Some(b)));
    let int_ok = int == "0" || (!int.is_empty() && !int.starts_with('0') && int.len() <= 15 && int.bytes().all(|b| b.is_ascii_digit()));
    let frac_ok = frac.is_none_or(|f| !f.is_empty() && f.len() <= 10 && f.bytes().all(|b| b.is_ascii_digit()));
    int_ok && frac_ok
}

fn col_name(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

/// The workbook. With no tables it holds one empty sheet (a workbook needs at least one).
pub fn xlsx(tables: &[Found]) -> Vec<u8> {
    let empty = [Found { page: 0, n: 0, rows: Vec::new() }];
    let list: &[Found] = if tables.is_empty() { &empty } else { tables };
    let mut sheets = Vec::new();
    for t in list {
        let mut data = String::new();
        for (r, row) in t.rows.iter().enumerate() {
            data.push_str(&format!("<row r=\"{}\">", r + 1));
            for (c, cell) in row.iter().enumerate() {
                let at = format!("{}{}", col_name(c), r + 1);
                let v = cell.trim();
                if is_plain_number(v) {
                    data.push_str(&format!("<c r=\"{at}\"><v>{v}</v></c>"));
                } else {
                    data.push_str(&format!("<c r=\"{at}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", esc(cell)));
                }
            }
            data.push_str("</row>");
        }
        sheets.push(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{data}</sheetData></worksheet>"
        ));
    }
    let mut wb_sheets = String::new();
    let mut rels = String::new();
    let mut ct_sheets = String::new();
    for (i, t) in list.iter().enumerate() {
        let name = if t.page == 0 { "Tables".to_string() } else { format!("p{}-t{}", t.page, t.n) };
        wb_sheets.push_str(&format!("<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rId{}\"/>", i + 1, i + 1));
        rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>",
            i + 1,
            i + 1
        ));
        ct_sheets.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            i + 1
        ));
    }
    let head = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    let ct = format!(
        "{head}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>{ct_sheets}</Types>"
    );
    let root_rels = format!("{head}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let wb = format!("{head}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>{wb_sheets}</sheets></workbook>");
    let wb_rels = format!("{head}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>");
    let names: Vec<String> = (1..=list.len()).map(|i| format!("xl/worksheets/sheet{i}.xml")).collect();
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", ct.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
    ];
    for (n, s) in names.iter().zip(&sheets) {
        parts.push((n.as_str(), s.as_bytes()));
    }
    crate::zipmini::write(&parts)
}

/// One HTML page with every table, a heading per table naming its page.
pub fn html(tables: &[Found], title: &str) -> String {
    let mut o = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{}</title><style>table{{border-collapse:collapse;margin:0 0 1.5em}}td{{border:1px solid #888;padding:.2em .5em}}</style></head><body>\n<h1>{}</h1>\n",
        esc(title),
        esc(title)
    );
    if tables.is_empty() {
        o.push_str("<p>No tables were found.</p>\n");
    }
    for t in tables {
        o.push_str(&format!("<h2>Page {} - table {}</h2>\n<table>\n", t.page, t.n));
        for row in &t.rows {
            o.push_str("<tr>");
            for c in row {
                o.push_str(&format!("<td>{}</td>", esc(c)));
            }
            o.push_str("</tr>\n");
        }
        o.push_str("</table>\n");
    }
    o.push_str("</body></html>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Found> {
        let b = vec![
            vec![Block::Para("x".into())],
            vec![Block::Columns(vec![vec![Block::Table(vec![
                vec!["Nume".into(), "Suma".into()],
                vec!["Școala & Co".into(), "1250.50".into()],
                vec!["007".into(), "-3".into()],
            ])]])],
        ];
        find(&b)
    }

    #[test]
    fn tables_are_found_inside_columns_and_numbered_per_page() {
        let f = sample();
        assert_eq!((f.len(), f[0].page, f[0].n), (1, 2, 1));
    }

    #[test]
    fn numbers_and_text_are_kept_apart() {
        for n in ["0", "12", "-3", "1250.50", "0.5"] {
            assert!(is_plain_number(n), "{n}");
        }
        for t in ["007", "1.234,56", "12.", ".5", "1e5", "", "-", "12/03/2026", "1234567890123456"] {
            assert!(!is_plain_number(t), "{t}");
        }
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
    }

    #[test]
    fn html_escapes_and_empty_workbook_is_valid_zip() {
        let h = html(&sample(), "T <1>");
        assert!(h.contains("Școala &amp; Co") && h.contains("T &lt;1&gt;"));
        let z = xlsx(&[]);
        assert!(crate::zipmini::list(&z).unwrap().iter().any(|e| e.name == "xl/worksheets/sheet1.xml"));
    }
}
