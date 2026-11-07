//! "All options" window: every command-line option of the build, searchable and grouped, with the
//! equivalent command and a Run button. The list is read from the `legatorocr` program shipped
//! next to the window (`legatorocr --dump-options`), so it always matches this build.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Flag,
    Value,
    /// the flag is repeated for each value
    Repeat,
    /// several values after one flag
    Multi,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Cat {
    Pages,
    Clean,
    Output,
    Ocr,
    Archive,
    Search,
    System,
    Other,
}

impl Cat {
    pub const ALL: [Cat; 8] = [
        Cat::Pages,
        Cat::Clean,
        Cat::Output,
        Cat::Ocr,
        Cat::Archive,
        Cat::Search,
        Cat::System,
        Cat::Other,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Cat::Pages => "all.cat.pages",
            Cat::Clean => "all.cat.clean",
            Cat::Output => "all.cat.output",
            Cat::Ocr => "all.cat.ocr",
            Cat::Archive => "all.cat.archive",
            Cat::Search => "all.cat.search",
            Cat::System => "all.cat.system",
            Cat::Other => "all.cat.other",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathKind {
    Dir,
    Open,
    Save,
}

#[derive(Clone, Debug)]
pub struct OptDef {
    pub long: String,
    pub kind: Kind,
    pub vname: String,
    pub values: Vec<String>,
    pub default: String,
    pub help: String,
    pub cat: Cat,
}

impl OptDef {
    /// File or folder picker that fits this option, if it takes a path.
    pub fn path_kind(&self) -> Option<PathKind> {
        if self.kind == Kind::Flag || !self.values.is_empty() {
            return None;
        }
        let v = self.vname.to_ascii_uppercase();
        if v.contains("DIR") || v.contains("FOLDER") {
            return Some(PathKind::Dir);
        }
        let h = self.help.to_ascii_lowercase();
        let pathy = [
            "FILE", "PATH", "PDF", "TXT", "HOCR", "JSON", "DOCX", "EPUB", "TEI", "ALTO", "CA",
            "XML", "REPORT", "TIFF", "HTML", "DJVU", "KEY", "P12", "XLSX", "CSV", "LIST",
        ]
        .iter()
        .any(|p| v.contains(p));
        if !pathy {
            return None;
        }
        if h.starts_with("write") || h.starts_with("also write") || h.starts_with("save") {
            Some(PathKind::Save)
        } else {
            Some(PathKind::Open)
        }
    }

    /// First sentence of the help, for the one-line description.
    pub fn short_help(&self) -> String {
        let h = self.help.trim();
        let end = h
            .char_indices()
            .find(|(i, c)| (*c == '.' && h[*i + 1..].starts_with(' ')) || *c == ';' || *c == ':')
            .map(|(i, _)| i)
            .unwrap_or(h.len());
        let s = &h[..end];
        if s.chars().count() > 130 {
            let cut: String = s.chars().take(127).collect();
            format!("{cut}…")
        } else {
            s.to_string()
        }
    }
}

/// Which group an option belongs to (by name; anything unknown goes to "Other").
pub fn category(long: &str) -> Cat {
    let starts = |ps: &[&str]| ps.iter().any(|p| long == *p || long.starts_with(p));
    if long == "package-manager" {
        return Cat::System;
    }
    if starts(&[
        "pages",
        "duplex",
        "reverse",
        "dry-run",
        "split",
        "skip-blank",
        "skip-duplicates",
        "burst",
        "fuse",
        "uniform-size",
        "align",
        "rotate",
        "auto-rotate",
        "no-auto-orient",
        "shard",
        "merge-pdf",
        "extract-pages",
        "number-pages",
        "front-matter",
        "make-separator",
        "page-range",
    ]) {
        Cat::Pages
    } else if starts(&[
        "no-deskew",
        "skew",
        "max-skew",
        "margin",
        "page-size",
        "fit-",
        "sauvola",
        "threshold",
        "invert",
        "auto-levels",
        "sharpen",
        "punch",
        "fingers",
        "remove-lines",
        "erase",
        "work-area",
        "cut-at",
        "picture-zone",
        "no-picture-zone",
        "no-auto-pictures",
        "dewarp",
        "dropout",
        "show-through",
        "perspective",
        "trim-background",
        "descreen",
        "despeckle",
        "no-despeckle",
        "normalize",
        "mode",
        "dpi",
        "output-dpi",
        "hybrid-dpi",
        "phone-paper",
        "picture-sens",
        "measure-edge",
        "edge-roi",
    ]) {
        Cat::Clean
    } else if starts(&[
        "sign",
        "timestamp",
        "tsa",
        "redact",
        "certify",
        "provenance",
        "merkle",
        "checksums",
        "audit-log",
        "bagit",
        "ocfl",
        "verify-ocfl",
        "verify-bag",
        "verify-sums",
        "rights",
        "retain-until",
        "legal-hold",
        "archive-package",
        "embed-project",
        "extract-project",
        "pdfa",
        "verify-pdf",
        "mrz",
        "integrity",
        "verify-document",
        "efactura",
        "package",
    ]) {
        Cat::Archive
    } else if starts(&["search", "compare-text", "diff-html"]) {
        Cat::Search
    } else if starts(&[
        "ocr",
        "psm",
        "min-conf",
        "engine",
        "tesseract",
        "tessdata",
        "lang",
        "accurate",
        "clean-text",
        "cedilla",
        "drop-running",
        "find-replace",
        "doubtful",
        "verify-",
        "page-lang",
        "page-labels",
        "explain",
        "auto-retry",
        "proof-sheet",
        "skip-text",
        "plugin",
        "force-ocr",
        "redo-ocr",
        "binarize-by-ocr",
        "txt",
        "hocr",
        "ocr-",
    ]) {
        Cat::Ocr
    } else if starts(&[
        "pdf",
        "format",
        "jpeg-quality",
        "multitiff",
        "docx",
        "epub",
        "tei",
        "alto",
        "page-xml",
        "json",
        "djvu",
        "vectorize",
        "tables-",
        "bookmarks",
        "tagged",
        "form-fields",
        "report",
        "catalog",
        "dublin-core",
        "passages",
        "fff-",
        "author",
        "subject",
        "keywords",
        "watermark",
        "target-size",
        "out-dir",
        "fff",
        "output",
        "split-pdf",
    ]) {
        Cat::Output
    } else if starts(&[
        "watch",
        "keep-going",
        "max-memory",
        "isolate",
        "decoder",
        "backup",
        "mcp",
        "doctor",
        "install-modules",
        "yes",
        "package-manager",
        "scan-active",
        "list-profiles",
        "save-",
        "profile",
        "timings",
        "quality-report",
        "offline",
        "shard",
        "project",
        "embed",
    ]) {
        Cat::System
    } else {
        Cat::Other
    }
}

/// Parse the output of `legatorocr --dump-options`.
pub fn parse_dump(text: &str) -> Vec<OptDef> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 || f[0].is_empty() {
            continue;
        }
        let kind = match f[2] {
            "flag" => Kind::Flag,
            "repeat" => Kind::Repeat,
            "multi" => Kind::Multi,
            _ => Kind::Value,
        };
        out.push(OptDef {
            long: f[0].to_string(),
            kind,
            vname: f[3].to_string(),
            values: f[4]
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            default: f[5].to_string(),
            help: f[6..].join(" "),
            cat: category(f[0]),
        });
    }
    out
}

