//! BagIt 1.0 packaging (RFC 8493): the files a run wrote are copied into `DIR/data/` with a
//! SHA-256 manifest, so an archive can check later that nothing changed or went missing.
//! `validate` checks a bag against the rules of the RFC and reports findings with a category;
//! `verify` is the short form that returns only the problems as text.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Summary {
    pub files: usize,
    pub bytes: u64,
}

/// Percent-encode the characters a manifest line cannot carry (RFC 8493, 2.1.3): LF, CR, `%`.
fn escape_path(p: &str) -> String {
    p.replace('%', "%25")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn civil(when: u64) -> String {
    let z = (when / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn tag_manifest(dir: &Path, names: &[&str]) -> Result<String> {
    let mut s = String::new();
    for n in names {
        s.push_str(&format!(
            "{}  {n}\n",
            crate::checksums::file_hash(&dir.join(n))?
        ));
    }
    Ok(s)
}

/// Build a bag at `dest` (which must not exist or must be empty) from `files`.
pub fn write(
    dest: &Path,
    files: &[PathBuf],
    when: u64,
    description: Option<&str>,
) -> Result<Summary> {
    if dest.exists()
        && std::fs::read_dir(dest)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        bail!(
            "{} exists and is not empty; a bag is written into a new folder",
            dest.display()
        );
    }
    let data = dest.join("data");
    std::fs::create_dir_all(&data).with_context(|| format!("{}: cannot create", data.display()))?;
    let mut manifest = String::new();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for f in files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{}: unusable file name", f.display()))?;
        if !seen.insert(name.to_string()) {
            bail!("two files are named {name}; a bag keeps one flat data folder");
        }
        let target = data.join(name);
        std::fs::copy(f, &target).with_context(|| format!("copying {}", f.display()))?;
        bytes += std::fs::metadata(&target)?.len();
        manifest.push_str(&format!(
            "{}  data/{}\n",
            crate::checksums::file_hash(&target)?,
            escape_path(name)
        ));
    }
    std::fs::write(
        dest.join("bagit.txt"),
        "BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n",
    )?;
    std::fs::write(dest.join("manifest-sha256.txt"), &manifest)?;
    let mut info = format!(
        "Bagging-Date: {}\nBag-Software-Agent: LegatorOCR {}\nPayload-Oxum: {}.{}\n",
        civil(when),
        env!("CARGO_PKG_VERSION"),
        bytes,
        files.len()
    );
    if let Some(d) = description {
        info.push_str(&format!("External-Description: {}\n", d.replace('\n', " ")));
    }
    std::fs::write(dest.join("bag-info.txt"), info)?;
    let tags = tag_manifest(dest, &["bagit.txt", "bag-info.txt", "manifest-sha256.txt"])?;
    std::fs::write(dest.join("tagmanifest-sha256.txt"), tags)?;
    Ok(Summary {
        files: files.len(),
        bytes,
    })
}

/// What kind of thing a finding is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Structure,
    Missing,
    Unlisted,
    Checksum,
    UnsafePath,
    Malformed,
    Encoding,
    Compat,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Structure => "structure",
            Category::Missing => "missing",
            Category::Unlisted => "unlisted",
            Category::Checksum => "checksum",
            Category::UnsafePath => "unsafe path",
            Category::Malformed => "malformed",
            Category::Encoding => "encoding",
            Category::Compat => "compatibility",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub category: Category,
    pub severity: Severity,
    /// Path relative to the bag (or the file the finding is about).
    pub path: String,
    pub message: String,
}

/// Outcome of [`validate`]. `valid` implies `complete`.
#[derive(Debug)]
pub struct Report {
    pub complete: bool,
    pub valid: bool,
    pub findings: Vec<Finding>,
}

#[derive(Default)]
struct Collector(Vec<Finding>);

impl Collector {
    fn add(&mut self, severity: Severity, category: Category, path: &str, message: String) {
        self.0.push(Finding {
            category,
            severity,
            path: path.to_string(),
            message,
        });
    }
    fn err(&mut self, category: Category, path: &str, message: impl Into<String>) {
        self.add(Severity::Error, category, path, message.into());
    }
    fn warn(&mut self, category: Category, path: &str, message: impl Into<String>) {
        self.add(Severity::Warning, category, path, message.into());
    }
}

