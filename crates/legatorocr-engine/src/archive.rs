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
    std::env::temp_dir().join(format!("legatorocr-archives-{}", std::process::id()))
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
        .take(120)
        .collect();
    let clean = if clean.trim_matches('.').is_empty() {
        "member".into()
    } else {
        clean
    };
    // The running number keeps archive order and makes equal names unique.
    format!("{i:06}_{clean}")
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
            bail!("damaged tar header");
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
            let mut data = vec![0u8; size as usize];
            r.read_exact(&mut data).context("truncated tar member")?;
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
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {name}"))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None if start.elapsed() > Duration::from_secs(3600) => {
                let _ = child.kill();
                bail!("{name} timed out");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    if !status.success() {
        let mut e = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut e);
        }
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
    match k {
        Kind::Zip => {
            let md = std::fs::metadata(archive)?;
            if md.len() > 4 << 30 {
                bail!("{}: zip file too large", archive.display());
            }
            let data = std::fs::read(archive)?;
            let Some(list) = zipmini::list(&data) else {
                // ZIP64 / odd layouts: let an installed extractor try.
                return external_extract(archive, want, &mut sink).map(|_| sink.out);
            };
            for e in &list {
                if e.name.ends_with('/') || !want(&e.name) {
                    continue;
                }
                if let Some(b) = zipmini::read(&data, e) {
                    sink.put(&e.name, &b)?;
                }
            }
        }
        Kind::Tar => tar_extract(std::fs::File::open(archive)?, want, &mut sink)?,
        Kind::TarGz => tar_extract(
            flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(archive)?)),
            want,
            &mut sink,
        )?,
        Kind::External => external_extract(archive, want, &mut sink)?,
    }
    Ok(sink.out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
