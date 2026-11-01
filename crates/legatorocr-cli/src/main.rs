use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use legatorocr_core::{HAlign, OutputMode, Settings, SplitMode, Threshold, VAlign};
use legatorocr_engine::project::{Project, SheetAdj};
use legatorocr_engine::{
    check_ocr,
    files::{collect_images, detect_dpi},
    ocr::OcrOptions,
    run_export, write_atomic, ExportJob, PdfExport, SheetInput,
};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex};

#[derive(Copy, Clone, ValueEnum)]
enum Mode {
    Color,
    Gray,
    Bw,
    /// 1-bit text + colour picture patches.
    Mixed,
    /// 1-bit text over a low-resolution colour background (smallest colour PDFs).
    Hybrid,
}

#[derive(Copy, Clone, ValueEnum)]
enum Split {
    Auto,
    Off,
    Middle,
}

#[derive(Copy, Clone, ValueEnum)]
enum Fmt {
    Png,
    Tiff,
    Jpeg,
    None,
}

/// LegatorOCR — clean-room scan post-processor: split, deskew, crop, clean,
/// then export to PDF with optional OCR text layer.
#[derive(Parser)]
#[command(name = "legatorocr", version, about)]
struct Cli {
    /// Input page images (PNG, JPEG, TIFF, BMP) and scanned PDFs, in reading order.
    #[arg(required_unless_present_any = ["project", "list_profiles", "save_profile", "make_separator", "ocr_languages"])]
    inputs: Vec<PathBuf>,

    /// Load settings and page list (with per-page adjustments) from a `.lgr` project
    /// saved by the GUI or `--save-project`. Its processing settings replace the
    /// command-line ones; output options (`--pdf`, `--ocr`, …) still apply.
    #[arg(long)]
    project: Option<PathBuf>,

    /// Start from a named profile (built-in or saved in the user's profiles folder);
    /// like `--project`, its processing settings replace the command-line ones.
    #[arg(long)]
    profile: Option<String>,

    /// List the available profiles and exit.
    #[arg(long)]
    list_profiles: bool,

    /// Save the effective processing settings as a user profile with this name.
    #[arg(long)]
    save_profile: Option<String>,

    /// Write the effective settings and page list to a `.lgr` project file.
    #[arg(long)]
    save_project: Option<PathBuf>,

    /// Directory for processed page images.
    #[arg(short = 'o', long, default_value = "legatorocr-out")]
    out_dir: PathBuf,

    /// Image format for processed pages (`none` = PDF only).
    #[arg(long, value_enum, default_value = "png")]
    format: Fmt,

    /// JPEG quality for `--format jpeg` page images.
    #[arg(long, default_value_t = 90)]
    jpeg_quality: u8,

    /// Only report pages whose size differs clearly from the rest of the book
    /// (missing corners, foldouts, wrong-size inserts) and exit.
    #[arg(long)]
    report_outliers: bool,

    /// Write an HTML contact sheet (thumbnails, sizes, skew; odd pages highlighted).
    #[arg(long)]
    report: Option<PathBuf>,

    /// Write all (black & white) pages into one multi-page Group 4 TIFF.
    #[arg(long)]
    multitiff: Option<PathBuf>,

    /// Write all processed pages into one PDF.
    #[arg(long)]
    pdf: Option<PathBuf>,

    /// JPEG quality inside the PDF for colour/gray pages (0 = lossless Flate).
    #[arg(long, default_value_t = 80)]
    pdf_quality: u8,