/// Split on LF, CR or CRLF; a final terminator does not start another line.
fn split_lines(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\n' || b[i] == b'\r' {
            out.push(&s[start..i]);
            if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < b.len() {
        out.push(&s[start..]);
    }
    out
}

/// Undo the escaping of 2.1.3: only `%0A`, `%0D` and `%25` are defined; any other `%` sequence
/// is refused (the RFC text read for the spec does not say what to do with it).
fn decode_path(raw: &str) -> std::result::Result<String, String> {
    let b = raw.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let seq = raw.get(i + 1..i + 3).unwrap_or("");
        match seq.to_ascii_uppercase().as_str() {
            "0A" => out.push(b'\n'),
            "0D" => out.push(b'\r'),
            "25" => out.push(b'%'),
            _ => {
                return Err(format!(
                    "percent sequence %{seq} is not one of %0A, %0D, %25"
                ))
            }
        }
        i += 3;
    }
    String::from_utf8(out).map_err(|_| "path is not UTF-8".to_string())
}

/// Why a path must not be followed, if it must not (RFC 8493, 5.1).
fn unsafe_reason(p: &str) -> Option<&'static str> {
    if p.is_empty() {
        Some("empty path")
    } else if p.contains('\0') {
        Some("path contains a NUL")
    } else if p.starts_with('/') {
        Some("absolute path")
    } else if p.starts_with('~') {
        Some("home-directory path")
    } else if p.split('/').any(|s| s == "..") {
        Some("path climbs out with ..")
    } else {
        None
    }
}

fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut st: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = st;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = a
                .wrapping_add(f)
                .wrapping_add(k[i])
                .wrapping_add(m[g])
                .rotate_left(S[(i / 16) * 4 + i % 4]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(t);
        }
        st[0] = st[0].wrapping_add(a);
        st[1] = st[1].wrapping_add(b);
        st[2] = st[2].wrapping_add(c);
        st[3] = st[3].wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, w) in st.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Hex digest length for an algorithm name already normalised (lower case, alphanumerics only).
fn digest_len(alg: &str) -> Option<usize> {
    match alg {
        "md5" => Some(32),
        "sha1" => Some(40),
        "sha256" => Some(64),
        "sha512" => Some(128),
        _ => None,
    }
}

fn digest_hex(alg: &str, data: &[u8]) -> Option<String> {
    Some(match alg {
        "md5" => hex(&md5(data)),
        "sha1" => hex(&crate::zipcrypt::sha1(data)),
        "sha256" => crate::checksums::sha256_hex(data),
        "sha512" => hex(&legatorocr_pdf::crypt::sha512(data)),
        _ => return None,
    })
}