/// The `legatorocr` command-line program next to this window (or on the PATH).
pub fn cli_path() -> PathBuf {
    let name = if cfg!(windows) {
        "legatorocr.exe"
    } else {
        "legatorocr"
    };
    if let Some(p) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
    {
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(name)
}

/// Open a web address or a file with the system's default program.
pub fn open_target(target: &str) {
    #[cfg(windows)]
    let r = Command::new("cmd")
        .args(["/C", "start", "", target])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = Command::new("xdg-open").arg(target).spawn();
    let _ = r;
}

/// The guide that travels with this edition (next to the program), if it is there.
pub fn local_guide() -> Option<PathBuf> {
    let d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    ["Ghid-LegatorOCR.pdf", "../Resources/Ghid-LegatorOCR.pdf"]
        .iter()
        .map(|n| d.join(n))
        .find(|p| p.is_file())
}

/// Always the newest guide (the file published with the site).
pub const GUIDE_URL: &str = "https://legatorocr.chiuta.ro/Ghid-LegatorOCR.pdf";

pub fn load_defs() -> Result<Vec<OptDef>, String> {
    let o = Command::new(cli_path())
        .arg("--dump-options")
        .output()
        .map_err(|e| format!("{}: {e}", cli_path().display()))?;
    // `--project` is what the window itself uses to pass its pages and settings
    let defs: Vec<OptDef> = parse_dump(&String::from_utf8_lossy(&o.stdout))
        .into_iter()
        .filter(|d| d.long != "project")
        .collect();
    if defs.is_empty() {
        Err("no options listed".into())
    } else {
        Ok(defs)
    }
}

#[derive(Clone, Default)]
pub struct OptVal {
    pub on: bool,
    pub text: String,
}

/// Split a line of extra arguments, honouring "double" and 'single' quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Show an argument the way a shell needs it.
pub fn quote(a: &str) -> String {
    if !a.is_empty()
        && a.chars()
            .all(|c| c.is_alphanumeric() || "-_./:=,+@%#~".contains(c))
    {
        a.to_string()
    } else {
        format!("\"{}\"", a.replace('"', "\\\""))
    }
}

pub struct Run {
    pub rx: Receiver<String>,
    child: Arc<Mutex<Option<Child>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    pub done: bool,
}

impl Run {
    pub fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(c) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = c.kill();
        }
    }
}

