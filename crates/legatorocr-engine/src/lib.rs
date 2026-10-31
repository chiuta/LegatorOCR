//! Shared orchestration used by the CLI and the GUI: process sheets in bounded
//! batches (so a 300-page colour book never sits in memory at once), OCR every
//! page, assemble the PDF, write files atomically.

use anyhow::{bail, Context, Result};
use image::DynamicImage;
use legatorocr_core::{process_page, Page, Settings, Word};
use legatorocr_ocr::OcrOptions;
use legatorocr_pdf::{Compress, PdfWriter};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub mod files;
pub mod profiles;
pub mod project;
pub use legatorocr_ocr as ocr;

pub use legatorocr_pdf::Meta as PdfMeta;

#[derive(Debug, Clone)]
pub struct PdfExport {
    /// JPEG quality for colour/gray pages; 0 = lossless Flate.
    pub quality: u8,
    /// `Some` → run OCR and embed an invisible text layer.
    pub ocr: Option<OcrOptions>,
    pub title: Option<String>,
    /// Author / subject / keywords / creation time for the document information.
    pub meta: PdfMeta,
}

impl Default for PdfExport {
    fn default() -> Self {
        Self {
            quality: 80,
            ocr: None,
            title: None,
            meta: PdfMeta::default(),
        }
    }
}

/// Seconds since the Unix epoch now (honours `SOURCE_DATE_EPOCH` for reproducible builds).
pub fn now_unix() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
}

/// Pages kept from one sheet (each with its OCR words) and how many blank ones were skipped.
type SheetOutput = (Vec<(Page, Vec<Word>)>, usize);

/// One scanned sheet (may yield two pages when it is a spread).
#[derive(Debug, Clone)]
pub struct SheetInput {
    pub path: PathBuf,
    pub settings: Settings,
}

pub struct ExportJob<'a> {
    pub sheets: &'a [SheetInput],
    /// `None` → only run the pipeline (e.g. to save page images via `on_page`).
    pub pdf: Option<PdfExport>,
    pub skip_blank: bool,
    /// `true` → a sheet that cannot be read/processed is recorded in
    /// [`ExportResult::failed`] and the rest continues; `false` → first error aborts.
    pub keep_going: bool,
    /// `Some` → use Tesseract OSD to turn each sheet upright before processing
    /// (resolves 90° vs 270° and upside-down pages that the built-in test cannot).
    pub osd: Option<OcrOptions>,
    /// `true` → give every page the same size (the largest ordinary page; outliers
    /// still grow rather than get cropped). Costs a measuring pass over all sheets.
    pub uniform_size: bool,
    /// `Some` → run OCR for [`ExportResult::text`] / [`ExportResult::hocr`] even when no PDF
    /// text layer is wanted (the PDF's own OCR options win when both are set).
    pub text_ocr: Option<OcrOptions>,
    /// Drop a page that is (almost) identical to the previous kept page (double feed).
    pub skip_duplicates: bool,
    /// Blank pages become separators: the PDF is cut there into several documents
    /// ([`ExportResult::docs`]); the separators themselves are dropped.
    pub split_on_blank: bool,
}

/// 96×128 grayscale thumbnail of a page, for duplicate detection.
pub fn page_thumb(pg: &Page) -> Vec<u8> {
    let g = pg.image.to_gray();
    image::imageops::resize(&g, 96, 128, image::imageops::FilterType::Triangle).into_raw()
}

/// Mean absolute difference (0–255) of two thumbnails; ≤ 2 means "the same page".
pub fn thumb_diff(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 255.0;
    }
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).sum::<u32>() as f32 / a.len() as f32
}

/// Pass 1 of uniform sizing: the common page size in mm, from the pages' natural sizes.
/// Pages more than 25 % larger than the median are treated as outliers.
pub fn measure_uniform_size(
    sheets: &[SheetInput],
    osd: Option<&OcrOptions>,
    cancel: &AtomicBool,
) -> Result<Option<(f32, f32)>> {
    let sizes: Vec<Vec<(f32, f32)>> = sheets
        .par_iter()
        .map(|sheet| {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            let img = load_image(&sheet.path)
                .with_context(|| format!("reading {}", sheet.path.display()))?;
            let (img, settings) = orient_with_osd(img, &sheet.settings, osd);
            Ok(legatorocr_core::measure_page(&img, &settings)
                .into_iter()
                .flatten()
                .collect())
        })
        .collect::<Result<_>>()?;
    let all: Vec<(f32, f32)> = sizes.into_iter().flatten().collect();
    if all.is_empty() {
        return Ok(None);
    }
    let median = |mut v: Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let (mw, mh) = (
        median(all.iter().map(|s| s.0).collect()),
        median(all.iter().map(|s| s.1).collect()),
    );
    let fit = all.iter().filter(|s| s.0 <= mw * 1.25 && s.1 <= mh * 1.25);
    Ok(Some(
        fit.fold((mw, mh), |(w, h), s| (w.max(s.0), h.max(s.1))),
    ))
}

#[derive(Debug, Default)]
pub struct ExportResult {
    /// Sheets that failed (path, message) when `keep_going` was set.
    pub failed: Vec<(PathBuf, String)>,
    pub pdf: Option<Vec<u8>>,
    pub pages: usize,
    pub words: usize,
    pub skipped_blank: usize,
    /// Plain text of all pages (pages separated by form feed) when OCR ran.
    pub text: Option<String>,
    /// hOCR document when OCR ran.
    pub hocr: Option<String>,
    /// Consecutive duplicate pages dropped.
    pub skipped_duplicates: usize,
    /// With `split_on_blank`: one PDF per document (then `pdf` is `None`).
    pub docs: Vec<Vec<u8>>,
}

