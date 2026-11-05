//! Decoder isolation (roadmap 5.5): with `--isolate-decoders` every input file is decoded by a
//! separate worker process (this same program with `--decode-worker`), under a memory limit
//! (`ulimit -v`) and a CPU-time limit (`ulimit -t`) on Unix and a wall-clock timeout everywhere.
//! A decoder that crashes, runs out of memory or loops on a hostile file kills only the worker;
//! the run reports that file as unreadable. The worker hands the pixels back as a BMP in a private
//! temporary directory, which the parent reads with the image crate's BMP decoder.
//!
//! This is process isolation with resource limits, **not** a security sandbox: the worker has the
//! user's file and network rights (no seccomp, namespaces or Windows job object), and a
//! memory-safety bug exploited for code execution is not contained. The page-size reading
//! (`detect_dpi`) of the parent still parses image headers.

use anyhow::{bail, Context, Result};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Config {
    pub exe: PathBuf,
    pub memory_mb: u64,
    pub timeout_s: u64,
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn enable(c: Config) {
    *CONFIG.lock().unwrap() = Some(c);
}

pub fn enabled() -> bool {
    CONFIG.lock().unwrap().is_some()
}

/// Decode `path` in a worker process. `password` is passed in the environment, not the command line.
pub fn decode(path: &Path, password: &[u8]) -> Result<DynamicImage> {
    let (exe, mem, secs) = {
        let g = CONFIG.lock().unwrap();
        let c = g.as_ref().context("decoder isolation is not enabled")?;
        (c.exe.clone(), c.memory_mb, c.timeout_s)
    };
    decode_with(&exe, path, password, mem, secs)
}

pub fn decode_with(
    exe: &Path,
    path: &Path,
    password: &[u8],
    memory_mb: u64,
    timeout_s: u64,
) -> Result<DynamicImage> {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("lgr_dec_{}_{n}_{t}", std::process::id()));
    std::fs::create_dir(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let out = dir.join("page.bmp");
    let res = run_worker(exe, path, &out, password, memory_mb, timeout_s)
        .and_then(|()| image::open(&out).with_context(|| "reading the worker's result"));
    let _ = std::fs::remove_dir_all(&dir);
    res.map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

fn run_worker(
    exe: &Path,
    input: &Path,
    out: &Path,
    password: &[u8],
    memory_mb: u64,
    timeout_s: u64,
) -> Result<()> {
    let mut cmd;
    #[cfg(unix)]
    {
        // limits are set in a shell wrapper so no unsafe code or extra crate is needed; the
        // paths travel as positional arguments, never inside the script text
        cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("ulimit -v {} 2>/dev/null; ulimit -t {} 2>/dev/null; exec \"$0\" --decode-worker \"$1\" \"$2\"", memory_mb * 1024, timeout_s.max(1)))
            .arg(exe)
            .arg(input)
            .arg(out);
    }
    #[cfg(not(unix))]
    {
        cmd = Command::new(exe);
        cmd.arg("--decode-worker").arg(input).arg(out);
        let _ = memory_mb;
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if !password.is_empty() {
        cmd.env(
            "LEGATOROCR_WORKER_PW",
            String::from_utf8_lossy(password).as_ref(),
        );
    }
    let mut child = cmd.spawn().context("cannot start the decoder worker")?;
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(st) if st.success() => return Ok(()),
            Some(st) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    use std::io::Read;
                    let _ = e.read_to_string(&mut err);
                }
                let why = err.lines().last().unwrap_or("").trim().to_string();
                bail!(
                    "the decoder worker failed ({st}){}",
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(": {why}")
                    }
                );
            }
            None => {
                if start.elapsed() > Duration::from_secs(timeout_s) {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("the decoder worker took longer than {timeout_s} s and was stopped");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Body of `--decode-worker IN OUT`.
pub fn worker(input: &Path, out: &Path) -> Result<()> {
    if let Ok(pw) = std::env::var("LEGATOROCR_WORKER_PW") {
        crate::set_pdf_password(&pw);
    }
    let img = crate::load_image_direct(input)?;
    DynamicImage::ImageRgb8(img.to_rgb8()).save_with_format(out, image::ImageFormat::Bmp)?;
    Ok(())
}