fn normalise_alg(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

struct Entry {
    hash: String,
    path: String,
}

struct Manifest {
    name: String,
    alg: String,
    entries: Vec<Entry>,
}

fn parse_manifest(dir: &Path, name: &str, alg: &str, bytes: &[u8], c: &mut Collector) -> Manifest {
    let mut text = match std::str::from_utf8(bytes) {
        Ok(t) => t.to_string(),
        Err(_) => {
            c.err(Category::Encoding, name, "not valid UTF-8");
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    if let Some(rest) = text.strip_prefix('\u{feff}') {
        c.err(Category::Encoding, name, "starts with a byte order mark");
        text = rest.to_string();
    }
    let want = digest_len(alg);
    let mut entries = Vec::new();
    for line in split_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        let Some(cut) = line.find([' ', '\t']) else {
            c.err(
                Category::Malformed,
                name,
                format!("no separator after the checksum: {line}"),
            );
            continue;
        };
        let hash = &line[..cut];
        let rest = line[cut..].trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            c.err(
                Category::Malformed,
                name,
                format!("no path after the checksum: {line}"),
            );
            continue;
        }
        if hash.is_empty()
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || want.is_some_and(|w| hash.len() != w)
        {
            c.err(
                Category::Malformed,
                name,
                format!("not a hexadecimal {alg} checksum: {hash}"),
            );
            continue;
        }
        // Versions before the RFC 8493 escaping wrote a `%` in a file name as it is. The strict
        // reading comes first; when it finds no file but the text taken literally does, the
        // literal reading is used, with a warning (it fails strict checking).
        let literal_ok = unsafe_reason(rest).is_none() && dir.join(rest).exists();
        let mut path = match decode_path(rest) {
            Ok(p) if literal_ok && rest != p && !dir.join(&p).exists() => {
                c.warn(
                    Category::Compat,
                    name,
                    format!("{rest}: '%' not escaped as %25 (older writer); fails strict checking"),
                );
                rest.to_string()
            }
            Ok(p) => p,
            Err(_) if literal_ok => {
                c.warn(
                    Category::Compat,
                    name,
                    format!("{rest}: '%' not escaped as %25 (older writer); fails strict checking"),
                );
                rest.to_string()
            }
            Err(e) => {
                c.err(Category::Malformed, name, format!("{rest}: {e}"));
                continue;
            }
        };
        if let Some(why) = unsafe_reason(&path) {
            c.err(
                Category::UnsafePath,
                name,
                format!("{path}: {why}; not read"),
            );
            continue;
        }
        // Old manifests put `*` in front of the path (binary-mode marker): tolerated, with a warning.
        if let Some(stripped) = path.strip_prefix('*') {
            if !dir.join(&path).exists()
                && unsafe_reason(stripped).is_none()
                && dir.join(stripped).exists()
            {
                c.warn(
                    Category::Compat,
                    name,
                    format!("{stripped}: old-style '*' before the path; fails strict checking"),
                );
                path = stripped.to_string();
            }
        }
        entries.push(Entry {
            hash: hash.to_string(),
            path,
        });
    }
    Manifest {
        name: name.to_string(),
        alg: alg.to_string(),
        entries,
    }
}

fn walk_payload(root: &Path, rel: &str, out: &mut Vec<(String, u64)>) {
    let Ok(rd) = std::fs::read_dir(root.join(rel)) else {
        return;
    };
    let mut names: Vec<_> = rd.flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for e in names {
        let name = e.file_name().to_string_lossy().into_owned();
        let r = format!("{rel}/{name}");
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            walk_payload(root, &r, out);
        } else if let Ok(m) = std::fs::metadata(e.path()) {
            if m.is_file() {
                out.push((r, m.len()));
            }
        }
    }
}

fn check_declaration(dir: &Path, c: &mut Collector) {
    let Ok(bytes) = std::fs::read(dir.join("bagit.txt")) else {
        c.err(
            Category::Structure,
            "bagit.txt",
            "the bag declaration is missing",
        );
        return;
    };
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        c.err(
            Category::Structure,
            "bagit.txt",
            "starts with a byte order mark",
        );
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        c.err(Category::Encoding, "bagit.txt", "not valid UTF-8");
        return;
    };
    let text = text.trim_start_matches('\u{feff}');
    let lines = split_lines(text);
    if lines.len() != 2 {
        c.err(
            Category::Structure,
            "bagit.txt",
            format!("must have exactly two lines, has {}", lines.len()),
        );
    }
    let version = lines
        .first()
        .and_then(|l| l.strip_prefix("BagIt-Version:"))
        .map(str::trim);
    match version {
        Some(v)
            if v.split_once('.').is_some_and(|(a, b)| {
                !a.is_empty()
                    && !b.is_empty()
                    && a.bytes().all(|x| x.is_ascii_digit())
                    && b.bytes().all(|x| x.is_ascii_digit())
            }) =>
        {
            if v != "1.0" {
                c.warn(
                    Category::Compat,
                    "bagit.txt",
                    format!("version {v}; this check knows the rules of 1.0"),
                );
            }
        }
        _ => c.err(
            Category::Structure,
            "bagit.txt",
            "first line must be BagIt-Version: M.N",
        ),
    }
    let enc = lines
        .get(1)
        .and_then(|l| l.strip_prefix("Tag-File-Character-Encoding:"))
        .map(str::trim);
    match enc {
        Some(e) if !e.is_empty() => {
            if !normalise_alg(e).eq("utf8") {
                c.warn(
                    Category::Encoding,
                    "bagit.txt",
                    format!("tag files declared as {e}; they are read as UTF-8"),
                );
            }
        }
        _ => c.err(
            Category::Structure,
            "bagit.txt",
            "second line must be Tag-File-Character-Encoding: NAME",
        ),
    }
}

