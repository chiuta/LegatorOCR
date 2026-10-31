use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use croitor_core::{process_page, OutputMode, Page, Settings, SplitMode};
use croitor_ocr::OcrOptions;
use croitor_pdf::{Compress, PdfWriter};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

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

/// CROITOR — clean-room scan post-processor: split, deskew, crop, clean,
/// then export to PDF with optional OCR text layer.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Input page images (PNG, JPEG, TIFF, BMP), in reading order.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// Directory for processed page images.
    #[arg(short = 'o', long, default_value = "croitor-out")]
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

    /// Scan resolution of the inputs.
    #[arg(long, default_value_t = 300)]
    dpi: u32,

    #[arg(long, value_enum, default_value = "color")]
    mode: Mode,

    #[arg(long, value_enum, default_value = "auto")]
    split: Split,

    /// Disable skew correction.
    #[arg(long)]
    no_deskew: bool,

    /// Maximum skew searched, in degrees.
    #[arg(long, default_value_t = 7.0)]
    max_skew: f32,

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
}

fn parse_page_size(s: &str) -> Result<(f32, f32)> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "a4" => (210.0, 297.0),
        "a5" => (148.0, 210.0),
        "letter" => (215.9, 279.4),
        other => {
            let (w, h) = other.split_once('x').context("page size must be a4|a5|letter|WxH")?;
            (w.parse()?, h.parse()?)
        }
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if !matches!(cli.rotate, 0 | 90 | 180 | 270) {
        bail!("--rotate must be 0, 90, 180 or 270");
    }
    let settings = Settings {
        dpi: cli.dpi,
        rotate_quarters: (cli.rotate / 90) as u8,
        auto_orient: !cli.no_auto_orient,
        split: match cli.split {
            Split::Auto => SplitMode::Auto,
            Split::Off => SplitMode::Off,
            Split::Middle => SplitMode::Middle,
        },
        deskew: !cli.no_deskew,
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
        let v = croitor_ocr::check_engine(&ocr_opts)?;
        let installed = croitor_ocr::list_languages(&ocr_opts)?;
        for l in cli.lang.split('+') {
            if !installed.iter().any(|i| i == l) {
                bail!("OCR language '{l}' is not installed for {v} (installed: {})", installed.join(", "));
            }
        }
        eprintln!("OCR engine: {v}, language: {}", cli.lang);
    }
    if cli.pdf.is_none() && matches!(cli.format, Fmt::None) {
        bail!("nothing to do: give --pdf and/or an image --format");
    }

    // --- process sheets in parallel (order preserved) ----------------------
    let sheets: Vec<Result<Vec<Page>>> = cli
        .inputs
        .par_iter()
        .map(|p| {
            let img = image::open(p).with_context(|| format!("reading {}", p.display()))?;
            Ok(process_page(&img, &settings))
        })
        .collect();

    let mut pages: Vec<(String, Page)> = Vec::new();
    for (path, res) in cli.inputs.iter().zip(sheets) {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("page").to_string();
        for pg in res? {
            if cli.skip_blank && pg.blank {
                eprintln!("{}: blank, skipped", path.display());
                continue;
            }
            let name = match pg.split_index {
                Some(0) => format!("{stem}_L"),
                Some(_) => format!("{stem}_R"),
                None => stem.clone(),
            };
            eprintln!(
                "{name}: {}x{} px, skew {:+.2}°{}",
                pg.image.width(),
                pg.image.height(),
                pg.skew_deg,
                if pg.blank { ", blank" } else { "" }
            );
            pages.push((name, pg));
        }
    }
    if pages.is_empty() {
        bail!("no pages produced");
    }

    // --- processed images -------------------------------------------------
    if !matches!(cli.format, Fmt::None) {
        std::fs::create_dir_all(&cli.out_dir)?;
        let ext = if matches!(cli.format, Fmt::Png) { "png" } else { "tif" };
        pages.par_iter().try_for_each(|(name, pg)| -> Result<()> {
            let path = cli.out_dir.join(format!("{name}.{ext}"));
            pg.image.to_dynamic().save(&path).with_context(|| path.display().to_string())?;
            Ok(())
        })?;
        eprintln!("wrote {} page image(s) to {}", pages.len(), cli.out_dir.display());
    }

    // --- OCR + PDF --------------------------------------------------------
    if let Some(pdf_path) = &cli.pdf {
        let words: Vec<Vec<croitor_core::Word>> = if cli.ocr {
            pages
                .par_iter()
                .map(|(name, pg)| {
                    croitor_ocr::recognize(&pg.image, pg.dpi, &ocr_opts)
                        .with_context(|| format!("OCR of {name}"))
                })
                .collect::<Result<_>>()?
        } else {
            vec![Vec::new(); pages.len()]
        };
        let mut w = PdfWriter::new(if cli.pdf_quality == 0 {
            Compress::Flate
        } else {
            Compress::Jpeg(cli.pdf_quality.min(100))
        });
        if let Some(t) = pdf_path.file_stem().and_then(|s| s.to_str()) {
            w.set_title(t);
        }
        for ((_, pg), ws) in pages.iter().zip(&words) {
            w.add_page(pg, ws);
        }
        write_atomic(pdf_path, &w.finish())?;
        let nwords: usize = words.iter().map(Vec::len).sum();
        eprintln!(
            "wrote {} ({} page(s){})",
            pdf_path.display(),
            pages.len(),
            if cli.ocr { format!(", {nwords} OCR words") } else { String::new() }
        );
    }
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("pdf.part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
