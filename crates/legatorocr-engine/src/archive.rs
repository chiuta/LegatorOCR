//! Read files straight out of archives.
//!
//! * **Built in** (no external program): `.zip` / `.cbz`, `.tar`, `.tar.gz` / `.tgz`.
//! * **Through an installed extractor**: `.7z`, `.rar` / `.cbr`, `.cab`, `.iso`, `.tar.bz2`,
//!   `.tar.xz`, `.arj`, `.lzh`, `.wim` … — the first of `7zz`, `7z`, `7za`, `unrar`, `bsdtar`,
//!   `unar` found on `PATH` (like Tesseract and the PDF renderers, a separate program; nothing
//!   is linked or bundled). Without one, the archive is reported as unreadable.
//!
//! Members are extracted into one per-process temporary folder ([`cleanup`] removes it);
//! only members the caller wants are written, names are flattened (no path can escape), and
//! size and count are capped.

use crate::zipmini;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_MEMBER: u64 = 1 << 30;
const MAX_TOTAL: u64 = 8 << 30;
const MAX_MEMBERS: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    TarGz,
    /// Needs an installed extractor.
    External,
}

/// Which kind of archive a path is, by file name.
pub fn kind(p: &Path) -> Option<Kind> {
    let n = p.file_name()?.to_str()?.to_ascii_lowercase();
    let ends = |s: &[&str]| s.iter().any(|x| n.ends_with(x));
    if ends(&[".zip", ".cbz"]) {
        Some(Kind::Zip)
    } else if ends(&[".tar.gz", ".tgz"]) {
        Some(Kind::TarGz)
    } else if ends(&[".tar"]) {
        Some(Kind::Tar)
    } else if ends(&[
        ".7z", ".cb7", ".rar", ".cbr", ".cab", ".iso", ".tar.bz2", ".tbz2", ".tar.xz", ".txz",
        ".tar.zst", ".arj", ".lzh", ".lha", ".wim", ".001",
    ]) {
        Some(Kind::External)
    } else {
        None
    }
}

/// Extensions for file dialogs (a `.tar.gz` ends in `gz`).
pub const EXT: &[&str] = &[
    "zip", "cbz", "tar", "tgz", "gz", "7z", "cb7", "rar", "cbr", "cab", "iso", "bz2", "xz", "tbz2",
    "txz", "zst", "arj", "lzh", "lha", "wim", "001",
];

pub fn is_archive(p: &Path) -> bool {
    kind(p).is_some()
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The folder all extractions of this process go to (created on demand).
pub fn work_dir() -> PathBuf {
    use std::sync::OnceLock;
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        // unpredictable name and owner-only permissions: another local user cannot pre-create
        // the folder (or plant symlinks in it) in the shared temp directory
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let stack = 0u8;
        let salt = t ^ ((&stack as *const u8 as u64) << 20) ^ ((std::process::id() as u64) << 40);
        let d = std::env::temp_dir().join(format!("legatorocr-archives-{}-{:016x}", std::process::id(), salt));
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        let _ = b.create(&d);
        d
    })
    .clone()
}

/// Remove everything extracted so far. Call when the program ends (the GUI keeps the files
/// while pages are open).
pub fn cleanup() {
    let _ = std::fs::remove_dir_all(work_dir());
}

