//! A Model Context Protocol server over stdio (roadmap 7.1): `legatorocr --mcp`. Newline-
//! delimited JSON-RPC 2.0 as used by MCP's stdio transport. Dual era: the legacy revision 2024-11-05 (with
//! `initialize`) and the stateless revision 2026-07-28 (per-request `_meta` version, `server/discover`,
//! `resultType`, `ttlMs`/`cacheScope`, error -32022), chosen per request.
//!
//! Each tool runs this same program as a child process with a fixed argument list built from the
//! tool's arguments (no shell, no free-form flags), so the server can do nothing the command line
//! cannot. Paths are the caller's business: an assistant connected to this server can read and
//! write whatever the user account can, which is why only a few narrow tools are offered.

use anyhow::Result;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(v) => v.iter().find(|(n, _)| n == k).map(|(_, x)| x),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    fn strings(&self) -> Option<Vec<String>> {
        match self {
            J::Arr(a) => a.iter().map(|x| x.str().map(str::to_string)).collect(),
            _ => None,
        }
    }
    pub fn render(&self) -> String {
        match self {
            J::Null => "null".into(),
            J::Bool(b) => b.to_string(),
            J::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    n.to_string()
                }
            }
            J::Str(s) => quote(s),
            J::Arr(a) => format!(
                "[{}]",
                a.iter().map(J::render).collect::<Vec<_>>().join(",")
            ),
            J::Obj(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}:{}", quote(k), v.render()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

pub fn parse(s: &str) -> Option<J> {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let v = value(&b, &mut i, 0)?;
    ws(&b, &mut i);
    (i == b.len()).then_some(v)
}

fn ws(b: &[char], i: &mut usize) {
    while *i < b.len() && b[*i].is_whitespace() {
        *i += 1;
    }
}

fn value(b: &[char], i: &mut usize, depth: u32) -> Option<J> {
    if depth > 32 {
        return None;
    }
    ws(b, i);
    match *b.get(*i)? {
        'n' => lit(b, i, "null", J::Null),
        't' => lit(b, i, "true", J::Bool(true)),
        'f' => lit(b, i, "false", J::Bool(false)),
        '"' => string(b, i).map(J::Str),
        '[' => {
            *i += 1;
            let mut v = Vec::new();
            ws(b, i);
            if *b.get(*i)? == ']' {
                *i += 1;
                return Some(J::Arr(v));
            }
            loop {
                v.push(value(b, i, depth + 1)?);
                ws(b, i);
                match *b.get(*i)? {
                    ',' => *i += 1,
                    ']' => {
                        *i += 1;
                        return Some(J::Arr(v));
                    }
                    _ => return None,
                }
            }
        }
        '{' => {
            *i += 1;
            let mut v = Vec::new();
            ws(b, i);
            if *b.get(*i)? == '}' {
                *i += 1;
                return Some(J::Obj(v));
            }
            loop {
                ws(b, i);
                let k = string(b, i)?;
                ws(b, i);
                if *b.get(*i)? != ':' {
                    return None;
                }
                *i += 1;
                v.push((k, value(b, i, depth + 1)?));
                ws(b, i);
                match *b.get(*i)? {
                    ',' => *i += 1,
                    '}' => {
                        *i += 1;
                        return Some(J::Obj(v));
                    }
                    _ => return None,
                }
            }
        }
        _ => {
            let st = *i;
            while *i < b.len() && matches!(b[*i], '0'..='9' | '-' | '+' | '.' | 'e' | 'E') {
                *i += 1;
            }
            b[st..*i]
                .iter()
                .collect::<String>()
                .parse()
                .ok()
                .map(J::Num)
        }
    }
}

fn lit(b: &[char], i: &mut usize, w: &str, v: J) -> Option<J> {
    let n = w.chars().count();
    if b.get(*i..*i + n)?.iter().collect::<String>() == w {
        *i += n;
        Some(v)
    } else {
        None
    }
}

