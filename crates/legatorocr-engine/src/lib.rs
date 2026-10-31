//! Shared orchestration used by the CLI and the GUI: process sheets in bounded
//! batches (so a 300-page colour book never sits in memory at once), OCR every
//! page, assemble the PDF, write files atomically.

use anyhow::{bail, Context, Result};
use legatorocr_core::{process_page, Page, Settings, Word};
use legatorocr_ocr::OcrOptions;
use legatorocr_pdf::{Compress, PdfWriter};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub use legatorocr_ocr as ocr;

#[derive(Debug, Clone)]
pub struct PdfExport {
    /// JPEG quality for colour/gray pages; 0 = lossless Flate.
    pub quality: u8,
    /// `Some` → run OCR and embed an invisible text layer.
    pub ocr: Option<OcrOptions>,
    pub title: Option<String>,
}

impl Default for PdfExport {
    fn default() -> Self {
        Self { quality: 80, ocr: None, title: None }
    }
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
}

#[derive(Debug, Default)]
pub struct ExportResult {
    pub pdf: Option<Vec<u8>>,
    pub pages: usize,
    pub words: usize,
    pub skipped_blank: usize,
}

/// Check that the OCR engine runs and every requested language is installed.
/// Returns the engine's version string.
pub fn check_ocr(o: &OcrOptions) -> Result<String> {
    let v = legatorocr_ocr::check_engine(o)?;
    let installed = legatorocr_ocr::list_languages(o)?;
    for l in o.lang.split('+') {
        if !installed.iter().any(|i| i == l) {
            bail!("OCR language '{l}' is not installed for {v} (installed: {})", installed.join(", "));
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
    let mut writer = job.pdf.as_ref().map(|p| {
        let mut w = PdfWriter::new(if p.quality == 0 { Compress::Flate } else { Compress::Jpeg(p.quality.min(100)) });
        if let Some(t) = &p.title {
            w.set_title(t);
        }
        w
    });
    let ocr_opts = job.pdf.as_ref().and_then(|p| p.ocr.as_ref());
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
                let img = image::open(&sheet.path).with_context(|| format!("reading {}", sheet.path.display()))?;
                let mut kept = Vec::new();
                let mut skipped = 0;
                for pg in process_page(&img, &sheet.settings) {
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
        for out in outs {
            let (pages, skipped) = out?;
            res.skipped_blank += skipped;
            for (pg, words) in pages {
                res.pages += 1;
                res.words += words.len();
                if let Some(w) = writer.as_mut() {
                    w.add_page(&pg, &words);
                }
            }
        }
    }
    if res.pages == 0 {
        bail!("no pages produced");
    }
    res.pdf = writer.map(PdfWriter::finish);
    Ok(res)
}

/// Write via a temporary sibling file, then rename (no half-written outputs).
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
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
        Settings { dpi: 150, split: legatorocr_core::SplitMode::Off, deskew: false, auto_orient: false, ..Default::default() }
    }

    #[test]
    fn exports_pdf_in_order_and_skips_blank() {
        let d = tmpdir("a");
        let sheets: Vec<SheetInput> = [("1.png", false), ("2.png", true), ("3.png", false)]
            .iter()
            .map(|(n, b)| SheetInput { path: sheet(&d, n, *b), settings: st() })
            .collect();
        let seen = Mutex::new(Vec::new());
        let ticks = AtomicUsize::new(0);
        let job = ExportJob { sheets: &sheets, pdf: Some(PdfExport::default()), skip_blank: true };
        let r = run_export(&job, &AtomicBool::new(false), &|i, _| seen.lock().unwrap().push(i), &|_, t| {
            assert_eq!(t, 3);
            ticks.fetch_add(1, Ordering::Relaxed);
        })
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
        let sheets = vec![SheetInput { path: sheet(&d, "1.png", false), settings: st() }];
        let job = ExportJob { sheets: &sheets, pdf: None, skip_blank: false };
        let r = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).unwrap();
        assert!(r.pdf.is_none() && r.pages == 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn errors_are_reported_with_context() {
        let none = ExportJob { sheets: &[], pdf: None, skip_blank: false };
        assert!(run_export(&none, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).is_err());
        let bad = [SheetInput { path: "/no/such/file.png".into(), settings: st() }];
        let job = ExportJob { sheets: &bad, pdf: None, skip_blank: false };
        let e = run_export(&job, &AtomicBool::new(false), &|_, _| {}, &|_, _| {}).unwrap_err();
        assert!(format!("{e:#}").contains("file.png"), "{e:#}");
        let d = tmpdir("c");
        let ok = [SheetInput { path: sheet(&d, "1.png", false), settings: st() }];
        let job = ExportJob { sheets: &ok, pdf: None, skip_blank: false };
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
}