/// Reads `bag-info.txt`; returns the declared Payload-Oxum, if one parses.
fn check_info(dir: &Path, c: &mut Collector) -> Option<(u64, usize)> {
    let name = "bag-info.txt";
    let bytes = std::fs::read(dir.join(name)).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut labels: Vec<(String, String)> = Vec::new();
    for line in split_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            match labels.last_mut() {
                Some(last) => {
                    last.1.push(' ');
                    last.1.push_str(line.trim());
                }
                None => c.warn(
                    Category::Malformed,
                    name,
                    "continuation line before any label",
                ),
            }
        } else if let Some((l, v)) = line.split_once(':') {
            if l.is_empty() || l != l.trim() {
                c.warn(
                    Category::Malformed,
                    name,
                    format!("label {l:?} is empty or has spaces at its ends"),
                );
            }
            labels.push((l.trim().to_ascii_lowercase(), v.trim().to_string()));
        } else {
            c.warn(
                Category::Malformed,
                name,
                format!("line without a label and colon: {line}"),
            );
        }
    }
    for (l, v) in labels.iter().filter(|(l, _)| l == "bagging-date") {
        let _ = l;
        let b = v.as_bytes();
        let ok = b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && v[..4].bytes().all(|x| x.is_ascii_digit())
            && v[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
            && v[8..].parse::<u32>().is_ok_and(|d| (1..=31).contains(&d));
        if !ok {
            c.warn(
                Category::Malformed,
                name,
                format!("Bagging-Date {v:?} is not YYYY-MM-DD"),
            );
        }
    }
    let oxums: Vec<_> = labels.iter().filter(|(l, _)| l == "payload-oxum").collect();
    if oxums.len() > 1 {
        c.warn(
            Category::Malformed,
            name,
            "Payload-Oxum appears more than once",
        );
    }
    let (_, v) = oxums.first()?;
    let parsed = v.split_once('.').and_then(|(a, b)| {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|x| x.is_ascii_digit());
        (digits(a) && digits(b))
            .then(|| a.parse::<u64>().ok().zip(b.parse::<usize>().ok()))
            .flatten()
    });
    if parsed.is_none() {
        c.warn(
            Category::Malformed,
            name,
            format!("Payload-Oxum {v:?} is not BYTES.FILES"),
        );
    }
    parsed
}

/// Line of `fetch.txt`: URL, length (digits or `-`), path. Returns the payload paths it lists.
fn check_fetch(dir: &Path, c: &mut Collector) -> Vec<String> {
    let name = "fetch.txt";
    let Ok(bytes) = std::fs::read(dir.join(name)) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut paths = Vec::new();
    for line in split_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        let ws = |s: &str| s.trim_start_matches([' ', '\t']).to_string();
        let tok = |s: &str| -> (String, String) {
            let s = ws(s);
            match s.find([' ', '\t']) {
                Some(i) => (s[..i].to_string(), ws(&s[i..])),
                None => (s, String::new()),
            }
        };
        let (url, rest) = tok(line);
        let (len, path) = tok(&rest);
        let scheme_ok = url.split_once(':').is_some_and(|(s, r)| {
            s.chars().next().is_some_and(|f| f.is_ascii_alphabetic())
                && s.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || "+-.".contains(ch))
                && !r.is_empty()
        });
        if !scheme_ok
            || path.is_empty()
            || !(len == "-" || len.bytes().all(|b| b.is_ascii_digit()) && !len.is_empty())
        {
            c.err(
                Category::Malformed,
                name,
                format!("expected URL LENGTH PATH: {line}"),
            );
            continue;
        }
        let path = match decode_path(&path) {
            Ok(p) => p,
            Err(e) => {
                c.err(Category::Malformed, name, format!("{path}: {e}"));
                continue;
            }
        };
        if let Some(why) = unsafe_reason(&path) {
            c.err(Category::UnsafePath, name, format!("{path}: {why}"));
            continue;
        }
        if !path.starts_with("data/") {
            c.err(
                Category::Structure,
                name,
                format!("{path}: fetch.txt must not list tag files"),
            );
            continue;
        }
        paths.push(path);
    }
    paths
}

