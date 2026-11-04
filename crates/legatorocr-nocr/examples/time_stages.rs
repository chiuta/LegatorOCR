//! Developer tool: `time_stages MODEL IMAGE [dpi]` prints the time spent in page preparation, line finding
//! and the network (single thread), to see where the OCR time goes.
use legatorocr_core::PageImage;
use legatorocr_nocr::{layout, Model, LINE_H};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = Model::from_bytes(&std::fs::read(&a[1]).expect("model")).expect("valid model");
    let img = image::open(&a[2]).expect("image").to_luma8();
    let dpi = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);
    let t0 = Instant::now();
    let prep = layout::prepare(&PageImage::Gray(img), dpi);
    let t1 = Instant::now();
    let lines = layout::find_lines(&prep.ink, prep.med);
    let t2 = Instant::now();
    let mut net = std::time::Duration::ZERO;
    let mut cols = 0usize;
    for ln in &lines {
        for piece in layout::split_long(&prep.ink, ln, 2000) {
            if let Some(li) = layout::extract(&prep.gray, &piece, LINE_H, false) {
                let t = Instant::now();
                let _ = model.forward(&li.data, li.w);
                net += t.elapsed();
                cols += li.w;
            }
        }
    }
    println!(
        "prepare {:?}  find_lines {:?}  network {:?} ({} lines, {} columns)",
        t1 - t0,
        t2 - t1,
        net,
        lines.len(),
        cols
    );
}
