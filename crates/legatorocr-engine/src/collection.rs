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

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The kind the key words of a name point to, if any.
fn word_kind(low: &[String]) -> Option<Kind> {
    let has = |list: &[&str]| low.iter().any(|w| list.contains(&w.as_str()));
    if has(&[
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
        Some(Kind::Newspaper)
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
        Some(Kind::Magazine)
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
        Some(Kind::Book)
    } else {
        None
    }
}

/// Guess kind, title, date and issue from a file or folder name (without extension), e.g.
/// `scinteia_tineretului_1977_09_8801` -> newspaper "Scinteia Tineretului", 1977-09, no. 8801.
/// Only a guess (diacritics are lost in file names; the kind rests on a few key words and on
/// how the issue is numbered): the person corrects it in the detail pane.
///
/// For `<name>_<year>_<month>_<number>` a number of three or more digits is the issue
/// (`..._1977_09_8801`). A number of one or two digits right after the month is a day only
/// when it is a real day of that month (`..._02_30` is not), and then, when it is the last
/// part, only if the name does not say it is a magazine or a book (`revista_1985_03_12` is
/// issue 12, `gazeta_1914_05_12` is the 12th of May); when another number follows
/// (`..._1977_09_12_8801`) it is always the day. A day written with dashes
/// (`1977-09-12`) is a day.
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
                // a month only when something (an issue number or its marker) follows or
                // nothing does
                if toks.get(j + 1).is_some_and(|n| is_digits(n))
                    || low.get(j + 1).is_some_and(|n| marker(n) || vol_marker(n))
                    || j + 1 == toks.len()
                {
                    date = format!("{}-{:02}", toks[i], m.parse::<u32>().unwrap_or(0));
                    has_month = true;
                    j += 1;
                    // YYYY_MM_DD: a two-digit day (a number of the year would be written
                    // with its marker, `nr_14`, or have more digits)
                    let ymonth = m.parse::<u32>().unwrap_or(0);
                    let yyear = toks[i].parse::<u32>().unwrap_or(0);
                    let last = j + 1 == toks.len();
                    let not_daily = matches!(word_kind(&low), Some(Kind::Magazine | Kind::Book));
                    if let Some(d) = toks.get(j).filter(|d| {
                        d.len() == 2
                            && is_digits(d)
                            && d.parse::<u32>()
                                .is_ok_and(|v| (1..=days_in_month(yyear, ymonth)).contains(&v))
                            && !(last && not_daily)
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

    let kind = if let Some(k) = word_kind(&low) {
        k
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

/// Word key of each raw word already seen: the same few thousand words come again and again,
/// and making the key (case, diacritics, old spelling) is the slow part of indexing.
type KeyCache = HashMap<String, String>;

/// Like [`tokens`], but the keys come from (and go to) `cache`.
fn tokens_with<'a>(text: &str, cache: &'a mut KeyCache) -> Vec<(&'a str, usize, usize)> {
    if cache.len() > 400_000 {
        cache.clear();
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            spans.push((a, i));
        }
    }
    if let Some(a) = start {
        spans.push((a, text.len()));
    }
    for &(a, b) in &spans {
        if !cache.contains_key(&text[a..b]) {
            cache.insert(text[a..b].to_string(), word_key(&text[a..b]));
        }
    }
    let cache: &'a KeyCache = cache;
    spans
        .into_iter()
        .map(|(a, b)| (cache[&text[a..b]].as_str(), a, b))
        .collect()
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

/// First line of an index file in the current format (one more line per item follows).
const INDEX_HEAD: &str = "{\"format\":\"lgrindex\",\"version\":2}";
/// An index file is streamed, so it may be far larger than a collection file.
const INDEX_MAX: u64 = 8 << 30;
const WORDS_MAGIC: &[u8] = b"LGRW2\n";

/// One item of the index. Its pages are numbered `base..base + pages` in the word lists.
#[derive(Debug, Clone)]
struct Doc {
    id: String,
    base: u32,
    pages: u32,
    /// Where the item's line is in the index file (offset, length), once it was written.
    loc: Option<(u64, u32)>,
    /// The text, only while it is not yet in the file (just recognised, or an old-format file).
    text: Option<Vec<String>>,
}

/// Pages that contain one word, as differences between ascending page numbers (LEB128).
#[derive(Debug, Clone, Default)]
struct Post {
    last: u32,
    bytes: Vec<u8>,
}

fn put_varint(b: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        b.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    b.push(v as u8);
}

fn get_varint(b: &[u8], i: &mut usize) -> Option<u32> {
    let (mut v, mut sh) = (0u32, 0u32);
    loop {
        let x = *b.get(*i)?;
        *i += 1;
        if sh > 28 {
            return None;
        }
        v |= ((x & 0x7f) as u32) << sh;
        if x & 0x80 == 0 {
            return Some(v);
        }
        sh += 7;
    }
}

impl Post {
    fn push(&mut self, gid: u32) {
        let d = if self.bytes.is_empty() {
            gid
        } else {
            gid - self.last
        };
        put_varint(&mut self.bytes, d);
        self.last = gid;
    }

    fn decode(&self) -> Vec<u32> {
        let (mut v, mut i, mut cur) = (Vec::new(), 0usize, 0u32);
        while let Some(d) = get_varint(&self.bytes, &mut i) {
            cur = cur.wrapping_add(d);
            v.push(cur);
        }
        v
    }
}

fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (mut i, mut j, mut o) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                o.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    o
}

fn doc_line(id: &str, pages: &[String]) -> Vec<u8> {
    format!(
        "{{\"id\":{},\"pages\":{}}}",
        s(id).render(),
        arr(pages).render()
    )
    .into_bytes()
}

/// Length, and a checksum of the first and last 64 KiB, of an index file: tells whether the
/// word-list cache still belongs to it.
fn file_check(path: &Path) -> Option<(u64, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut h: u64 = 0xcbf29ce484222325 ^ len;
    let mut buf = vec![0u8; 65536];
    for from in [0, len.saturating_sub(65536)] {
        f.seek(SeekFrom::Start(from)).ok()?;
        let n = f.read(&mut buf).ok()?;
        for b in &buf[..n] {
            h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
    }
    Some((len, h))
}

fn words_path(index_file: &Path) -> PathBuf {
    index_file.with_extension("lgrwords")
}

/// The search index of a collection: the text of every page of every item that was processed
/// or indexed, searchable by word.
///
/// Memory and time (measured, see the README): the text of the pages stays **in the file**
/// (`NAME.lgrindex`, one line per item); in memory there is only a compact list of pages per
/// word (a few bytes per page and word). The lists are also cached in `NAME.lgrwords`, so
/// opening a large index does not read the text again; that file is only a cache and may be
/// deleted (the next opening rebuilds it from the text). The text of the pages found is read
/// back when a search shows its snippet.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// The file the `loc` of each item refers to.
    file: Option<PathBuf>,
    /// In ascending order of `base`.
    docs: Vec<Doc>,
    inv: HashMap<String, Post>,
    next_base: u32,
    /// Pages of items replaced or removed that are still in the lists (skipped when searching).
    dead: u64,
}

impl Index {
    pub fn load(path: &Path) -> Result<Index> {
        if !path.exists() {
            return Ok(Index::default());
        }
        let len = std::fs::metadata(path)
            .with_context(|| format!("{}: cannot open the search index", path.display()))?
            .len();
        if len > INDEX_MAX {
            bail!(
                "{}: too large to be a search index ({len} bytes)",
                path.display()
            );
        }
        let mut rd = std::io::BufReader::new(
            std::fs::File::open(path)
                .with_context(|| format!("{}: cannot read the search index", path.display()))?,
        );
        let mut head = String::new();
        {
            use std::io::BufRead;
            rd.read_line(&mut head).ok();
        }
        if !head.contains("\"version\":2") {
            return Index::load_v1(path);
        }
        if let Some(ix) = Index::from_words(path) {
            return Ok(ix);
        }
        // scan the text (first opening, or the cache is missing or out of date)
        let mut ix = Index {
            file: Some(path.to_path_buf()),
            ..Default::default()
        };
        let mut off = head.len() as u64;
        let mut cache = KeyCache::new();
        let mut line = String::new();
        let mut n = 1usize;
        loop {
            use std::io::BufRead;
            line.clear();
            let got = rd
                .read_line(&mut line)
                .with_context(|| format!("{}: cannot read the search index", path.display()))?;
            if got == 0 {
                break;
            }
            n += 1;
            let complete = line.ends_with('\n');
            let body = line.trim_end_matches(['\n', '\r']);
            if body.trim().is_empty() {
                off += got as u64;
                continue;
            }
            let Some(j) = mcp::parse(body) else {
                if !complete {
                    break; // a last line cut short by a crash: ignored
                }
                bail!(
                    "{}: damaged search index (line {n} is not valid JSON)",
                    path.display()
                );
            };
            let id = jstr(&j, "id");
            if !id.is_empty() {
                let pages = jstrs(&j, "pages");
                ix.add_doc(
                    &id,
                    pages,
                    Some((off, body.len() as u32)),
                    false,
                    &mut cache,
                );
            }
            off += got as u64;
        }
        Ok(ix)
    }

    /// Files of the first format (version 1: one JSON document); kept readable, rewritten in
    /// the current format on the next save.
    fn load_v1(path: &Path) -> Result<Index> {
        let j = read_json(path, "search index")?;
        let Some(J::Arr(items)) = j.get("items") else {
            bail!("{}: damaged search index (no item list)", path.display());
        };
        let mut ix = Index::default();
        for it in items {
            let id = jstr(it, "id");
            if !id.is_empty() {
                ix.set(&id, jstrs(it, "pages"));
            }
        }
        Ok(ix)
    }

    /// The index from the text file and its word-list cache, when the cache fits the file.
    fn from_words(path: &Path) -> Option<Index> {
        let data = std::fs::read(words_path(path)).ok()?;
        let (len, chk) = file_check(path)?;
        let mut p = 0usize;
        let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
            let s = data.get(*p..p.checked_add(n)?)?;
            *p += n;
            Some(s)
        };
        let u64_ = |p: &mut usize| -> Option<u64> {
            Some(u64::from_le_bytes(take(p, 8)?.try_into().ok()?))
        };
        let var = |p: &mut usize| get_varint(&data, p);
        if take(&mut p, WORDS_MAGIC.len())? != WORDS_MAGIC
            || u64_(&mut p)? != len
            || u64_(&mut p)? != chk
        {
            return None;
        }
        let next_base = var(&mut p)?;
        let nd = var(&mut p)? as usize;
        let mut docs = Vec::with_capacity(nd.min(1 << 20));
        for _ in 0..nd {
            let il = var(&mut p)? as usize;
            let id = String::from_utf8(take(&mut p, il)?.to_vec()).ok()?;
            let base = var(&mut p)?;
            let pages = var(&mut p)?;
            let off = u64_(&mut p)?;
            let ln = var(&mut p)?;
            docs.push(Doc {
                id,
                base,
                pages,
                loc: Some((off, ln)),
                text: None,
            });
        }
        if docs.windows(2).any(|w| w[0].base >= w[1].base) {
            return None;
        }
        let nw = var(&mut p)? as usize;
        let mut inv = HashMap::with_capacity(nw.min(1 << 22));
        for _ in 0..nw {
            let wl = var(&mut p)? as usize;
            let w = String::from_utf8(take(&mut p, wl)?.to_vec()).ok()?;
            let last = var(&mut p)?;
            let bl = var(&mut p)? as usize;
            let bytes = take(&mut p, bl)?.to_vec();
            inv.insert(w, Post { last, bytes });
        }
        (p == data.len()).then_some(Index {
            file: Some(path.to_path_buf()),
            docs,
            inv,
            next_base,
            dead: 0,
        })
    }

    fn write_words(&self, path: &Path) -> Result<()> {
        let (len, chk) = file_check(path).context("cannot read the index just written")?;
        let mut o: Vec<u8> =
            Vec::with_capacity(self.inv.values().map(|p| p.bytes.len() + 24).sum());
        o.extend_from_slice(WORDS_MAGIC);
        o.extend_from_slice(&len.to_le_bytes());
        o.extend_from_slice(&chk.to_le_bytes());
        put_varint(&mut o, self.next_base);
        put_varint(&mut o, self.docs.len() as u32);
        for d in &self.docs {
            let (off, ln) = d.loc.context("item not written")?;
            put_varint(&mut o, d.id.len() as u32);
            o.extend_from_slice(d.id.as_bytes());
            put_varint(&mut o, d.base);
            put_varint(&mut o, d.pages);
            o.extend_from_slice(&off.to_le_bytes());
            put_varint(&mut o, ln);
        }
        put_varint(&mut o, self.inv.len() as u32);
        for (w, p) in &self.inv {
            put_varint(&mut o, w.len() as u32);
            o.extend_from_slice(w.as_bytes());
            put_varint(&mut o, p.last);
            put_varint(&mut o, p.bytes.len() as u32);
            o.extend_from_slice(&p.bytes);
        }
        crate::write_atomic(&words_path(path), &o)
    }

    /// Write the index to `path`: the text of unchanged items is copied from the old file
    /// (not read into memory all at once), then the word-list cache is written next to it.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        use std::io::Write;
        if self.dead > 0 && self.dead * 4 > self.page_count() as u64 + self.dead {
            self.rebuild_words()?;
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("lgrindex.part");
        let mut old = self.file.as_ref().and_then(|f| std::fs::File::open(f).ok());
        let mut locs: Vec<(u64, u32)> = Vec::with_capacity(self.docs.len());
        let r = (|| -> Result<()> {
            let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            w.write_all(INDEX_HEAD.as_bytes())?;
            w.write_all(b"\n")?;
            let mut off = INDEX_HEAD.len() as u64 + 1;
            for d in &self.docs {
                let bytes = match (&d.text, d.loc, old.as_mut()) {
                    (Some(t), _, _) => doc_line(&d.id, t),
                    (None, Some(l), Some(f)) => read_line_at(f, l)?,
                    _ => bail!("the text of '{}' is no longer available", d.id),
                };
                w.write_all(&bytes)?;
                w.write_all(b"\n")?;
                locs.push((off, bytes.len() as u32));
                off += bytes.len() as u64 + 1;
            }
            w.flush()?;
            Ok(())
        })();
        drop(old);
        if let Err(e) = r {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.context(format!("writing {}", path.display())));
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow!("writing {}: {e}", path.display()));
        }
        for (d, l) in self.docs.iter_mut().zip(locs) {
            d.loc = Some(l);
            d.text = None;
        }
        self.file = Some(path.to_path_buf());
        if self.write_words(path).is_err() {
            let _ = std::fs::remove_file(words_path(path)); // a cache: the next opening rebuilds it
        }
        Ok(())
    }

    /// Text of an item's pages: in memory if it is not saved yet, else read from the file.
    fn pages_of(&self, d: &Doc) -> Result<Vec<String>> {
        if let Some(t) = &d.text {
            return Ok(t.clone());
        }
        let (Some(l), Some(f)) = (d.loc, &self.file) else {
            bail!("the text of '{}' is not available", d.id);
        };
        let mut file = std::fs::File::open(f)?;
        let line = read_line_at(&mut file, l)?;
        let j = mcp::parse(&String::from_utf8_lossy(&line)).context("damaged search index line")?;
        if jstr(&j, "id") != d.id {
            bail!("the search index file changed under the program; reopen the collection");
        }
        Ok(jstrs(&j, "pages"))
    }

    /// Add the pages of `id` to the word lists (nothing replaced here).
    fn add_doc(
        &mut self,
        id: &str,
        pages: Vec<String>,
        loc: Option<(u64, u32)>,
        keep: bool,
        cache: &mut KeyCache,
    ) {
        if self.has(id) {
            self.remove(id);
        }
        let base = self.next_base;
        let n = pages.len() as u32;
        for (p, text) in pages.iter().enumerate() {
            let gid = base + p as u32;
            let mut keys: Vec<&str> = tokens_with(text, cache).into_iter().map(|t| t.0).collect();
            keys.sort_unstable();
            keys.dedup();
            for k in keys {
                match self.inv.get_mut(k) {
                    Some(p) => p.push(gid),
                    None => {
                        let mut p = Post::default();
                        p.push(gid);
                        self.inv.insert(k.to_string(), p);
                    }
                }
            }
        }
        self.next_base = base + n.max(1);
        self.docs.push(Doc {
            id: id.to_string(),
            base,
            pages: n,
            loc,
            text: (keep || loc.is_none()).then_some(pages),
        });
    }

    /// Word lists made again from the text (drops what replaced or removed items left behind).
    fn rebuild_words(&mut self) -> Result<()> {
        let docs = std::mem::take(&mut self.docs);
        let mut fresh = Index {
            file: self.file.clone(),
            ..Default::default()
        };
        let mut cache = KeyCache::new();
        for d in docs {
            let pages = self.pages_of(&d)?;
            fresh.add_doc(&d.id, pages, d.loc, d.text.is_some(), &mut cache);
        }
        *self = fresh;
        Ok(())
    }

    /// Replace (or add) the text of an item, one string per page.
    pub fn set(&mut self, id: &str, pages: Vec<String>) {
        self.add_doc(id, pages, None, true, &mut KeyCache::new());
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(i) = self.docs.iter().position(|d| d.id == id) {
            self.dead += self.docs[i].pages as u64;
            self.docs.remove(i);
        }
    }

    pub fn has(&self, id: &str) -> bool {
        self.docs.iter().any(|d| d.id == id)
    }

    pub fn page_count(&self) -> usize {
        self.docs.iter().map(|d| d.pages as usize).sum()
    }

    fn doc_of(&self, gid: u32) -> Option<usize> {
        let i = self
            .docs
            .partition_point(|d| d.base <= gid)
            .checked_sub(1)?;
        (gid < self.docs[i].base + self.docs[i].pages).then_some(i)
    }

    /// Pages containing the words of `query` (ignoring case, diacritics and old spelling).
    /// Pages with the words next to each other, in that order, come first; then (marked
    /// `loose`) pages that have all the words somewhere. At most `max` hits (0 = no limit),
    /// in the order the items were indexed.
    pub fn search(&self, query: &str, max: usize) -> Vec<Hit> {
        let q: Vec<String> = tokens(query).into_iter().map(|t| t.0).collect();
        if q.is_empty() {
            return Vec::new();
        }
        let mut lists: Vec<&Post> = Vec::new();
        for k in &q {
            let Some(p) = self.inv.get(k) else {
                return Vec::new();
            };
            lists.push(p);
        }
        lists.sort_by_key(|p| p.bytes.len()); // the rarest word first
        let mut cand = lists[0].decode();
        for p in &lists[1..] {
            if cand.is_empty() {
                break;
            }
            cand = intersect(&cand, &p.decode());
        }
        let (mut exact, mut loose) = (Vec::new(), Vec::new());
        let mut cache: Option<(usize, Vec<String>)> = None;
        let mut keys = KeyCache::new();
        for gid in cand {
            let Some(di) = self.doc_of(gid) else {
                continue; // an item that was replaced or removed
            };
            let d = &self.docs[di];
            if cache.as_ref().is_none_or(|c| c.0 != di) {
                cache = Some((di, self.pages_of(d).unwrap_or_default()));
            }
            let p = (gid - d.base) as usize;
            let Some(text) = cache.as_ref().and_then(|c| c.1.get(p)) else {
                continue;
            };
            let toks = tokens_with(text, &mut keys);
            let found = (0..toks.len()).find(|&i| {
                i + q.len() <= toks.len() && q.iter().enumerate().all(|(n, k)| toks[i + n].0 == k)
            });
            let (at, is_loose) = match found {
                Some(i) => (toks[i].1, false),
                None => match toks.iter().find(|t| t.0 == q[0]) {
                    Some(t) => (t.1, true),
                    None => continue,
                },
            };
            let hit = Hit {
                item_id: d.id.clone(),
                page: p + 1,
                snippet: snippet(text, at),
                loose: is_loose,
            };
            if is_loose {
                loose.push(hit)
            } else {
                exact.push(hit);
                if max > 0 && exact.len() >= max {
                    break; // the rest would come after these
                }
            }
        }
        exact.extend(loose);
        if max > 0 {
            exact.truncate(max);
        }
        exact
    }
}

