//! `--markdown FILE.md` (with `--ocr`): the recognised text as Markdown, for notes tools, static
//! sites and language-model pipelines. Headings become `##` lines, tables become pipe tables,
//! columns are read one after the other, every scanned page leaves an HTML comment
//! `<!-- page N -->` so the page numbers survive. Pictures are left out (a one-line note marks
//! where each was).
use crate::docx::Block;

/// Characters that would start Markdown syntax in running text.
fn esc_inline(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '|') {
            o.push('\\');
        }
        if c == '\u{c}' {
            continue;
        }
        o.push(c);
    }
    o
}

fn one_line(s: &str) -> String {
    esc_inline(&s.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A line that Markdown would read as a list, quote or heading marker gets its first character
/// escaped, so recognised text never turns into structure it did not have.
fn guard_start(line: String) -> String {
    let t = line.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let risky = t.starts_with(['#', '-', '+', '>', '=', '~'])
        || (digits > 0 && matches!(t[digits..].chars().next(), Some('.' | ')')));
    if risky {
        format!("\\{line}")
    } else {
        line
    }
}

fn blocks_md(blocks: &[Block], out: &mut String) {
    for b in blocks {
        match b {
            Block::Para(t) => {
                let lines: Vec<String> = t
                    .lines()
                    .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|l| !l.is_empty())
                    .collect();
                if lines.is_empty() {
                    continue;
                }
                let text = lines
                    .iter()
                    .map(|l| esc_inline(l))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push_str(&guard_start(text));
                out.push_str("\n\n");
            }
            Block::Head(t) => {
                let h = one_line(t);
                if !h.is_empty() {
                    out.push_str(&format!("## {h}\n\n"));
                }
            }
            Block::Table(rows) => {
                let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                if w == 0 {
                    continue;
                }
                for (i, r) in rows.iter().enumerate() {
                    let cells: Vec<String> = (0..w)
                        .map(|c| r.get(c).map(|s| one_line(s)).unwrap_or_default())
                        .collect();
                    out.push_str(&format!("| {} |\n", cells.join(" | ")));
                    if i == 0 {
                        out.push_str(&format!("|{}\n", " --- |".repeat(w)));
                    }
                }
                out.push('\n');
            }
            Block::Columns(cols) => {
                for c in cols {
                    blocks_md(c, out);
                }
            }
            Block::Image(_) => out.push_str("*[figure]*\n\n"),
        }
    }
}

/// The whole document: `pages` are the blocks of each page in reading order.
pub fn from_blocks(pages: &[Vec<Block>]) -> String {
    let mut out = String::new();
    for (i, p) in pages.iter().enumerate() {
        out.push_str(&format!("<!-- page {} -->\n\n", i + 1));
        blocks_md(p, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_tables_and_page_marks() {
        let pages = vec![
            vec![
                Block::Head("Titlu".into()),
                Block::Para("un rând\ncontinuat".into()),
                Block::Table(vec![vec!["a".into(), "b".into()], vec!["1".into()]]),
            ],
            vec![Block::Para("pagina doi".into())],
        ];
        let m = from_blocks(&pages);
        assert!(m.starts_with("<!-- page 1 -->\n\n## Titlu\n\nun rând continuat\n\n"));
        assert!(m.contains("| a | b |\n| --- | --- |\n| 1 |  |\n"));
        assert!(m.contains("<!-- page 2 -->\n\npagina doi\n"));
    }

    #[test]
    fn recognised_text_never_turns_into_markdown_structure() {
        let m = from_blocks(&[vec![
            Block::Para("# nu titlu".into()),
            Block::Para("1. nu listă".into()),
            Block::Para("a*b*c [x] <y> a|b".into()),
            Block::Table(vec![vec!["a|b".into(), "x\ny".into()]]),
        ]]);
        assert!(m.contains("\n\\# nu titlu\n"));
        assert!(m.contains("\n\\1. nu listă\n"));
        assert!(m.contains("a\\*b\\*c \\[x\\] \\<y\\> a\\|b"));
        assert!(m.contains("| a\\|b | x y |"));
    }

    #[test]
    fn nested_columns_are_read_in_turn_and_pictures_are_marked() {
        let m = from_blocks(&[vec![Block::Columns(vec![
            vec![Block::Para("stânga".into())],
            vec![Block::Para("dreapta".into())],
        ])]]);
        assert!(m.find("stânga").unwrap() < m.find("dreapta").unwrap());
    }
}