/// Pick the JPEG quality (30–95) whose PDF is expected to stay within `target_bytes`:
/// up to 6 evenly spaced sheets are exported at each quality of a ladder, the size is
/// extrapolated to all sheets. Returns the best quality that fits, or 30 when none does.
/// (1-bit pages do not depend on the quality, so they only add a fixed cost.)
pub fn quality_for_target(
    sheets: &[SheetInput],
    osd: Option<&OcrOptions>,
    target_bytes: u64,
    cancel: &AtomicBool,
) -> Result<u8> {
    if sheets.is_empty() {
        bail!("no pages to export");
    }
    let n = sheets.len().min(6);
    let sample: Vec<SheetInput> = (0..n)
        .map(|i| sheets[i * sheets.len() / n].clone())
        .collect();
    for q in [95u8, 90, 85, 80, 70, 60, 50, 40] {
        let job = ExportJob {
            sheets: &sample,
            pdf: Some(PdfExport {
                quality: q,
                ..Default::default()
            }),
            skip_blank: false,
            keep_going: true,
            osd: osd.cloned(),
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let res = run_export(&job, cancel, &|_, _| {}, &|_, _| {})?;
        let bytes = res.pdf.map_or(0, |p| p.len() as u64);
        let est = bytes as f64 / res.pages.max(1) as f64 * sheets.len() as f64;
        if est <= target_bytes as f64 {
            return Ok(q);
        }
    }
    Ok(30)
}

/// Sheets whose natural page size differs clearly from the book's typical size:
/// width or height further than `max(6 %, 3·MAD, 5 mm)` from the median. Needs at
/// least 5 measured pages; returns indices into `sizes` (None entries are ignored).
pub fn outlier_indices(sizes: &[Option<(f32, f32)>]) -> Vec<usize> {
    let known: Vec<(usize, (f32, f32))> = sizes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            s.filter(|(w, h)| w.is_finite() && h.is_finite())
                .map(|s| (i, s))
        })
        .collect();
    if known.len() < 5 {
        return Vec::new();
    }
    let median = |mut v: Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let spread = |vals: Vec<f32>| {
        let m = median(vals.clone());
        let mad = median(vals.iter().map(|v| (v - m).abs()).collect());
        (m, (0.06 * m).max(3.0 * 1.4826 * mad).max(5.0))
    };
    let (mw, tw) = spread(known.iter().map(|k| k.1 .0).collect());
    let (mh, th) = spread(known.iter().map(|k| k.1 .1).collect());
    known
        .into_iter()
        .filter(|(_, (w, h))| (w - mw).abs() > tw || (h - mh).abs() > th)
        .map(|(i, _)| i)
        .collect()
}

/// Measure every sheet and report the outliers as `(sheet index, (width_mm, height_mm))`.
/// Unreadable sheets are skipped (they show up when exporting).
pub fn find_outliers(
    sheets: &[SheetInput],
    osd: Option<&OcrOptions>,
    cancel: &AtomicBool,
) -> Result<Vec<(usize, (f32, f32))>> {
    let sizes: Vec<Option<(f32, f32)>> = sheets
        .par_iter()
        .map(|sheet| {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            let Ok(img) = load_image(&sheet.path) else {
                return Ok(None);
            };
            let (img, settings) = orient_with_osd(img, &sheet.settings, osd);
            // a split sheet is judged by its larger half
            Ok(legatorocr_core::measure_page(&img, &settings)
                .into_iter()
                .flatten()
                .max_by(|a, b| (a.0 * a.1).total_cmp(&(b.0 * b.1))))
        })
        .collect::<Result<_>>()?;
    Ok(outlier_indices(&sizes)
        .into_iter()
        .filter_map(|i| sizes[i].map(|s| (i, s)))
        .collect())
}

/// Save a processed page as an image file. PNG gets its resolution (`pHYs`);
/// black & white TIFF is written Group 4 compressed (a typical page is ~30 KB);
/// everything else goes through the `image` crate.
pub fn save_page_image(pg: &Page, path: &Path) -> Result<()> {
    save_page_image_q(pg, path, 90)
}

/// Like [`save_page_image`], with the JPEG quality (1–100) used for `.jpg` / `.jpeg`.
/// JPEG files carry the page's resolution in their JFIF header.
pub fn save_page_image_q(pg: &Page, path: &Path, jpeg_quality: u8) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = match (&pg.image, ext.as_str()) {
        (legatorocr_core::PageImage::Bw(b), "tif" | "tiff") => {
            legatorocr_pdf::g4::tiff_g4(b, pg.dpi)
        }
        (img, "png") => {
            let mut buf = std::io::Cursor::new(Vec::new());
            img.to_dynamic()
                .write_to(&mut buf, image::ImageFormat::Png)?;
            png_with_dpi(buf.into_inner(), pg.dpi)
        }
        (img, "jpg" | "jpeg") => {
            use image::codecs::jpeg::{JpegEncoder, PixelDensity};
            let mut buf = Vec::new();
            let mut enc = JpegEncoder::new_with_quality(&mut buf, jpeg_quality.clamp(1, 100));
            enc.set_pixel_density(PixelDensity::dpi(pg.dpi.min(u16::MAX as u32) as u16));
            match img {
                legatorocr_core::PageImage::Rgb(i) => enc.encode(
                    i.as_raw(),
                    i.width(),
                    i.height(),
                    image::ExtendedColorType::Rgb8,
                )?,
                other => {
                    let g = other.to_gray();
                    enc.encode(
                        g.as_raw(),
                        g.width(),
                        g.height(),
                        image::ExtendedColorType::L8,
                    )?
                }
            }
            buf
        }
        (img, _) => {
            img.to_dynamic().save(path)?;
            return Ok(());
        }
    };
    write_atomic(path, &bytes)
}