    /// PDF document information: author, subject, keywords.
    #[arg(long)]
    author: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long)]
    keywords: Option<String>,

    /// Also write the recognised text of all pages to this file (needs --ocr).
    #[arg(long)]
    txt: Option<PathBuf>,

    /// Also write an hOCR file with word boxes and confidences (needs --ocr).
    #[arg(long)]
    hocr: Option<PathBuf>,

    /// Also write the recognised text as an editable Word document, one page break per
    /// scanned page (needs --ocr).
    #[arg(long)]
    docx: Option<PathBuf>,

    /// SEARCH instead of processing: find this phrase in the given files and folders (PDF
    /// text layers, .txt, .hocr/.html, .docx, and all of these inside .zip). Ignores case
    /// and diacritics (`mobila` finds `Mobilă`, also across a line break). Exit status 1
    /// when nothing is found.
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,

    /// With --search: TEXT is a regular expression, matched line by line.
    #[arg(long, requires = "search")]
    search_regex: bool,

    /// With --search: match case and diacritics exactly.
    #[arg(long, requires = "search")]
    search_exact: bool,

    /// With --search: stop after this many hits.
    #[arg(long, requires = "search", value_name = "N", default_value_t = 0)]
    search_max: usize,

    /// Drop a page that is almost identical to the previous one (double feed).
    #[arg(long)]
    skip_duplicates: bool,

    /// Treat blank pages as separators and write one PDF per document
    /// (`name_001.pdf`, `name_002.pdf`, …). Needs --pdf.
    #[arg(long)]
    split_on_blank: bool,

    /// Treat pages carrying a bar code (Code 128, EAN-13, ITF or Code 39) as separators (the page is dropped) and
    /// write one PDF per document, named after the bar code (`name_001_<code>.pdf`).
    /// Print separators with --make-separator. Needs --pdf.
    #[arg(long)]
    split_on_barcode: bool,

    /// Compare two text files, REFERENCE then OCR-OUTPUT (the two INPUTS), and print the
    /// character and word error rates. Case counts, whitespace and ş/ţ vs ș/ț do not.
    #[arg(long)]
    compare_text: bool,

    /// Show which of the eight interface languages (ro en fr it es pt de hu) have a
    /// recognition model installed, and how to get the missing ones; then exit.
    #[arg(long)]
    ocr_languages: bool,

    /// Add one PDF outline entry (bookmark) per source file.
    #[arg(long)]
    bookmarks: bool,

    /// Write PDF/A-2b structure (embedded text font, XMP metadata, sRGB output intent).
    #[arg(long)]
    pdfa: bool,

    /// Label the first N pages i, ii, iii … in PDF viewers; numbering restarts at 1 after them.
    #[arg(long, value_name = "N", default_value_t = 0)]
    front_matter: u32,

    /// Write a printable Code 128 separator sheet (PNG, 300 dpi A4) for this text and exit.
    #[arg(long, value_name = "TEXT")]
    make_separator: Option<String>,

    /// Choose the JPEG quality so the PDF stays within this many megabytes (estimated
    /// from a sample of pages). Needs --pdf.
    #[arg(long)]
    target_size: Option<f32>,

    /// Run OCR and embed an invisible, searchable text layer in the PDF.
    #[arg(long)]
    ocr: bool,

    /// Tesseract language(s), e.g. `ron+eng`.
    #[arg(long, default_value = "eng")]
    lang: String,

    /// Tesseract page-segmentation mode.
    #[arg(long, default_value_t = 3)]
    psm: u32,

    /// Drop OCR words below this confidence (0–100).
    #[arg(long, default_value_t = 0.0)]
    min_conf: f32,

    /// OCR engine: `auto` (Tesseract when installed with the languages, else the built-in
    /// native engine), `native` (built-in, nothing to install) or `tesseract`.
    #[arg(long, default_value = "auto")]
    engine: legatorocr_engine::ocr::Engine,

    /// Path to the tesseract executable.
    #[arg(long, default_value = "tesseract")]
    tesseract: PathBuf,

    /// tessdata directory (sets TESSDATA_PREFIX).
    #[arg(long)]
    tessdata: Option<PathBuf>,

    /// Scan resolution of the inputs. Default: read from each PNG/JPEG header
    /// when it is 150–1200 dpi, otherwise 300.
    #[arg(long)]
    dpi: Option<u32>,

    #[arg(long, value_enum, default_value = "color")]
    mode: Mode,

    #[arg(long, value_enum, default_value = "auto")]
    split: Split,

    /// Disable skew correction.
    #[arg(long)]
    no_deskew: bool,

    /// Fixed skew correction in degrees (clockwise positive); overrides detection.
    #[arg(long, allow_negative_numbers = true)]
    skew: Option<f32>,

    /// Maximum skew searched, in degrees.
    #[arg(long, default_value_t = 7.0)]
    max_skew: f32,

    /// Use Tesseract's orientation detection (needs the `osd` language data) to turn
    /// pages upright, including 90° vs 270° and upside-down pages.
    #[arg(long)]
    auto_rotate: bool,

    /// Disable automatic detection of sideways pages.
    #[arg(long)]
    no_auto_orient: bool,

    /// Rotate inputs by 0/90/180/270 degrees clockwise before anything else.
    #[arg(long, default_value_t = 0)]
    rotate: u16,

    /// Margin around the content in mm (all sides).
    #[arg(long, default_value_t = 5.0)]
    margin: f32,

    /// Fixed page size: a4, a5, letter, or WxH in mm (e.g. 148x210).
    #[arg(long)]
    page_size: Option<String>,

    /// Sauvola sensitivity for black & white output.
    #[arg(long, default_value_t = 0.3)]
    sauvola_k: f32,

    /// Binarisation method: sauvola, wolf, niblack, nick, bradley, otsu, auto (per page).
    #[arg(long, default_value = "sauvola")]
    threshold: String,

    /// Invert negative scans (light text on a dark page).
    #[arg(long)]
    invert: bool,

    /// Stretch contrast so paper turns white and ink black.
    #[arg(long)]
    auto_levels: bool,

    /// Unsharp-mask amount, 0–2 (0 = off).
    #[arg(long, default_value_t = 0.0)]
    sharpen: f32,

    /// Erase punch / binder holes near the page edge.
    #[arg(long)]
    punch_holes: bool,

    /// Erase fingers holding the page at the border.
    #[arg(long)]
    fingers: bool,

    /// Erase ruled lines (black & white / mixed / hybrid output).
    #[arg(long)]
    remove_lines: bool,

    /// Paint a rectangle with the paper colour on every page: X,Y,WIDTH,HEIGHT in mm from the
    /// top-left (repeatable). For stamps, notes, scanner marks.
    #[arg(long, value_name = "X,Y,W,H")]
    erase: Vec<String>,

    /// Always keep this rectangle as a colour picture (mixed / hybrid output): X,Y,WIDTH,HEIGHT in
    /// mm from the top-left of the deskewed page, like --erase (repeatable). For photos the
    /// automatic detector misses.
    #[arg(long, value_name = "X,Y,W,H")]
    picture_zone: Vec<String>,

    /// Drop one printed colour: off, red, green, blue.
    #[arg(long, default_value = "off")]
    dropout: String,

    /// Fade print showing through thin paper (also levels the lighting).
    #[arg(long)]
    show_through: bool,

    /// Straighten pages photographed at an angle (phone photos on a darker desk).
    #[arg(long)]
    perspective: bool,

    /// Smooth halftone screens of printed photographs.
    #[arg(long)]
    descreen: bool,

    /// Despeckle strength 0.25–4 (1 = default).
    #[arg(long, default_value_t = 1.0)]
    despeckle_strength: f32,

    /// Even out uneven lighting / shadows in colour, gray and hybrid output.
    #[arg(long)]
    normalize: bool,

    /// Straighten curved text lines (page curl).
    #[arg(long)]
    dewarp: bool,

    /// Downsample output to this dpi (never upsamples).
    #[arg(long)]
    output_dpi: Option<u32>,

    /// Picture detection sensitivity for mixed/hybrid (0–1).
    #[arg(long, default_value_t = 0.5)]
    picture_sensitivity: f32,

    /// Resolution of the hybrid background layer.
    #[arg(long, default_value_t = 100)]
    hybrid_dpi: u32,

    /// Where content sits on a fixed-size page: e.g. `center`, `top-left`, `bottom`.
    #[arg(long, default_value = "center")]
    align: String,

    /// Give every page the same size (the largest ordinary page of the book).
    #[arg(long)]
    uniform_size: bool,

    /// Keep tiny specks in black & white output.
    #[arg(long)]
    no_despeckle: bool,

    /// Drop pages where no content was found.
    #[arg(long)]
    skip_blank: bool,

    /// Skip unreadable/corrupt inputs instead of stopping (exit status 2 if any were skipped).
    #[arg(long)]
    keep_going: bool,
}