fn string(b: &[char], i: &mut usize) -> Option<String> {
    if *b.get(*i)? != '"' {
        return None;
    }
    *i += 1;
    let mut o = String::new();
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        match c {
            '"' => return Some(o),
            '\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                match e {
                    'n' => o.push('\n'),
                    't' => o.push('\t'),
                    'r' => o.push('\r'),
                    'b' => o.push('\u{8}'),
                    'f' => o.push('\u{c}'),
                    '/' | '\\' | '"' => o.push(e),
                    'u' => {
                        let h: String = b.get(*i..*i + 4)?.iter().collect();
                        *i += 4;
                        let mut cp = u32::from_str_radix(&h, 16).ok()?;
                        if (0xD800..0xDC00).contains(&cp)
                            && b.get(*i) == Some(&'\\')
                            && b.get(*i + 1) == Some(&'u')
                        {
                            let l: String = b.get(*i + 2..*i + 6)?.iter().collect();
                            let lo = u32::from_str_radix(&l, 16).ok()?;
                            *i += 6;
                            cp =
                                0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                        }
                        o.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                    }
                    _ => return None,
                }
            }
            c => o.push(c),
        }
    }
}

fn schema(props: &[(&str, &str, &str)], required: &[&str]) -> J {
    J::Obj(vec![
        ("type".into(), J::Str("object".into())),
        (
            "properties".into(),
            J::Obj(
                props
                    .iter()
                    .map(|(n, t, d)| {
                        let ty = if *t == "strings" {
                            J::Obj(vec![
                                ("type".into(), J::Str("array".into())),
                                (
                                    "items".into(),
                                    J::Obj(vec![("type".into(), J::Str("string".into()))]),
                                ),
                            ])
                        } else {
                            J::Obj(vec![("type".into(), J::Str((*t).into()))])
                        };
                        let mut o = match ty {
                            J::Obj(v) => v,
                            _ => vec![],
                        };
                        o.push(("description".into(), J::Str((*d).into())));
                        (n.to_string(), J::Obj(o))
                    })
                    .collect(),
            ),
        ),
        (
            "required".into(),
            J::Arr(required.iter().map(|r| J::Str((*r).into())).collect()),
        ),
    ])
}

fn tools() -> J {
    let t = |name: &str, desc: &str, s: J| {
        J::Obj(vec![
            ("name".into(), J::Str(name.into())),
            ("description".into(), J::Str(desc.into())),
            ("inputSchema".into(), s),
        ])
    };
    J::Arr(vec![
        t(
            "scan_to_pdf",
            "Clean up scanned page images or scanned PDFs (deskew, crop, binarise) and write one PDF, optionally with an OCR text layer.",
            schema(
                &[
                    ("inputs", "strings", "Input image or PDF paths, in reading order"),
                    ("pdf", "string", "Output PDF path"),
                    ("ocr", "boolean", "Add an OCR text layer (needs Tesseract)"),
                    ("lang", "string", "OCR language codes such as ron+eng"),
                    ("mode", "string", "bw, gray, color, mixed or hybrid"),
                ],
                &["inputs", "pdf"],
            ),
        ),
        t(
            "pdf_toolbox",
            "Merge, extract pages, split, watermark or number pages of existing PDFs without re-rendering them.",
            schema(
                &[
                    ("inputs", "strings", "PDF paths"),
                    ("out_dir", "string", "Output folder"),
                    ("merge", "boolean", "Join all inputs into one file"),
                    ("pages", "string", "Pages to keep, e.g. 1-3,7"),
                    ("split_every", "integer", "Cut into files of this many pages"),
                    ("watermark", "string", "Diagonal watermark text"),
                    ("number_pages", "string", "Page number format, e.g. {n} / {total}"),
                ],
                &["inputs", "out_dir"],
            ),
        ),
        t("efactura", "Read a Romanian e-Factura invoice (UBL XML or ANAF ZIP): key fields and arithmetic / CUI checks.", schema(&[("file", "string", "Path to the .xml or .zip")], &["file"])),
        t(
            "measure_edge",
            "Measure sharpness (MTF50) on a slanted edge in an image.",
            schema(&[("image", "string", "Image path"), ("roi", "string", "x,y,w,h around the edge"), ("dpi", "integer", "Scan resolution")], &["image"]),
        ),
        t("search", "Search the text of PDFs and images in a folder or archive; boolean and fuzzy queries.", schema(&[("query", "string", "Query, e.g. 'tratat AND Utrecht'"), ("inputs", "strings", "Files or folders")], &["query", "inputs"])),
    ])
}

fn safe_path(s: &str) -> Result<String, String> {
    if s.is_empty() || s.starts_with('-') {
        return Err(format!("not an acceptable path: {s:?}"));
    }
    Ok(s.to_string())
}

