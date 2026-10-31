//! OCR through the Tesseract command-line engine (Apache-2.0, run as a separate
//! process — no linking, no code reuse). Output TSV is parsed into [`Word`]s.

use legatorocr_core::{PageImage, Word};
use std::path::PathBuf;
use std::process::{Command, Stdio};

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
        Self {
            lang: "eng".into(),
            psm: 3,
            min_conf: 0.0,
            tesseract: "tesseract".into(),
            tessdata: None,
        }
    }
}

/// Folders where Tesseract is usually installed but which a GUI app launched from
/// Finder / the Start menu does not have on its `PATH`.
fn well_known_dirs() -> Vec<PathBuf> {
    #[allow(unused_mut)] // only extended on macOS and Windows
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    v.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                v.push(base.join("Tesseract-OCR"));
                v.push(base.join("Programs").join("Tesseract-OCR"));
            }
        }
    }
    v
}

/// The program to run: the configured path as given when it works or is explicit,
/// else (bare name only) the first well-known install location that has it.
pub fn resolve_tesseract(configured: &std::path::Path) -> PathBuf {
    if configured.components().count() != 1 {
        return configured.to_path_buf(); // an explicit path: use it as is
    }
    let on_path = std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            d.join(configured).is_file()
                || d.join(format!("{}.exe", configured.display())).is_file()
        })
    });
    if on_path {
        return configured.to_path_buf();
    }
    for d in well_known_dirs() {
        for name in [
            configured.to_path_buf(),
            PathBuf::from(format!("{}.exe", configured.display())),
        ] {
            let cand = d.join(name);
            if cand.is_file() {
                return cand;
            }
        }
    }
    configured.to_path_buf()
}

fn base_cmd(o: &OcrOptions) -> Command {
    let mut c = Command::new(resolve_tesseract(&o.tesseract));
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

/// The eight languages of the interface and their recognition-model names:
/// (ISO 639-1, Tesseract code, English name).
pub const UI_LANGUAGES: [(&str, &str, &str); 8] = [
    ("ro", "ron", "Romanian"),
    ("en", "eng", "English"),
    ("fr", "fra", "French"),
    ("it", "ita", "Italian"),
    ("es", "spa", "Spanish"),
    ("pt", "por", "Portuguese"),
    ("de", "deu", "German"),
    ("hu", "hun", "Hungarian"),
];

/// Accept two-letter codes (`ro+en`) and return Tesseract's three-letter names
/// (`ron+eng`); anything else passes through unchanged.
pub fn canonical_lang(l: &str) -> String {
    l.split('+')
        .map(|p| {
            UI_LANGUAGES
                .iter()
                .find(|(a, _, _)| p.eq_ignore_ascii_case(a))
                .map(|(_, t, _)| *t)
                .unwrap_or(p)
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// How to get a missing recognition model, for the current system.
pub fn install_hint(tess_code: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("brew install tesseract-lang   (provides {tess_code}.traineddata)")
    } else if cfg!(windows) {
        format!(
            "re-run the Tesseract installer and tick the language, or put {tess_code}.traineddata \
             into Tesseract's tessdata folder"
        )
    } else {
        format!("sudo apt install tesseract-ocr-{tess_code}   (or put {tess_code}.traineddata into the tessdata folder)")
    }
}

/// Tesseract language list syntax: names joined by `+` (e.g. `ron+eng`, `script/Latin`).
pub fn valid_lang(l: &str) -> bool {
    !l.is_empty()
        && l.split('+').all(|p| {
            !p.is_empty()
                && !p.starts_with('-')
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '/' || c == '-')
        })
}

/// Returns the engine's version line, or an error if it cannot be run.
pub fn check_engine(o: &OcrOptions) -> Result<String, OcrError> {
    let out = base_cmd(o)
        .arg("--version")
        .output()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    let txt = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    });
    Ok(txt.lines().next().unwrap_or("tesseract").to_string())
}

