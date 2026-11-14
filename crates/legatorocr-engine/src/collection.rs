//! Collections: newspapers, magazines, books … gathered in one `.lgrcol` file (JSON), with a
//! status per item, batch processing with each item's profile, and full-text search across the
//! OCR text of everything that was processed.
//!
//! * The file stores paths relative to its own folder when the target lies under a folder
//!   they share, so the whole folder can be moved or copied ([`to_stored`] / [`from_stored`]).
//! * Loading never panics: unknown fields are ignored, missing ones get defaults, a file that
//!   is not valid JSON (or has the wrong shape) is reported as an error that names the file.
//! * Saving writes a temporary sibling and renames it ([`crate::write_atomic`]).
//! * The search index (`NAME.lgrindex`, next to the collection) keeps the recognised text of
//!   each page; the inverted word list is built from it in memory when it is loaded.
//!   Words are compared ignoring case and diacritics, and in modern Romanian spelling, so
//!   `Scînteia` finds `Scânteia` and `sînt` finds `sunt`.
//! * The batch loop ([`run_batch`]) is independent of how an item is processed (a closure);
//!   [`run_item_with_engine`] is the real one and calls [`crate::run_export`], the same code
//!   the command line and the window use.

use crate::mcp::{self, J};
use anyhow::{anyhow, bail, Context, Result};
use legatorocr_core::text::{fold, old_ro_to_modern};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Extension of a collection file.
pub const EXT: &str = "lgrcol";
const FORMAT: &str = "lgrcol";
const VERSION: u32 = 1;
/// A collection or index file larger than this is refused (a damaged or foreign file).
const MAX_FILE: u64 = 400 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    Newspaper,
    Magazine,
    Book,
    #[default]
    Other,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Newspaper, Kind::Magazine, Kind::Book, Kind::Other];
    pub fn code(self) -> &'static str {
        match self {
            Kind::Newspaper => "newspaper",
            Kind::Magazine => "magazine",
            Kind::Book => "book",
            Kind::Other => "other",
        }
    }
    pub fn from_code(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.code() == s.trim())
    }
    /// One capital letter for narrow tables: N, M, B, O.
    pub fn letter(self) -> char {
        match self {
            Kind::Newspaper => 'N',
            Kind::Magazine => 'M',
            Kind::Book => 'B',
            Kind::Other => 'O',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub enum Status {
    #[default]
    Todo,
    Processed,
    OcrDone,
    Checked,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Todo,
        Status::Processed,
        Status::OcrDone,
        Status::Checked,
    ];
    pub fn code(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::Processed => "processed",
            Status::OcrDone => "ocr_done",
            Status::Checked => "checked",
        }
    }
    pub fn from_code(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|k| k.code() == s.trim())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    /// Stable identifier inside the collection (`i1`, `i2` …).
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// Issue, number or volume as written (`8801`, `vol. 3`).
    pub issue: String,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (free text is kept as typed).
    pub date: String,
    /// Language codes (`ro`, `en` …); they choose the OCR language.
    pub languages: Vec<String>,
    /// Images, folders of images, PDFs or `.lgr` projects, as stored (relative when possible).
    pub sources: Vec<String>,
    /// The PDF made from the sources, as stored; empty until the item was processed.
    pub output: String,
    pub status: Status,
    pub pages: Option<u32>,
    pub tags: Vec<String>,
    pub notes: String,
    /// Name of the profile (built-in or user) to process with; empty = the collection's.
    pub profile: String,
    /// Why the last run failed (or a warning from a run that succeeded).
    pub error: String,
    /// Seconds since the epoch of the last successful run.
    pub processed_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collection {
    pub name: String,
    pub description: String,
    /// Profile used by items that name none (empty = the engine's defaults).
    pub default_profile: String,
    /// Recognise text while processing (needed for the search). Default on.
    pub ocr: bool,
    /// Folder for the PDFs, as stored; empty = next to the collection file.
    pub output_dir: String,
    pub items: Vec<Item>,
    next_id: u64,
}

impl Default for Collection {
    fn default() -> Self {
        Collection {
            name: String::new(),
            description: String::new(),
            default_profile: String::new(),
            ocr: true,
            output_dir: String::new(),
            items: Vec::new(),
            next_id: 1,
        }
    }
}

// ---------------------------------------------------------------- paths

fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn absolute(p: &Path) -> PathBuf {
    let p = if p.as_os_str().is_empty() {
        Path::new(".")
    } else {
        p
    };
    let a = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    lexical(&a)
}

/// The path as it is stored in a collection file whose folder is `base`: relative (with `/`)
/// when both share at least one folder below the root (`scans/a.png`, `../other/x.pdf`),
/// otherwise absolute.
pub fn to_stored(base: &Path, p: &Path) -> String {
    let (b, t) = (absolute(base), absolute(p));
    let (bc, tc): (Vec<_>, Vec<_>) = (b.components().collect(), t.components().collect());
    let root = bc
        .iter()
        .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        .count();
    let common = bc.iter().zip(&tc).take_while(|(x, y)| x == y).count();
    if common <= root || !matches!(bc.get(common.wrapping_sub(1)), Some(Component::Normal(_))) {
        return t.to_string_lossy().into_owned();
    }
    let mut parts: Vec<String> = vec!["..".to_string(); bc.len() - common];
    parts.extend(
        tc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// Inverse of [`to_stored`].
pub fn from_stored(base: &Path, s: &str) -> PathBuf {
    let p = Path::new(s);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(s)
    }
}

/// File-name-safe stem: lowercase ASCII letters and digits, `_` between words, no dots or
/// separators (so nothing can climb out of the output folder), no Windows device names,
/// at most 80 characters, never empty (`fallback` then).
pub fn safe_stem(s: &str, fallback: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        let f = fold(&c.to_string());
        for d in f.chars() {
            if d.is_ascii_alphanumeric() {
                o.push(d.to_ascii_lowercase());
            } else if !o.ends_with('_') && !o.is_empty() {
                o.push('_');
            }
        }
    }
    let mut o: String = o.trim_matches('_').chars().take(80).collect();
    o = o.trim_matches('_').to_string();
    if o.is_empty() {
        o = fallback.to_string();
    }
    let reserved = ["con", "prn", "aux", "nul"].iter().any(|r| o == *r)
        || ((o.starts_with("com") || o.starts_with("lpt"))
            && o.len() == 4
            && o.as_bytes()[3].is_ascii_digit());
    if reserved {
        o.insert(0, '_');
    }
    o
}

// ---------------------------------------------------------------- guessing from names

#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    pub kind: Kind,
    pub title: String,
    pub date: String,
    pub issue: String,
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn year_ok(s: &str) -> bool {
    s.len() == 4 && s.parse::<u32>().is_ok_and(|y| (1450..=2100).contains(&y))
}

/// `YYYY-MM-DD` / `YYYY-MM` / `YYYYMMDD` as the normalised date, when it is one.
fn date_token(t: &str) -> Option<String> {
    let (y, m, d): (&str, &str, Option<&str>) = if t.len() == 8 && is_digits(t) {
        (&t[..4], &t[4..6], Some(&t[6..]))
    } else {
        let p: Vec<&str> = t.split('-').collect();
        match p.as_slice() {
            [y, m] if y.len() == 4 && m.len() == 2 => (y, m, None),
            [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => (y, m, Some(d)),
            _ => return None,
        }
    };
    if !(year_ok(y) && is_digits(m) && d.is_none_or(is_digits)) {
        return None;
    }
    let mo: u32 = m.parse().ok()?;
    if !(1..=12).contains(&mo) {
        return None;
    }
    match d {
        None => Some(format!("{y}-{m}")),
        Some(d) => {
            let dd: u32 = d.parse().ok()?;
            (1..=31).contains(&dd).then(|| format!("{y}-{m}-{d}"))
        }
    }
}

fn title_case(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Put `_` around `YYYY-MM-DD` / `YYYY-MM` inside a name so that it stays one token.
fn isolate_iso_dates(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut o = String::new();
    let mut i = 0;
    'outer: while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            for len in [10usize, 7] {
                if i + len > b.len() {
                    continue;
                }
                let cand: String = b[i..i + len].iter().collect();
                let after = b.get(i + len).copied();
                let more =
                    after == Some('-') && b.get(i + len + 1).is_some_and(|c| c.is_ascii_digit());
                if after.is_none_or(|c| !c.is_ascii_digit())
                    && !(len == 7 && more)
                    && date_token(&cand).is_some()
                {
                    o.push('_');
                    o.push_str(&cand);
                    o.push('_');
                    i += len;
                    continue 'outer;
                }
            }
        }
        o.push(b[i]);
        i += 1;
    }
    o
}

