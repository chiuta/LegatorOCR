//! Fallback for PDF pages the built-in reader cannot decode (encryption, JPEG 2000,
//! JBIG2 symbol coding, rotated layers …): ask an installed PDF renderer to rasterise the
//! page. Like Tesseract, these are separate programs found on `PATH` — nothing is linked,
//! nothing is bundled. Tried in order: poppler `pdftoppm`, MuPDF `mutool`, Ghostscript.

use anyhow::{bail, Context, Result};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Resolution used for rasterised pages.
pub const DPI: u32 = 300;

fn candidates(name: &str) -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(name)];
    #[cfg(windows)]
    {
        v.push(PathBuf::from(format!("{name}.exe")));
        for base in [
            "C:\\Program Files\\poppler\\Library\\bin",
            "C:\\Program Files\\MuPDF",
            "C:\\Program Files\\gs\\bin",
        ] {
            v.push(Path::new(base).join(format!("{name}.exe")));
        }
    }
    for base in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        v.push(Path::new(base).join(name));
    }
    v
}

fn run(prog: &Path, args: &[&std::ffi::OsStr]) -> Result<std::process::Output> {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if start.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn first_working(names: &[&str], probe: &[&str]) -> Option<(String, PathBuf)> {
    for n in names {
        for c in candidates(n) {
            let a: Vec<&std::ffi::OsStr> = probe.iter().map(std::ffi::OsStr::new).collect();
            if run(&c, &a).is_ok() {
                return Some((n.to_string(), c));
            }
        }
    }
    None
}

/// Name of the first available renderer (for messages and tests).
pub fn renderer() -> Option<String> {
    first_working(&["pdftoppm"], &["-v"])
        .or_else(|| first_working(&["mutool"], &["-v"]))
        .or_else(|| first_working(&["gs", "gswin64c"], &["-v"]))
        .map(|(n, _)| n)
}

/// Number of pages according to qpdf, pdfinfo or mutool (handles encrypted files).
pub fn page_count(path: &Path) -> Option<u32> {
    let p = path.as_os_str();
    let num = |out: std::process::Output, key: Option<&str>| -> Option<u32> {
        let t = String::from_utf8_lossy(&out.stdout).into_owned();
        match key {
            None => t.trim().parse().ok(),
            Some(k) => t
                .lines()
                .find_map(|l| l.strip_prefix(k))
                .and_then(|v| v.trim().parse().ok()),
        }
    };
    if let Some((_, q)) = first_working(&["qpdf"], &["--version"]) {
        if let Ok(o) = run(&q, &[std::ffi::OsStr::new("--show-npages"), p]) {
            if let Some(n) = num(o, None).filter(|&n| n > 0) {
                return Some(n);
            }
        }
    }
    if let Some((_, q)) = first_working(&["pdfinfo"], &["-v"]) {
        if let Ok(o) = run(&q, &[p]) {
            if let Some(n) = num(o, Some("Pages:")).filter(|&n| n > 0) {
                return Some(n);
            }
        }
    }
    None
}

/// Rasterise page `page` (1-based) at [`DPI`].
pub fn rasterize(path: &Path, page: u32) -> Result<DynamicImage> {
    let dir = std::env::temp_dir().join(format!(
        "legatorocr-ext-{}-{page}-{:x}",
        std::process::id(),
        crate::now_unix() ^ (page as u64) << 32
    ));
    std::fs::create_dir_all(&dir)?;
    let result = (|| -> Result<DynamicImage> {
        let out = dir.join("page.png");
        let dpi = DPI.to_string();
        let pg = page.to_string();
        let o = std::ffi::OsStr::new;
        if let Some((_, t)) = first_working(&["pdftoppm"], &["-v"]) {
            let prefix = dir.join("page");
            let r = run(
                &t,
                &[
                    o("-r"),
                    o(&dpi),
                    o("-f"),
                    o(&pg),
                    o("-l"),
                    o(&pg),
                    o("-png"),
                    o("-singlefile"),
                    path.as_os_str(),
                    prefix.as_os_str(),
                ],
            );
            if r.as_ref().is_ok_and(|r| r.status.success()) && out.exists() {
                return image::open(&out).context("reading the rendered page");
            }
        }
        if let Some((_, t)) = first_working(&["mutool"], &["-v"]) {
            let r = run(
                &t,
                &[
                    o("draw"),
                    o("-r"),
                    o(&dpi),
                    o("-o"),
                    out.as_os_str(),
                    path.as_os_str(),
                    o(&pg),
                ],
            );
            if r.as_ref().is_ok_and(|r| r.status.success()) && out.exists() {
                return image::open(&out).context("reading the rendered page");
            }
        }
        if let Some((_, t)) = first_working(&["gs", "gswin64c"], &["-v"]) {
            let of = format!("-sOutputFile={}", out.display());
            let r = run(
                &t,
                &[
                    o("-q"),
                    o("-dSAFER"),
                    o("-dBATCH"),
                    o("-dNOPAUSE"),
                    o("-sDEVICE=png16m"),
                    o(&format!("-r{dpi}")),
                    o(&format!("-dFirstPage={pg}")),
                    o(&format!("-dLastPage={pg}")),
                    o(&of),
                    path.as_os_str(),
                ],
            );
            if r.as_ref().is_ok_and(|r| r.status.success()) && out.exists() {
                return image::open(&out).context("reading the rendered page");
            }
        }
        bail!("no PDF renderer (pdftoppm, mutool or Ghostscript) could draw this page")
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}
