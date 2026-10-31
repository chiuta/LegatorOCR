#![allow(
    clippy::manual_is_multiple_of,
    clippy::field_reassign_with_default,
    clippy::type_complexity
)]
//! End-to-end export fuzzing: batches mixing good, corrupt, missing and multi-page
//! sheets with random settings, random `keep_going` / `skip_blank` / `uniform_size`,
//! and cancellation at a random moment. The engine must either produce a structurally
//! valid PDF or return a clean error — never panic, hang or leave progress inconsistent.

use image::{DynamicImage, Rgb, RgbImage};
use legatorocr_core::{HAlign, OutputMode, Settings, SplitMode, Threshold, VAlign};
use legatorocr_engine::{run_export, ExportJob, PdfExport, SheetInput};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn page_image(r: &mut Rng) -> RgbImage {
    let (w, h) = (20 + r.below(500) as u32, 20 + r.below(700) as u32);
    let mut img = RgbImage::from_pixel(w, h, Rgb([235, 232, 220]));
    if r.below(5) != 0 {
        let pitch = 10 + r.below(30) as u32;
        let mut y = h / 8;
        while y + 6 < h - h / 8 {
            for x in (w / 8..w - w / 8).step_by(3) {
                if (x / 5) % 2 == 0 {
                    for dy in 0..5 {
                        img.put_pixel(x, y + dy, Rgb([30, 30, 30]));
                    }
                }
            }
            y += pitch;
        }
    }
    img
}

fn settings(r: &mut Rng) -> Settings {
    Settings {
        dpi: [72u32, 150, 300][r.below(3) as usize],
        split: [SplitMode::Off, SplitMode::Auto, SplitMode::Middle][r.below(3) as usize],
        deskew: r.below(2) == 0,
        dewarp: r.below(3) == 0,
        mode: [
            OutputMode::Color,
            OutputMode::Gray,
            OutputMode::BlackWhite,
            OutputMode::Mixed,
            OutputMode::Hybrid,
        ][r.below(5) as usize],
        threshold: Threshold::ALL[r.below(Threshold::ALL.len() as u64) as usize],
        normalize_bg: r.below(2) == 0,
        output_dpi: (r.below(3) == 0).then_some(50),
        page_size_mm: (r.below(3) == 0)
            .then_some((100.0 + r.below(100) as f32, 150.0 + r.below(100) as f32)),
        align: (HAlign::Right, VAlign::Bottom),
        margins_mm: [r.below(15) as f32; 4],
        ..Settings::default()
    }
}

#[test]
fn export_is_robust_to_bad_sheets_and_cancellation() {
    let n: u64 = std::env::var("LEGATOROCR_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let mut r = Rng(std::env::var("LEGATOROCR_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0xABCDEF)
        | 1);
    let dir = std::env::temp_dir().join(format!("lgr-expfuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut oks, mut errs, mut cancels) = (0, 0, 0);
    for it in 0..n {
        // --- build the batch
        let count = 1 + r.below(7) as usize;
        let mut sheets = Vec::new();
        for i in 0..count {
            let kind = r.below(10);
            let path: PathBuf = dir.join(format!(
                "{it}_{i}.{}",
                if kind == 5 { "jpg" } else { "png" }
            ));
            match kind {
                0 => {} // missing file
                1 => std::fs::write(&path, b"not an image at all").unwrap(),
                2 => {
                    DynamicImage::ImageRgb8(page_image(&mut r))
                        .save(&path)
                        .unwrap();
                    let mut b = std::fs::read(&path).unwrap();
                    b.truncate(b.len() / 2); // truncated PNG
                    std::fs::write(&path, b).unwrap();
                }
                3 => std::fs::write(&path, b"").unwrap(),
                _ => DynamicImage::ImageRgb8(page_image(&mut r))
                    .save(&path)
                    .unwrap(),
            }
            sheets.push(SheetInput {
                path,
                settings: settings(&mut r),
            });
        }
        let keep_going = r.below(2) == 0;
        let job = ExportJob {
            sheets: &sheets,
            pdf: (r.below(5) != 0).then(|| PdfExport {
                quality: [0u8, 50, 90][r.below(3) as usize],
                ocr: None,
                title: Some("fuzz".into()),
                pdfa: r.below(3) == 0,
                ..Default::default()
            }),
            skip_blank: r.below(2) == 0,
            keep_going,
            osd: None,
            uniform_size: r.below(2) == 0,
            text_ocr: None,
            skip_duplicates: r.below(2) == 0,
            split_on_blank: false,
            split_on_barcode: false,
        };
        let cancel = AtomicBool::new(false);
        let cancel_after = (r.below(3) == 0).then(|| r.below(3) as usize);
        let seen = AtomicUsize::new(0);
        let ticks = Mutex::new(Vec::new());
        let res = run_export(
            &job,
            &cancel,
            &|_, _| {
                if Some(seen.fetch_add(1, Ordering::SeqCst)) == cancel_after {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            &|done, total| {
                assert_eq!(total, count);
                assert!(done >= 1 && done <= total, "progress {done}/{total}");
                ticks.lock().unwrap().push(done);
            },
        );
        match res {
            Ok(res) => {
                oks += 1;
                assert!(res.pages >= 1);
                if let Some(pdf) = &res.pdf {
                    assert!(pdf.starts_with(b"%PDF-"), "iter {it}");
                    assert!(
                        String::from_utf8_lossy(pdf).contains(&format!("/Count {}", res.pages)),
                        "iter {it}: page count"
                    );
                }
                if !keep_going {
                    assert!(
                        res.failed.is_empty(),
                        "iter {it}: failures recorded without keep_going"
                    );
                }
            }
            Err(e) => {
                if e.to_string() == "cancelled" {
                    cancels += 1;
                } else {
                    errs += 1;
                }
                assert!(!e.to_string().is_empty());
            }
        }
        // progress ticks never exceed the number of sheets
        assert!(
            ticks.lock().unwrap().len() <= count,
            "iter {it}: too many progress ticks"
        );
    }
    eprintln!("export fuzz: {oks} ok, {errs} errors, {cancels} cancelled of {n}");
    let _ = std::fs::remove_dir_all(dir);
}