/// Guess kind, title, date and issue from a file or folder name (without extension), e.g.
/// `scinteia_tineretului_1977_09_8801` -> newspaper "Scinteia Tineretului", 1977-09, no. 8801.
/// Only a guess (diacritics are lost in file names; the kind rests on a few key words and on
/// how the issue is numbered): the person corrects it in the detail pane.
pub fn guess_from_name(stem: &str) -> Guess {
    // tokens: split on _ space . ( ) [ ]; a token that is a date keeps its dashes, any other
    // splits on dashes too
    let isolated = isolate_iso_dates(stem);
    let mut toks: Vec<String> = Vec::new();
    for t in isolated.split(['_', ' ', '.', '(', ')', '[', ']', ',']) {
        if t.is_empty() {
            continue;
        }
        if date_token(t).is_some() {
            toks.push(t.to_string());
        } else {
            toks.extend(t.split('-').filter(|x| !x.is_empty()).map(str::to_string));
        }
    }
    let low: Vec<String> = toks.iter().map(|t| fold(t)).collect();
    let marker = |w: &str| {
        matches!(
            w,
            "nr" | "no" | "numarul" | "numar" | "n" | "issue" | "num" | "numero"
        )
    };
    let vol_marker = |w: &str| matches!(w, "vol" | "volum" | "volumul" | "tom" | "t" | "band");

    let mut date = String::new();
    let mut issue = String::new();
    let mut title_end = toks.len();
    let mut has_full_date = false;
    let mut has_month = false;

    let pos = (0..toks.len()).find(|&i| date_token(&toks[i]).is_some() || year_ok(&toks[i]));
    if let Some(i) = pos {
        title_end = i;
        let mut j = i + 1;
        if let Some(d) = date_token(&toks[i]) {
            has_full_date = d.len() == 10;
            has_month = true;
            date = d;
        } else {
            date = toks[i].clone();
            if let Some(m) = toks.get(j).filter(|m| {
                m.len() <= 2
                    && is_digits(m)
                    && m.parse::<u32>().is_ok_and(|v| (1..=12).contains(&v))
            }) {
                // a month only when something (an issue number) follows or nothing does
                if toks.get(j + 1).is_some_and(|n| is_digits(n)) || j + 1 == toks.len() {
                    date = format!("{}-{:02}", toks[i], m.parse::<u32>().unwrap_or(0));
                    has_month = true;
                    j += 1;
                    // YYYY_MM_DD: a two-digit day (a number of the year would be written
                    // with its marker, `nr_14`, or have more digits)
                    if let Some(d) = toks.get(j).filter(|d| {
                        d.len() == 2
                            && is_digits(d)
                            && d.parse::<u32>().is_ok_and(|v| (1..=31).contains(&v))
                    }) {
                        date = format!("{date}-{d}");
                        has_full_date = true;
                        j += 1;
                    }
                }
            }
        }
        // issue: marker + number, volume marker + number, or the next bare number
        while j < toks.len() {
            if (marker(&low[j]) || vol_marker(&low[j]))
                && toks.get(j + 1).is_some_and(|n| is_digits(n))
            {
                issue = if vol_marker(&low[j]) {
                    format!("vol. {}", toks[j + 1])
                } else {
                    toks[j + 1].clone()
                };
                break;
            }
            if is_digits(&toks[j]) && toks[j].len() <= 7 {
                issue = toks[j].clone();
                break;
            }
            j += 1;
        }
    } else {
        // no date: a trailing number (after an optional marker) is the issue
        if let Some(last) = toks.last().filter(|l| is_digits(l) && l.len() <= 7) {
            issue = last.clone();
            title_end = toks.len() - 1;
            if title_end > 0 && (marker(&low[title_end - 1]) || vol_marker(&low[title_end - 1])) {
                if vol_marker(&low[title_end - 1]) {
                    issue = format!("vol. {issue}");
                }
                title_end -= 1;
            }
        }
    }

    let words: Vec<String> = toks[..title_end]
        .iter()
        .filter(|w| !is_digits(w) || title_end == toks.len())
        .cloned()
        .collect();
    let mut title = title_case(&words);
    if title.is_empty() {
        title = stem.replace(['_', '-'], " ").trim().to_string();
    }

    let has = |list: &[&str]| low.iter().any(|w| list.contains(&w.as_str()));
    let kind = if has(&[
        "ziar",
        "ziarul",
        "gazeta",
        "gazette",
        "journal",
        "newspaper",
        "zeitung",
        "giornale",
        "periodico",
        "diario",
        "tageblatt",
        "jurnal",
    ]) {
        Kind::Newspaper
    } else if has(&[
        "revista",
        "magazin",
        "magazine",
        "review",
        "rivista",
        "zeitschrift",
        "revue",
        "buletin",
    ]) {
        Kind::Magazine
    } else if has(&[
        "carte",
        "book",
        "livre",
        "libro",
        "buch",
        "libro",
        "roman",
        "manual",
        "tratat",
        "enciclopedia",
    ]) {
        Kind::Book
    } else if has_full_date || issue.parse::<u32>().is_ok_and(|n| n >= 500) {
        // a day-by-day date, or an issue number only a daily reaches
        Kind::Newspaper
    } else if !issue.is_empty() && !date.is_empty() {
        Kind::Magazine
    } else if !date.is_empty() && !has_month {
        Kind::Book
    } else {
        Kind::Other
    };
    Guess {
        kind,
        title,
        date,
        issue,
    }
}

// ---------------------------------------------------------------- JSON helpers

fn jstr(o: &J, k: &str) -> String {
    o.get(k).and_then(J::str).unwrap_or("").to_string()
}
fn jstrs(o: &J, k: &str) -> Vec<String> {
    match o.get(k) {
        Some(J::Arr(a)) => a
            .iter()
            .filter_map(|x| x.str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}
fn jnum(o: &J, k: &str) -> Option<u64> {
    match o.get(k) {
        Some(J::Num(n)) if n.is_finite() && *n >= 0.0 && *n < 1e15 => Some(*n as u64),
        _ => None,
    }
}
fn s(x: &str) -> J {
    J::Str(x.to_string())
}
fn arr(v: &[String]) -> J {
    J::Arr(v.iter().map(|x| s(x)).collect())
}

fn read_json(path: &Path, what: &str) -> Result<J> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("{}: cannot open the {what}", path.display()))?
        .len();
    if len > MAX_FILE {
        bail!("{}: too large to be a {what} ({len} bytes)", path.display());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("{}: cannot read the {what}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("{}: damaged {what} (not valid UTF-8 text)", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    mcp::parse(text).ok_or_else(|| {
        anyhow!(
            "{}: damaged {what} (not valid JSON; the file may be truncated or edited by hand)",
            path.display()
        )
    })
}