fn read_line_at(f: &mut std::fs::File, (off, len): (u64, u32)) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(off))?;
    let mut b = vec![0u8; len as usize];
    f.read_exact(&mut b)
        .context("the search index file is shorter than expected")?;
    Ok(b)
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

// ---------------------------------------------------------------- editing an item (CLI)

/// Fields to change on an item; `None` leaves the field as it is. Text values are checked
/// before anything is changed, so a bad value changes nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ItemEdit {
    pub title: Option<String>,
    pub issue: Option<String>,
    pub date: Option<String>,
    /// `newspaper`, `magazine`, `book` or `other`.
    pub kind: Option<String>,
    /// Profile name; an empty text clears it (the collection's profile is then used).
    pub profile: Option<String>,
    /// `todo`, `processed`, `ocr_done` or `checked`.
    pub status: Option<String>,
    /// Comma-separated; replaces the item's tags.
    pub tags: Option<String>,
    pub notes: Option<String>,
}

impl ItemEdit {
    pub fn is_empty(&self) -> bool {
        *self == ItemEdit::default()
    }
}

/// Split `a, b,,c` into `["a", "b", "c"]` (no empty entries, no repeats).
pub fn split_tags(s: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !v.iter().any(|x| x == t) {
            v.push(t.to_string());
        }
    }
    v
}