/// Insert a `pHYs` chunk (pixels per metre) right after IHDR.
fn png_with_dpi(png: Vec<u8>, dpi: u32) -> Vec<u8> {
    const IHDR_END: usize = 8 + 25; // signature + IHDR chunk (4+4+13+4)
    if png.len() < IHDR_END || &png[12..16] != b"IHDR" {
        return png;
    }
    let ppm = (dpi as f64 / 0.0254).round() as u32;
    let mut data = Vec::with_capacity(9);
    data.extend_from_slice(&ppm.to_be_bytes());
    data.extend_from_slice(&ppm.to_be_bytes());
    data.push(1); // unit: metre
    let mut crc = flate2::Crc::new();
    crc.update(b"pHYs");
    crc.update(&data);
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&data);
    chunk.extend_from_slice(&crc.sum().to_be_bytes());
    let mut out = png[..IHDR_END].to_vec();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[IHDR_END..]);
    out
}

/// Page `index` (0-based) of a multi-page TIFF, as an 8-bit image.
fn load_tiff_frame(path: &Path, index: u32) -> Result<DynamicImage> {
    use tiff::decoder::{Decoder, DecodingResult, Limits};
    use tiff::ColorType;
    let f = std::fs::File::open(path)?;
    let mut d = Decoder::new(std::io::BufReader::new(f))?.with_limits(Limits::default());
    d.seek_to_image(index as usize)
        .with_context(|| format!("page {} not found", index + 1))?;
    let (w, h) = d.dimensions()?;
    if (w as u64) * (h as u64) > 400_000_000 {
        bail!("page is too large ({w}×{h})");
    }
    let ct = d.colortype()?;
    let data = d.read_image()?;
    let to8 = |v: DecodingResult| -> Result<Vec<u8>> {
        Ok(match v {
            DecodingResult::U8(v) => v,
            DecodingResult::U16(v) => v.into_iter().map(|x| (x >> 8) as u8).collect(),
            _ => bail!("unsupported TIFF sample format"),
        })
    };
    let raw = to8(data)?;
    let bad = || anyhow::anyhow!("TIFF page data has the wrong size");
    Ok(match ct {
        ColorType::Gray(1) => {
            // packed bits, rows padded to bytes; 0 = black (the common case)
            let rb = (w as usize).div_ceil(8);
            if raw.len() < rb * h as usize {
                return Err(bad());
            }
            let mut g = image::GrayImage::new(w, h);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let bit = raw[y * rb + x / 8] >> (7 - x % 8) & 1;
                    g.put_pixel(
                        x as u32,
                        y as u32,
                        image::Luma([if bit == 1 { 255 } else { 0 }]),
                    );
                }
            }
            DynamicImage::ImageLuma8(g)
        }
        ColorType::Gray(8) | ColorType::Gray(16) => {
            DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, raw).ok_or_else(bad)?)
        }
        ColorType::RGB(8) | ColorType::RGB(16) => {
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw).ok_or_else(bad)?)
        }
        ColorType::RGBA(8) | ColorType::RGBA(16) => {
            // flatten onto white, like load_image does for other formats
            let rgba = image::RgbaImage::from_raw(w, h, raw).ok_or_else(bad)?;
            let mut out = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
            for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
                let a = p[3] as u32;
                for c in 0..3 {
                    o[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
                }
            }
            DynamicImage::ImageRgb8(out)
        }
        other => bail!("unsupported TIFF colour type {other:?}"),
    })
}

/// Parsed PDFs, kept for the pages of one book (a few at a time).
type CachedPdf = (
    PathBuf,
    u64,
    u64,
    std::sync::Arc<legatorocr_pdf::reader::PdfDoc>,
);
static PDF_CACHE: std::sync::Mutex<Vec<CachedPdf>> = std::sync::Mutex::new(Vec::new());

/// The parsed PDF at `path` (cached by path, size and modification time).
pub fn pdf_doc(path: &Path) -> Result<std::sync::Arc<legatorocr_pdf::reader::PdfDoc>> {
    let meta = std::fs::metadata(path).with_context(|| path.display().to_string())?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    if let Some((_, _, _, d)) = PDF_CACHE
        .lock()
        .unwrap()
        .iter()
        .find(|(p, l, m, _)| p == path && *l == meta.len() && *m == stamp)
    {
        return Ok(d.clone());
    }
    let data = std::fs::read(path).with_context(|| path.display().to_string())?;
    let doc = std::sync::Arc::new(
        legatorocr_pdf::reader::PdfDoc::open(data)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
    );
    let mut c = PDF_CACHE.lock().unwrap();
    c.retain(|(p, ..)| p != path);
    if c.len() >= 3 {
        c.remove(0);
    }
    c.push((path.to_path_buf(), meta.len(), stamp, doc.clone()));
    Ok(doc)
}

/// Pages in a PDF; 1 when it cannot be read (the error then shows up when the page is loaded).
pub fn pdf_page_count(path: &Path) -> u32 {
    pdf_doc(path).map_or(1, |d| d.page_count().max(1) as u32)
}