impl Item {
    fn to_json(&self) -> J {
        let mut o = vec![
            ("id".to_string(), s(&self.id)),
            ("kind".into(), s(self.kind.code())),
            ("title".into(), s(&self.title)),
            ("issue".into(), s(&self.issue)),
            ("date".into(), s(&self.date)),
            ("languages".into(), arr(&self.languages)),
            ("sources".into(), arr(&self.sources)),
            ("output".into(), s(&self.output)),
            ("status".into(), s(self.status.code())),
        ];
        if let Some(p) = self.pages {
            o.push(("pages".into(), J::Num(p as f64)));
        }
        o.push(("tags".into(), arr(&self.tags)));
        o.push(("notes".into(), s(&self.notes)));
        o.push(("profile".into(), s(&self.profile)));
        o.push(("error".into(), s(&self.error)));
        if let Some(t) = self.processed_at {
            o.push(("processed_at".into(), J::Num(t as f64)));
        }
        J::Obj(o)
    }

    fn from_json(o: &J) -> Item {
        Item {
            id: jstr(o, "id"),
            kind: Kind::from_code(&jstr(o, "kind")).unwrap_or_default(),
            title: jstr(o, "title"),
            issue: jstr(o, "issue"),
            date: jstr(o, "date"),
            languages: jstrs(o, "languages"),
            sources: jstrs(o, "sources"),
            output: jstr(o, "output"),
            status: Status::from_code(&jstr(o, "status")).unwrap_or_default(),
            pages: jnum(o, "pages").map(|n| n.min(u32::MAX as u64) as u32),
            tags: jstrs(o, "tags"),
            notes: jstr(o, "notes"),
            profile: jstr(o, "profile"),
            error: jstr(o, "error"),
            processed_at: jnum(o, "processed_at"),
        }
    }

    /// Short text for the table and the search: `1977-09 · no. 8801`.
    pub fn issue_line(&self) -> String {
        let mut v = Vec::new();
        if !self.date.is_empty() {
            v.push(self.date.clone());
        }
        if !self.issue.is_empty() {
            if is_digits(&self.issue) {
                v.push(format!("no. {}", self.issue));
            } else {
                v.push(self.issue.clone());
            }
        }
        v.join(" · ")
    }
}

/// Result of adding files or folders.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AddReport {
    /// Ids of the new items.
    pub added: Vec<String>,
    /// Already in the collection (same first source).
    pub duplicates: usize,
    /// Entries of a folder that are not scans and were left out.
    pub ignored: usize,
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn has_images(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let p = e.path();
            p.is_file() && crate::files::is_image(&p)
        })
    })
}

fn count_pages(p: &Path) -> Option<u32> {
    if p.is_dir() || ext_of(p) != "lgr" {
        let n = crate::files::collect_images(&[p.to_path_buf()]).len();
        (n > 0).then_some(n.min(u32::MAX as usize) as u32)
    } else {
        crate::project::Project::load(p)
            .ok()
            .map(|pr| pr.sheets.len() as u32)
            .filter(|&n| n > 0)
    }
}

impl Collection {
    pub fn new(name: &str) -> Collection {
        Collection {
            name: name.trim().to_string(),
            ..Default::default()
        }
    }

    pub fn to_text(&self) -> String {
        let head = [
            ("format", s(FORMAT)),
            ("version", J::Num(VERSION as f64)),
            ("name", s(&self.name)),
            ("description", s(&self.description)),
            ("default_profile", s(&self.default_profile)),
            ("ocr", J::Bool(self.ocr)),
            ("output_dir", s(&self.output_dir)),
            ("next_id", J::Num(self.next_id as f64)),
        ];
        let mut o = String::from("{\n");
        for (k, v) in head {
            o.push_str(&format!(
                "  {}: {},\n",
                J::Str(k.into()).render(),
                v.render()
            ));
        }
        o.push_str("  \"items\": [");
        for (i, it) in self.items.iter().enumerate() {
            o.push_str(if i == 0 { "\n    " } else { ",\n    " });
            o.push_str(&it.to_json().render());
        }
        o.push_str(if self.items.is_empty() {
            "]\n}\n"
        } else {
            "\n  ]\n}\n"
        });
        o
    }

    pub fn from_text(text: &str, what: &Path) -> Result<Collection> {
        let j = mcp::parse(text.trim_start_matches('\u{feff}')).ok_or_else(|| {
            anyhow!(
                "{}: damaged collection file (not valid JSON; truncated or edited by hand?)",
                what.display()
            )
        })?;
        Collection::from_json(&j, what)
    }

    fn from_json(j: &J, what: &Path) -> Result<Collection> {
        if !matches!(j, J::Obj(_)) {
            bail!(
                "{}: not a collection file (the top level is not an object)",
                what.display()
            );
        }
        if let Some(f) = j.get("format").and_then(J::str) {
            if f != FORMAT {
                bail!("{}: not a collection file (format '{f}')", what.display());
            }
        }
        if jnum(j, "version").is_some_and(|v| v > VERSION as u64) {
            bail!(
                "{}: made by a newer LegatorOCR (file version {}, this one reads up to {VERSION})",
                what.display(),
                jnum(j, "version").unwrap_or(0)
            );
        }
        let mut c = Collection {
            name: jstr(j, "name"),
            description: jstr(j, "description"),
            default_profile: jstr(j, "default_profile"),
            ocr: !matches!(j.get("ocr"), Some(J::Bool(false))),
            output_dir: jstr(j, "output_dir"),
            items: Vec::new(),
            next_id: jnum(j, "next_id").unwrap_or(1).max(1),
        };
        match j.get("items") {
            None | Some(J::Null) => {}
            Some(J::Arr(a)) => {
                for (n, it) in a.iter().enumerate() {
                    if !matches!(it, J::Obj(_)) {
                        bail!(
                            "{}: damaged collection file (item {} is not an object)",
                            what.display(),
                            n + 1
                        );
                    }
                    c.items.push(Item::from_json(it));
                }
            }
            Some(_) => bail!(
                "{}: damaged collection file (\"items\" is not a list)",
                what.display()
            ),
        }
        c.repair_ids();
        Ok(c)
    }

    /// Items without an id, or sharing one, get a new id; `next_id` stays above all of them.
    fn repair_ids(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        for it in &self.items {
            if let Some(n) = it.id.strip_prefix('i').and_then(|n| n.parse::<u64>().ok()) {
                self.next_id = self.next_id.max(n.saturating_add(1));
            }
        }
        for i in 0..self.items.len() {
            if self.items[i].id.is_empty() || !seen.insert(self.items[i].id.clone()) {
                loop {
                    let id = format!("i{}", self.next_id);
                    self.next_id += 1;
                    if seen.insert(id.clone()) {
                        self.items[i].id = id;
                        break;
                    }
                }
            }
        }
    }

    pub fn load(path: &Path) -> Result<Collection> {
        let j = read_json(path, "collection file")?;
        Collection::from_json(&j, path)
    }