impl Drop for Run {
    /// Closing the window (or the app) must not leave the CLI running unseen.
    fn drop(&mut self) {
        if !self.done {
            self.stop();
        }
    }
}

pub fn spawn(args: &[String], cwd: Option<PathBuf>) -> Result<Run, String> {
    let mut cmd = Command::new(cli_path());
    cmd.args(args);
    if let Some(d) = cwd.filter(|d| d.is_dir()) {
        cmd.current_dir(d);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", cli_path().display()))?;
    let (tx, rx) = channel::<String>();
    let out = child.stdout.take();
    let err = child.stderr.take();
    let shared = Arc::new(Mutex::new(Some(child)));
    let mut readers = Vec::new();
    for pipe in [
        out.map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        err.map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for l in BufReader::new(pipe).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        }));
    }
    let waiter = shared.clone();
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let was_stopped = stopped.clone();
    std::thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
        // Poll instead of blocking in `wait` with the lock held: Stop must never wait for the exit.
        let status = loop {
            let mut g = waiter.lock().unwrap_or_else(|e| e.into_inner());
            match g.as_mut().map(|c| c.try_wait()) {
                Some(Ok(Some(s))) => break Some(s),
                Some(Ok(None)) => {}
                _ => break None,
            }
            drop(g);
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let _ = tx.send(match status {
            _ if was_stopped.load(std::sync::atomic::Ordering::SeqCst) => {
                "\u{1}stopped".to_string()
            }
            Some(s) if s.success() => "\u{1}ok".to_string(),
            Some(s) => format!("\u{1}exit {}", s.code().unwrap_or(-1)),
            None => "\u{1}stopped".to_string(),
        });
    });
    Ok(Run {
        rx,
        child: shared,
        stopped,
        done: false,
    })
}

#[derive(Default)]
pub struct AllOptions {
    pub show: bool,
    pub defs: Option<Result<Vec<OptDef>, String>>,
    pub vals: HashMap<String, OptVal>,
    pub filter: String,
    pub only_chosen: bool,
    pub use_pages: bool,
    pub extra: String,
    pub log: Vec<String>,
    pub run: Option<Run>,
    pub last: Option<String>,
}

impl AllOptions {
    pub fn open(&mut self) {
        self.show = true;
        if self.defs.is_none() {
            self.defs = Some(load_defs());
            self.use_pages = true;
        }
    }

    pub fn chosen(&self) -> usize {
        self.vals
            .values()
            .filter(|v| v.on || !v.text.trim().is_empty())
            .count()
    }

    pub fn reset(&mut self) {
        self.vals.clear();
        self.extra.clear();
    }

