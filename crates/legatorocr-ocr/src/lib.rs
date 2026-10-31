//! OCR through the Tesseract command-line engine (Apache-2.0, run as a separate
//! process — no linking, no code reuse). Output TSV is parsed into [`Word`]s.

use legatorocr_core::{PageImage, Word};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("tesseract not found ({0}); install it or pass --tesseract <path>")]
    NotFound(String),
    #[error("tesseract failed: {0}")]
    Failed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
}

#[derive(Debug, Clone)]
pub struct OcrOptions {
    /// Tesseract language(s), e.g. `eng`, `ron+eng`.
    pub lang: String,
    /// Page segmentation mode (3 = fully automatic, 6 = single block …).
    pub psm: u32,
    /// Words below this confidence are dropped (0 keeps everything).
    pub min_conf: f32,
    pub tesseract: PathBuf,
    /// Optional tessdata directory.
    pub tessdata: Option<PathBuf>,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self { lang: "eng".into(), psm: 3, min_conf: 0.0, tesseract: "tesseract".into(), tessdata: None }
    }
}

fn base_cmd(o: &OcrOptions) -> Command {
    let mut c = Command::new(&o.tesseract);
    c.env("OMP_THREAD_LIMIT", "1");
    if let Some(t) = &o.tessdata {
        c.env("TESSDATA_PREFIX", t);
    }
    #[cfg(windows)]
    {
        // A GUI app must not flash a console window for every OCR call.
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Tesseract language list syntax: names joined by `+` (e.g. `ron+eng`, `script/Latin`).
pub fn valid_lang(l: &str) -> bool {
    !l.is_empty()
        && l.split('+').all(|p| {
            !p.is_empty() && !p.starts_with('-') && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/' || c == '-')
        })
}

/// Returns the engine's version line, or an error if it cannot be run.
pub fn check_engine(o: &OcrOptions) -> Result<String, OcrError> {
    let out = base_cmd(o)
        .arg("--version")
        .output()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    let txt = String::from_utf8_lossy(if out.stdout.is_empty() { &out.stderr } else { &out.stdout });
    Ok(txt.lines().next().unwrap_or("tesseract").to_string())
}

/// Languages installed for the engine.
pub fn list_languages(o: &OcrOptions) -> Result<Vec<String>, OcrError> {
    let out = base_cmd(o)
        .arg("--list-langs")
        .output()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    let txt = String::from_utf8_lossy(&out.stdout);
    Ok(txt.lines().skip(1).map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Recognise one processed page.
pub fn recognize(page: &PageImage, dpi: u32, o: &OcrOptions) -> Result<Vec<Word>, OcrError> {
    let tmp = std::env::temp_dir().join(format!(
        "legatorocr-ocr-{}-{}.png",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    page.to_dynamic().save(&tmp)?;
    let res = run(&tmp, dpi, o);
    let _ = std::fs::remove_file(&tmp);
    res
}

fn run(png: &std::path::Path, dpi: u32, o: &OcrOptions) -> Result<Vec<Word>, OcrError> {
    let out = base_cmd(o)
        .arg(png)
        .arg("stdout")
        .args(["-l", &o.lang, "--psm", &o.psm.to_string(), "--dpi", &dpi.to_string()])
        .arg("tsv")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    if !out.status.success() {
        return Err(OcrError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout), o.min_conf))
}

/// Parse Tesseract TSV (`level page block par line word left top width height conf text`).
pub fn parse_tsv(tsv: &str, min_conf: f32) -> Vec<Word> {
    let mut words = Vec::new();
    let mut line_ids: Vec<(u32, u32, u32)> = Vec::new();
    for l in tsv.lines().skip(1) {
        let f: Vec<&str> = l.splitn(12, '\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let text = f[11].trim();
        let conf: f32 = f[10].parse().unwrap_or(-1.0);
        if text.is_empty() || conf < 0.0 || conf < min_conf {
            continue;
        }
        let n = |i: usize| f[i].parse::<u32>().unwrap_or(0);
        let key = (n(2), n(3), n(4));
        let line = match line_ids.iter().position(|k| *k == key) {
            Some(p) => p,
            None => {
                line_ids.push(key);
                line_ids.len() - 1
            }
        } as u32;
        words.push(Word { text: text.to_string(), x: n(6), y: n(7), w: n(8), h: n(9), conf, line });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lang_syntax() {
        for ok in ["eng", "ron+eng", "script/Latin", "chi_sim"] {
            assert!(valid_lang(ok), "{ok}");
        }
        for bad in ["", "+", "eng+", "--psm", "-l", "a b", "en;g", "ron+-x"] {
            assert!(!valid_lang(bad), "{bad}");
        }
    }

    #[test]
    fn parses_tsv() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t20\t30\t12\t96.5\tBună\n\
                   5\t1\t1\t1\t1\t2\t50\t20\t40\t12\t12.0\tziua\n\
                   5\t1\t1\t1\t2\t1\t10\t50\t30\t12\t90\tscris\n";
        let w = parse_tsv(tsv, 0.0);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].text, "Bună");
        assert_eq!((w[0].line, w[1].line, w[2].line), (0, 0, 1));
        assert_eq!(parse_tsv(tsv, 50.0).len(), 2);
    }
}