    /// Write the file atomically (temporary sibling, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::write_atomic(path, self.to_text().as_bytes())
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// Append an item (id assigned); returns the id.
    pub fn push(&mut self, mut it: Item) -> String {
        it.id = format!("i{}", self.next_id);
        self.next_id += 1;
        let id = it.id.clone();
        self.items.push(it);
        id
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.id != id);
        self.items.len() != n
    }

    /// Ids of the items that are not done yet; with `include_failed = false` those whose last
    /// run failed are left out (so a batch does not retry them in a loop).
    pub fn pending_ids(&self, include_failed: bool) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.status == Status::Todo && (include_failed || i.error.is_empty()))
            .map(|i| i.id.clone())
            .collect()
    }

    fn already_has(&self, first_source: &str) -> bool {
        self.items
            .iter()
            .any(|i| i.sources.first().is_some_and(|s| s == first_source))
    }

    /// Add one item made of `paths` (images, folders, a PDF or a `.lgr` project). Title, kind,
    /// date and issue are guessed from the first name. `col_file` is the collection's file
    /// (paths are stored relative to its folder).
    pub fn add_item(&mut self, col_file: &Path, paths: &[PathBuf]) -> Result<String> {
        let first = paths.first().context("nothing to add")?;
        if paths.iter().any(|p| !p.exists()) {
            bail!(
                "{}: not found",
                paths
                    .iter()
                    .find(|p| !p.exists())
                    .map_or_else(String::new, |p| p.display().to_string())
            );
        }
        if let Some(bad) = paths.iter().find(|p| {
            if p.is_dir() {
                count_pages(p).is_none()
            } else {
                !(crate::files::is_image(p) || ext_of(p) == "lgr" || crate::archive::is_archive(p))
            }
        }) {
            bail!(
                "{}: not a scan (expected images, a PDF, a .lgr project, an archive or a folder of images)",
                bad.display()
            );
        }
        let base = col_file.parent().unwrap_or(Path::new("."));
        let stem = if first.is_dir() {
            first.file_name()
        } else {
            first.file_stem()
        }
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
        let g = guess_from_name(&stem);
        let pages: u32 = paths.iter().filter_map(|p| count_pages(p)).sum();
        Ok(self.push(Item {
            kind: g.kind,
            title: g.title,
            issue: g.issue,
            date: g.date,
            sources: paths.iter().map(|p| to_stored(base, p)).collect(),
            pages: (pages > 0).then_some(pages),
            ..Default::default()
        }))
    }

    /// Add one item per scan file (PDF, `.lgr`) and per subfolder with images in `dir`
    /// (not recursive). Names are never overwritten: what is already present is counted
    /// in [`AddReport::duplicates`]; anything else in the folder in `ignored`.
    pub fn add_folder(&mut self, col_file: &Path, dir: &Path) -> Result<AddReport> {
        let base = col_file.parent().unwrap_or(Path::new("."));
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("{}: cannot read the folder", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries
            .sort_by(|a, b| crate::files::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        let mut rep = AddReport::default();
        for p in entries {
            let is_item = if p.is_dir() {
                has_images(&p)
            } else {
                p.is_file() && matches!(ext_of(&p).as_str(), "pdf" | "lgr")
            };
            if !is_item {
                rep.ignored += 1; // loose images, notes, folders without scans
                continue;
            }
            if self.already_has(&to_stored(base, &p)) {
                rep.duplicates += 1;
                continue;
            }
            rep.added.push(self.add_item(col_file, &[p])?);
        }
        Ok(rep)
    }

    /// The folder where PDFs go: the collection's `output_dir` if set, else `override_dir`
    /// from the caller, else the folder of the collection file.
    pub fn out_dir(&self, col_file: &Path, override_dir: Option<&Path>) -> PathBuf {
        let base = col_file.parent().unwrap_or(Path::new("."));
        match override_dir {
            Some(d) => d.to_path_buf(),
            None if !self.output_dir.is_empty() => from_stored(base, &self.output_dir),
            None => base.to_path_buf(),
        }
    }

    /// A file name for the item's PDF inside `dir` that no other item uses and no file has.
    pub fn output_name_for(&self, it: &Item, dir: &Path) -> String {
        let mut stem = safe_stem(&it.title, &format!("item_{}", it.id));
        for part in [&it.date, &it.issue] {
            let p = safe_stem(part, "");
            if !p.is_empty() && !stem.contains(&p) {
                stem = format!("{stem}_{p}");
            }
        }
        let stem: String = stem.chars().take(120).collect();
        let used: HashSet<String> = self
            .items
            .iter()
            .filter(|o| o.id != it.id && !o.output.is_empty())
            .filter_map(|o| {
                Path::new(&o.output)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_lowercase())
            })
            .collect();
        let mut n = 1;
        loop {
            let name = if n == 1 {
                format!("{stem}.pdf")
            } else {
                format!("{stem}_{n}.pdf")
            };
            if !used.contains(&name.to_lowercase()) && !dir.join(&name).exists() {
                return name;
            }
            n += 1;
        }
    }
}

// ---------------------------------------------------------------- search index

/// Words are compared by this key: lower case, no diacritics, modern Romanian spelling.
pub fn word_key(w: &str) -> String {
    fold(&old_ro_to_modern(w))
}

fn tokens(text: &str) -> Vec<(String, usize, usize)> {
    let mut v = Vec::new();
    let mut start: Option<usize> = None;
    let push = |a: usize, b: usize, v: &mut Vec<(String, usize, usize)>| {
        v.push((word_key(&text[a..b]), a, b));
    };
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            push(a, i, &mut v);
        }
    }
    if let Some(a) = start {
        push(a, text.len(), &mut v);
    }
    v
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub item_id: String,
    /// 1-based page of the PDF.
    pub page: usize,
    pub snippet: String,
    /// All the words (not the exact phrase) were on the page but not next to each other.
    pub loose: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    docs: Vec<(String, Vec<String>)>,
    /// word key -> sorted (doc, page index) pairs
    inv: HashMap<String, Vec<(u32, u32)>>,
}

impl Index {
    pub fn load(path: &Path) -> Result<Index> {
        if !path.exists() {
            return Ok(Index::default());
        }
        let j = read_json(path, "search index")?;
        let Some(J::Arr(items)) = j.get("items") else {
            bail!("{}: damaged search index (no item list)", path.display());
        };
        let mut docs = Vec::new();
        for it in items {
            let id = jstr(it, "id");
            if id.is_empty() {
                continue;
            }
            docs.push((id, jstrs(it, "pages")));
        }
        let mut ix = Index {
            docs,
            inv: HashMap::new(),
        };
        ix.rebuild();
        Ok(ix)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut o = String::from("{\"format\":\"lgrindex\",\"version\":1,\"items\":[");
        for (i, (id, pages)) in self.docs.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            o.push_str(&format!(
                "\n{{\"id\":{},\"pages\":{}}}",
                s(id).render(),
                arr(pages).render()
            ));
        }
        o.push_str("\n]}\n");
        crate::write_atomic(path, o.as_bytes())
    }

    fn rebuild(&mut self) {
        self.inv.clear();
        for (d, (_, pages)) in self.docs.iter().enumerate() {
            for (p, text) in pages.iter().enumerate() {
                let mut seen: HashSet<String> = HashSet::new();
                for (k, _, _) in tokens(text) {
                    if seen.insert(k.clone()) {
                        self.inv.entry(k).or_default().push((d as u32, p as u32));
                    }
                }
            }
        }
    }

    /// Replace (or add) the text of an item, one string per page.
    pub fn set(&mut self, id: &str, pages: Vec<String>) {
        match self.docs.iter_mut().find(|(i, _)| i == id) {
            Some(d) => d.1 = pages,
            None => self.docs.push((id.to_string(), pages)),
        }
        self.rebuild();
    }

    pub fn remove(&mut self, id: &str) {
        let n = self.docs.len();
        self.docs.retain(|(i, _)| i != id);
        if self.docs.len() != n {
            self.rebuild();
        }
    }

    pub fn has(&self, id: &str) -> bool {
        self.docs.iter().any(|(i, _)| i == id)
    }

    pub fn page_count(&self) -> usize {
        self.docs.iter().map(|(_, p)| p.len()).sum()
    }

    /// Pages containing the words of `query` (ignoring case, diacritics and old spelling).
    /// Pages with the words next to each other, in that order, come first; then (marked
    /// `loose`) pages that have all the words somewhere. At most `max` hits (0 = no limit),
    /// in collection order.
    pub fn search(&self, query: &str, max: usize) -> Vec<Hit> {
        let q: Vec<String> = tokens(query).into_iter().map(|t| t.0).collect();
        if q.is_empty() {
            return Vec::new();
        }
        let mut cand: Option<Vec<(u32, u32)>> = None;
        for k in &q {
            let Some(list) = self.inv.get(k) else {
                return Vec::new();
            };
            cand = Some(match cand {
                None => list.clone(),
                Some(c) => {
                    let set: HashSet<&(u32, u32)> = list.iter().collect();
                    c.into_iter().filter(|x| set.contains(x)).collect()
                }
            });
        }
        let mut cand = cand.unwrap_or_default();
        cand.sort_unstable();
        let (mut exact, mut loose) = (Vec::new(), Vec::new());
        for (d, p) in cand {
            let (id, pages) = &self.docs[d as usize];
            let text = &pages[p as usize];
            let toks = tokens(text);
            let found = (0..toks.len()).find(|&i| {
                i + q.len() <= toks.len() && q.iter().enumerate().all(|(n, k)| toks[i + n].0 == *k)
            });
            let (at, is_loose) = match found {
                Some(i) => (toks[i].1, false),
                None => match toks.iter().find(|t| t.0 == q[0]) {
                    Some(t) => (t.1, true),
                    None => continue,
                },
            };
            let hit = Hit {
                item_id: id.clone(),
                page: p as usize + 1,
                snippet: snippet(text, at),
                loose: is_loose,
            };
            if is_loose {
                loose.push(hit)
            } else {
                exact.push(hit)
            }
        }
        exact.extend(loose);
        if max > 0 {
            exact.truncate(max);
        }
        exact
    }
}