/// Languages installed for the engine.
pub fn list_languages(o: &OcrOptions) -> Result<Vec<String>, OcrError> {
    let out = base_cmd(o)
        .arg("--list-langs")
        .output()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    let txt = String::from_utf8_lossy(&out.stdout);
    Ok(txt
        .lines()
        .skip(1)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Parse Tesseract OSD output (`Rotate: 90`, `Orientation confidence: 5.2`) into the
/// clockwise rotation (0/90/180/270) that makes the text upright. Low confidence → `None`.
pub fn parse_osd(out: &str) -> Option<u16> {
    let val = |key: &str| -> Option<f32> {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .and_then(|v| v.trim().parse().ok())
    };
    let rot = val("Rotate:")? as i32;
    let conf = val("Orientation confidence:")?;
    (conf >= 1.5 && matches!(rot, 0 | 90 | 180 | 270)).then_some(rot as u16)
}

/// Ask Tesseract's orientation/script detection how far `img` must be turned
/// clockwise to be upright. `Ok(None)` when it cannot tell (too little text,
/// low confidence, or the `osd` language data is not installed).
pub fn detect_rotation(img: &image::DynamicImage, o: &OcrOptions) -> Result<Option<u16>, OcrError> {
    let small = if img.width().max(img.height()) > 1800 {
        img.thumbnail(1800, 1800)
    } else {
        img.clone()
    };
    let png = png_bytes(&image::DynamicImage::ImageLuma8(small.to_luma8()))?;
    let mut cmd = base_cmd(o);
    cmd.args(["stdin", "stdout", "--psm", "0", "-l", "osd"]);
    let out = run_stdin(cmd, png)?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(parse_osd(&String::from_utf8_lossy(&out.stdout)))
}

/// Encode as PNG in memory (no temp files: nothing to clean up, and no trouble
/// with non-ASCII user-profile paths that Tesseract on Windows cannot open).
fn png_bytes(img: &image::DynamicImage) -> Result<Vec<u8>, OcrError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Run Tesseract reading the image from stdin; returns its output.
fn run_stdin(mut cmd: Command, png: Vec<u8>) -> Result<std::process::Output, OcrError> {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| OcrError::NotFound(e.to_string()))?;
    let mut stdin = child.stdin.take().expect("piped");
    // Feed from a thread so a full output pipe can never deadlock us; a write
    // error just means Tesseract stopped reading (its exit status tells why).
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&png);
    });
    let out = child.wait_with_output()?;
    let _ = feeder.join();
    Ok(out)
}

/// Integer enlargement applied before recognition: Tesseract reads best at about
/// 300 dpi, so a 150 dpi page is doubled. 1 = leave alone (already ≥ 250 dpi, or the
/// enlarged page would exceed 14 000 px / 120 megapixels).
pub fn upscale_factor(dpi: u32, w: u32, h: u32) -> u32 {
    if dpi == 0 || dpi >= 250 {
        return 1;
    }
    let k = 300u32.div_ceil(dpi).min(4);
    let (sw, sh) = (w as u64 * k as u64, h as u64 * k as u64);
    if sw.max(sh) > 14_000 || sw * sh > 120_000_000 {
        1
    } else {
        k
    }
}

/// Recognise one processed page. Low-resolution pages are enlarged first
/// ([`upscale_factor`]); word boxes are mapped back to the page's own pixels.
pub fn recognize(page: &PageImage, dpi: u32, o: &OcrOptions) -> Result<Vec<Word>, OcrError> {
    let k = upscale_factor(dpi, page.width(), page.height());
    let dpi = dpi * k;
    let img = page.to_dynamic();
    let img = if k > 1 {
        img.resize_exact(
            img.width() * k,
            img.height() * k,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        img
    };
    let png = png_bytes(&img)?;
    let lang = canonical_lang(&o.lang);
    let mut cmd = base_cmd(o);
    cmd.args([
        "stdin",
        "stdout",
        "-l",
        &lang,
        "--psm",
        &o.psm.to_string(),
        "--dpi",
        &dpi.to_string(),
        "tsv",
    ]);
    let out = run_stdin(cmd, png)?;
    if !out.status.success() {
        return Err(OcrError::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let mut words = parse_tsv(&String::from_utf8_lossy(&out.stdout), o.min_conf);
    // Romanian: Tesseract (and old fonts) sometimes give cedilla ş/ţ; the correct letters
    // are comma-below ș/ț, and search depends on it.
    if lang.split('+').any(|l| l == "ron") {
        for w in &mut words {
            w.text = legatorocr_core::text::normalize_ro(&w.text);
        }
    }
    if k > 1 {
        for w in &mut words {
            let (x1, y1) = (w.x + w.w, w.y + w.h);
            w.x /= k;
            w.y /= k;
            w.w = (x1.div_ceil(k)).saturating_sub(w.x).max(1);
            w.h = (y1.div_ceil(k)).saturating_sub(w.y).max(1);
        }
    }
    Ok(words)
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
        words.push(Word {
            text: text.to_string(),
            x: n(6),
            y: n(7),
            w: n(8),
            h: n(9),
            conf,
            line,
        });
    }
    words
}

/// Words grouped into lines (in the order the engine produced them), joined by
/// spaces; lines by `\n`. Empty input → empty string.
pub fn words_to_text(words: &[Word]) -> String {
    let mut out = String::new();
    let mut cur: Option<u32> = None;
    for w in words {
        match cur {
            Some(l) if l == w.line => out.push(' '),
            Some(_) => out.push('\n'),
            None => {}
        }
        cur = Some(w.line);
        out.push_str(&w.text);
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if (c as u32) < 0x20 && c != '\t' => {} // not allowed in XML 1.0
            c => o.push(c),
        }
    }
    o
}

/// One hOCR `ocr_page` block (`page_no` starts at 1).
pub fn hocr_page(words: &[Word], width: u32, height: u32, dpi: u32, page_no: usize) -> String {
    let mut o = format!(
        "<div class='ocr_page' id='page_{page_no}' title='bbox 0 0 {width} {height}; ppageno {}; scan_res {dpi} {dpi}'>\n",
        page_no - 1
    );
    let mut i = 0;
    let mut line_no = 0;
    while i < words.len() {
        let l = words[i].line;
        let mut j = i;
        while j < words.len() && words[j].line == l {
            j += 1;
        }
        line_no += 1;
        let (x0, y0) = (
            words[i..j].iter().map(|w| w.x).min().unwrap_or(0),
            words[i..j].iter().map(|w| w.y).min().unwrap_or(0),
        );
        let x1 = words[i..j]
            .iter()
            .map(|w| w.x.saturating_add(w.w))
            .max()
            .unwrap_or(0);
        let y1 = words[i..j]
            .iter()
            .map(|w| w.y.saturating_add(w.h))
            .max()
            .unwrap_or(0);
        o.push_str(&format!("  <span class='ocr_line' id='line_{page_no}_{line_no}' title='bbox {x0} {y0} {x1} {y1}'>"));
        for (k, w) in words[i..j].iter().enumerate() {
            o.push_str(&format!(
                "<span class='ocrx_word' id='word_{page_no}_{line_no}_{}' title='bbox {} {} {} {}; x_wconf {}'>{}</span> ",
                k + 1,
                w.x,
                w.y,
                w.x.saturating_add(w.w),
                w.y.saturating_add(w.h),
                w.conf.round().clamp(0.0, 100.0) as u32,
                xml_escape(&w.text)
            ));
        }
        o.push_str("</span>\n");
        i = j;
    }
    o.push_str("</div>\n");
    o
}

/// Wrap hOCR pages into a complete document.
pub fn hocr_document(pages: &[String]) -> String {
    let mut o = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\" lang=\"en\">\n<head>\n<title></title>\n\
         <meta http-equiv=\"Content-Type\" content=\"text/html;charset=utf-8\" />\n\
         <meta name='ocr-system' content='LegatorOCR (Tesseract engine)' />\n\
         <meta name='ocr-capabilities' content='ocr_page ocr_line ocrx_word' />\n</head>\n<body>\n",
    );
    for p in pages {
        o.push_str(p);
    }
    o.push_str("</body>\n</html>\n");
    o
}