fn load_pdf_page(path: &Path, index: u32) -> Result<DynamicImage> {
    let doc = pdf_doc(path)?;
    if (index as usize) >= doc.page_count() {
        bail!(
            "{}: the PDF has {} page(s), page {} was requested",
            path.display(),
            doc.page_count(),
            index + 1
        );
    }
    let page = doc
        .page_image(index as usize)
        .map_err(|e| anyhow::anyhow!("{} page {}: {e}", path.display(), index + 1))?;
    if page.info.layered {
        bail!(
            "{} page {}: the page is built from several images (layered / MRC PDF); only single-scan pages are supported",
            path.display(),
            index + 1
        );
    }
    Ok(page.image)
}

/// Open an image the way a person sees it: EXIF orientation applied (phone and
/// camera JPEGs), transparency flattened onto white.
pub fn load_image(path: &Path) -> Result<DynamicImage> {
    let (base, frame) = files::split_frame(path);
    if files::is_pdf(&base) {
        return load_pdf_page(&base, frame.unwrap_or(1) - 1);
    }
    if let Some(n) = frame {
        return load_tiff_frame(&base, n - 1);
    }
    use image::{ImageDecoder, ImageReader};
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    // Bounded, so a hostile or corrupt header cannot request tens of gigabytes.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(3 << 30);
    limits.max_image_width = Some(60_000);
    limits.max_image_height = Some(60_000);
    reader.limits(limits);
    let mut dec = reader.into_decoder()?;
    let orientation = dec
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec)?;
    img.apply_orientation(orientation);
    if img.color().has_alpha() {
        let mut rgb = image::RgbImage::new(img.width(), img.height());
        for (o, p) in rgb.pixels_mut().zip(img.to_rgba8().pixels()) {
            let a = p[3] as u32;
            for c in 0..3 {
                o[c] = ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
            }
        }
        img = DynamicImage::ImageRgb8(rgb);
    }
    Ok(img)
}

/// Apply OSD-detected rotation to a freshly loaded sheet. Returns the (possibly
/// rotated) image and the settings to use for it (built-in orientation guess
/// is switched off once OSD has decided).
pub fn orient_with_osd(
    img: DynamicImage,
    settings: &Settings,
    osd: Option<&OcrOptions>,
) -> (DynamicImage, Settings) {
    let mut s = settings.clone();
    let Some(o) = osd else { return (img, s) };
    match legatorocr_ocr::detect_rotation(&img, o) {
        Ok(Some(deg)) => {
            s.auto_orient = false;
            // `rotate_quarters` is applied first by the pipeline.
            s.rotate_quarters = (s.rotate_quarters + (deg / 90) as u8) % 4;
            (img, s)
        }
        _ => (img, s),
    }
}

/// Check that the OCR engine runs and every requested language is installed.
/// Returns the engine's version string.
pub fn check_ocr(o: &OcrOptions) -> Result<String> {
    if !legatorocr_ocr::valid_lang(&o.lang) {
        bail!(
            "invalid OCR language list '{}' (expected e.g. 'eng' or 'ron+eng')",
            o.lang
        );
    }
    let v = legatorocr_ocr::check_engine(o)?;
    let installed = legatorocr_ocr::list_languages(o)?;
    for l in o.lang.split('+') {
        if !installed.iter().any(|i| i == l) {
            bail!(
                "OCR language '{l}' is not installed for {v} (installed: {})",
                installed.join(", ")
            );
        }
    }
    Ok(v)
}