impl Item {
    pub fn apply(&mut self, e: &ItemEdit) -> Result<()> {
        let kind = e
            .kind
            .as_deref()
            .map(|k| {
                Kind::from_code(k).with_context(|| {
                    format!("unknown kind '{k}' (newspaper, magazine, book, other)")
                })
            })
            .transpose()?;
        let status = e
            .status
            .as_deref()
            .map(|k| {
                Status::from_code(k).with_context(|| {
                    format!("unknown status '{k}' (todo, processed, ocr_done, checked)")
                })
            })
            .transpose()?;
        if let Some(t) = &e.title {
            self.title = t.trim().to_string();
        }
        if let Some(t) = &e.issue {
            self.issue = t.trim().to_string();
        }
        if let Some(t) = &e.date {
            self.date = t.trim().to_string();
        }
        if let Some(k) = kind {
            self.kind = k;
        }
        if let Some(p) = &e.profile {
            self.profile = p.trim().to_string();
        }
        if let Some(s) = status {
            self.status = s;
        }
        if let Some(t) = &e.tags {
            self.tags = split_tags(t);
        }
        if let Some(n) = &e.notes {
            self.notes = n.clone();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Tesseract of the window

/// The Tesseract program the main window's settings name, for the batch: `None` when the
/// setting is empty or the bare default `tesseract` (then the item's profile, else the PATH,
/// decides, as on the command line).
pub fn configured_tesseract(setting: &str) -> Option<PathBuf> {
    let t = setting.trim();
    (!t.is_empty() && !t.eq_ignore_ascii_case("tesseract")).then(|| PathBuf::from(t))
}

// ---------------------------------------------------------------- indexing a PDF's text layer

impl Item {
    /// The PDF the index describes: the one made by `collection run`, else the item's source
    /// when that is one single PDF file. Page numbers of a search hit refer to this file.
    pub fn text_pdf(&self, base: &Path) -> Option<PathBuf> {
        if !self.output.is_empty() {
            let p = from_stored(base, &self.output);
            if p.is_file() {
                return Some(p);
            }
        }
        if let [one] = self.sources.as_slice() {
            let p = from_stored(base, one);
            if p.is_file() && ext_of(&p) == "pdf" {
                return Some(p);
            }
        }
        None
    }
}

/// The text of each page of a PDF, from its text layer (no recognition is done). A scan
/// without text gives empty strings.
pub fn pdf_page_texts(path: &Path) -> Result<Vec<String>> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("{}: cannot open", path.display()))?
        .len();
    if len > 2 << 30 {
        bail!("{}: too large to read ({len} bytes)", path.display());
    }
    let data = std::fs::read(path).with_context(|| format!("{}: cannot read", path.display()))?;
    let doc = legatorocr_pdf::reader::PdfDoc::open_with_password(data, &crate::pdf_password())
        .map_err(|e| anyhow!("{}: not a readable PDF ({e})", path.display()))?;
    Ok((0..doc.page_count())
        .map(|i| {
            doc.page_text(i)
                .map(|t| t.trim().to_string())
                .unwrap_or_default()
        })
        .collect())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReindexReport {
    /// Items whose text layer was read and indexed.
    pub indexed: Vec<String>,
    /// Items whose PDF has no text (a scan that was not recognised): not indexed, and an old
    /// entry for them is dropped.
    pub no_text: Vec<String>,
    /// (item id, message)
    pub failed: Vec<(String, String)>,
    pub pages: usize,
    pub cancelled: bool,
}

/// Read the text layer of the PDF of each item in `ids` ([`Item::text_pdf`]) and put it in the
/// index, **without processing anything again**. It serves items whose source is a PDF that
/// already has text, and items processed earlier (e.g. by a version that did not index). The
/// item's status, PDF and sources are not touched (only a missing page count is filled in).
/// The index is saved at the end (also after a cancel).
pub fn reindex(
    col_file: &Path,
    col: &mut Collection,
    index: &mut Index,
    ids: &[String],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<ReindexReport> {
    let base = col_file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let total = ids.len();
    let mut rep = ReindexReport::default();
    let mut col_changed = false;
    for (n, id) in ids.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            rep.cancelled = true;
            break;
        }
        let Some(it) = col.item(id) else {
            rep.failed.push((id.clone(), "no such item".into()));
            continue;
        };
        progress(n, total, &it.title);
        let Some(pdf) = it.text_pdf(&base) else {
            rep.failed.push((
                id.clone(),
                "no PDF to read: process the item first, or give it one PDF as its source".into(),
            ));
            continue;
        };
        let r = std::panic::catch_unwind(|| pdf_page_texts(&pdf))
            .unwrap_or_else(|p| Err(anyhow!("internal error: {}", panic_text(&*p))));
        match r {
            Ok(pages) if pages.iter().all(|t| t.is_empty()) => {
                index.remove(id);
                rep.no_text.push(id.clone());
            }
            Ok(pages) => {
                rep.pages += pages.len();
                let count = pages.len().min(u32::MAX as usize) as u32;
                index.set(id, pages);
                if let Some(x) = col.item_mut(id) {
                    if x.pages.is_none() {
                        x.pages = Some(count);
                        col_changed = true;
                    }
                }
                rep.indexed.push(id.clone());
            }
            Err(e) => rep.failed.push((id.clone(), format!("{e:#}"))),
        }
    }
    index.save(&index_path(col_file))?;
    if col_changed {
        col.save(col_file)?;
    }
    if !rep.cancelled {
        progress(total, total, "");
    }
    Ok(rep)
}

// ---------------------------------------------------------------- opening a PDF at a page

/// `file:///…` address of a local file, with the characters that need it written as `%XX`.
pub fn file_url(p: &Path) -> String {
    let t = p.to_string_lossy().replace('\\', "/");
    let mut o = String::from("file://");
    if !t.starts_with('/') {
        o.push('/');
    }
    for (i, b) in t.bytes().enumerate() {
        let drive_colon = b == b':' && i == 1;
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) || drive_colon {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// A program and its arguments that open `pdf` at `page` (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerCall {
    pub program: String,
    pub args: Vec<String>,
}

fn browser_call(program: &str, pdf: &Path, page: usize) -> ViewerCall {
    ViewerCall {
        program: program.into(),
        args: vec![format!("{}#page={page}", file_url(pdf))],
    }
}

/// For the default PDF viewer of a Linux desktop (`xdg-mime query default application/pdf`,
/// e.g. `org.kde.okular.desktop`): how to open it at a page, only for viewers whose
/// command-line option for that is known. `None` for any other (open it plainly).
pub fn linux_viewer_call(desktop_id: &str, pdf: &Path, page: usize) -> Option<ViewerCall> {
    let id = desktop_id.trim().to_ascii_lowercase();
    let file = pdf.to_string_lossy().into_owned();
    let call = |p: &str, a: Vec<String>| {
        Some(ViewerCall {
            program: p.into(),
            args: a,
        })
    };
    if id.contains("okular") {
        call("okular", vec!["--page".into(), page.to_string(), file])
    } else if id.contains("evince") {
        call("evince", vec![format!("--page-index={page}"), file])
    } else if id.contains("zathura") {
        call("zathura", vec!["--page".into(), page.to_string(), file])
    } else if id.contains("mupdf") {
        call("mupdf", vec![file, page.to_string()])
    } else if id.contains("firefox") {
        Some(browser_call("firefox", pdf, page))
    } else if id.contains("google-chrome") {
        Some(browser_call("google-chrome", pdf, page))
    } else if id.contains("chromium") {
        Some(browser_call("chromium", pdf, page))
    } else if id.contains("microsoft-edge") {
        Some(browser_call("microsoft-edge", pdf, page))
    } else {
        None
    }
}

/// The program (full path) at the start of a Windows "open" command such as
/// `"C:\Program Files\SumatraPDF\SumatraPDF.exe" "%1"`.
fn windows_program(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    if let Some(r) = c.strip_prefix('"') {
        return r.split('"').next().map(str::to_string);
    }
    let low = c.to_ascii_lowercase();
    low.find(".exe").map(|i| c[..i + 4].to_string())
}

/// For the "open" command of the default PDF program of Windows (from the registry): how to
/// open it at a page, for SumatraPDF, Adobe Acrobat/Reader, Chrome, Edge and Firefox.
pub fn windows_viewer_call(command: &str, pdf: &Path, page: usize) -> Option<ViewerCall> {
    let program = windows_program(command)?;
    let exe = program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let file = pdf.to_string_lossy().into_owned();
    let args = if exe.contains("sumatrapdf") {
        vec!["-page".into(), page.to_string(), file]
    } else if exe.contains("acrord") || exe.contains("acrobat") {
        vec!["/A".into(), format!("page={page}"), file]
    } else if ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"].contains(&exe.as_str()) {
        vec![format!("{}#page={page}", file_url(pdf))]
    } else {
        return None;
    };
    Some(ViewerCall { program, args })
}

fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let o = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

#[cfg(windows)]
fn reg_value(key: &str, name: Option<&str>) -> Option<String> {
    let out = match name {
        Some(n) => run_capture("reg", &["query", key, "/v", n])?,
        None => run_capture("reg", &["query", key, "/ve"])?,
    };
    out.lines()
        .find_map(|l| {
            l.split_once("REG_SZ")
                .or_else(|| l.split_once("REG_EXPAND_SZ"))
        })
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// How to open `pdf` at `page` with the computer's default PDF program, when that program is
/// one this code knows to honour a page number (see [`linux_viewer_call`],
/// [`windows_viewer_call`]); `None` on macOS and for any other program. Looking the default up
/// can fail; then it is `None` too and the caller opens the file plainly.
pub fn default_viewer_call(pdf: &Path, page: usize) -> Option<ViewerCall> {
    #[cfg(windows)]
    {
        let prog = reg_value(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.pdf\\UserChoice",
            Some("ProgId"),
        )
        .or_else(|| reg_value("HKCR\\.pdf", None))?;
        let cmd = reg_value(&format!("HKCR\\{prog}\\shell\\open\\command"), None)?;
        return windows_viewer_call(&cmd, pdf, page);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let id = run_capture("xdg-mime", &["query", "default", "application/pdf"])?;
        return linux_viewer_call(&id, pdf, page);
    }
    #[allow(unreachable_code)]
    {
        let _ = (pdf, page);
        None
    }
}

/// Open `pdf` at `page` when the default viewer is known to honour that. Returns whether it
/// was started; on `false` nothing was opened and the caller opens the file plainly (and
/// shows the page number to the person).
pub fn open_pdf_at_page(pdf: &Path, page: usize) -> bool {
    let Some(c) = default_viewer_call(pdf, page) else {
        return false;
    };
    std::process::Command::new(&c.program)
        .args(&c.args)
        .spawn()
        .is_ok()
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
        // (a path is absolute in the sense of the platform: on Windows "/abs/x.pdf" has no drive)
        let abs = std::env::current_dir().unwrap().join("abs").join("x.pdf");
        assert!(abs.is_absolute());
        assert_eq!(from_stored(&base, &abs.to_string_lossy()), abs);
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

    #[test]
    fn name_guess_year_month_number() {
        let g = |n: &str| {
            let g = guess_from_name(n);
            (g.kind, g.title, g.date, g.issue)
        };
        let n = |t: &str, d: &str, i: &str| {
            (Kind::Newspaper, t.to_string(), d.to_string(), i.to_string())
        };
        // the issue after the month: three or more digits are never a day
        assert_eq!(
            g("scinteia_tineretului_1977_09_8801"),
            n("Scinteia Tineretului", "1977-09", "8801")
        );
        assert_eq!(
            g("scinteia_tineretului_1977_11_8862"),
            n("Scinteia Tineretului", "1977-11", "8862")
        );
        // with a day: the day, then the issue
        assert_eq!(
            g("scinteia_tineretului_1977_11_15_8862"),
            n("Scinteia Tineretului", "1977-11-15", "8862")
        );
        assert_eq!(
            g("scinteia_tineretului_1977-11-15_8862"),
            n("Scinteia Tineretului", "1977-11-15", "8862")
        );
        assert_eq!(
            g("scinteia_tineretului_1977-11-15"),
            n("Scinteia Tineretului", "1977-11-15", "")
        );
        assert_eq!(
            g("scinteia_tineretului_19771115_8862"),
            n("Scinteia Tineretului", "1977-11-15", "8862")
        );
        // a number that is not a day of that month is the issue
        assert_eq!(g("scinteia_1977_09_31").2, "1977-09");
        assert_eq!(g("scinteia_1977_09_31").3, "31");
        assert_eq!(g("scinteia_1977_02_30").3, "30");
        assert_eq!(g("scinteia_1976_02_29").2, "1976-02-29"); // a leap year
        assert_eq!(g("scinteia_1977_02_29").3, "29"); // not one
                                                      // a last two-digit number in a magazine or a book is the issue, in a daily the day
        let m = guess_from_name("revista_sinteza_1985_03_12");
        assert_eq!(
            (m.kind, m.date.as_str(), m.issue.as_str()),
            (Kind::Magazine, "1985-03", "12")
        );
        let m = guess_from_name("gazeta_1914_05_12");
        assert_eq!(
            (m.kind, m.date.as_str(), m.issue.as_str()),
            (Kind::Newspaper, "1914-05-12", "")
        );
        // one digit, or a marker, is the issue
        assert_eq!(guess_from_name("scinteia_1977_09_8").issue, "8");
        let m = guess_from_name("scinteia_1977_09_nr_12");
        assert_eq!((m.date.as_str(), m.issue.as_str()), ("1977-09", "12"));
        // even after a month written as a number the title stays free of digits
        assert_eq!(
            guess_from_name("scinteia_tineretului_1977_09_8801").title,
            "Scinteia Tineretului"
        );
    }

    #[test]
    fn item_edit_changes_only_what_is_given_and_checks_first() {
        let mut it = Item {
            id: "i1".into(),
            title: "A".into(),
            tags: vec!["x".into()],
            ..Default::default()
        };
        it.apply(&ItemEdit {
            title: Some("  Scânteia ".into()),
            issue: Some("8801".into()),
            date: Some("1977-09".into()),
            kind: Some("newspaper".into()),
            profile: Some("flux-1-optim".into()),
            status: Some("checked".into()),
            tags: Some("a, b,,a , c".into()),
            notes: Some("o notă".into()),
        })
        .unwrap();
        assert_eq!(it.title, "Scânteia");
        assert_eq!((it.issue.as_str(), it.date.as_str()), ("8801", "1977-09"));
        assert_eq!((it.kind, it.status), (Kind::Newspaper, Status::Checked));
        assert_eq!(it.tags, vec!["a", "b", "c"]);
        assert_eq!(
            (it.profile.as_str(), it.notes.as_str()),
            ("flux-1-optim", "o notă")
        );
        // only the title: the rest stays; an empty text clears
        it.apply(&ItemEdit {
            profile: Some(String::new()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(it.profile, "");
        assert_eq!(it.title, "Scânteia");
        // a bad value changes nothing at all
        let before = it.clone();
        let bad = ItemEdit {
            title: Some("nou".into()),
            status: Some("gata".into()),
            ..Default::default()
        };
        assert!(it
            .apply(&bad)
            .unwrap_err()
            .to_string()
            .contains("unknown status"));
        assert_eq!(it, before);
        let bad = ItemEdit {
            title: Some("nou".into()),
            kind: Some("ziar".into()),
            ..Default::default()
        };
        assert!(it.apply(&bad).is_err());
        assert_eq!(it, before);
        assert!(ItemEdit::default().is_empty());
    }

    #[test]
    fn tesseract_of_the_window_is_used_unless_it_is_the_default() {
        assert_eq!(configured_tesseract("tesseract"), None);
        assert_eq!(configured_tesseract("  Tesseract "), None);
        assert_eq!(configured_tesseract(""), None);
        let p = std::env::temp_dir().join("tess").join("tesseract.exe");
        assert_eq!(configured_tesseract(&p.to_string_lossy()), Some(p.clone()));
        // it reaches the item that is run
        let d = tmp("tessrun");
        let f = d.join("c.lgrcol");
        std::fs::write(d.join("a.png"), b"x").unwrap();
        let mut c = Collection::new("t");
        let id = c.push(Item {
            title: "A".into(),
            sources: vec!["a.png".into()],
            ..Default::default()
        });
        let seen = std::sync::Mutex::new(None);
        let runner = |run: &ItemRun, _: &AtomicBool| -> Result<ItemOutput> {
            *seen.lock().unwrap() = run.tesseract.clone();
            Ok(ItemOutput {
                pdf: b"%PDF".to_vec(),
                pages: 1,
                texts: vec!["x".into()],
                failed: vec![],
            })
        };
        let mut ix = Index::default();
        let cfg = RunConfig {
            tesseract: configured_tesseract(&p.to_string_lossy()),
            ..Default::default()
        };
        run_batch(
            &f,
            &mut c,
            &mut ix,
            &[id],
            &cfg,
            &AtomicBool::new(false),
            &runner,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(p));
    }

    fn words_of(ix: &Index, q: &str) -> Vec<(String, usize, bool)> {
        ix.search(q, 0)
            .into_iter()
            .map(|h| (h.item_id, h.page, h.loose))
            .collect()
    }

    #[test]
    fn index_file_round_trip_with_and_without_the_cache() {
        let d = tmp("ixrt");
        let f = d.join("c.lgrindex");
        let mut ix = Index::default();
        ix.set(
            "i1",
            vec![
                "nimic aici".into(),
                "Ziarul Scânteia anunță \"ceva\"\nnou".into(),
            ],
        );
        ix.set(
            "i2",
            vec![
                "Scînteia tineretului".into(),
                String::new(),
                "tineretului Scânteia".into(),
            ],
        );
        ix.save(&f).unwrap();
        assert!(words_path(&f).is_file());
        let head = std::fs::read_to_string(&f).unwrap();
        assert!(
            head.starts_with(INDEX_HEAD) && head.lines().count() == 3,
            "{head}"
        );
        let want = vec![
            ("i1".to_string(), 2, false),
            ("i2".to_string(), 1, false),
            ("i2".to_string(), 3, false),
        ];
        assert_eq!(words_of(&ix, "Scînteia"), want);
        // from the cache
        let a = Index::load(&f).unwrap();
        assert_eq!(a.page_count(), 5);
        assert_eq!(words_of(&a, "Scînteia"), want);
        assert_eq!(
            a.search("scanteia anunta", 0)[0].snippet,
            "Ziarul Scânteia anunță \"ceva\" nou"
        );
        // exact phrase first, then the words apart
        let ph = words_of(&a, "scanteia tineretului");
        assert_eq!(ph, vec![("i2".into(), 1, false), ("i2".into(), 3, true)]);
        // without the cache: the same
        std::fs::remove_file(words_path(&f)).unwrap();
        let b = Index::load(&f).unwrap();
        assert_eq!(words_of(&b, "Scînteia"), want);
        assert_eq!(b.page_count(), 5);
        // a cache that does not fit the file is ignored, not trusted
        let mut c = Index::load(&f).unwrap();
        c.save(&f).unwrap();
        let mut text = std::fs::read_to_string(&f).unwrap();
        text = text.replace("nimic aici", "totul aici");
        std::fs::write(&f, text).unwrap(); // same length, other text: the check differs only if the edge changed
        let c = Index::load(&f).unwrap();
        assert_eq!(c.page_count(), 5);
        assert_eq!(
            words_of(&c, "totul"),
            vec![("i1".to_string(), 1, false)],
            "the cache was not trusted"
        );
        assert!(words_of(&c, "nimic").is_empty());
        // maximum number of hits and the limit
        assert_eq!(a.search("Scînteia", 2).len(), 2);
        assert!(a.search("inexistent", 0).is_empty());
        assert!(a.search("  ", 0).is_empty());
    }

    #[test]
    fn index_replace_remove_and_append_keep_the_file_consistent() {
        let d = tmp("ixmod");
        let f = d.join("c.lgrindex");
        let mut ix = Index::default();
        for i in 0..6 {
            ix.set(
                &format!("i{i}"),
                vec![format!("cuvânt{i} comun"), "comun".into()],
            );
        }
        ix.save(&f).unwrap();
        let mut ix = Index::load(&f).unwrap();
        ix.set("i1", vec!["nou text".into()]); // replaced
        ix.remove("i2");
        ix.remove("nu-exista");
        assert!(ix.has("i1") && !ix.has("i2"));
        assert_eq!(ix.page_count(), 4 * 2 + 1);
        assert!(words_of(&ix, "cuvant1").is_empty(), "old text of i1 gone");
        assert!(words_of(&ix, "cuvant2").is_empty());
        assert_eq!(words_of(&ix, "nou"), vec![("i1".to_string(), 1, false)]);
        // saved, the unchanged items are copied from the old file
        ix.save(&f).unwrap();
        for ix in [Index::load(&f).unwrap(), {
            std::fs::remove_file(words_path(&f)).unwrap();
            Index::load(&f).unwrap()
        }] {
            assert_eq!(ix.page_count(), 9);
            assert_eq!(words_of(&ix, "cuvant3"), vec![("i3".to_string(), 1, false)]);
            assert_eq!(words_of(&ix, "nou"), vec![("i1".to_string(), 1, false)]);
            assert_eq!(words_of(&ix, "comun").len(), 4 * 2);
        }
        // a lot of replacing builds the word lists again (the dead pages do not pile up)
        let mut ix = Index::load(&f).unwrap();
        for _ in 0..30 {
            ix.set("i0", vec!["a".into(), "b".into()]);
        }
        ix.save(&f).unwrap();
        let ix = Index::load(&f).unwrap();
        assert_eq!(ix.page_count(), 9);
        assert_eq!(ix.dead, 0);
        assert_eq!(words_of(&ix, "a"), vec![("i0".to_string(), 1, false)]);
    }

    #[test]
    fn index_files_of_the_first_format_and_damaged_ones() {
        let d = tmp("ixold");
        let f = d.join("c.lgrindex");
        std::fs::write(
            &f,
            "{\"format\":\"lgrindex\",\"version\":1,\"items\":[\n{\"id\":\"i1\",\"pages\":[\"nimic\",\"Ziarul Scânteia\"]}\n]}\n",
        )
        .unwrap();
        let mut ix = Index::load(&f).unwrap();
        assert_eq!(
            words_of(&ix, "Scînteia"),
            vec![("i1".to_string(), 2, false)]
        );
        ix.set("i2", vec!["alt text".into()]);
        ix.save(&f).unwrap(); // now in the current format
        assert!(std::fs::read_to_string(&f).unwrap().starts_with(INDEX_HEAD));
        let ix = Index::load(&f).unwrap();
        assert_eq!(ix.page_count(), 3);
        assert_eq!(
            words_of(&ix, "scanteia"),
            vec![("i1".to_string(), 2, false)]
        );
        // a line in the middle that is not JSON: an error naming the file
        std::fs::remove_file(words_path(&f)).unwrap();
        let mut t = std::fs::read_to_string(&f).unwrap();
        t = t.replacen("{\"id\":\"i1\"", "{\"id\":", 1);
        std::fs::write(&f, t).unwrap();
        let e = format!("{:#}", Index::load(&f).unwrap_err());
        assert!(e.contains("damaged") && e.contains("c.lgrindex"), "{e}");
        // a last line cut short (a crash while writing) is ignored
        let good = format!(
            "{INDEX_HEAD}\n{{\"id\":\"i1\",\"pages\":[\"unu doi\"]}}\n{{\"id\":\"i2\",\"pa"
        );
        std::fs::write(&f, good).unwrap();
        let ix = Index::load(&f).unwrap();
        assert!(ix.has("i1") && !ix.has("i2"));
        // not an index at all
        std::fs::write(&f, "hello").unwrap();
        assert!(Index::load(&f).is_err());
        // a missing file is an empty index
        assert_eq!(Index::load(&d.join("nu.lgrindex")).unwrap().page_count(), 0);
    }

    #[test]
    fn varint_lists_round_trip() {
        let mut p = Post::default();
        let ids = [0u32, 1, 5, 130, 131, 70_000, 70_001, 4_000_000_000];
        for g in ids {
            p.push(g);
        }
        assert_eq!(p.decode(), ids.to_vec());
        assert_eq!(intersect(&[1, 3, 5, 9], &[2, 3, 9, 10]), vec![3, 9]);
        assert!(Post::default().decode().is_empty());
    }

    fn text_pdf_bytes(pages: &[&[&str]]) -> Vec<u8> {
        use legatorocr_core::{text::Word, Page, PageImage};
        use legatorocr_pdf::{Compress, PdfWriter};
        let pg = || Page {
            image: PageImage::Rgb(image::RgbImage::new(200, 100)),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        };
        let mut w = PdfWriter::new(Compress::Flate);
        for words in pages {
            let ws: Vec<Word> = words
                .iter()
                .enumerate()
                .map(|(i, t)| Word {
                    text: (*t).into(),
                    x: 10,
                    y: 5 + 30 * i as u32,
                    w: 80,
                    h: 18,
                    conf: 90.0,
                    line: i as u32,
                })
                .collect();
            w.add_page(&pg(), &ws);
        }
        w.finish()
    }

    #[test]
    fn pdf_with_a_text_layer_is_indexed_without_processing() {
        let d = tmp("reidx");
        let f = d.join("c.lgrcol");
        std::fs::write(
            d.join("carte_veche_1938.pdf"),
            text_pdf_bytes(&[&["Argeș"], &["Ornamente", "populare"], &["Scânteia"]]),
        )
        .unwrap();
        std::fs::write(d.join("scan_1950.pdf"), text_pdf_bytes(&[&[], &[]])).unwrap();
        std::fs::write(d.join("rupt.pdf"), b"not a pdf").unwrap();
        let mut c = Collection::new("t");
        let rep = c.add_folder(&f, &d).unwrap();
        assert_eq!(rep.added.len(), 3, "{rep:?}");
        c.save(&f).unwrap();
        let id_of = |c: &Collection, t: &str| {
            c.items
                .iter()
                .find(|i| i.title.contains(t))
                .unwrap()
                .id
                .clone()
        };
        let (carte, scan, rupt) = (id_of(&c, "Carte"), id_of(&c, "Scan"), id_of(&c, "Rupt"));
        let mut ix = Index::default();
        let ids: Vec<String> = c.items.iter().map(|i| i.id.clone()).collect();
        let mut seen = 0;
        let r = reindex(
            &f,
            &mut c,
            &mut ix,
            &ids,
            &AtomicBool::new(false),
            &mut |_, _, _| seen += 1,
        )
        .unwrap();
        assert_eq!(r.indexed, vec![carte.clone()]);
        assert_eq!(r.no_text, vec![scan.clone()]);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(r.failed[0].0, rupt);
        assert!(
            r.failed[0].1.contains("not a readable PDF"),
            "{:?}",
            r.failed
        );
        assert_eq!(r.pages, 3);
        assert!(seen > 0);
        // the status and the sources are untouched; the page count was filled in
        let it = c.item(&carte).unwrap();
        assert_eq!((it.status, it.output.as_str()), (Status::Todo, ""));
        assert_eq!(it.pages, Some(3));
        // searchable, and the file was saved
        let ix2 = Index::load(&index_path(&f)).unwrap();
        let h = ix2.search("ornamente populare", 0);
        assert_eq!(
            (h.len(), h[0].page, h[0].item_id.as_str()),
            (1, 2, carte.as_str())
        );
        assert_eq!(words_of(&ix2, "Scînteia"), vec![(carte.clone(), 3, false)]);
        // the collection file was saved with the page count
        assert_eq!(
            Collection::load(&f).unwrap().item(&carte).unwrap().pages,
            Some(3)
        );
        // an item without a readable PDF is reported, not a crash; an unknown id too
        let mut c2 = Collection::new("t2");
        let img = c2.push(Item {
            title: "Imagini".into(),
            sources: vec!["folder".into()],
            ..Default::default()
        });
        let r = reindex(
            &f,
            &mut c2,
            &mut ix,
            &[img.clone(), "i99".into()],
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(r.failed.len(), 2);
        assert!(r.failed[0].1.contains("no PDF to read"));
        // a text-less scan drops an older entry; cancel stops before the first item
        ix.set(&scan, vec!["vechi".into()]);
        let r = reindex(
            &f,
            &mut c,
            &mut ix,
            std::slice::from_ref(&scan),
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(r.no_text, vec![scan.clone()]);
        assert!(!ix.has(&scan));
        let r = reindex(
            &f,
            &mut c,
            &mut ix,
            &ids,
            &AtomicBool::new(true),
            &mut |_, _, _| {},
        )
        .unwrap();
        assert!(r.cancelled && r.indexed.is_empty());
    }

    #[test]
    fn the_output_pdf_is_preferred_and_a_multi_source_item_has_no_text_pdf() {
        let d = tmp("textpdf");
        std::fs::write(d.join("a.pdf"), b"x").unwrap();
        std::fs::write(d.join("out.pdf"), b"x").unwrap();
        std::fs::write(d.join("b.pdf"), b"x").unwrap();
        let mut it = Item {
            sources: vec!["a.pdf".into()],
            ..Default::default()
        };
        assert_eq!(it.text_pdf(&d), Some(d.join("a.pdf")));
        it.output = "out.pdf".into();
        assert_eq!(it.text_pdf(&d), Some(d.join("out.pdf")));
        it.output = "gone.pdf".into(); // missing: back to the source
        assert_eq!(it.text_pdf(&d), Some(d.join("a.pdf")));
        it.sources.push("b.pdf".into());
        assert_eq!(it.text_pdf(&d), None);
        it.sources = vec!["img.png".into()];
        assert_eq!(it.text_pdf(&d), None);
    }

    #[test]
    fn viewers_known_to_honour_a_page() {
        let pdf = std::env::temp_dir().join("Arhivă și ziare").join("a b.pdf");
        let f = pdf.to_string_lossy().into_owned();
        let call = |id: &str| linux_viewer_call(id, &pdf, 7);
        assert_eq!(
            call("org.kde.okular.desktop").unwrap(),
            ViewerCall {
                program: "okular".into(),
                args: vec!["--page".into(), "7".into(), f.clone()]
            }
        );
        assert_eq!(
            call("org.gnome.Evince.desktop").unwrap().args[0],
            "--page-index=7"
        );
        assert_eq!(
            call("zathura.desktop\n").unwrap().args[..2],
            ["--page", "7"]
        );
        assert_eq!(
            call("mupdf.desktop").unwrap().args,
            vec![f.clone(), "7".to_string()]
        );
        let ff = call("firefox.desktop").unwrap();
        assert_eq!(ff.program, "firefox");
        assert!(
            ff.args[0].starts_with("file://") && ff.args[0].ends_with("#page=7"),
            "{ff:?}"
        );
        assert!(
            !ff.args[0].contains(' ') && !ff.args[0].contains('ă'),
            "{ff:?}"
        );
        // unknown or unsupported: plain open
        for id in [
            "",
            "org.gnome.Papers.desktop",
            "libreoffice-draw.desktop",
            "gimp.desktop",
        ] {
            assert_eq!(call(id), None, "{id}");
        }
        // Windows: the program comes from the registry command line
        let w = |c: &str| windows_viewer_call(c, &pdf, 3);
        let s = w("\"C:\\Program Files\\SumatraPDF\\SumatraPDF.exe\" \"%1\"").unwrap();
        assert_eq!(s.program, "C:\\Program Files\\SumatraPDF\\SumatraPDF.exe");
        assert_eq!(s.args[..2], ["-page", "3"]);
        let a = w("\"C:\\Program Files\\Adobe\\Acrobat DC\\Acrobat\\Acrobat.exe\" \"%1\"").unwrap();
        assert_eq!(a.args[..2], ["/A", "page=3"]);
        let e = w("\"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe\" --single-argument %1").unwrap();
        assert!(
            e.args[0].ends_with("#page=3") && e.args[0].starts_with("file://"),
            "{e:?}"
        );
        assert!(w("C:\\Windows\\System32\\foxit.exe %1").is_none());
        assert!(w("").is_none());
        assert!(w("\"C:\\Windows\\notepad.exe\" %1").is_none());
        assert_eq!(
            windows_program("C:\\Tools\\SumatraPDF.exe -x \"%1\"").as_deref(),
            Some("C:\\Tools\\SumatraPDF.exe")
        );
    }

    #[test]
    fn file_urls_are_escaped_and_have_a_drive_form_on_windows_paths() {
        assert_eq!(
            file_url(Path::new("/a b/ș.pdf")),
            "file:///a%20b/%C8%99.pdf"
        );
        assert_eq!(
            file_url(Path::new("C:\\Arhiva\\x y.pdf")),
            "file:///C:/Arhiva/x%20y.pdf"
        );
        assert_eq!(file_url(Path::new("C:/x.pdf")), "file:///C:/x.pdf");
    }

    fn rss_kb(key: &str) -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with(key))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
            })
            .unwrap_or(0)
    }

    /// Measurement, not a check. Run it twice in separate processes so that the memory of the
    /// build does not count in the second:
    /// `LGR_BENCH_FILE=/tmp/big.lgrindex cargo test --release -p legatorocr-engine bench_index -- --ignored --nocapture`
    /// builds a synthetic index (LGR_BENCH_ITEMS x LGR_BENCH_PAGES, default 5000 x 20, ~350
    /// words per page) when the file does not exist; with the file there it only opens it and
    /// searches (delete `*.lgrwords` to measure the opening without the cache).
    #[test]
    #[ignore]
    fn bench_index_synthetic() {
        let f = PathBuf::from(
            std::env::var("LGR_BENCH_FILE").expect("LGR_BENCH_FILE=/some/dir/big.lgrindex"),
        );
        let vocab: Vec<String> = (0..30000u32)
            .map(|i| {
                let sy = ["ra", "ti", "no", "su", "me", "la", "po", "ci", "da", "ve"];
                let mut w = String::new();
                let mut x = i + 7;
                for _ in 0..(2 + i % 3) {
                    w.push_str(sy[(x % 10) as usize]);
                    x /= 10;
                    x += 3;
                }
                format!("{w}{i}")
            })
            .collect();
        if !f.exists() {
            let n: usize = std::env::var("LGR_BENCH_ITEMS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000);
            let pp: usize = std::env::var("LGR_BENCH_PAGES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20);
            let mut seed = 0x9e3779b97f4a7c15u64;
            let mut rnd = move || {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 11) as f64 / (1u64 << 53) as f64
            };
            let t = std::time::Instant::now();
            let mut ix = Index::default();
            for i in 0..n {
                let pages: Vec<String> = (0..pp)
                    .map(|_| {
                        let mut t = String::new();
                        for _ in 0..350 {
                            let u = rnd();
                            t.push_str(&vocab[((u * u * u) * 30000.0) as usize]);
                            t.push(' ');
                        }
                        t
                    })
                    .collect();
                ix.set(&format!("i{i}"), pages);
            }
            println!(
                "build   : {:?}, RSS {} MB",
                t.elapsed(),
                rss_kb("VmRSS:") >> 10
            );
            let t = std::time::Instant::now();
            ix.save(&f).unwrap();
            println!(
                "save    : {:?}, text file {} MB, cache {} MB",
                t.elapsed(),
                std::fs::metadata(&f).unwrap().len() >> 20,
                std::fs::metadata(words_path(&f)).map_or(0, |m| m.len()) >> 20
            );
            return;
        }
        let before = rss_kb("VmRSS:");
        let t = std::time::Instant::now();
        let ix = Index::load(&f).unwrap();
        println!(
            "load    : {:?}, {} pages, cache {}, RSS +{} MB (peak {} MB)",
            t.elapsed(),
            ix.page_count(),
            words_path(&f).exists(),
            rss_kb("VmRSS:").saturating_sub(before) >> 10,
            rss_kb("VmHWM:") >> 10
        );
        for (name, q) in [
            ("1 common word  ", vocab[10].clone()),
            ("1 rare word    ", vocab[29000].clone()),
            ("2 common words ", format!("{} {}", vocab[3], vocab[40])),
        ] {
            let t = std::time::Instant::now();
            let h = ix.search(&q, 300);
            println!("search {name}: {:?}, {} hits", t.elapsed(), h.len());
        }
        println!("RSS after searches: {} MB", rss_kb("VmRSS:") >> 10);
    }
}