    /// Arguments for the options chosen so far (not including the page list or --project).
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(Ok(defs)) = &self.defs else {
            return out;
        };
        for d in defs {
            let Some(v) = self.vals.get(&d.long) else {
                continue;
            };
            let flag = format!("--{}", d.long);
            match d.kind {
                Kind::Flag => {
                    if v.on {
                        out.push(flag);
                    }
                }
                Kind::Value => {
                    if !v.text.trim().is_empty() {
                        out.push(flag);
                        out.push(v.text.trim().to_string());
                    }
                }
                Kind::Repeat => {
                    for l in v.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                        out.push(flag.clone());
                        out.push(l.to_string());
                    }
                }
                Kind::Multi => {
                    let vals: Vec<String> = split_args(&v.text);
                    if !vals.is_empty() {
                        out.push(flag);
                        out.extend(vals);
                    }
                }
            }
        }
        out.extend(split_args(&self.extra));
        out
    }

    pub fn command_line(&self, project: Option<&str>) -> String {
        let mut a = vec!["legatorocr".to_string()];
        if let Some(p) = project {
            a.push("--project".into());
            a.push(p.to_string());
        }
        a.extend(self.args());
        a.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    }

    /// Does this option match the search text (name or help)?
    pub fn matches(d: &OptDef, filter: &str) -> bool {
        let f = filter.trim().to_lowercase();
        f.is_empty()
            || d.long.to_lowercase().contains(f.trim_start_matches('-'))
            || d.help.to_lowercase().contains(&f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "pages\t\tvalue\tRANGE\t\t\tKeep only these input pages: `1-10,15`\n\
format\t\tvalue\tFORMAT\tpng,tiff,jpeg,none\tpng\tImage format\n\
dry-run\t\tflag\t\t\t\tShow what would be done\n\
erase\t\trepeat\tX,Y,W,H\t\t\tPaint a rectangle\n\
pdf\t\tvalue\tPDF\t\t\tWrite all processed pages into one PDF. More text\n\
sign-pdf\t\tmulti\tIN OUT\t\t\tSign it\n\
bogus line\n";

    fn opts() -> AllOptions {
        AllOptions {
            defs: Some(Ok(parse_dump(SAMPLE))),
            ..Default::default()
        }
    }

    #[test]
    fn parses_kinds_values_and_categories() {
        let d = parse_dump(SAMPLE);
        assert_eq!(d.len(), 6);
        assert_eq!(d[0].cat, Cat::Pages);
        assert_eq!(d[1].values, ["png", "tiff", "jpeg", "none"]);
        assert_eq!(d[1].default, "png");
        assert_eq!(d[2].kind, Kind::Flag);
        assert_eq!(d[3].kind, Kind::Repeat);
        assert_eq!(d[3].cat, Cat::Clean);
        assert_eq!(d[4].cat, Cat::Output);
        assert_eq!(d[4].path_kind(), Some(PathKind::Save));
        assert_eq!(d[0].path_kind(), None);
        assert_eq!(d[5].cat, Cat::Archive);
        assert_eq!(d[4].short_help(), "Write all processed pages into one PDF");
    }

    #[test]
    fn builds_arguments_for_every_kind() {
        let mut o = opts();
        let set = |o: &mut AllOptions, k: &str, on: bool, t: &str| {
            o.vals.insert(k.into(), OptVal { on, text: t.into() });
        };
        set(&mut o, "pages", false, " 1-3,7 ");
        set(&mut o, "dry-run", true, "");
        set(&mut o, "erase", false, "1,2,3,4\n5,6,7,8\n");
        set(&mut o, "sign-pdf", false, "in.pdf \"out file.pdf\"");
        set(&mut o, "format", false, "");
        o.extra = "--yes 'a b'".into();
        assert_eq!(
            o.args(),
            [
                "--pages",
                "1-3,7",
                "--dry-run",
                "--erase",
                "1,2,3,4",
                "--erase",
                "5,6,7,8",
                "--sign-pdf",
                "in.pdf",
                "out file.pdf",
                "--yes",
                "a b"
            ]
        );
        assert_eq!(o.chosen(), 4);
        let c = o.command_line(Some("/tmp/p x.lgr"));
        assert!(c.starts_with("legatorocr --project \"/tmp/p x.lgr\" --pages 1-3,7"));
        assert!(c.contains("\"out file.pdf\""));
        o.reset();
        assert!(o.args().is_empty());
    }

    #[test]
    fn search_matches_name_and_help() {
        let d = parse_dump(SAMPLE);
        assert!(AllOptions::matches(&d[0], "--pages"));
        assert!(AllOptions::matches(&d[0], "keep only"));
        assert!(!AllOptions::matches(&d[0], "xyz"));
        assert!(AllOptions::matches(&d[0], ""));
    }

    #[test]
    fn split_args_handles_quotes() {
        assert_eq!(split_args("a \"b c\" '' d"), ["a", "b c", "", "d"]);
        assert!(split_args("   ").is_empty());
    }
}