fn in_range(name: &str, v: f32, lo: f32, hi: f32) -> Result<()> {
    if !v.is_finite() || v < lo || v > hi {
        bail!("--{name} must be between {lo} and {hi}");
    }
    Ok(())
}

/// Output file stems: the input stem, made unique (`001_0003`) when two inputs
/// from different folders share one, so no processed page overwrites another.
fn output_stems(paths: &[PathBuf]) -> Vec<String> {
    let stems: Vec<String> = paths
        .iter()
        .map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page")
                .to_string()
        })
        .collect();
    stems
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if stems.iter().filter(|o| o.eq_ignore_ascii_case(s)).count() > 1 {
                format!("{s}_{:04}", i + 1)
            } else {
                s.clone()
            }
        })
        .collect()
}

/// The first page-image path (`stem`, `stem_L`, `stem_R` + extension, in `out_dir`) that is one of
/// the input files themselves, compared by canonical path (and case-insensitively, for the
/// case-folding file systems of Windows and macOS).
fn output_clobbers_input(out_dir: &Path, stems: &[String], ext: &str, inputs: &[PathBuf]) -> Option<PathBuf> {
    let dir = out_dir.canonicalize().ok()?;
    let canon: Vec<String> = inputs
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();
    for stem in stems {
        for suffix in ["", "_L", "_R"] {
            let cand = dir.join(format!("{stem}{suffix}.{ext}"));
            if canon.contains(&cand.to_string_lossy().to_lowercase()) {
                return Some(cand);
            }
        }
    }
    None
}