/// Check the bag at `dir` against the rules of RFC 8493 and report every finding. Nothing is
/// downloaded; files listed in `fetch.txt` count as missing while they are not on disk.
pub fn validate(dir: &Path) -> Result<Report> {
    if !dir.is_dir() {
        bail!("{}: not a directory", dir.display());
    }
    let mut c = Collector::default();
    check_declaration(dir, &mut c);
    if !dir.join("data").is_dir() {
        c.err(Category::Structure, "data", "the data directory is missing");
    }
    let mut top: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("{}: cannot read", dir.display()))?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    top.sort();
    let (mut payload, mut tags) = (Vec::new(), Vec::new());
    for n in &top {
        let (list, rest) = if let Some(r) = n.strip_prefix("tagmanifest-") {
            (&mut tags, r)
        } else if let Some(r) = n.strip_prefix("manifest-") {
            (&mut payload, r)
        } else {
            continue;
        };
        let Some(alg) = rest.strip_suffix(".txt").map(normalise_alg) else {
            continue;
        };
        if alg.is_empty() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(dir.join(n)) {
            list.push(parse_manifest(dir, n, &alg, &bytes, &mut c));
        }
    }
    if payload.is_empty() {
        c.err(
            Category::Structure,
            "manifest-ALGORITHM.txt",
            "no payload manifest",
        );
    }
    // Payload manifests: only data/ files, each exactly once, and every file in data/.
    let mut files = Vec::new();
    walk_payload(dir, "data", &mut files);
    let fetch = check_fetch(dir, &mut c);
    for m in &mut payload {
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::new();
        for e in std::mem::take(&mut m.entries) {
            if !e.path.starts_with("data/") {
                c.err(
                    Category::Structure,
                    &m.name,
                    format!(
                        "{}: a payload manifest may only list files in data/",
                        e.path
                    ),
                );
            } else if !seen.insert(e.path.clone()) {
                c.err(
                    Category::Malformed,
                    &m.name,
                    format!("{}: listed more than once", e.path),
                );
            } else {
                kept.push(e);
            }
        }
        for e in &kept {
            if !dir.join(&e.path).is_file() {
                let why = if fetch.contains(&e.path) {
                    "listed in fetch.txt and not fetched (nothing is downloaded)"
                } else {
                    "listed but not on disk"
                };
                c.err(Category::Missing, &e.path, why);
            }
        }
        for (f, _) in &files {
            if !seen.contains(f) {
                c.err(
                    Category::Unlisted,
                    f,
                    format!("payload file not listed in {}", m.name),
                );
            }
        }
        for f in &fetch {
            if !seen.contains(f) {
                c.err(
                    Category::Structure,
                    "fetch.txt",
                    format!("{f}: not listed in {}", m.name),
                );
            }
        }
        m.entries = kept;
    }
    // Tag manifests.
    let payload_names: Vec<String> = payload.iter().map(|m| m.name.clone()).collect();
    let tag_files: Vec<&String> = top
        .iter()
        .filter(|n| !n.starts_with("tagmanifest-"))
        .collect();
    for m in &mut tags {
        let mut kept = Vec::new();
        for e in std::mem::take(&mut m.entries) {
            if e.path.starts_with("data/") {
                c.err(
                    Category::Structure,
                    &m.name,
                    format!("{}: a tag manifest must not list payload files", e.path),
                );
            } else if e.path.starts_with("tagmanifest-") {
                c.err(
                    Category::Structure,
                    &m.name,
                    format!("{}: a tag manifest must not list tag manifests", e.path),
                );
            } else {
                kept.push(e);
            }
        }
        for e in &kept {
            if !dir.join(&e.path).is_file() {
                c.err(Category::Missing, &e.path, "listed but not on disk");
            }
        }
        for pm in &payload_names {
            if !kept.iter().any(|e| &e.path == pm) {
                c.err(
                    Category::Structure,
                    &m.name,
                    format!("{pm}: payload manifest not listed"),
                );
            }
        }
        for f in &tag_files {
            if !kept.iter().any(|e| &e.path == *f) {
                c.warn(
                    Category::Unlisted,
                    f,
                    format!("tag file not listed in {}", m.name),
                );
            }
        }
        m.entries = kept;
    }
    if !tags.is_empty() {
        let mut a: Vec<&str> = payload.iter().map(|m| m.alg.as_str()).collect();
        let mut b: Vec<&str> = tags.iter().map(|m| m.alg.as_str()).collect();
        a.sort_unstable();
        b.sort_unstable();
        if a != b {
            c.warn(
                Category::Compat,
                "tagmanifest-ALGORITHM.txt",
                "tag manifests should use the same algorithms as the payload manifests",
            );
        }
    }
    // Quick check: Payload-Oxum. A mismatch ends the work before any checksum is computed.
    let mut skip_sums = false;
    if let Some((bytes, count)) = check_info(dir, &mut c) {
        let real: u64 = files.iter().map(|(_, n)| n).sum();
        if real != bytes || files.len() != count {
            c.err(
                Category::Structure,
                "bag-info.txt",
                format!(
                    "Payload-Oxum says {bytes}.{count}, data/ holds {real}.{}; checksums not computed",
                    files.len()
                ),
            );
            skip_sums = true;
        }
    }
    if !skip_sums {
        for m in payload.iter().chain(tags.iter()) {
            if digest_len(&m.alg).is_none() {
                c.err(
                    Category::Checksum,
                    &m.name,
                    format!(
                        "algorithm {} is not supported; nothing in it was verified",
                        m.alg
                    ),
                );
                continue;
            }
            for e in &m.entries {
                let Ok(data) = std::fs::read(dir.join(&e.path)) else {
                    continue;
                };
                let got = digest_hex(&m.alg, &data).unwrap_or_default();
                if !got.eq_ignore_ascii_case(&e.hash) {
                    c.err(
                        Category::Checksum,
                        &e.path,
                        "content changed (checksum differs)",
                    );
                }
            }
        }
    }
    let f = c.0;
    let errors = |only_complete: bool| {
        f.iter().any(|x| {
            x.severity == Severity::Error && !(only_complete && x.category == Category::Checksum)
        })
    };
    Ok(Report {
        complete: !errors(true),
        valid: !errors(false),
        findings: f,
    })
}