/// About 70 characters either side of byte offset `at`, on one line.
fn snippet(text: &str, at: usize) -> String {
    let before: Vec<(usize, char)> = text[..at].char_indices().rev().take(70).collect();
    let from = before.last().map_or(at, |x| x.0);
    let to = text[at..]
        .char_indices()
        .nth(90)
        .map_or(text.len(), |(i, _)| at + i);
    let body = text[from..to]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}{body}{}",
        if from > 0 { "… " } else { "" },
        if to < text.len() { " …" } else { "" }
    )
}

/// The index file that belongs to a collection file: `archive.lgrcol` -> `archive.lgrindex`.
pub fn index_path(col_file: &Path) -> PathBuf {
    col_file.with_extension("lgrindex")
}

// ---------------------------------------------------------------- batch

/// What the real processing of one item needs, with every path resolved.
#[derive(Debug, Clone)]
pub struct ItemRun {
    pub title: String,
    pub sources: Vec<PathBuf>,
    /// Loaded profile (`None` = engine defaults).
    pub profile: Option<crate::project::Project>,
    pub languages: Vec<String>,
    pub ocr: bool,
    pub tesseract: Option<PathBuf>,
    pub keywords: String,
    pub subject: String,
}

#[derive(Debug, Default)]
pub struct ItemOutput {
    pub pdf: Vec<u8>,
    pub pages: usize,
    /// Recognised text per output page (empty when OCR was off).
    pub texts: Vec<String>,
    /// Sheets that could not be read (the rest was made).
    pub failed: Vec<(PathBuf, String)>,
}

pub type Runner<'a> = &'a (dyn Fn(&ItemRun, &AtomicBool) -> Result<ItemOutput> + Sync);

#[derive(Debug, Clone, Default)]
pub struct RunConfig {
    /// PDFs go here instead of the collection's output folder.
    pub out_dir: Option<PathBuf>,
    /// `Some` overrides the collection's OCR switch for this run.
    pub ocr: Option<bool>,
    /// Profile for items that name none, overriding the collection's.
    pub profile: Option<String>,
    pub tesseract: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchReport {
    pub ok: Vec<String>,
    /// (item id, message)
    pub failed: Vec<(String, String)>,
    /// Stopped by the cancel flag before this many items were tried.
    pub not_run: usize,
    pub cancelled: bool,
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "internal error".into())
}

/// Process the items `ids` one after the other. One item that fails (missing source, unknown
/// profile, a reading error, even a panic) is marked and the next one runs. After each item
/// the collection file and the index are saved, so a cancel or a crash loses nothing done.
/// `progress(done, total, title)` is called before each item and at the end.
#[allow(clippy::too_many_arguments)]
pub fn run_batch(
    col_file: &Path,
    col: &mut Collection,
    index: &mut Index,
    ids: &[String],
    cfg: &RunConfig,
    cancel: &AtomicBool,
    runner: Runner,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<BatchReport> {
    let base = col_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let idx_file = index_path(col_file);
    let out_dir = col.out_dir(col_file, cfg.out_dir.as_deref());
    let total = ids.len();
    let mut rep = BatchReport::default();
    for (n, id) in ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            rep.cancelled = true;
            rep.not_run = total - n;
            break;
        }
        let Some(it) = col.item(id).cloned() else {
            rep.failed.push((id.clone(), "no such item".into()));
            continue;
        };
        progress(n, total, &it.title);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<(ItemOutput, PathBuf, bool)> {
                if it.sources.is_empty() {
                    bail!("the item has no source files");
                }
                let sources: Vec<PathBuf> =
                    it.sources.iter().map(|s| from_stored(&base, s)).collect();
                if let Some(m) = sources.iter().find(|p| !p.exists()) {
                    bail!("source not found: {}", m.display());
                }
                let pname = if !it.profile.is_empty() {
                    it.profile.clone()
                } else {
                    cfg.profile
                        .clone()
                        .unwrap_or_else(|| col.default_profile.clone())
                };
                let profile = if pname.is_empty() {
                    None
                } else {
                    Some(crate::profiles::load(&pname)?)
                };
                let ocr = cfg.ocr.unwrap_or(col.ocr);
                let run = ItemRun {
                    title: it.title.clone(),
                    sources,
                    profile,
                    languages: it.languages.clone(),
                    ocr,
                    tesseract: cfg.tesseract.clone(),
                    keywords: it.tags.join(", "),
                    subject: it.issue_line(),
                };
                let out_path = if it.output.is_empty() {
                    out_dir.join(col.output_name_for(&it, &out_dir))
                } else {
                    from_stored(&base, &it.output)
                };
                crate::check_writable(&out_path)?;
                let out = runner(&run, cancel)?;
                if out.pdf.is_empty() {
                    bail!("nothing was produced");
                }
                crate::write_atomic(&out_path, &out.pdf)?;
                Ok((out, out_path, ocr))
            },
        ));
        let r = match r {
            Ok(r) => r,
            Err(p) => Err(anyhow!("internal error: {}", panic_text(&*p))),
        };
        match r {
            Ok((out, out_path, ocr)) => {
                let warn = match out.failed.first() {
                    Some((p, m)) => format!(
                        "{} sheet(s) could not be read and were left out (first: {}: {m})",
                        out.failed.len(),
                        p.display()
                    ),
                    None => String::new(),
                };
                if let Some(x) = col.item_mut(id) {
                    x.output = to_stored(&base, &out_path);
                    x.status = if ocr {
                        Status::OcrDone
                    } else {
                        Status::Processed
                    };
                    x.pages = Some(out.pages.min(u32::MAX as usize) as u32);
                    x.error = warn;
                    x.processed_at = Some(crate::now_unix());
                }
                if ocr {
                    index.set(id, out.texts);
                } else {
                    index.remove(id);
                }
                rep.ok.push(id.clone());
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if msg == "cancelled" || cancel.load(Ordering::Relaxed) {
                    rep.cancelled = true;
                    rep.not_run = total - n;
                    break;
                }
                if let Some(x) = col.item_mut(id) {
                    x.error = msg.clone();
                }
                rep.failed.push((id.clone(), msg));
            }
        }
        col.save(col_file)?;
        index.save(&idx_file)?;
    }
    if !rep.cancelled {
        progress(total, total, "");
    }
    col.save(col_file)?;
    Ok(rep)
}