#[cfg(test)]
mod lang_tests {
    use super::*;

    #[test]
    fn two_letter_codes_map_to_tesseract_names() {
        assert_eq!(canonical_lang("ro+en"), "ron+eng");
        assert_eq!(canonical_lang("HU"), "hun");
        assert_eq!(canonical_lang("ron+eng"), "ron+eng");
        assert_eq!(canonical_lang("script/Latin"), "script/Latin");
        assert_eq!(UI_LANGUAGES.len(), 8);
        assert!(install_hint("ron").contains("ron"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn osd_parsing() {
        let ok = "Page number: 0\nOrientation in degrees: 270\nRotate: 90\nOrientation confidence: 7.31\nScript: Latin\n";
        assert_eq!(parse_osd(ok), Some(90));
        assert_eq!(parse_osd(&ok.replace("7.31", "0.4")), None);
        assert_eq!(parse_osd("Warning. Invalid resolution 0 dpi.\n"), None);
        assert_eq!(parse_osd(&ok.replace("Rotate: 90", "Rotate: 45")), None);
    }

    #[test]
    fn explicit_paths_are_left_alone() {
        let p = std::path::Path::new("/opt/custom/tesseract");
        assert_eq!(resolve_tesseract(p), p);
        // A bare name that exists nowhere stays as is (the caller then reports "not found").
        let q = std::path::Path::new("definitely-not-installed-xyz");
        assert_eq!(resolve_tesseract(q), q);
    }

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
    fn upscale_rules() {
        assert_eq!(upscale_factor(300, 2000, 3000), 1);
        assert_eq!(upscale_factor(150, 1200, 1800), 2);
        assert_eq!(upscale_factor(100, 800, 1100), 3);
        assert_eq!(upscale_factor(36, 100, 100), 4);
        assert_eq!(upscale_factor(150, 7000, 9000), 1, "too big to enlarge");
        assert_eq!(upscale_factor(0, 10, 10), 1);
    }

    #[test]
    fn text_and_hocr_output() {
        let w = |t: &str, x, line| Word {
            text: t.into(),
            x,
            y: 10,
            w: 20,
            h: 8,
            conf: 91.4,
            line,
        };
        let words = vec![w("Hello", 5, 0), w("<&>", 40, 0), w("next", 5, 1)];
        assert_eq!(words_to_text(&words), "Hello <&>\nnext");
        assert_eq!(words_to_text(&[]), "");
        let page = hocr_page(&words, 100, 50, 300, 1);
        assert!(page.contains("bbox 0 0 100 50; ppageno 0; scan_res 300 300"));
        assert!(page.contains("id='line_1_2'") && page.contains("x_wconf 91"));
        assert!(page.contains("&lt;&amp;&gt;") && !page.contains("<&>"));
        let doc = hocr_document(&[page]);
        assert!(
            doc.starts_with("<?xml") && doc.ends_with("</html>\n") && doc.contains("ocrx_word")
        );
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