/// Problems (errors only) found in the bag at `dir`, as text; empty = complete and valid.
pub fn verify(dir: &Path) -> Result<Vec<String>> {
    Ok(validate(dir)?
        .findings
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .map(|f| format!("{}: {}", f.path, f.message))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lgr_bag_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Minimal valid bag, built by hand: files are (relative name in data/, bytes).
    fn hand_bag(root: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let bag = root.join("bag");
        fs::create_dir_all(bag.join("data")).unwrap();
        let mut m = String::new();
        for (n, b) in files {
            fs::write(bag.join("data").join(n), b).unwrap();
            m.push_str(&format!(
                "{}  data/{}\n",
                crate::checksums::sha256_hex(b),
                escape_path(n)
            ));
        }
        fs::write(
            bag.join("bagit.txt"),
            "BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n",
        )
        .unwrap();
        fs::write(bag.join("manifest-sha256.txt"), m).unwrap();
        bag
    }

    fn cats(r: &Report) -> Vec<Category> {
        r.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .map(|f| f.category)
            .collect()
    }

    #[test]
    fn minimal_bag_is_complete_and_valid() {
        let root = scratch("min");
        let bag = hand_bag(&root, &[("a.txt", b"hello")]);
        let r = validate(&bag).unwrap();
        assert!(r.complete && r.valid, "{:?}", r.findings);
        assert!(r.findings.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unlisted_payload_file_makes_the_bag_incomplete() {
        let root = scratch("unl");
        let bag = hand_bag(&root, &[("a.txt", b"hello")]);
        fs::write(bag.join("data/b.txt"), "x").unwrap();
        let r = validate(&bag).unwrap();
        assert!(!r.complete && !r.valid);
        assert_eq!(cats(&r), vec![Category::Unlisted]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn one_changed_byte_is_complete_but_not_valid() {
        let root = scratch("chg");
        let bag = hand_bag(&root, &[("a.txt", b"hello")]);
        fs::write(bag.join("data/a.txt"), "hellp").unwrap();
        let r = validate(&bag).unwrap();
        assert!(r.complete && !r.valid);
        assert_eq!(cats(&r), vec![Category::Checksum]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn oxum_mismatch_is_incomplete_and_skips_checksums() {
        let root = scratch("oxum");
        let bag = hand_bag(&root, &[("a", &[1u8; 10]), ("b", &[2u8; 5])]);
        fs::write(bag.join("bag-info.txt"), "Payload-Oxum: 15.2\n").unwrap();
        assert!(validate(&bag).unwrap().valid);
        fs::write(bag.join("bag-info.txt"), "payload-oxum: 15.3\n").unwrap();
        // change a byte too: with a wrong oxum no checksum is computed, so it goes unnoticed
        fs::write(bag.join("data/a"), [9u8; 10]).unwrap();
        let r = validate(&bag).unwrap();
        assert!(!r.complete && !r.valid);
        assert_eq!(cats(&r), vec![Category::Structure]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn percent_names_and_unsafe_paths() {
        let root = scratch("pct");
        let bag = hand_bag(&root, &[("100%.txt", b"x")]);
        let m = fs::read_to_string(bag.join("manifest-sha256.txt")).unwrap();
        assert!(m.contains("data/100%25.txt"));
        let r = validate(&bag).unwrap();
        assert!(r.valid, "{:?}", r.findings);
        for bad in ["../x", "/etc/x", "~root/x", "data/../../x"] {
            let h = crate::checksums::sha256_hex(b"x");
            fs::write(bag.join("manifest-sha256.txt"), format!("{h}  {bad}\n")).unwrap();
            let r = validate(&bag).unwrap();
            assert!(
                r.findings
                    .iter()
                    .any(|f| f.category == Category::UnsafePath),
                "{bad}: {:?}",
                r.findings
            );
            assert!(!r.valid);
        }
        // a percent sequence outside the three defined ones is refused, not guessed at
        let h = crate::checksums::sha256_hex(b"x");
        fs::write(bag.join("manifest-sha256.txt"), format!("{h}  data/%41\n")).unwrap();
        assert!(cats(&validate(&bag).unwrap()).contains(&Category::Malformed));
        // bags written before the escaping (main): a lone '%' or a literal "%25" in the name
        // is read as written, with a warning, and the bag stays valid
        for name in ["100%.txt", "a%25b.txt", "x%41.txt"] {
            let root = scratch("pct-old");
            let bag = hand_bag(&root, &[(name, b"x")]);
            let h = crate::checksums::sha256_hex(b"x");
            fs::write(
                bag.join("manifest-sha256.txt"),
                format!("{h}  data/{name}\n"),
            )
            .unwrap();
            let r = validate(&bag).unwrap();
            assert!(r.valid, "{name}: {:?}", r.findings);
            assert!(
                r.findings
                    .iter()
                    .any(|f| f.severity == Severity::Warning && f.category == Category::Compat),
                "{name}: {:?}",
                r.findings
            );
            let _ = fs::remove_dir_all(&root);
        }
        assert_eq!(decode_path("a%0Ab%0dc%25").unwrap(), "a\nb\rc%");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_declarations_are_structure_findings() {
        let root = scratch("decl");
        let bag = hand_bag(&root, &[("a", b"x")]);
        for bad in [
            &b"\xef\xbb\xbfBagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n"[..],
            b"Tag-File-Character-Encoding: UTF-8\nBagIt-Version: 1.0\n",
            b"BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\nExtra: 1\n",
        ] {
            fs::write(bag.join("bagit.txt"), bad).unwrap();
            let r = validate(&bag).unwrap();
            assert!(
                r.findings
                    .iter()
                    .any(|f| f.category == Category::Structure && f.path == "bagit.txt"),
                "{:?}",
                r.findings
            );
            assert!(!r.valid);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tag_manifest_rules() {
        let root = scratch("tag");
        let bag = hand_bag(&root, &[("a.txt", b"x")]);
        let sum = |p: &str| crate::checksums::file_hash(&bag.join(p)).unwrap();
        let good = format!(
            "{}  bagit.txt\n{}  manifest-sha256.txt\n",
            sum("bagit.txt"),
            sum("manifest-sha256.txt")
        );
        fs::write(bag.join("tagmanifest-sha256.txt"), &good).unwrap();
        let r = validate(&bag).unwrap();
        assert!(r.valid, "{:?}", r.findings);
        // lists a payload file
        fs::write(
            bag.join("tagmanifest-sha256.txt"),
            format!("{good}{}  data/a.txt\n", sum("data/a.txt")),
        )
        .unwrap();
        assert!(cats(&validate(&bag).unwrap()).contains(&Category::Structure));
        // does not list the payload manifest
        fs::write(
            bag.join("tagmanifest-sha256.txt"),
            format!("{}  bagit.txt\n", sum("bagit.txt")),
        )
        .unwrap();
        let r = validate(&bag).unwrap();
        assert!(r
            .findings
            .iter()
            .any(|f| f.message.contains("payload manifest not listed")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn uppercase_hex_is_fine_and_duplicates_are_not() {
        let root = scratch("dup");
        let bag = hand_bag(&root, &[("a", b"x")]);
        let h = crate::checksums::sha256_hex(b"x");
        fs::write(
            bag.join("manifest-sha256.txt"),
            format!("{}\t data/a\n", h.to_ascii_uppercase()),
        )
        .unwrap();
        assert!(validate(&bag).unwrap().valid);
        fs::write(
            bag.join("manifest-sha256.txt"),
            format!("{h}  data/a\r\n{h}  data/a\r\n"),
        )
        .unwrap();
        let r = validate(&bag).unwrap();
        assert!(cats(&r).contains(&Category::Malformed));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn other_algorithms_and_fetch_list() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            digest_hex("sha1", b"abc").unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let root = scratch("alg");
        let bag = hand_bag(&root, &[("a", b"abc")]);
        fs::write(
            bag.join("manifest-MD5.txt"),
            "900150983cd24fb0d6963f7d28e17f72 *data/a\n",
        )
        .unwrap();
        let r = validate(&bag).unwrap();
        assert!(r.valid, "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.category == Category::Compat));
        fs::write(
            bag.join("fetch.txt"),
            "https://example.org/a 3 data/a\nnot a line\n",
        )
        .unwrap();
        let r = validate(&bag).unwrap();
        assert!(cats(&r).contains(&Category::Malformed));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bag_verifies_and_tampering_is_found() {
        let root = scratch("rt");
        let a = root.join("a.pdf");
        let b = root.join("b.txt");
        fs::write(&a, b"%PDF-1.4 fake").unwrap();
        fs::write(&b, "text").unwrap();
        let bag = root.join("bag");
        let s = write(&bag, &[a.clone(), b.clone()], 1_793_541_789, Some("test")).unwrap();
        assert_eq!((s.files, s.bytes), (2, 17));
        let info = fs::read_to_string(bag.join("bag-info.txt")).unwrap();
        assert!(info.contains("Bagging-Date: 2026-11-01"));
        assert!(info.contains("Payload-Oxum: 17.2"));
        let r = validate(&bag).unwrap();
        assert!(r.valid && r.findings.is_empty(), "{:?}", r.findings);
        assert!(verify(&bag).unwrap().is_empty());
        assert!(
            write(&bag, std::slice::from_ref(&a), 0, None).is_err(),
            "no writing into a used folder"
        );
        fs::write(bag.join("data/b.txt"), "TEXT").unwrap();
        let p = verify(&bag).unwrap();
        assert!(
            p.iter().any(|x| x.contains("b.txt: content changed")),
            "{p:?}"
        );
        fs::remove_file(bag.join("data/a.pdf")).unwrap();
        assert!(verify(&bag)
            .unwrap()
            .iter()
            .any(|x| x.contains("a.pdf: listed but not on disk")));
        let _ = fs::remove_dir_all(&root);
    }
}