/// Argument list for a tool call, or an error text for the caller.
pub fn build_args(name: &str, a: &J) -> Result<Vec<String>, String> {
    let s = |k: &str| a.get(k).and_then(J::str).map(str::to_string);
    let paths = |k: &str| -> Result<Vec<String>, String> {
        a.get(k)
            .and_then(J::strings)
            .ok_or_else(|| format!("{k} must be a list of strings"))?
            .iter()
            .map(|p| safe_path(p))
            .collect()
    };
    let flag = |k: &str| matches!(a.get(k), Some(J::Bool(true)));
    let mut v: Vec<String> = Vec::new();
    match name {
        "scan_to_pdf" => {
            v.extend(paths("inputs")?);
            v.extend([
                "--pdf".into(),
                safe_path(&s("pdf").ok_or("pdf is required")?)?,
            ]);
            if flag("ocr") {
                v.push("--ocr".into());
            }
            if let Some(l) = s("lang") {
                if !l
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '_')
                {
                    return Err("lang: letters, digits, + and _ only".into());
                }
                v.extend(["--lang".into(), l]);
            }
            if let Some(m) = s("mode") {
                if !["bw", "gray", "color", "mixed", "hybrid"].contains(&m.as_str()) {
                    return Err("mode: bw, gray, color, mixed or hybrid".into());
                }
                v.extend(["--mode".into(), m]);
            }
        }
        "pdf_toolbox" => {
            v.extend(paths("inputs")?);
            v.extend([
                "-o".into(),
                safe_path(&s("out_dir").ok_or("out_dir is required")?)?,
            ]);
            let mut any = false;
            if flag("merge") {
                v.push("--merge-pdf".into());
                any = true;
            }
            for (k, f) in [
                ("pages", "--extract-pages"),
                ("watermark", "--watermark"),
                ("number_pages", "--number-pages"),
            ] {
                if let Some(x) = s(k) {
                    v.push(format!("{f}={x}"));
                    any = true;
                }
            }
            if let Some(J::Num(n)) = a.get("split_every") {
                v.push(format!("--split-pdf-every={}", *n as u64));
                any = true;
            }
            if !any {
                return Err(
                    "choose at least one of merge, pages, split_every, watermark, number_pages"
                        .into(),
                );
            }
        }
        "efactura" => v.extend([format!(
            "--efactura={}",
            safe_path(&s("file").ok_or("file is required")?)?
        )]),
        "measure_edge" => {
            v.push(format!(
                "--measure-edge={}",
                safe_path(&s("image").ok_or("image is required")?)?
            ));
            if let Some(r) = s("roi") {
                v.push(format!("--edge-roi={r}"));
            }
            if let Some(J::Num(d)) = a.get("dpi") {
                v.push(format!("--dpi={}", *d as u32));
            }
        }
        "search" => {
            v.push(format!(
                "--search={}",
                s("query").ok_or("query is required")?
            ));
            v.push("--search-query".into());
            v.extend(paths("inputs")?);
        }
        other => return Err(format!("unknown tool {other}")),
    }
    Ok(v)
}

fn reply(id: &J, result: J) -> String {
    J::Obj(vec![
        ("jsonrpc".into(), J::Str("2.0".into())),
        ("id".into(), id.clone()),
        ("result".into(), result),
    ])
    .render()
}

fn error(id: &J, code: i64, msg: &str) -> String {
    J::Obj(vec![
        ("jsonrpc".into(), J::Str("2.0".into())),
        ("id".into(), id.clone()),
        (
            "error".into(),
            J::Obj(vec![
                ("code".into(), J::Num(code as f64)),
                ("message".into(), J::Str(msg.into())),
            ]),
        ),
    ])
    .render()
}

const MODERN_VERSION: &str = "2026-07-28";
const META_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

fn server_info() -> J {
    J::Obj(vec![
        ("name".into(), J::Str("legatorocr".into())),
        ("version".into(), J::Str(env!("CARGO_PKG_VERSION").into())),
    ])
}

fn error_with_data(id: &J, code: i64, msg: &str, data: J) -> String {
    J::Obj(vec![
        ("jsonrpc".into(), J::Str("2.0".into())),
        ("id".into(), id.clone()),
        (
            "error".into(),
            J::Obj(vec![
                ("code".into(), J::Num(code as f64)),
                ("message".into(), J::Str(msg.into())),
                ("data".into(), data),
            ]),
        ),
    ])
    .render()
}