fn parse_page_size(s: &str) -> Result<(f32, f32)> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "a4" => (210.0, 297.0),
        "a5" => (148.0, 210.0),
        "letter" => (215.9, 279.4),
        other => {
            let (w, h) = other
                .split_once('x')
                .context("page size must be a4|a5|letter|WxH")?;
            let (w, h): (f32, f32) = (w.parse()?, h.parse()?);
            in_range("page-size width", w, 10.0, 2000.0)?;
            in_range("page-size height", h, 10.0, 2000.0)?;
            (w, h)
        }
    })
}

fn main() -> Result<()> {
    // `legatorocr … | head` closes our stdout early: end quietly like other Unix tools
    // instead of panicking inside println!.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default_hook(info);
    }));
    let r = real_main();
    legatorocr_engine::archive::cleanup();
    r
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    if cli.list_profiles {
        for (n, d) in legatorocr_engine::profiles::list() {
            println!("{n:<20} {d}");
        }
        return Ok(());
    }
    if cli.compare_text {
        let [r, h] = cli.inputs.as_slice() else {
            bail!("--compare-text needs exactly two files: REFERENCE and OCR-OUTPUT");
        };
        let rd = |p: &PathBuf| {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
        };
        let x = legatorocr_engine::eval::rates(&rd(r)?, &rd(h)?);
        println!(
            "CER {:.2}%  WER {:.2}%  (reference: {} characters, {} words)",
            x.cer * 100.0,
            x.wer * 100.0,
            x.ref_chars,
            x.ref_words
        );
        return Ok(());
    }
    if cli.ocr_languages {
        let o = OcrOptions {
            lang: "eng".into(),
            ..OcrOptions::default()
        };
        let v = check_ocr(&o).context("no OCR engine is available")?;
        let have = legatorocr_engine::ocr::list_languages(&o)?;
        println!("{v}");
        for (iso, tess, name) in legatorocr_engine::ocr::UI_LANGUAGES {
            if have.iter().any(|h| h == tess) {
                println!("{iso}  {tess}  {name:<11} installed");
            } else {
                println!(
                    "{iso}  {tess}  {name:<11} MISSING — {}",
                    legatorocr_engine::ocr::install_hint(tess)
                );
            }
        }
        return Ok(());
    }
    if let Some(q) = &cli.search {
        return run_search(&cli, q);
    }
    if let Some(text) = &cli.make_separator {
        let Some(bc) = legatorocr_core::barcode::render(text, 4, 500) else {
            bail!("--make-separator: use 1–80 printable ASCII characters");
        };
        if bc.width() > 2480 {
            bail!(
                "--make-separator: this text needs a {}-px wide code, more than the 2480-px page; use at most about 45 characters",
                bc.width()
            );
        }
        let mut sheet = image::GrayImage::from_pixel(2480, 3508, image::Luma([255]));
        let (ox, oy) = (
            2480u32.saturating_sub(bc.width()) / 2,
            (3508 - bc.height()) / 2,
        );
        image::imageops::replace(&mut sheet, &bc, ox as i64, oy as i64);
        let safe: String = text
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(40)
            .collect();
        let out = std::path::PathBuf::from(format!("separator_{safe}.png"));
        sheet.save(&out)?;
        eprintln!("wrote {} (A4, 300 dpi: print at 100 %)", out.display());
        return Ok(());
    }
    if !matches!(cli.rotate, 0 | 90 | 180 | 270) {
        bail!("--rotate must be 0, 90, 180 or 270");
    }
    if cli.dpi.is_some_and(|d| !(36..=2400).contains(&d)) {
        bail!("--dpi must be between 36 and 2400");
    }
    if cli.pdf_quality > 100 {
        bail!("--pdf-quality must be 0..=100");
    }
    if !(1..=13).contains(&cli.psm) {
        bail!("--psm must be 1..=13 (0 is orientation-only and yields no text)");
    }
    in_range("margin", cli.margin, 0.0, 200.0)?;
    in_range("max-skew", cli.max_skew, 0.0, 15.0)?;
    in_range("sauvola-k", cli.sauvola_k, 0.0, 1.0)?;
    in_range("min-conf", cli.min_conf, 0.0, 100.0)?;
    if let Some(s) = cli.skew {
        in_range("skew", s, -45.0, 45.0)?;
    }
    let threshold = Threshold::from_name(&cli.threshold).with_context(|| {
        format!(
            "unknown --threshold '{}' (sauvola, wolf, niblack, nick, bradley, otsu, auto)",
            cli.threshold
        )
    })?;
    in_range("picture-sensitivity", cli.picture_sensitivity, 0.0, 1.0)?;
    in_range("sharpen", cli.sharpen, 0.0, 2.0)?;
    in_range("despeckle-strength", cli.despeckle_strength, 0.25, 4.0)?;
    if let Some(d) = cli.output_dpi {
        if !(36..=1200).contains(&d) {
            bail!("--output-dpi must be between 36 and 1200 (got {d})");
        }
    }
    if !(36..=300).contains(&cli.hybrid_dpi) {
        bail!("--hybrid-dpi must be between 36 and 300 (got {})", cli.hybrid_dpi);
    }
    if !(1..=100).contains(&cli.jpeg_quality) {
        bail!("--jpeg-quality must be between 1 and 100 (got {})", cli.jpeg_quality);
    }
    let align = parse_align(&cli.align)?;
    let settings = Settings {
        dpi: cli.dpi.unwrap_or(300),
        rotate_quarters: (cli.rotate / 90) as u8,
        auto_orient: !cli.no_auto_orient,
        split: match cli.split {
            Split::Auto => SplitMode::Auto,
            Split::Off => SplitMode::Off,
            Split::Middle => SplitMode::Middle,
        },
        deskew: !cli.no_deskew,
        manual_skew_deg: cli.skew,
        max_skew_deg: cli.max_skew,
        margins_mm: [cli.margin; 4],
        page_size_mm: cli.page_size.as_deref().map(parse_page_size).transpose()?,
        mode: match cli.mode {
            Mode::Color => OutputMode::Color,
            Mode::Gray => OutputMode::Gray,
            Mode::Bw => OutputMode::BlackWhite,
            Mode::Mixed => OutputMode::Mixed,
            Mode::Hybrid => OutputMode::Hybrid,
        },
        sauvola_k: cli.sauvola_k,
        despeckle: !cli.no_despeckle,
        dewarp: cli.dewarp,
        content_adjust_mm: [0.0; 4],
        align,
        threshold,
        normalize_bg: cli.normalize,
        output_dpi: cli.output_dpi,
        picture_sensitivity: cli.picture_sensitivity,
        hybrid_bg_dpi: cli.hybrid_dpi,
        auto_invert: cli.invert,
        auto_levels: cli.auto_levels,
        sharpen: cli.sharpen,
        punch_holes: cli.punch_holes,
        fingers: cli.fingers,
        remove_lines: cli.remove_lines,
        sheet_erase_zones: Vec::new(),
        erase_zones: legatorocr_core::cleanup::parse_zones(&cli.erase.join(";")).ok_or_else(
            || anyhow::anyhow!("--erase needs X,Y,WIDTH,HEIGHT in mm (width and height > 0)"),
        )?,
        picture_zones: legatorocr_core::cleanup::parse_zones(&cli.picture_zone.join(";"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--picture-zone needs X,Y,WIDTH,HEIGHT in mm (width and height > 0)"
                )
            })?,
        dropout: legatorocr_core::Dropout::from_name(&cli.dropout)
            .ok_or_else(|| anyhow::anyhow!("--dropout must be off, red, green or blue"))?,
        despeckle_strength: cli.despeckle_strength,
        show_through: cli.show_through,
        descreen: cli.descreen,
        perspective: cli.perspective,
    };

    let project = cli.project.as_deref().map(Project::load).transpose()?;
    let profile = cli
        .profile
        .as_deref()
        .map(legatorocr_engine::profiles::load)
        .transpose()?;
    let settings = match (&project, &profile) {
        (Some(p), _) => p.settings.clone(),
        (None, Some(p)) => p.settings.clone(),
        _ => settings,
    };
    if let Some(name) = &cli.save_profile {
        let p = Project {
            settings: settings.clone(),
            extra: vec![("pdf_quality".into(), cli.pdf_quality.to_string())],
            sheets: Vec::new(),
        };
        let f = legatorocr_engine::profiles::save_user(name, &p)?;
        eprintln!("profile '{name}' saved: {}", f.display());
        if cli.inputs.is_empty() && project.is_none() && cli.pdf.is_none() {
            return Ok(());
        }
    }

    let ocr_opts = OcrOptions {
        engine: cli.engine,
        lang: legatorocr_engine::ocr::canonical_lang(&cli.lang),
        psm: cli.psm,
        min_conf: cli.min_conf,
        tesseract: cli.tesseract.clone(),
        tessdata: cli.tessdata.clone(),
    };
    if cli.ocr {
        let v = check_ocr(&ocr_opts)?;
        eprintln!("OCR engine: {v}, language: {}", cli.lang);
    }
    if cli.auto_rotate {
        let o = OcrOptions {
            lang: "osd".into(),
            ..ocr_opts.clone()
        };
        check_ocr(&o).context("--auto-rotate needs Tesseract with the 'osd' language data")?;
    }
    if (cli.txt.is_some() || cli.hocr.is_some() || cli.docx.is_some()) && !cli.ocr {
        bail!("--txt, --hocr and --docx need --ocr");
    }
    for p in cli.txt.iter().chain(cli.hocr.iter()).chain(cli.docx.iter()) {
        legatorocr_engine::check_writable(p)?;
    }
    if cli.pdf.is_none()
        && matches!(cli.format, Fmt::None)
        && cli.txt.is_none()
        && cli.hocr.is_none()
        && cli.docx.is_none()
        && cli.report.is_none()
        && cli.multitiff.is_none()
        && !cli.report_outliers
    {
        bail!("nothing to do: give --pdf and/or an image --format");
    }

    // --- run: process → (save images) → OCR → PDF ---------------------------
    // Fail now, not after an hour of OCR: make sure the outputs can be written.
    if let Some(pdf) = &cli.pdf {
        legatorocr_engine::check_writable(pdf)?;
    }
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut adjusts: Vec<SheetAdj> = Vec::new();
    if let Some(pr) = &project {
        for (p, a) in &pr.sheets {
            inputs.push(p.clone());
            adjusts.push(a.clone());
        }
    }
    for p in &cli.inputs {
        if p.is_dir() {
            let found = collect_images(std::slice::from_ref(p));
            if found.is_empty() {
                bail!("no images (png, jpg, tif, bmp) in folder {}", p.display());
            }
            inputs.extend(found);
        } else {
            // multi-page TIFFs become one sheet per page; anything else is passed on as is
            let pages = collect_images(std::slice::from_ref(p));
            if pages.is_empty() {
                inputs.push(p.clone());
            } else {
                inputs.extend(pages);
            }
        }
    }
    adjusts.resize(inputs.len(), SheetAdj::default());
    if inputs.is_empty() {
        bail!("the project lists no pages");
    }
    if let Some(path) = &cli.save_project {
        let mut pr = Project {
            settings: settings.clone(),
            extra: Vec::new(),
            sheets: Vec::new(),
        };
        pr.extra
            .push(("pdf_quality".into(), cli.pdf_quality.to_string()));
        pr.extra
            .push(("skip_blank".into(), cli.skip_blank.to_string()));
        pr.extra
            .push(("uniform_size".into(), cli.uniform_size.to_string()));
        for (p, a) in inputs.iter().zip(&adjusts) {
            pr.sheets.push((
                std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()),
                a.clone(),
            ));
        }
        pr.save(path)?;
        eprintln!("project saved: {}", path.display());
    }
    let stems = output_stems(&inputs);
    let from_project = project.is_some();
    let sheets: Vec<SheetInput> = inputs
        .iter()
        .zip(&adjusts)
        .map(|(p, a)| {
            // a project fixes the resolution; otherwise read it from the image header
            let dpi = cli
                .dpi
                .or_else(|| if from_project { None } else { detect_dpi(p) })
                .unwrap_or(settings.dpi);
            SheetInput {
                path: p.clone(),
                settings: a.apply(&Settings {
                    dpi,
                    ..settings.clone()
                }),
            }
        })
        .collect();
    if cli.report_outliers {
        let out = legatorocr_engine::find_outliers(
            &sheets,
            cli.auto_rotate.then_some(&ocr_opts),
            &AtomicBool::new(false),
        )?;
        if out.is_empty() {
            println!("no outlier pages among {}", sheets.len());
        }
        for (i, (w, h)) in &out {
            println!("{}: {:.0} x {:.0} mm", inputs[*i].display(), w, h);
        }
        return Ok(());
    }
    let save_images = !matches!(cli.format, Fmt::None);
    if save_images {
        std::fs::create_dir_all(&cli.out_dir)?;
    }
    let ext = match cli.format {
        Fmt::Png => "png",
        Fmt::Jpeg => "jpg",
        _ => "tif",
    };
    if save_images {
        if let Some(clash) = output_clobbers_input(&cli.out_dir, &stems, ext, &inputs) {
            bail!(
                "the page image {} would overwrite an input scan; choose another folder with -o (or --format none)",
                clash.display()
            );
        }
    }
    if cli.multitiff.is_some() && !matches!(cli.mode, Mode::Bw) {
        bail!("--multitiff needs black & white pages (use --mode bw)");
    }
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    if cli.multitiff.is_some()
        && (cli.skip_duplicates || cli.split_on_blank || cli.split_on_barcode || cli.skip_blank)
    {
        bail!("--multitiff writes every processed page; do not combine it with --skip-duplicates, --split-on-blank or --skip-blank");
    }
    type Key = (usize, u8);
    let report_pages: Mutex<Vec<(Key, legatorocr_engine::report::ReportPage)>> =
        Mutex::new(Vec::new());
    let tiff_pages: Mutex<Vec<(Key, Option<legatorocr_engine::g4::G4Strip>)>> =
        Mutex::new(Vec::new());
    let mut pdf_quality = cli.pdf_quality;
    if let Some(mb) = cli.target_size {
        if cli.pdf.is_none() || !(mb.is_finite() && mb > 0.0) {
            bail!("--target-size needs --pdf and a positive number of megabytes");
        }
        if pdf_quality > 0 {
            pdf_quality = legatorocr_engine::quality_for_target(
                &sheets,
                cli.auto_rotate.then_some(&ocr_opts),
                (mb as f64 * 1e6) as u64,
                &AtomicBool::new(false),
            )?;
            eprintln!("target {mb} MB: JPEG quality {pdf_quality}");
        }
    }
    if (cli.split_on_blank || cli.split_on_barcode) && cli.pdf.is_none() {
        bail!("--split-on-blank / --split-on-barcode need --pdf");
    }
    let job = ExportJob {
        sheets: &sheets,
        pdf: cli.pdf.as_ref().map(|pdf_path| PdfExport {
            quality: pdf_quality,
            ocr: cli.ocr.then(|| ocr_opts.clone()),
            title: pdf_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string),
            meta: legatorocr_engine::PdfMeta {
                author: cli.author.clone(),
                subject: cli.subject.clone(),
                keywords: cli.keywords.clone(),
                created_unix: Some(legatorocr_engine::now_unix()),
            },
            bookmarks: cli.bookmarks,
            front_matter: cli.front_matter,
            pdfa: cli.pdfa,
        }),
        text_ocr: cli.ocr.then(|| ocr_opts.clone()),
        skip_blank: cli.skip_blank,
        keep_going: cli.keep_going,
        osd: cli.auto_rotate.then(|| ocr_opts.clone()),
        uniform_size: cli.uniform_size,
        skip_duplicates: cli.skip_duplicates,
        split_on_blank: cli.split_on_blank,
        split_on_barcode: cli.split_on_barcode,
    };
    let res = run_export(
        &job,
        &AtomicBool::new(false),
        &|i, pg| {
            let stem = &stems[i];
            let name = match pg.split_index {
                Some(0) => format!("{stem}_L"),
                Some(_) => format!("{stem}_R"),
                None => stem.to_string(),
            };
            eprintln!(
                "{name}: {}x{} px @ {} dpi, skew {:+.2}°{}",
                pg.image.width(),
                pg.image.height(),
                pg.dpi,
                pg.skew_deg,
                if pg.blank { ", blank" } else { "" }
            );
            let key = (i, pg.split_index.unwrap_or(0));
            if cli.report.is_some() {
                report_pages.lock().unwrap().push((
                    key,
                    legatorocr_engine::report::ReportPage::from_page(name.clone(), pg),
                ));
            }
            if cli.multitiff.is_some() {
                let strip = match &pg.image {
                    legatorocr_core::PageImage::Bw(b) => {
                        Some(legatorocr_engine::g4::G4Strip::from_bitimage(b, pg.dpi))
                    }
                    _ => None,
                };
                tiff_pages.lock().unwrap().push((key, strip));
            }
            if save_images {
                let path = cli.out_dir.join(format!("{name}.{ext}"));
                if let Err(e) = legatorocr_engine::save_page_image_q(pg, &path, cli.jpeg_quality) {
                    first_err
                        .lock()
                        .unwrap()
                        .get_or_insert(format!("{}: {e}", path.display()));
                }
            }
        },
        &|_, _| {},
    )?;
    let save_error = first_err.into_inner().unwrap();
    for (p, e) in &res.failed {
        eprintln!("SKIPPED {}: {e}", p.display());
    }
    if res.skipped_blank > 0 {
        eprintln!("{} blank page(s) skipped", res.skipped_blank);
    }
    if save_images {
        eprintln!(
            "wrote {} page image(s) to {}",
            res.pages,
            cli.out_dir.display()
        );
    }
    if let Some(path) = &cli.report {
        let mut v = report_pages.into_inner().unwrap();
        v.sort_by_key(|(k, _)| *k);
        let pages: Vec<_> = v.into_iter().map(|(_, p)| p).collect();
        let (html, flagged) = legatorocr_engine::report::html(&pages);
        write_atomic(path, html.as_bytes())?;
        eprintln!(
            "wrote {} ({} page(s), {} flagged)",
            path.display(),
            pages.len(),
            flagged
        );
    }
    if let Some(path) = &cli.multitiff {
        let mut v = tiff_pages.into_inner().unwrap();
        v.sort_by_key(|(k, _)| *k);
        if v.iter().any(|(_, s)| s.is_none()) {
            bail!("--multitiff needs black & white pages (use --mode bw)");
        }
        let strips: Vec<_> = v.into_iter().filter_map(|(_, s)| s).collect();
        write_atomic(path, &legatorocr_engine::g4::tiff_g4_multi(&strips))?;
        eprintln!("wrote {} ({} page(s))", path.display(), strips.len());
    }
    if res.skipped_duplicates > 0 {
        eprintln!("{} duplicate page(s) skipped", res.skipped_duplicates);
    }
    let _ = &save_error; // reported after every other output is written
    if let (Some(pdf_path), true) = (&cli.pdf, !res.docs.is_empty()) {
        let stem = pdf_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document")
            .to_string();
        for (i, bytes) in res.docs.iter().enumerate() {
            let name = res.doc_names.get(i).and_then(|n| n.as_deref());
            let p = pdf_path.with_file_name(legatorocr_engine::doc_file_name(&stem, i, name));
            write_atomic(&p, bytes)?;
            eprintln!("wrote {}", p.display());
        }
    }
    if let (Some(pdf_path), Some(bytes)) = (&cli.pdf, &res.pdf) {
        write_atomic(pdf_path, bytes)?;
        eprintln!(
            "wrote {} ({} page(s){})",
            pdf_path.display(),
            res.pages,
            if cli.ocr {
                format!(", {} OCR words", res.words)
            } else {
                String::new()
            }
        );
    }
    if let (Some(p), Some(t)) = (&cli.txt, &res.text) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
    }
    if let (Some(p), Some(t)) = (&cli.hocr, &res.hocr) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
    }
    if let (Some(p), Some(t)) = (&cli.docx, res.paragraph_text.as_ref().or(res.text.as_ref())) {
        let pages: Vec<String> = t
            .split('\x0c')
            .map(|s| s.trim_start_matches('\n').to_string())
            .collect();
        let title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("LegatorOCR");
        write_atomic(p, &legatorocr_engine::docx::from_pages(&pages, title))?;
        eprintln!("wrote {}", p.display());
    }
    if let Some(e) = save_error {
        legatorocr_engine::archive::cleanup();
        bail!("saving page image failed (the PDF and text outputs above were written): {e}");
    }
    if !res.failed.is_empty() {
        eprintln!("{} input(s) could not be processed", res.failed.len());
        legatorocr_engine::archive::cleanup();
        std::process::exit(2);
    }
    Ok(())
}