fn flag(p: &crate::project::Project, k: &str) -> bool {
    p.get(k)
        .is_some_and(|v| matches!(v, "true" | "yes" | "on" | "1"))
}

/// The real processing of one item: the sources become sheets (a `.lgr` project brings its
/// own page list and per-page adjustments), [`crate::run_export`] cleans, recognises and
/// builds the PDF. The profile's settings and extras (`pdf_quality`, `skip_blank`, `ocr_langs`,
/// `psm`, `min_conf`, `tesseract` …) apply.
pub fn run_item_with_engine(run: &ItemRun, cancel: &AtomicBool) -> Result<ItemOutput> {
    use crate::{ExportJob, PdfExport, SheetInput};
    let mut profile = run.profile.clone().unwrap_or_default();
    let mut sheets: Vec<SheetInput> = Vec::new();
    for src in &run.sources {
        if src.is_file() && ext_of(src) == "lgr" {
            let pr = crate::project::Project::load(src)?;
            for (p, adj) in &pr.sheets {
                sheets.push(SheetInput {
                    path: p.clone(),
                    settings: adj.apply(&pr.settings),
                });
            }
            if run.profile.is_none() {
                profile.extra = pr.extra.clone();
            }
        } else {
            for p in crate::files::collect_images(std::slice::from_ref(src)) {
                let dpi = crate::files::detect_dpi(&p).unwrap_or(profile.settings.dpi);
                sheets.push(SheetInput {
                    path: p,
                    settings: legatorocr_core::Settings {
                        dpi,
                        ..profile.settings.clone()
                    },
                });
            }
        }
    }
    if sheets.is_empty() {
        bail!("no readable page images in the sources");
    }
    let ocr = if run.ocr {
        let langs: Vec<String> = if run.languages.is_empty() {
            profile
                .get("ocr_langs")
                .filter(|l| !l.is_empty())
                .map(|l| l.split('+').map(str::to_string).collect())
                .unwrap_or_else(|| vec!["ro".into()])
        } else {
            run.languages.clone()
        };
        let o = crate::OcrOptions {
            lang: crate::ocr::canonical_lang(&langs.join("+")),
            psm: profile.get("psm").and_then(|v| v.parse().ok()).unwrap_or(3),
            min_conf: profile
                .get("min_conf")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            tesseract: run
                .tesseract
                .clone()
                .or_else(|| {
                    profile
                        .get("tesseract")
                        .filter(|t| !t.is_empty())
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| "tesseract".into()),
            ..Default::default()
        };
        crate::check_ocr(&o)?;
        Some(o)
    } else {
        None
    };
    let quality = profile
        .get("pdf_quality")
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(80);
    let job = ExportJob {
        sheets: &sheets,
        pdf: Some(PdfExport {
            quality,
            ocr: ocr.clone(),
            title: Some(run.title.clone()).filter(|t| !t.is_empty()),
            meta: crate::PdfMeta {
                subject: Some(run.subject.clone()).filter(|t| !t.is_empty()),
                keywords: Some(run.keywords.clone()).filter(|t| !t.is_empty()),
                created_unix: Some(crate::now_unix()),
                ..Default::default()
            },
            bookmarks: flag(&profile, "bookmarks"),
            pdfa: flag(&profile, "pdfa"),
            ..Default::default()
        }),
        skip_blank: flag(&profile, "skip_blank"),
        keep_going: true,
        osd: None,
        uniform_size: flag(&profile, "uniform_size"),
        text_ocr: ocr,
        skip_duplicates: flag(&profile, "skip_duplicates"),
        split_on_blank: false,
        split_on_barcode: false,
    };
    let res = crate::run_export(&job, cancel, &|_, _| {}, &|_, _| {})?;
    let pdf = res.pdf.unwrap_or_default();
    let texts: Vec<String> = match (&res.text, run.ocr) {
        (Some(t), true) => {
            let mut v: Vec<String> = t.split('\x0c').map(|p| p.trim().to_string()).collect();
            v.resize(res.pages, String::new());
            v
        }
        _ => Vec::new(),
    };
    Ok(ItemOutput {
        pdf,
        pages: res.pages,
        texts,
        failed: res.failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn tmp(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "lgrcol-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn name_guess_newspaper_with_month_and_issue() {
        let g = guess_from_name("scinteia_tineretului_1977_09_8801");
        assert_eq!(g.kind, Kind::Newspaper);
        assert_eq!(g.title, "Scinteia Tineretului");
        assert_eq!(g.date, "1977-09");
        assert_eq!(g.issue, "8801");
    }

    #[test]
    fn name_guess_other_shapes() {
        let g = guess_from_name("Gazeta-Transilvaniei-1914-05-12");
        assert_eq!(
            (g.kind, g.title.as_str(), g.date.as_str(), g.issue.as_str()),
            (Kind::Newspaper, "Gazeta Transilvaniei", "1914-05-12", "")
        );
        let g = guess_from_name("revista_sinteza_1985_nr_14");
        assert_eq!(g.kind, Kind::Magazine);
        assert_eq!(
            (g.title.as_str(), g.date.as_str(), g.issue.as_str()),
            ("Revista Sinteza", "1985", "14")
        );
        let g = guess_from_name("enciclopedia_romaniei_vol_2_1938");
        assert_eq!(g.kind, Kind::Book);
        assert_eq!(g.date, "1938");
        let g = guess_from_name("gazeta_1914_05_12");
        assert_eq!(
            (g.date.as_str(), g.issue.as_str(), g.kind),
            ("1914-05-12", "", Kind::Newspaper)
        );
        let g = guess_from_name("revista_1985_03_7");
        assert_eq!((g.date.as_str(), g.issue.as_str()), ("1985-03", "7"));
        let g = guess_from_name("ceva_fara_data");
        assert_eq!(g.kind, Kind::Other);
        assert_eq!(g.title, "Ceva Fara Data");
        let g = guess_from_name("12345");
        assert!(!g.title.is_empty());
        assert_eq!(guess_from_name("").title, "");
    }

    #[test]
    fn stored_paths_are_relative_when_they_can_be() {
        let base = tmp("rel");
        let col = base.join("c.lgrcol");
        let inside = base.join("scans").join("a.png");
        assert_eq!(to_stored(&base, &inside), "scans/a.png");
        assert_eq!(to_stored(&base, &base.join("x.pdf")), "x.pdf");
        let sib = base.parent().unwrap().join("lgrcol-sibling").join("x.pdf");
        assert_eq!(to_stored(&base, &sib), "../lgrcol-sibling/x.pdf");
        assert_eq!(to_stored(&base, &base), ".");
        assert_eq!(from_stored(&base, "scans/a.png"), base.join("scans/a.png"));
        // a path with `..` inside is normalised, an absolute stored path is kept
        assert_eq!(to_stored(&base, &base.join("a/../b/c.pdf")), "b/c.pdf");
        assert_eq!(
            from_stored(&base, "/abs/x.pdf"),
            PathBuf::from("/abs/x.pdf")
        );
        // the folder can move: the stored text does not change
        let mut c = Collection::new("t");
        std::fs::create_dir_all(base.join("scans")).unwrap();
        std::fs::write(base.join("scans/a.png"), b"x").unwrap();
        c.add_item(&col, &[inside]).unwrap();
        assert_eq!(c.items[0].sources, vec!["scans/a.png"]);
    }

    #[test]
    fn save_and_load_round_trip_and_atomic() {
        let d = tmp("rt");
        let f = d.join("a.lgrcol");
        let mut c = Collection::new("Ziare \"vechi\"");
        c.description = "linia 1\nlinia 2\t\\".into();
        c.default_profile = "flux-1-optim".into();
        c.ocr = false;
        let id = c.push(Item {
            title: "Scânteia".into(),
            kind: Kind::Newspaper,
            languages: vec!["ro".into(), "fr".into()],
            sources: vec!["a.pdf".into()],
            status: Status::Checked,
            pages: Some(8),
            tags: vec!["1977".into()],
            processed_at: Some(1700000000),
            ..Default::default()
        });
        c.save(&f).unwrap();
        let l = Collection::load(&f).unwrap();
        assert_eq!(l, c);
        assert_eq!(l.item(&id).unwrap().status, Status::Checked);
        // no temporary file is left behind
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let t = r#"{"format":"lgrcol","version":1,"future":{"a":[1,2]},"items":[
            {"title":"A","extra":true,"kind":"zzz","status":"nope","pages":-3},
            {"id":"i7","title":"B","pages":12},
            {"id":"i7","title":"C"}]}"#;
        let c = Collection::from_text(t, Path::new("x.lgrcol")).unwrap();
        assert_eq!(c.items.len(), 3);
        assert!(c.ocr);
        assert_eq!(c.items[0].kind, Kind::Other);
        assert_eq!(c.items[0].status, Status::Todo);
        assert_eq!(c.items[0].pages, None);
        let ids: HashSet<_> = c.items.iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids.len(), 3, "ids repaired to be unique: {ids:?}");
        assert_eq!(c.items[1].id, "i7");
    }

    #[test]
    fn damaged_files_give_clear_errors_never_panic() {
        let d = tmp("bad");
        let f = d.join("a.lgrcol");
        let good = Collection::new("x").to_text();
        for (text, want) in [
            ("", "not valid JSON"),
            ("{\"items\":[", "not valid JSON"),
            (&good[..good.len() / 2], "not valid JSON"),
            ("[1,2]", "not a collection"),
            ("{\"format\":\"other\"}", "not a collection"),
            ("{\"items\":[1]}", "item 1 is not an object"),
            ("{\"items\":5}", "not a list"),
            ("{\"version\":9}", "newer LegatorOCR"),
        ] {
            std::fs::write(&f, text).unwrap();
            let e = Collection::load(&f).unwrap_err().to_string();
            assert!(e.contains(want), "{text:?} -> {e}");
            assert!(e.contains("a.lgrcol"), "error names the file: {e}");
        }
        std::fs::write(&f, [0xff, 0xfe, 0x00]).unwrap();
        assert!(Collection::load(&f)
            .unwrap_err()
            .to_string()
            .contains("UTF-8"));
        assert!(Collection::load(&d.join("missing.lgrcol")).is_err());
        // deep nesting must not overflow the stack
        std::fs::write(&f, "[".repeat(100_000)).unwrap();
        assert!(Collection::load(&f).is_err());
        let ix = d.join("a.lgrindex");
        std::fs::write(&ix, "nonsense").unwrap();
        assert!(Index::load(&ix)
            .unwrap_err()
            .to_string()
            .contains("search index"));
        assert!(Index::load(&d.join("none.lgrindex"))
            .unwrap()
            .docs
            .is_empty());
    }

    #[test]
    fn output_names_cannot_climb_out_and_are_unique() {
        for t in [
            "../../etc/passwd",
            "..",
            "",
            "C:\\Windows\\x",
            "con",
            "a/b\\c.pdf\0",
        ] {
            let s = safe_stem(t, "item");
            assert!(!s.contains(['/', '\\', '.', ':', '\0']), "{t:?} -> {s:?}");
            assert!(!s.is_empty());
            assert!(!["con", "prn", "aux", "nul"].contains(&s.as_str()));
        }
        assert_eq!(
            safe_stem("Scânteia Tineretului!", "x"),
            "scanteia_tineretului"
        );
        assert_eq!(safe_stem("日本語", "fb"), "fb");
        assert!(safe_stem(&"a".repeat(500), "x").len() <= 80);
        let d = tmp("names");
        let mut c = Collection::new("n");
        let a = c.push(Item {
            title: "../../Evil".into(),
            date: "1977-09".into(),
            issue: "8801".into(),
            ..Default::default()
        });
        let b = c.push(Item {
            title: "../../Evil".into(),
            date: "1977-09".into(),
            issue: "8801".into(),
            ..Default::default()
        });
        let na = c.output_name_for(c.item(&a).unwrap(), &d);
        assert_eq!(na, "evil_1977_09_8801.pdf");
        c.item_mut(&a).unwrap().output = na.clone();
        let nb = c.output_name_for(c.item(&b).unwrap(), &d);
        assert_eq!(nb, "evil_1977_09_8801_2.pdf");
        assert_eq!(Path::new(&nb).parent(), Some(Path::new("")));
        // an existing file that is not ours is not overwritten either
        std::fs::write(d.join(&nb), b"x").unwrap();
        assert_eq!(
            c.output_name_for(c.item(&b).unwrap(), &d),
            "evil_1977_09_8801_3.pdf"
        );
    }

    #[test]
    fn add_folder_makes_items_from_pdfs_and_subfolders() {
        let d = tmp("addf");
        let src = d.join("scans");
        std::fs::create_dir_all(src.join("gazeta_1914_05_12")).unwrap();
        std::fs::create_dir_all(src.join("empty")).unwrap();
        std::fs::write(src.join("gazeta_1914_05_12/p1.png"), b"x").unwrap();
        std::fs::write(src.join("scinteia_tineretului_1977_09_8801.pdf"), b"%PDF").unwrap();
        std::fs::write(src.join("notes.txt"), b"x").unwrap();
        let col = d.join("c.lgrcol");
        let mut c = Collection::new("c");
        let r = c.add_folder(&col, &src).unwrap();
        assert_eq!(r.added.len(), 2);
        assert_eq!(r.ignored, 2);
        let t: Vec<_> = c.items.iter().map(|i| i.title.as_str()).collect();
        assert!(
            t.contains(&"Scinteia Tineretului") && t.contains(&"Gazeta"),
            "{t:?}"
        );
        assert!(c.items.iter().all(|i| i.sources[0].starts_with("scans/")));
        // adding again changes nothing
        let r = c.add_folder(&col, &src).unwrap();
        assert_eq!((r.added.len(), r.duplicates), (0, 2));
        assert!(c.add_item(&col, &[d.join("nope.pdf")]).is_err());
        let e = c.add_item(&col, &[src.join("notes.txt")]).unwrap_err();
        assert!(e.to_string().contains("not a scan"), "{e}");
        assert!(c.add_item(&col, &[src.join("empty")]).is_err());
        assert!(c.add_folder(&col, &d.join("nope")).is_err());
    }

    #[test]
    fn search_folds_diacritics_and_old_spelling() {
        let mut ix = Index::default();
        ix.set(
            "i1",
            vec![
                "Ziarul Scânteia a apărut astăzi.\nCititorii sînt mulțumiți.".into(),
                "Fără legătură. Ştiinţa şi tehnica".into(),
            ],
        );
        ix.set("i2", vec!["Scînteia tineretului, numărul 8801".into()]);
        let h = ix.search("Scînteia", 0);
        assert_eq!(
            h.iter()
                .map(|h| (h.item_id.as_str(), h.page))
                .collect::<Vec<_>>(),
            vec![("i1", 1), ("i2", 1)]
        );
        assert!(h[0].snippet.contains("Scânteia"), "{}", h[0].snippet);
        assert_eq!(ix.search("SCANTEIA", 0).len(), 2);
        assert_eq!(
            ix.search("sunt mulțumiți", 0).len(),
            1,
            "old spelling sînt = sunt"
        );
        assert_eq!(ix.search("stiinta", 0)[0].page, 2);
        assert_eq!(ix.search("Scânteia tineretului", 0).len(), 1);
        // all words present but not adjacent: found, marked loose, after the exact ones
        let h = ix.search("scanteia a aparut", 0);
        assert_eq!(h.len(), 1);
        assert!(!h[0].loose);
        assert!(ix.search("scanteia astazi", 0)[0].loose);
        let h = ix.search("cititorii scanteia", 0);
        assert_eq!(h.len(), 1);
        assert!(h[0].loose);
        assert!(ix.search("inexistent", 0).is_empty());
        assert!(ix.search("   ", 0).is_empty());
        assert_eq!(ix.search("scanteia", 1).len(), 1);
        ix.remove("i1");
        assert_eq!(ix.search("scanteia", 0).len(), 1);
    }

    #[test]
    fn index_round_trips_and_snippet_is_char_safe() {
        let d = tmp("ix");
        let f = d.join("a.lgrindex");
        let mut ix = Index::default();
        let long = format!("{} ăîșț cuvântul {}", "ă".repeat(300), "ț ".repeat(300));
        ix.set("i1", vec![long, String::new()]);
        ix.save(&f).unwrap();
        let l = Index::load(&f).unwrap();
        let h = l.search("cuvantul", 0);
        assert_eq!(h.len(), 1);
        assert!(h[0].snippet.starts_with('…') && h[0].snippet.ends_with('…'));
        assert!(h[0].snippet.chars().count() < 220);
        assert_eq!(l.page_count(), 2);
    }

    fn fake_ok(run: &ItemRun, _c: &AtomicBool) -> Result<ItemOutput> {
        Ok(ItemOutput {
            pdf: b"%PDF-fake".to_vec(),
            pages: 2,
            texts: vec![format!("pagina unu {}", run.title), "pagina doi".into()],
            failed: Vec::new(),
        })
    }

    #[test]
    fn batch_continues_after_a_failure_and_saves_progress() {
        let d = tmp("batch");
        let col_file = d.join("c.lgrcol");
        std::fs::write(d.join("a.png"), b"x").unwrap();
        std::fs::write(d.join("c.png"), b"x").unwrap();
        let mut c = Collection::new("b");
        for (t, src) in [
            ("Alfa", "a.png"),
            ("Beta", "missing.png"),
            ("Gama", "c.png"),
            ("Delta", "c.png"),
        ] {
            c.push(Item {
                title: t.into(),
                sources: vec![src.into()],
                ..Default::default()
            });
        }
        // the runner fails for "Gama" and panics for "Delta"
        let runner = |run: &ItemRun, cancel: &AtomicBool| -> Result<ItemOutput> {
            if run.title == "Gama" {
                bail!("boom");
            }
            if run.title == "Delta" {
                panic!("kaboom");
            }
            fake_ok(run, cancel)
        };
        let mut ix = Index::default();
        let ids = c.pending_ids(true);
        let mut seen = Vec::new();
        let rep = run_batch(
            &col_file,
            &mut c,
            &mut ix,
            &ids,
            &RunConfig::default(),
            &AtomicBool::new(false),
            &runner,
            &mut |n, t, name| seen.push((n, t, name.to_string())),
        )
        .unwrap();
        assert_eq!(rep.ok, vec!["i1"]);
        assert_eq!(rep.failed.len(), 3);
        assert!(rep.failed[0].1.contains("source not found"));
        assert_eq!(rep.failed[1].1, "boom");
        assert!(rep.failed[2].1.contains("kaboom"));
        assert!(!rep.cancelled);
        assert_eq!(seen.last().unwrap().0, 4);
        let a = c.item("i1").unwrap();
        assert_eq!((a.status, a.pages), (Status::OcrDone, Some(2)));
        assert_eq!(a.output, "alfa.pdf");
        assert_eq!(std::fs::read(d.join("alfa.pdf")).unwrap(), b"%PDF-fake");
        assert_eq!(c.item("i2").unwrap().status, Status::Todo);
        assert!(c.item("i3").unwrap().error.contains("boom"));
        // files on disk match memory
        let l = Collection::load(&col_file).unwrap();
        assert_eq!(l, c);
        let ixl = Index::load(&index_path(&col_file)).unwrap();
        assert_eq!(ixl.search("alfa", 0).len(), 1);
        // failed items are not retried unless asked; the done one is not pending
        assert!(c.pending_ids(false).is_empty());
        assert_eq!(c.pending_ids(true).len(), 3);
    }

    #[test]
    fn batch_cancel_stops_and_keeps_what_was_done() {
        let d = tmp("cancel");
        let col_file = d.join("c.lgrcol");
        std::fs::write(d.join("a.png"), b"x").unwrap();
        let mut c = Collection::new("b");
        for t in ["A", "B", "C"] {
            c.push(Item {
                title: t.into(),
                sources: vec!["a.png".into()],
                ..Default::default()
            });
        }
        let cancel = AtomicBool::new(false);
        let runner = |run: &ItemRun, cc: &AtomicBool| {
            if run.title == "B" {
                cc.store(true, Ordering::Relaxed);
                bail!("cancelled");
            }
            fake_ok(run, cc)
        };
        let mut ix = Index::default();
        let ids = c.pending_ids(true);
        let rep = run_batch(
            &col_file,
            &mut c,
            &mut ix,
            &ids,
            &RunConfig::default(),
            &cancel,
            &runner,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert!(rep.cancelled);
        assert_eq!((rep.ok.len(), rep.not_run), (1, 2));
        let l = Collection::load(&col_file).unwrap();
        assert_eq!(l.item("i1").unwrap().status, Status::OcrDone);
        assert_eq!(l.item("i2").unwrap().status, Status::Todo);
        assert!(l.item("i2").unwrap().error.is_empty());
    }

    #[test]
    fn batch_without_ocr_marks_processed_and_unknown_profile_fails_only_that_item() {
        let d = tmp("noocr");
        let col_file = d.join("c.lgrcol");
        std::fs::write(d.join("a.png"), b"x").unwrap();
        let mut c = Collection::new("b");
        c.push(Item {
            title: "A".into(),
            sources: vec!["a.png".into()],
            profile: "no-such-profile".into(),
            ..Default::default()
        });
        c.push(Item {
            title: "B".into(),
            sources: vec!["a.png".into()],
            ..Default::default()
        });
        let mut ix = Index::default();
        let ids = c.pending_ids(true);
        let cfg = RunConfig {
            ocr: Some(false),
            out_dir: Some(d.join("out")),
            ..Default::default()
        };
        let rep = run_batch(
            &col_file,
            &mut c,
            &mut ix,
            &ids,
            &cfg,
            &AtomicBool::new(false),
            &fake_ok,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(rep.failed.len(), 1);
        assert!(rep.failed[0].1.contains("unknown profile"));
        assert_eq!(c.item("i2").unwrap().status, Status::Processed);
        assert!(d.join("out/b.pdf").is_file());
        assert!(!ix.has("i2"));
    }

    #[test]
    fn engine_runner_makes_a_pdf_from_an_image_without_ocr() {
        let d = tmp("engine");
        let img = image::GrayImage::from_pixel(200, 300, image::Luma([255]));
        img.save(d.join("p1.png")).unwrap();
        let run = ItemRun {
            title: "Test".into(),
            sources: vec![d.join("p1.png")],
            profile: None,
            languages: vec![],
            ocr: false,
            tesseract: None,
            keywords: "a, b".into(),
            subject: "1977".into(),
        };
        let out = run_item_with_engine(&run, &AtomicBool::new(false)).unwrap();
        assert!(out.pdf.starts_with(b"%PDF"));
        assert_eq!(out.pages, 1);
        let empty = ItemRun {
            sources: vec![d.clone()]
                .into_iter()
                .map(|p| p.join("nothing"))
                .collect(),
            ..run
        };
        assert!(run_item_with_engine(&empty, &AtomicBool::new(false)).is_err());
    }
}