/// Add the 2026-07-28 result fields (`resultType`, optional cache hints) to a result object.
fn modernise(mut result: J, cache: bool) -> J {
    if let J::Obj(v) = &mut result {
        v.push(("resultType".into(), J::Str("complete".into())));
        if cache {
            v.push(("ttlMs".into(), J::Num(0.0)));
            v.push(("cacheScope".into(), J::Str("public".into())));
        }
        v.push((
            "_meta".into(),
            J::Obj(vec![(
                "io.modelcontextprotocol/serverInfo".into(),
                server_info(),
            )]),
        ));
    }
    result
}

/// Handle one request line; `None` for notifications.
pub fn handle(line: &str, exe: &Path) -> Option<String> {
    let Some(req) = parse(line) else {
        return Some(error(&J::Null, -32700, "parse error"));
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(J::str).unwrap_or("");
    let id = id?;
    let requested = req
        .get("params")
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get(META_VERSION_KEY))
        .and_then(J::str);
    let modern = requested.is_some();
    if let Some(v) = requested {
        if v != MODERN_VERSION {
            let data = J::Obj(vec![
                (
                    "supported".into(),
                    J::Arr(vec![J::Str(MODERN_VERSION.into())]),
                ),
                ("requested".into(), J::Str(v.into())),
            ]);
            return Some(error_with_data(
                &id,
                -32022,
                "Unsupported protocol version",
                data,
            ));
        }
    }
    let wrap = |r: J, cache: bool| if modern { modernise(r, cache) } else { r };
    Some(match method {
        "server/discover" => reply(
            &id,
            modernise(
                J::Obj(vec![
                    (
                        "supportedVersions".into(),
                        J::Arr(vec![J::Str(MODERN_VERSION.into())]),
                    ),
                    (
                        "capabilities".into(),
                        J::Obj(vec![("tools".into(), J::Obj(vec![]))]),
                    ),
                    ("serverInfo".into(), server_info()),
                ]),
                true,
            ),
        ),
        "initialize" if modern => error(
            &id,
            -32601,
            "method not found: initialize is removed in 2026-07-28",
        ),
        "ping" if modern => error(
            &id,
            -32601,
            "method not found: ping is removed in 2026-07-28",
        ),
        "initialize" => reply(
            &id,
            J::Obj(vec![
                ("protocolVersion".into(), J::Str("2024-11-05".into())),
                (
                    "capabilities".into(),
                    J::Obj(vec![("tools".into(), J::Obj(vec![]))]),
                ),
                (
                    "serverInfo".into(),
                    J::Obj(vec![
                        ("name".into(), J::Str("legatorocr".into())),
                        ("version".into(), J::Str(env!("CARGO_PKG_VERSION").into())),
                    ]),
                ),
            ]),
        ),
        "ping" => reply(&id, J::Obj(vec![])),
        "tools/list" => reply(&id, wrap(J::Obj(vec![("tools".into(), tools())]), true)),
        "tools/call" => {
            let p = req.get("params").cloned().unwrap_or(J::Null);
            let name = p.get("name").and_then(J::str).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or(J::Obj(vec![]));
            let (text, is_err) = match build_args(name, &args) {
                Err(e) => (e, true),
                Ok(a) => match Command::new(exe)
                    .args(&a)
                    .stdin(std::process::Stdio::null())
                    .output()
                {
                    Ok(o) => {
                        let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
                        let e = String::from_utf8_lossy(&o.stderr);
                        if !e.trim().is_empty() {
                            t.push_str(&e);
                        }
                        (t, !o.status.success())
                    }
                    Err(e) => (format!("cannot start the program: {e}"), true),
                },
            };
            reply(
                &id,
                wrap(
                    J::Obj(vec![
                        (
                            "content".into(),
                            J::Arr(vec![J::Obj(vec![
                                ("type".into(), J::Str("text".into())),
                                ("text".into(), J::Str(text)),
                            ])]),
                        ),
                        ("isError".into(), J::Bool(is_err)),
                    ]),
                    false,
                ),
            )
        }
        _ => error(&id, -32601, "method not found"),
    })
}