/// Run the pipeline over all sheets.
///
/// * `on_page(sheet_index, &page)` is called from worker threads for every
///   page that will be kept (use it to save processed images).
/// * `progress(done_sheets, total_sheets)` is called after each sheet.
/// * Setting `cancel` aborts with an error whose text is `cancelled`.
pub fn run_export(
    job: &ExportJob,
    cancel: &AtomicBool,
    on_page: &(dyn Fn(usize, &Page) + Sync),
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<ExportResult> {
    let total = job.sheets.len();
    if total == 0 {
        bail!("no pages to export");
    }
    let new_writer = || {
        job.pdf.as_ref().map(|p| {
            let mut w = PdfWriter::new(if p.quality == 0 {
                Compress::Flate
            } else {
                Compress::Jpeg(p.quality.min(100))
            });
            if let Some(t) = &p.title {
                w.set_title(t);
            }
            w.set_meta(p.meta.clone());
            w
        })
    };
    let mut writer = new_writer();
    let mut last_thumb: Option<Vec<u8>> = None;
    let ocr_opts = job
        .pdf
        .as_ref()
        .and_then(|p| p.ocr.as_ref())
        .or(job.text_ocr.as_ref());
    let mut texts: Vec<String> = Vec::new();
    let mut hocr_pages: Vec<String> = Vec::new();
    // Pass 1 (optional): a common page size for the whole book.
    let uniform = if job.uniform_size {
        measure_uniform_size(job.sheets, job.osd.as_ref(), cancel)?
    } else {
        None
    };
    let done = AtomicUsize::new(0);
    let batch = rayon::current_num_threads().max(2);
    let mut res = ExportResult::default();

    for (b, chunk) in job.sheets.chunks(batch).enumerate() {
        let base = b * batch;
        let outs: Vec<Result<SheetOutput>> = chunk
            .par_iter()
            .enumerate()
            .map(|(i, sheet)| {
                if cancel.load(Ordering::Relaxed) {
                    bail!("cancelled");
                }
                let img = load_image(&sheet.path)
                    .with_context(|| format!("reading {}", sheet.path.display()))?;
                let (img, mut settings) = orient_with_osd(img, &sheet.settings, job.osd.as_ref());
                if settings.page_size_mm.is_none() {
                    settings.page_size_mm = uniform;
                }
                let mut kept = Vec::new();
                let mut skipped = 0;
                for pg in process_page(&img, &settings) {
                    if job.skip_blank && pg.blank {
                        skipped += 1;
                        continue;
                    }
                    on_page(base + i, &pg);
                    let words = match ocr_opts {
                        Some(o) => {
                            if cancel.load(Ordering::Relaxed) {
                                bail!("cancelled");
                            }
                            legatorocr_ocr::recognize(&pg.image, pg.dpi, o)
                                .with_context(|| format!("OCR of {}", sheet.path.display()))?
                        }
                        None => Vec::new(),
                    };
                    kept.push((pg, words));
                }
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                Ok((kept, skipped))
            })
            .collect();
        for (i, out) in outs.into_iter().enumerate() {
            let (pages, skipped) = match out {
                Ok(v) => v,
                Err(e) if job.keep_going && e.to_string() != "cancelled" => {
                    res.failed.push((chunk[i].path.clone(), short_chain(&e)));
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    continue;
                }
                Err(e) => return Err(e),
            };
            res.skipped_blank += skipped;
            for (pg, words) in pages {
                if job.split_on_blank && pg.blank {
                    if let Some(w) = writer.take().filter(|w| w.page_count() > 0) {
                        res.docs.push(w.finish());
                    }
                    writer = new_writer();
                    last_thumb = None;
                    res.skipped_blank += 1;
                    continue;
                }
                if job.skip_duplicates && !pg.blank {
                    let t = page_thumb(&pg);
                    if last_thumb.as_deref().is_some_and(|l| thumb_diff(l, &t) <= 2.0) {
                        res.skipped_duplicates += 1;
                        continue;
                    }
                    last_thumb = Some(t);
                }
                res.pages += 1;
                res.words += words.len();
                if ocr_opts.is_some() {
                    texts.push(legatorocr_ocr::words_to_text(&words));
                    hocr_pages.push(legatorocr_ocr::hocr_page(
                        &words,
                        pg.image.width(),
                        pg.image.height(),
                        pg.dpi,
                        res.pages,
                    ));
                }
                if let Some(w) = writer.as_mut() {
                    w.add_page(&pg, &words);
                }
            }
        }
    }
    if res.pages == 0 {
        match res.failed.first() {
            Some((p, e)) => bail!("no pages produced; first error ({}): {e}", p.display()),
            None => bail!("no pages produced"),
        }
    }
    if ocr_opts.is_some() {
        res.text = Some(texts.join("\n\x0c"));
        res.hocr = Some(legatorocr_ocr::hocr_document(&hocr_pages));
    }
    if job.split_on_blank {
        if let Some(w) = writer.filter(|w| w.page_count() > 0) {
            res.docs.push(w.finish());
        }
    } else {
        res.pdf = writer.map(PdfWriter::finish);
    }
    Ok(res)
}

/// `{:#}` of an error chain, without segments that merely repeat the previous one
/// (decoder errors often wrap their own message).
fn short_chain(e: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in e.chain() {
        let c = c.to_string();
        if parts
            .last()
            .is_none_or(|p| !p.contains(c.trim_end_matches('.')))
        {
            parts.push(c);
        }
    }
    parts.join(": ")
}

/// Verify early that `path` can be created (parent folder is made if missing).
pub fn check_writable(path: &Path) -> Result<()> {
    if path.is_dir() {
        bail!("{} is a folder, not a file", path.display());
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let mut probe = path.as_os_str().to_owned();
    probe.push(".part");
    let probe = PathBuf::from(probe);
    std::fs::write(&probe, b"").with_context(|| format!("cannot write {}", path.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Write via a temporary sibling file, then rename (no half-written outputs).
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let r = std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, path));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r.with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};
    use std::sync::Mutex;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgr-engine-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Text-like bars on paper, saved as PNG; `blank` leaves the sheet empty.
    fn sheet(dir: &Path, name: &str, blank: bool) -> PathBuf {
        let mut g = GrayImage::from_pixel(700, 900, Luma([238]));
        if !blank {
            let mut y = 100;
            while y < 800 {
                for x in (80..620).step_by(37) {
                    for yy in y..y + 12 {
                        for xx in x..x + 28 {
                            g.put_pixel(xx, yy, Luma([25]));
                        }
                    }
                }
                y += 30;
            }
        }
        let p = dir.join(name);
        g.save(&p).unwrap();
        p
    }

    fn st() -> Settings {
        Settings {
            dpi: 150,
            split: legatorocr_core::SplitMode::Off,
            deskew: false,
            auto_orient: false,
            ..Default::default()
        }
    }

    #[test]
    fn outliers_are_the_pages_that_differ_clearly() {
        let mut v: Vec<Option<(f32, f32)>> = (0..20)
            .map(|i| Some((148.0 + (i % 3) as f32, 210.0 - (i % 2) as f32)))
            .collect();
        v[7] = Some((300.0, 210.0));
        v[12] = None;
        v[15] = Some((148.0, 120.0));
        assert_eq!(outlier_indices(&v), vec![7, 15]);
        assert!(
            outlier_indices(&v[..4]).is_empty(),
            "too few pages to judge"
        );
        assert!(outlier_indices(&[Some((f32::NAN, 1.0)); 9]).is_empty());
    }

    #[test]
    fn jpeg_output_is_a_jpeg_with_resolution() {
        let pg = Page {
            image: legatorocr_core::PageImage::Rgb(image::RgbImage::from_pixel(
                40,
                30,
                image::Rgb([120, 80, 40]),
            )),
            dpi: 300,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        };
        let p = std::env::temp_dir().join(format!("lgr-jpeg-{}.jpg", std::process::id()));
        save_page_image_q(&pg, &p, 70).unwrap();
        let b = std::fs::read(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(&b[..3], &[0xFF, 0xD8, 0xFF]);
        // JFIF APP0: units=1 (dpi), Xdensity=300
        assert_eq!(&b[13..18], &[1, 0x01, 0x2C, 0x01, 0x2C]);
        assert_eq!(image::load_from_memory(&b).unwrap().width(), 40);
    }

    #[test]
    fn exports_pdf_in_order_and_skips_blank() {
        let d = tmpdir("a");
        let sheets: Vec<SheetInput> = [("1.png", false), ("2.png", true), ("3.png", false)]
            .iter()
            .map(|(n, b)| SheetInput {
                path: sheet(&d, n, *b),
                settings: st(),
            })
            .collect();
        let seen = Mutex::new(Vec::new());
        let ticks = AtomicUsize::new(0);
        let job = ExportJob {
            sheets: &sheets,
            pdf: Some(PdfExport::default()),
            skip_blank: true,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let r = run_export(
            &job,
            &AtomicBool::new(false),
            &|i, _| seen.lock().unwrap().push(i),
            &|_, t| {
                assert_eq!(t, 3);
                ticks.fetch_add(1, Ordering::Relaxed);
            },
        )
        .unwrap();
        assert_eq!((r.pages, r.skipped_blank, r.words), (2, 1, 0));
        let mut s = seen.into_inner().unwrap();
        s.sort();
        assert_eq!(s, vec![0, 2]);
        assert_eq!(ticks.load(Ordering::Relaxed), 3);
        let pdf = r.pdf.unwrap();
        assert!(pdf.starts_with(b"%PDF-") && String::from_utf8_lossy(&pdf).contains("/Count 2"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn images_only_job_has_no_pdf() {
        let d = tmpdir("b");
        let sheets = vec![SheetInput {
            path: sheet(&d, "1.png", false),
            settings: st(),
        }];
        let job = ExportJob {
            sheets: &sheets,
            pdf: None,
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let r = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).unwrap();
        assert!(r.pdf.is_none() && r.pages == 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn errors_are_reported_with_context() {
        let none = ExportJob {
            sheets: &[],
            pdf: None,
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        assert!(run_export(&none, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).is_err());
        let bad = [SheetInput {
            path: "/no/such/file.png".into(),
            settings: st(),
        }];
        let job = ExportJob {
            sheets: &bad,
            pdf: None,
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let e = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).unwrap_err();
        assert!(format!("{e:#}").contains("file.png"), "{e:#}");
        let d = tmpdir("c");
        let ok = [SheetInput {
            path: sheet(&d, "1.png", false),
            settings: st(),
        }];
        let job = ExportJob {
            sheets: &ok,
            pdf: None,
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let e = run_export(&job, &AtomicBool::new(true), &|_, _| {}, &|_, _| {}).unwrap_err();
        assert!(e.to_string().contains("cancelled"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn atomic_write_replaces_file() {
        let d = tmpdir("d");
        let f = d.join("sub/out.pdf");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert!(!d.join("sub/out.pdf.part").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn exif_orientation_and_alpha_are_honoured() {
        let d = tmpdir("e");
        // 40x20 JPEG carrying EXIF orientation 6 (needs a 90° clockwise turn) → 20x40.
        let img = image::RgbImage::from_pixel(40, 20, image::Rgb([30, 30, 30]));
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .encode(img.as_raw(), 40, 20, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut app1 = b"Exif\0\0MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01\x00\x06\x00\x00\x00\x00\x00\x00".to_vec();
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        seg.append(&mut app1);
        let mut out = jpg[..2].to_vec();
        out.extend(seg);
        out.extend_from_slice(&jpg[2..]);
        let p = d.join("rot.jpg");
        std::fs::write(&p, out).unwrap();
        let l = load_image(&p).unwrap();
        assert_eq!((l.width(), l.height()), (20, 40));
        // Fully transparent black must become paper white, not black.
        let mut rgba = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        rgba.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let p = d.join("a.png");
        rgba.save(&p).unwrap();
        let l = load_image(&p).unwrap().to_rgb8();
        assert_eq!(l.get_pixel(3, 3).0, [255, 255, 255]);
        assert_eq!(l.get_pixel(0, 0).0, [0, 0, 0]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn keep_going_skips_bad_sheets_but_fails_when_nothing_is_left() {
        let d = tmpdir("f");
        std::fs::write(d.join("bad.png"), b"not an image").unwrap();
        let sheets = vec![
            SheetInput {
                path: sheet(&d, "1.png", false),
                settings: st(),
            },
            SheetInput {
                path: d.join("bad.png"),
                settings: st(),
            },
            SheetInput {
                path: sheet(&d, "3.png", false),
                settings: st(),
            },
        ];
        let mut job = ExportJob {
            sheets: &sheets,
            pdf: Some(PdfExport::default()),
            skip_blank: false,
            keep_going: true,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let ticks = AtomicUsize::new(0);
        let r = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {
            ticks.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        assert_eq!((r.pages, r.failed.len()), (2, 1));
        assert!(r.failed[0].0.ends_with("bad.png"));
        assert_eq!(
            ticks.load(Ordering::Relaxed),
            3,
            "progress still reaches 100 %"
        );
        job.keep_going = false;
        assert!(run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).is_err());
        let only_bad = [SheetInput {
            path: d.join("bad.png"),
            settings: st(),
        }];
        job = ExportJob {
            sheets: &only_bad,
            pdf: None,
            skip_blank: false,
            keep_going: true,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let e = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).unwrap_err();
        assert!(e.to_string().contains("bad.png"), "{e}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn absurd_settings_do_not_explode() {
        let d = tmpdir("g");
        let wild = Settings {
            dpi: 0,
            margins_mm: [1e9, f32::NAN, -5.0, f32::INFINITY],
            page_size_mm: Some((1e9, 1e9)),
            max_skew_deg: 1e9,
            manual_skew_deg: Some(f32::NAN),
            sauvola_k: f32::NAN,
            ..st()
        };
        let sheets = [SheetInput {
            path: sheet(&d, "1.png", false),
            settings: wild,
        }];
        let job = ExportJob {
            sheets: &sheets,
            pdf: Some(PdfExport::default()),
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: false,
            split_on_blank: false,
        };
        let r = run_export(
            &job,
            &AtomicBool::new(false),
            &|_, pg| assert!(pg.image.width() < 20_000),
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(r.pages, 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn writability_precheck() {
        let d = tmpdir("h");
        assert!(check_writable(&d.join("new/dir/o.pdf")).is_ok());
        assert!(!d.join("new/dir/o.pdf.part").exists());
        assert!(check_writable(&d).is_err(), "a folder is not a file");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn uniform_size_gives_every_page_the_same_dimensions() {
        let d = tmpdir("u");
        // Second sheet has much less text, hence a smaller natural page.
        let small = {
            let mut g = GrayImage::from_pixel(700, 900, Luma([238]));
            for y in 300..420 {
                for x in 200..500 {
                    if (x / 9 + y / 7) % 2 == 0 {
                        g.put_pixel(x, y, Luma([25]));
                    }
                }
            }
            let p = d.join("small.png");
            g.save(&p).unwrap();
            p
        };
        let sheets = vec![
            SheetInput {
                path: sheet(&d, "big.png", false),
                settings: st(),
            },
            SheetInput {
                path: small,
                settings: st(),
            },
        ];
        let dims = Mutex::new(Vec::new());
        for uniform in [false, true] {
            dims.lock().unwrap().clear();
            let job = ExportJob {
                sheets: &sheets,
                pdf: None,
                skip_blank: false,
                keep_going: false,
                osd: None,
                uniform_size: uniform,
                text_ocr: None,
                skip_duplicates: false,
                split_on_blank: false,
            };
            run_export(
                &job,
                &AtomicBool::new(false),
                &|i, pg| {
                    dims.lock()
                        .unwrap()
                        .push((i, pg.image.width(), pg.image.height()))
                },
                &|_, _| {},
            )
            .unwrap();
            let mut v = dims.lock().unwrap().clone();
            v.sort();
            if uniform {
                assert_eq!((v[0].1, v[0].2), (v[1].1, v[1].2), "{v:?}");
            } else {
                assert_ne!((v[0].1, v[0].2), (v[1].1, v[1].2), "{v:?}");
            }
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn multipage_tiff_is_expanded_and_each_page_loads() {
        use tiff::encoder::{colortype, TiffEncoder};
        let d = tmpdir("tiff");
        let path = d.join("book.tif");
        {
            let mut enc = TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
            enc.write_image::<colortype::Gray8>(30, 20, &vec![200u8; 600])
                .unwrap();
            enc.write_image::<colortype::Gray8>(40, 25, &vec![100u8; 1000])
                .unwrap();
            enc.write_image::<colortype::RGB8>(10, 10, &vec![7u8; 300])
                .unwrap();
        }
        assert_eq!(files::tiff_frame_count(&path), 3);
        let pages = files::collect_images(std::slice::from_ref(&path));
        assert_eq!(pages.len(), 3);
        assert!(pages[1].to_string_lossy().ends_with("book.tif#2"));
        let dims: Vec<(u32, u32)> = pages
            .iter()
            .map(|p| load_image(p).unwrap())
            .map(|i| (i.width(), i.height()))
            .collect();
        assert_eq!(dims, vec![(30, 20), (40, 25), (10, 10)]);
        // an explicit page, a missing page, and a real file whose name contains '#'
        assert_eq!(
            files::collect_images(std::slice::from_ref(&pages[2])),
            vec![pages[2].clone()]
        );
        assert!(load_image(&d.join("book.tif#7")).is_err());
        let odd = d.join("odd#1.png");
        GrayImage::from_pixel(5, 5, Luma([9])).save(&odd).unwrap();
        assert_eq!(load_image(&odd).unwrap().width(), 5);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn saved_images_carry_dpi_and_g4_tiff_round_trips() {
        use legatorocr_core::{BitImage, PageImage};
        let d = tmpdir("save");
        let mut b = BitImage::new(203, 67);
        for y in 0..67 {
            for x in 0..203 {
                b.set(x, y, (x * 7 + y * 13) % 11 < 3 || (60..90).contains(&x));
            }
        }
        let pg = Page {
            image: PageImage::Bw(b.clone()),
            dpi: 300,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        };
        let tif = d.join("p.tif");
        save_page_image(&pg, &tif).unwrap();
        let mut dec = tiff::decoder::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(&tif).unwrap(),
        ))
        .unwrap();
        assert_eq!(dec.dimensions().unwrap(), (203, 67));
        let back = load_image(&tif).unwrap().to_luma8();
        for y in 0..67 {
            for x in 0..203 {
                assert_eq!(back.get_pixel(x, y)[0] < 128, b.get(x, y), "{x},{y}");
            }
        }
        let png = d.join("p.png");
        save_page_image(&pg, &png).unwrap();
        let bytes = std::fs::read(&png).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"pHYs"));
        assert_eq!(image::open(&png).unwrap().width(), 203); // still a valid PNG
        assert_eq!(files::detect_dpi(&png), Some(300));
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
mod pdf_input_tests {
    use super::*;
    use legatorocr_pdf::{Compress, PdfWriter};

    fn make_pdf(path: &Path, pages: usize) {
        let mut w = PdfWriter::new(Compress::Flate);
        for i in 0..pages {
            let img = image::RgbImage::from_pixel(120, 160, image::Rgb([250 - i as u8, 250, 250]));
            w.add_page(
                &Page {
                    image: legatorocr_core::PageImage::Rgb(img),
                    dpi: 200,
                    skew_deg: 0.0,
                    blank: false,
                    split_index: None,
                    layers: None,
                },
                &[],
            );
        }
        std::fs::write(path, w.finish()).unwrap();
    }

    #[test]
    fn pdf_pages_expand_load_and_report_dpi() {
        let dir = std::env::temp_dir().join(format!("lgr-pdfin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("book.pdf");
        make_pdf(&f, 3);
        let pages = files::collect_images(std::slice::from_ref(&dir));
        assert_eq!(pages.len(), 3);
        assert!(pages[2].to_string_lossy().ends_with("book.pdf#3"));
        let img = load_image(&pages[1]).unwrap();
        assert_eq!((img.width(), img.height()), (120, 160));
        assert_eq!(files::detect_dpi(&pages[0]), Some(200));
        assert!(load_image(&dir.join("book.pdf#9")).is_err());
        std::fs::write(dir.join("bad.pdf"), b"%PDF-1.4 nonsense").unwrap();
        assert!(load_image(&dir.join("bad.pdf")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod batch_features_tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn text_img(seed: u32) -> RgbImage {
        let mut img = RgbImage::from_pixel(600, 800, Rgb([245, 245, 245]));
        for l in 0..12u32 {
            let len = 200 + ((l * 37 + seed * 91) % 250);
            for y in 0..10 {
                for x in 0..len {
                    img.put_pixel(100 + x, 150 + l * 45 + y, Rgb([15, 15, 15]));
                }
            }
        }
        img
    }

    fn sheets(dir: &Path, kinds: &[Option<u32>]) -> Vec<SheetInput> {
        std::fs::create_dir_all(dir).unwrap();
        kinds
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let img = match k {
                    Some(s) => text_img(*s),
                    None => RgbImage::from_pixel(600, 800, Rgb([245, 245, 245])),
                };
                let path = dir.join(format!("{i:02}.png"));
                img.save(&path).unwrap();
                SheetInput {
                    path,
                    settings: Settings {
                        dpi: 150,
                        split: legatorocr_core::SplitMode::Off,
                        deskew: false,
                        auto_orient: false,
                        ..Default::default()
                    },
                }
            })
            .collect()
    }

    fn job<'a>(s: &'a [SheetInput], dup: bool, split: bool) -> ExportJob<'a> {
        ExportJob {
            sheets: s,
            pdf: Some(PdfExport::default()),
            skip_blank: false,
            keep_going: false,
            osd: None,
            uniform_size: false,
            text_ocr: None,
            skip_duplicates: dup,
            split_on_blank: split,
        }
    }

    #[test]
    fn duplicates_and_separators() {
        let dir = std::env::temp_dir().join(format!("lgr-batch-{}", std::process::id()));
        // A, A (duplicate), B, blank, C, C, blank, blank, D
        let s = sheets(
            &dir,
            &[
                Some(1),
                Some(1),
                Some(2),
                None,
                Some(3),
                Some(3),
                None,
                None,
                Some(4),
            ],
        );
        let no = AtomicBool::new(false);
        let r = run_export(&job(&s, true, false), &no, &|_, _| {}, &|_, _| {}).unwrap();
        assert_eq!((r.pages, r.skipped_duplicates), (7, 2));
        let r = run_export(&job(&s, true, true), &no, &|_, _| {}, &|_, _| {}).unwrap();
        assert!(r.pdf.is_none());
        assert_eq!(r.docs.len(), 3, "A B | C | D");
        assert_eq!(r.pages, 4);
        let count = |b: &Vec<u8>| {
            legatorocr_pdf::reader::PdfDoc::open(b.clone())
                .unwrap()
                .page_count()
        };
        assert_eq!(r.docs.iter().map(count).collect::<Vec<_>>(), vec![2, 1, 1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_size_lowers_quality_only_when_needed() {
        let dir = std::env::temp_dir().join(format!("lgr-target-{}", std::process::id()));
        let mut s = sheets(&dir, &[Some(1), Some(2), Some(3)]);
        // noisy colour page so JPEG size depends on quality
        let mut img = RgbImage::new(600, 800);
        let mut z = 12345u32;
        for p in img.pixels_mut() {
            z = z.wrapping_mul(1664525).wrapping_add(1013904223);
            *p = Rgb([(z >> 24) as u8, (z >> 16) as u8, (z >> 8) as u8]);
        }
        img.save(dir.join("noise.png")).unwrap();
        s.push(SheetInput {
            path: dir.join("noise.png"),
            settings: s[0].settings.clone(),
        });
        let no = AtomicBool::new(false);
        assert_eq!(quality_for_target(&s, None, 100_000_000, &no).unwrap(), 95);
        let q = quality_for_target(&s, None, 400_000, &no).unwrap();
        assert!(q < 95, "got {q}");
        assert_eq!(quality_for_target(&s, None, 1_000, &no).unwrap(), 30);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