fn fresh_dir() -> Result<PathBuf> {
    let d = work_dir().join(format!("a{}", COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).with_context(|| format!("cannot create {}", d.display()))?;
    Ok(d)
}

/// Safe file name for member number `i`: base name only, no separators or control chars.
fn flat_name(i: usize, member: &str) -> String {
    let base = member.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Cap by bytes (NAME_MAX is 255 bytes, not characters) but keep the extension: later code
    // decides pdf / tiff / image by it.
    let clean = cap_name(&clean, 180);
    let clean = if clean.trim_matches('.').is_empty() {
        "member".into()
    } else {
        clean
    };
    // The running number keeps archive order and makes equal names unique.
    format!("{i:06}_{clean}")
}

/// Shorten `name` to at most `max` bytes by cutting the stem, never the extension.
fn cap_name(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if name.len() - i <= 12 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut keep = max.saturating_sub(ext.len());
    while keep > 0 && !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{ext}", &stem[..keep])
}

/// Total size of the files below `dir` (symlinks not followed, depth-limited).
fn dir_size(dir: &Path, depth: u32) -> u64 {
    if depth > 32 {
        return 0;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path(), depth + 1),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

struct Sink {
    dir: PathBuf,
    out: Vec<(String, PathBuf)>,
    total: u64,
}

impl Sink {
    fn put(&mut self, member: &str, data: &[u8]) -> Result<()> {
        if self.out.len() >= MAX_MEMBERS {
            bail!("archive has too many members (limit {MAX_MEMBERS})");
        }
        self.total += data.len() as u64;
        if self.total > MAX_TOTAL {
            bail!("archive expands to more than {} GiB", MAX_TOTAL >> 30);
        }
        let p = self.dir.join(flat_name(self.out.len(), member));
        std::fs::write(&p, data).with_context(|| format!("writing {}", p.display()))?;
        self.out.push((member.to_string(), p));
        Ok(())
    }
}

fn octal(b: &[u8]) -> Option<u64> {
    // GNU base-256: a set high bit means the rest is a big-endian binary number (members > 8 GiB)
    if b.first().is_some_and(|&c| c & 0x80 != 0) {
        let mut v: u64 = (b[0] & 0x7f) as u64;
        for &c in &b[1..] {
            v = v.checked_mul(256)?.checked_add(c as u64)?;
        }
        return Some(v);
    }
    let s: String = b
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as char)
        .collect();
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

fn tar_extract(mut r: impl Read, want: &dyn Fn(&str) -> bool, sink: &mut Sink) -> Result<()> {
    let mut hdr = [0u8; 512];
    let mut long_name: Option<String> = None;
    // A truncated archive ends the loop: keep what was read.
    while r.read_exact(&mut hdr).is_ok() {
        if hdr.iter().all(|&b| b == 0) {
            break;
        }
        let Some(size) = octal(&hdr[124..136]) else {
            eprintln!("warning: damaged tar header; keeping what was read");
            break;
        };
        let typ = hdr[156];
        let str_of = |b: &[u8]| -> String {
            String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
                .into_owned()
        };
        let mut name = str_of(&hdr[0..100]);
        if &hdr[257..262] == b"ustar" {
            let prefix = str_of(&hdr[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        let padded = size.div_ceil(512) * 512;
        let wanted_type = matches!(typ, b'0' | 0);
        let meta_type = matches!(typ, b'L' | b'x');
        if (wanted_type && want(long_name.as_deref().unwrap_or(&name)) && size <= MAX_MEMBER)
            || (meta_type && size <= 1 << 20)
        {
            // read what is there: a header that promises a gigabyte must not allocate one up front
            let mut data = Vec::new();
            (&mut r).take(size).read_to_end(&mut data)?;
            if (data.len() as u64) < size {
                eprintln!("warning: tar archive is truncated inside a member; keeping what was read");
                break;
            }
            let skip = padded - size;
            std::io::copy(&mut (&mut r).take(skip), &mut std::io::sink())?;
            if typ == b'L' {
                long_name = Some(str_of(&data));
            } else if typ == b'x' {
                // pax: "len key=value\n" records; only `path` matters.
                let t = String::from_utf8_lossy(&data);
                long_name = t
                    .lines()
                    .find_map(|l| {
                        l.split_once(' ')
                            .and_then(|(_, kv)| kv.strip_prefix("path="))
                    })
                    .map(str::to_string);
            } else {
                let n = long_name.take().unwrap_or(name);
                sink.put(&n, &data)?;
            }
        } else {
            std::io::copy(&mut (&mut r).take(padded), &mut std::io::sink())?;
            if !meta_type {
                long_name = None;
            }
        }
    }
    Ok(())
}

/// macOS resource-fork leftovers (`__MACOSX/…`, `._name`) that archivers add: never pages.
fn is_junk(member: &str) -> bool {
    member
        .split(['/', '\\'])
        .any(|c| c == "__MACOSX" || c.starts_with("._"))
}

fn find_tool(names: &[&str]) -> Option<(String, PathBuf)> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut all = dirs;
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        all.push(PathBuf::from(d));
    }
    if cfg!(windows) {
        for d in [
            "C:\\Program Files\\7-Zip",
            "C:\\Program Files (x86)\\7-Zip",
            "C:\\Program Files\\WinRAR",
        ] {
            all.push(PathBuf::from(d));
        }
    }
    for n in names {
        for d in &all {
            for cand in [d.join(n), d.join(format!("{n}.exe"))] {
                if cand.is_file() {
                    return Some((n.to_string(), cand));
                }
            }
        }
    }
    None
}

/// Name of the external extractor that would be used, if any.
pub fn external_extractor() -> Option<String> {
    find_tool(&["7zz", "7z", "7za", "unrar", "bsdtar", "unar"]).map(|(n, _)| n)
}

fn walk_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 40 || out.len() > MAX_MEMBERS {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_files(&e.path(), depth + 1, out);
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
}

fn external_extract(archive: &Path, want: &dyn Fn(&str) -> bool, sink: &mut Sink) -> Result<()> {
    let safe_archive = crate::files::safe_path(archive);
    let archive = safe_archive.as_path();
    let Some((name, prog)) = find_tool(&["7zz", "7z", "7za", "unrar", "bsdtar", "unar"]) else {
        bail!(
            "{}: reading this archive type needs an installed extractor (7-Zip, unrar, bsdtar or unar)",
            archive.display()
        );
    };
    let raw = sink.dir.join("_raw");
    std::fs::create_dir_all(&raw)?;
    let mut cmd = Command::new(&prog);
    match name.as_str() {
        "7zz" | "7z" | "7za" => {
            let mut o = std::ffi::OsString::from("-o");
            o.push(&raw);
            cmd.args(["x", "-y", "-bd", "-p-"]).arg(o).arg(archive);
        }
        "unrar" => {
            let mut d = raw.clone().into_os_string();
            d.push(std::path::MAIN_SEPARATOR_STR);
            cmd.args(["x", "-y", "-idq", "-p-"]).arg(archive).arg(d);
        }
        "bsdtar" => {
            cmd.arg("-xf").arg(archive).arg("-C").arg(&raw);
        }
        _ => {
            cmd.arg("-o").arg(&raw).arg("-f").arg(archive);
        }
    }
    // the scratch folder goes away on every exit path (errors, timeouts, bombs)
    struct RawGuard(PathBuf);
    impl Drop for RawGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = RawGuard(raw.clone());
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {name}"))?;
    // keep the tail of stderr on a thread so a chatty extractor can never block on a full pipe
    let err_thread = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.by_ref().take(1 << 20).read_to_end(&mut buf);
            let _ = std::io::copy(&mut s, &mut std::io::sink()); // keep draining
            String::from_utf8_lossy(&buf).into_owned()
        })
    });
    let start = Instant::now();
    let mut last_check = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None if start.elapsed() > Duration::from_secs(3600) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{name} timed out");
            }
            None => {
                // an archive bomb must be stopped while it extracts, not after the disk is full
                if last_check.elapsed() > Duration::from_millis(500) {
                    last_check = Instant::now();
                    if dir_size(&raw, 0) > MAX_TOTAL {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!("archive expands to more than {} GiB", MAX_TOTAL >> 30);
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    // exit status 1 of the 7-Zip family means "warnings" (a bad CRC somewhere, a skipped file):
    // the other members are fine
    let warning_only = status.code() == Some(1) && matches!(name.as_str(), "7zz" | "7z" | "7za");
    if !status.success() && !warning_only {
        let e = err_thread.and_then(|t| t.join().ok()).unwrap_or_default();
        bail!(
            "{name} failed on {}: {}",
            archive.display(),
            e.trim().lines().last().unwrap_or("error")
        );
    }
    let mut files = Vec::new();
    walk_files(&raw, 0, &mut files);
    // Archive order is not preserved by the extractors; sort naturally by relative path.
    files.sort_by(|a, b| crate::files::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    for f in files {
        let rel = f
            .strip_prefix(&raw)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if !want(&rel) {
            continue;
        }
        let len = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
        if len > MAX_MEMBER {
            continue;
        }
        sink.total += len;
        if sink.total > MAX_TOTAL {
            bail!("archive expands to more than {} GiB", MAX_TOTAL >> 30);
        }
        let dest = sink.dir.join(flat_name(sink.out.len(), &rel));
        std::fs::rename(&f, &dest).or_else(|_| std::fs::copy(&f, &dest).map(|_| ()))?;
        sink.out.push((rel, dest));
    }
    let _ = std::fs::remove_dir_all(&raw);
    Ok(())
}

/// Extract the members of `archive` accepted by `want` (given the member path inside the
/// archive) and return `(member path, extracted file)` in archive order (natural order of
/// the member path for external extractors).
pub fn extract(archive: &Path, want: &dyn Fn(&str) -> bool) -> Result<Vec<(String, PathBuf)>> {
    let want = &|n: &str| !is_junk(n) && want(n);
    let Some(k) = kind(archive) else {
        bail!("{}: not a supported archive", archive.display());
    };
    let mut sink = Sink {
        dir: fresh_dir()?,
        out: Vec::new(),
        total: 0,
    };
    // whatever goes wrong half-way, the files already written for this archive are removed
    match extract_into(k, archive, want, &mut sink) {
        Ok(()) => Ok(sink.out),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&sink.dir);
            Err(e)
        }
    }
}

fn extract_into(k: Kind, archive: &Path, want: &dyn Fn(&str) -> bool, sink: &mut Sink) -> Result<()> {
    match k {
        Kind::Zip => {
            let md = std::fs::metadata(archive)?;
            if md.len() > 4 << 30 {
                bail!("{}: zip file too large", archive.display());
            }
            let data = std::fs::read(archive)?;
            let Some(list) = zipmini::list(&data) else {
                // ZIP64 / odd layouts: let an installed extractor try.
                return external_extract(archive, want, sink);
            };
            let mut unreadable: Vec<(String, String)> = Vec::new();
            for e in &list {
                if e.name.ends_with('/') || !want(&e.name) {
                    continue;
                }
                match zipmini::read_checked(&data, e) {
                    Ok(b) => sink.put(&e.name, &b)?,
                    Err(why) => unreadable.push((e.name.clone(), why)),
                }
            }
            if !unreadable.is_empty() {
                // another method (LZMA, bzip2, deflate64), ZIP64 or a damaged member: an installed
                // extractor may still read everything; otherwise say what is missing
                let mut again = Sink { dir: fresh_dir()?, out: Vec::new(), total: 0 };
                if external_extract(archive, want, &mut again).is_ok() && !again.out.is_empty() {
                    let _ = std::fs::remove_dir_all(&sink.dir);
                    sink.dir = again.dir;
                    sink.out = again.out;
                    return Ok(());
                }
                eprintln!(
                    "warning: {}: {} member(s) could not be read (first: {} — {}); install 7-Zip or bsdtar to read them",
                    archive.display(),
                    unreadable.len(),
                    unreadable[0].0,
                    unreadable[0].1
                );
            }
        }
        Kind::Tar => tar_extract(std::fs::File::open(archive)?, want, sink)?,
        Kind::TarGz => tar_extract(
            flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(archive)?)),
            want,
            sink,
        )?,
        Kind::External => external_extract(archive, want, sink)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_member_names_keep_their_extension_and_stay_under_name_max() {
        let long = format!("{}.pdf", "é".repeat(300));
        let n = flat_name(7, &long);
        assert!(n.len() <= 6 + 1 + 180 + 4, "{} bytes", n.len());
        assert!(n.ends_with(".pdf"), "{n}");
        assert!(n.starts_with("000007_"));
        assert_eq!(cap_name("short.png", 180), "short.png");
        assert_eq!(cap_name(&"x".repeat(400), 100).len(), 100);
    }

    fn tar_header(name: &str, size_field: [u8; 12], typ: u8) -> [u8; 512] {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..136].copy_from_slice(&size_field);
        h[156] = typ;
        h
    }

    fn octal_field(n: u64) -> [u8; 12] {
        let mut f = [0u8; 12];
        let t = format!("{n:011o}");
        f[..11].copy_from_slice(t.as_bytes());
        f
    }

    #[test]
    fn tar_base256_sizes_and_truncated_members_do_not_abort_the_archive() {
        // base-256 encoded size (high bit set) of 5 bytes is read like the octal one
        let mut f = [0u8; 12];
        f[0] = 0x80;
        f[11] = 5;
        assert_eq!(octal(&f), Some(5));
        assert_eq!(octal(&octal_field((8 << 30) - 1)), Some((8 << 30) - 1));
        // good member, then a member that promises 100 000 bytes but the file ends
        let mut t = Vec::new();
        t.extend_from_slice(&tar_header("a.png", octal_field(5), b'0'));
        t.extend_from_slice(b"HELLO");
        t.extend_from_slice(&[0u8; 507]);
        t.extend_from_slice(&tar_header("b.png", octal_field(100_000), b'0'));
        t.extend_from_slice(b"short");
        let d = tmp("tar_trunc");
        let mut sink = Sink { dir: d.clone(), out: Vec::new(), total: 0 };
        tar_extract(&t[..], &|_| true, &mut sink).expect("a truncated tar keeps what was read");
        assert_eq!(sink.out.len(), 1);
        assert_eq!(std::fs::read(&sink.out[0].1).unwrap(), b"HELLO");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn zip_members_that_cannot_be_read_say_why() {
        let z = zipmini::write(&[("ok.txt", b"fine"), ("bad.txt", b"payload")]);
        let list = zipmini::list(&z).unwrap();
        assert!(zipmini::read_checked(&z, &list[0]).is_ok());
        // flip a payload byte of the stored second member: the CRC no longer matches
        let mut damaged = z.clone();
        let at = damaged.windows(7).position(|w| w == b"payload").unwrap();
        damaged[at] ^= 0xFF;
        let list = zipmini::list(&damaged).unwrap();
        assert_eq!(zipmini::read_checked(&damaged, &list[1]).unwrap_err(), "checksum mismatch");
        // an unknown compression method in the central directory
        let mut m = z.clone();
        let cd = m.windows(4).rposition(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        m[cd + 10] = 99;
        let list = zipmini::list(&m).unwrap();
        assert!(zipmini::read_checked(&m, &list[1]).unwrap_err().contains("method 99"));
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgr-arch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind(Path::new("a/B.ZIP")), Some(Kind::Zip));
        assert_eq!(kind(Path::new("x.tar.gz")), Some(Kind::TarGz));
        assert_eq!(kind(Path::new("x.tgz")), Some(Kind::TarGz));
        assert_eq!(kind(Path::new("x.tar")), Some(Kind::Tar));
        assert_eq!(kind(Path::new("x.7z")), Some(Kind::External));
        assert_eq!(kind(Path::new("x.rar")), Some(Kind::External));
        assert_eq!(kind(Path::new("x.jpg")), None);
    }

    #[test]
    fn mac_resource_forks_are_ignored() {
        assert!(is_junk("__MACOSX/a/b.png"));
        assert!(is_junk("in/._p1.png"));
        assert!(!is_junk("in/p1.png"));
        assert!(!is_junk("in/a._b.png"));
    }

    #[test]
    fn names_cannot_escape() {
        assert_eq!(flat_name(3, "../../etc/passwd"), "000003_passwd");
        assert_eq!(flat_name(0, "a\\b\\c:d.png"), "000000_c_d.png");
        assert_eq!(flat_name(1, "dir/"), "000001_member");
    }

    #[test]
    fn zip_members_are_filtered_and_flattened() {
        let d = tmp("zip");
        let z = zipmini::write(&[("p/b.jpg", b"B"), ("../x.txt", b"X"), ("p/a.png", b"A")]);
        std::fs::write(d.join("s.zip"), z).unwrap();
        let v = extract(&d.join("s.zip"), &|n| {
            n.ends_with(".jpg") || n.ends_with(".png")
        })
        .unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "p/b.jpg");
        assert_eq!(std::fs::read(&v[1].1).unwrap(), b"A");
        assert!(v[0].1.starts_with(work_dir()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tar_and_targz_with_system_tar() {
        let Ok(out) = Command::new("tar").arg("--version").output() else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let d = tmp("tar");
        std::fs::create_dir_all(d.join("in/sub")).unwrap();
        std::fs::write(d.join("in/sub/p1.png"), b"one").unwrap();
        let long = "l".repeat(130);
        std::fs::write(d.join(format!("in/{long}.png")), b"two").unwrap();
        std::fs::write(d.join("in/n.txt"), b"nope").unwrap();
        for (flag, name) in [("-cf", "a.tar"), ("-czf", "a.tgz")] {
            let ok = Command::new("tar")
                .current_dir(&d)
                .args([flag, name, "in"])
                .status()
                .unwrap()
                .success();
            assert!(ok);
            let v = extract(&d.join(name), &|n| n.ends_with(".png")).unwrap();
            assert_eq!(v.len(), 2, "{name}: {v:?}");
            let mut data: Vec<Vec<u8>> = v.iter().map(|x| std::fs::read(&x.1).unwrap()).collect();
            data.sort();
            assert_eq!(data, vec![b"one".to_vec(), b"two".to_vec()]);
            assert!(v.iter().any(|x| x.0.contains(&long)), "{v:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_archives_error_cleanly() {
        let d = tmp("bad");
        std::fs::write(d.join("x.tgz"), b"not gzip at all").unwrap();
        assert!(extract(&d.join("x.tgz"), &|_| true)
            .map(|v| v.is_empty())
            .unwrap_or(true));
        std::fs::write(d.join("y.7z"), b"garbage").unwrap();
        // With or without an extractor installed: an error or nothing, never a panic.
        let _ = extract(&d.join("y.7z"), &|_| true);
        let _ = std::fs::remove_dir_all(&d);
    }
}