pub fn serve(exe: &Path, input: impl BufRead, mut out: impl Write) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(r) = handle(&line, exe) {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_and_escapes() {
        let j = parse(r#"{"a":[1,2.5,true,null,"x\né😀"],"b":{"c":"q\"q"}}"#).unwrap();
        assert_eq!(j.get("b").unwrap().get("c").unwrap().str(), Some("q\"q"));
        assert_eq!(parse(&j.render()).unwrap(), j);
        assert!(parse("{\"a\":").is_none() && parse("[1,]").is_none() && parse("{} x").is_none());
        let deep = "[".repeat(100);
        assert!(parse(&deep).is_none());
    }

    #[test]
    fn protocol_flow() {
        let exe = Path::new("/nonexistent/legatorocr");
        let r = handle(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            exe,
        )
        .unwrap();
        assert!(r.contains("\"protocolVersion\":\"2024-11-05\"") && r.contains("\"id\":1"));
        assert!(handle(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            exe
        )
        .is_none());
        let l = handle(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#, exe).unwrap();
        for t in [
            "scan_to_pdf",
            "pdf_toolbox",
            "efactura",
            "measure_edge",
            "search",
        ] {
            assert!(l.contains(t), "{t}");
        }
        assert!(handle(r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#, exe)
            .unwrap()
            .contains("-32601"));
        assert!(handle("garbage", exe).unwrap().contains("-32700"));
        // an invalid argument is reported as a tool error, not run
        let e = handle(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"efactura","arguments":{"file":"--watch"}}}"#, exe).unwrap();
        assert!(e.contains("\"isError\":true") && e.contains("not an acceptable path"));
    }

    #[test]
    fn modern_era_2026_07_28() {
        let exe = Path::new("/nonexistent/legatorocr");
        let m = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
        let d = handle(
            &format!(r#"{{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{{{m}}}}}"#),
            exe,
        )
        .unwrap();
        for k in [
            "supportedVersions",
            "2026-07-28",
            "capabilities",
            "resultType",
            "complete",
        ] {
            assert!(d.contains(k), "{k}: {d}");
        }
        // legacy clients can probe too
        assert!(handle(
            r#"{"jsonrpc":"2.0","id":9,"method":"server/discover"}"#,
            exe
        )
        .unwrap()
        .contains("supportedVersions"));
        let l = handle(
            &format!(r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{{m}}}}}"#),
            exe,
        )
        .unwrap();
        assert!(
            l.contains("\"resultType\":\"complete\"")
                && l.contains("ttlMs")
                && l.contains("cacheScope"),
            "{l}"
        );
        // the legacy answer stays untouched
        let old = handle(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#, exe).unwrap();
        assert!(!old.contains("resultType") && !old.contains("ttlMs"));
        // removed methods and unknown versions
        assert!(handle(
            &format!(r#"{{"jsonrpc":"2.0","id":4,"method":"ping","params":{{{m}}}}}"#),
            exe
        )
        .unwrap()
        .contains("-32601"));
        let bad = handle(r#"{"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2031-01-01"}}}"#, exe).unwrap();
        assert!(
            bad.contains("-32022")
                && bad.contains("\"supported\":[\"2026-07-28\"]")
                && bad.contains("\"requested\":\"2031-01-01\""),
            "{bad}"
        );
    }

    #[test]
    fn arguments_are_built_without_free_flags() {
        let a = parse(
            r#"{"inputs":["a.png","b.png"],"pdf":"o.pdf","ocr":true,"lang":"ron+eng","mode":"bw"}"#,
        )
        .unwrap();
        assert_eq!(
            build_args("scan_to_pdf", &a).unwrap(),
            ["a.png", "b.png", "--pdf", "o.pdf", "--ocr", "--lang", "ron+eng", "--mode", "bw"]
        );
        let bad = parse(r#"{"inputs":["-o"],"pdf":"o.pdf"}"#).unwrap();
        assert!(build_args("scan_to_pdf", &bad).is_err());
        let bad = parse(r#"{"inputs":["a"],"pdf":"o.pdf","lang":"ron --x"}"#).unwrap();
        assert!(build_args("scan_to_pdf", &bad).is_err());
        assert!(build_args("rm_rf", &J::Null).is_err());
        let t = parse(r#"{"inputs":["x.pdf"],"out_dir":"d"}"#).unwrap();
        assert!(build_args("pdf_toolbox", &t).is_err());
    }
}
