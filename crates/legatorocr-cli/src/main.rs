use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use legatorocr_core::{OutputMode, Settings, SplitMode};
use legatorocr_engine::{
    check_ocr, files::{collect_images, detect_dpi}, ocr::OcrOptions, run_export, write_atomic, ExportJob, PdfExport, SheetInput,
};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Mutex};

#[derive(Copy, Clone, ValueEnum)]
enum Mode {
    Color,
    Gray,
    Bw,
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
    None,
}

/// LegatorOCR — clean-room scan post-processor: split, deskew, crop, clean,
/// then export to PDF with optional OCR text layer.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Input page images (PNG, JPEG, TIFF, BMP), in reading order.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// Directory for processed page images.
    #[arg(short = 'o', long, default_value = "legatorocr-out")]
    out_dir: PathBuf,

    /// Image format for processed pages (`none` = PDF only).
    #[arg(long, value_enum, default_value = "png")]
    format: Fmt,

    /// Write all processed pages into one PDF.
    #[arg(long)]
    pdf: Option<PathBuf>,

    /// JPEG quality inside the PDF for colour/gray pages (0 = lossless Flate).
    #[arg(long, default_value_t = 80)]
    pdf_quality: u8,

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
        .map(|p| p.file_stem().and_then(|s| s.to_str()).unwrap_or("page").to_string())
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

fn parse_page_size(s: &str) -> Result<(f32, f32)> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "a4" => (210.0, 297.0),
        "a5" => (148.0, 210.0),
        "letter" => (215.9, 279.4),
        other => {
            let (w, h) = other.split_once('x').context("page size must be a4|a5|letter|WxH")?;
            let (w, h): (f32, f32) = (w.parse()?, h.parse()?);
            in_range("page-size width", w, 10.0, 2000.0)?;
            in_range("page-size height", h, 10.0, 2000.0)?;
            (w, h)
        }
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
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
        },
        sauvola_k: cli.sauvola_k,
        despeckle: !cli.no_despeckle,
    };

    let ocr_opts = OcrOptions {
        lang: cli.lang.clone(),
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
        let o = OcrOptions { lang: "osd".into(), ..ocr_opts.clone() };
        check_ocr(&o).context("--auto-rotate needs Tesseract with the 'osd' language data")?;
    }
    if cli.pdf.is_none() && matches!(cli.format, Fmt::None) {
        bail!("nothing to do: give --pdf and/or an image --format");
    }

    // --- run: process → (save images) → OCR → PDF ---------------------------
    // Fail now, not after an hour of OCR: make sure the outputs can be written.
    if let Some(pdf) = &cli.pdf {
        legatorocr_engine::check_writable(pdf)?;
    }
    let mut inputs: Vec<PathBuf> = Vec::new();
    for p in &cli.inputs {
        if p.is_dir() {
            let found = collect_images(std::slice::from_ref(p));
            if found.is_empty() {
                bail!("no images (png, jpg, tif, bmp) in folder {}", p.display());
            }
            inputs.extend(found);
        } else {
            inputs.push(p.clone());
        }
    }
    let stems = output_stems(&inputs);
    let sheets: Vec<SheetInput> =
        inputs
            .iter()
            .map(|p| SheetInput {
                path: p.clone(),
                settings: Settings { dpi: cli.dpi.or_else(|| detect_dpi(p)).unwrap_or(300), ..settings.clone() },
            })
            .collect();
    let save_images = !matches!(cli.format, Fmt::None);
    if save_images {
        std::fs::create_dir_all(&cli.out_dir)?;
    }
    let ext = if matches!(cli.format, Fmt::Png) { "png" } else { "tif" };
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    let job = ExportJob {
        sheets: &sheets,
        pdf: cli.pdf.as_ref().map(|pdf_path| PdfExport {
            quality: cli.pdf_quality,
            ocr: cli.ocr.then(|| ocr_opts.clone()),
            title: pdf_path.file_stem().and_then(|s| s.to_str()).map(str::to_string),
        }),
        skip_blank: cli.skip_blank,
        keep_going: cli.keep_going,
        osd: cli.auto_rotate.then(|| ocr_opts.clone()),
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
            if save_images {
                let path = cli.out_dir.join(format!("{name}.{ext}"));
                if let Err(e) = pg.image.to_dynamic().save(&path) {
                    first_err.lock().unwrap().get_or_insert(format!("{}: {e}", path.display()));
                }
            }
        },
        &|_, _| {},
    )?;
    if let Some(e) = first_err.into_inner().unwrap() {
        bail!("saving image failed: {e}");
    }
    for (p, e) in &res.failed {
        eprintln!("SKIPPED {}: {e}", p.display());
    }
    if res.skipped_blank > 0 {
        eprintln!("{} blank page(s) skipped", res.skipped_blank);
    }
    if save_images {
        eprintln!("wrote {} page image(s) to {}", res.pages, cli.out_dir.display());
    }
    if let (Some(pdf_path), Some(bytes)) = (&cli.pdf, &res.pdf) {
        write_atomic(pdf_path, bytes)?;
        eprintln!(
            "wrote {} ({} page(s){})",
            pdf_path.display(),
            res.pages,
            if cli.ocr { format!(", {} OCR words", res.words) } else { String::new() }
        );
    }
    if !res.failed.is_empty() {
        eprintln!("{} input(s) could not be processed", res.failed.len());
        std::process::exit(2);
    }
    Ok(())
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