fn run_search(cli: &Cli, query: &str) -> Result<()> {
    if cli.inputs.is_empty() {
        bail!("--search needs at least one file or folder to look in");
    }
    let o = legatorocr_engine::search::SearchOptions {
        regex: cli.search_regex,
        exact: cli.search_exact,
        max_hits: cli.search_max,
    };
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut write_err = None;
    let s = legatorocr_engine::search::search(&cli.inputs, query, &o, &mut |h| {
        use std::io::Write;
        let page = h.page.map(|p| format!(" p.{p}")).unwrap_or_default();
        if let Err(e) = writeln!(out, "{}{page}:{}: {}", h.source, h.line, h.text) {
            write_err.get_or_insert(e);
        }
    })?;
    use std::io::Write;
    let _ = out.flush();
    if let Some(e) = write_err {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(e.into());
        }
    }
    eprintln!(
        "{} hit(s) in {} file(s){}",
        s.hits,
        s.files,
        if s.truncated {
            " (stopped at the limit)"
        } else {
            ""
        }
    );
    if s.image_only_pdfs > 0 {
        eprintln!(
            "{} PDF(s) are scans without a text layer: run them through LegatorOCR with --ocr first",
            s.image_only_pdfs
        );
    }
    if s.unreadable > 0 {
        eprintln!("{} file(s) could not be read", s.unreadable);
    }
    if s.hits == 0 {
        legatorocr_engine::archive::cleanup();
        std::process::exit(1);
    }
    Ok(())
}

fn parse_align(s: &str) -> Result<(HAlign, VAlign)> {
    let (mut h, mut v) = (HAlign::Center, VAlign::Center);
    for part in s.to_ascii_lowercase().split('-') {
        match part {
            "center" | "centre" => {}
            "left" => h = HAlign::Left,
            "right" => h = HAlign::Right,
            "top" => v = VAlign::Top,
            "bottom" => v = VAlign::Bottom,
            _ => bail!("--align: '{s}' is not valid (e.g. center, top-left, bottom-right)"),
        }
    }
    Ok((h, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_stems_get_unique_names() {
        let v = output_stems(&["a/001.png".into(), "b/001.PNG".into(), "b/002.png".into()]);
        assert_eq!(v, ["001_0001", "001_0002", "002"]);
    }

    #[test]
    fn page_sizes() {
        assert_eq!(parse_page_size("A5").unwrap(), (148.0, 210.0));
        assert_eq!(parse_page_size("100x150").unwrap(), (100.0, 150.0));
        for bad in ["0x0", "100000x100000", "nanx5", "5", "axb"] {
            assert!(parse_page_size(bad).is_err(), "{bad}");
        }
    }
}
