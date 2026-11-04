//! Developer tool for training on real scans: `dump_lines MODEL IMAGE OUT_DIR [dpi]` writes every
//! text line the engine would read as a normalised 32-pixel PNG (`OUT_DIR/<stem>_<n>.png`) and one
//! `OUT_DIR/<stem>.jsonl` row per line: its box in page pixels (after de-skewing) and the text
//! the model reads (used to place word spaces when labelling with an external text layer).
use legatorocr_core::PageImage;
use legatorocr_nocr::{ctc, layout, Model, LINE_H};
use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = Model::from_bytes(&std::fs::read(&a[1]).expect("model")).expect("valid model");
    let img = image::open(&a[2]).expect("image").to_luma8();
    let out = std::path::Path::new(&a[3]);
    let dpi = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(300);
    std::fs::create_dir_all(out).unwrap();
    let stem = std::path::Path::new(&a[2]).file_stem().unwrap().to_string_lossy().to_string();
    let prep = layout::prepare(&PageImage::Gray(img), dpi);
    let mut rows = std::fs::File::create(out.join(format!("{stem}.jsonl"))).unwrap();
    let mut n = 0;
    for ln in layout::find_lines(&prep.ink, prep.med) {
        for piece in layout::split_long(&prep.ink, &ln, 2000) {
            let Some(li) = layout::extract(&prep.gray, &piece, LINE_H, false) else {
                continue;
            };
            let (t, lp) = model.forward(&li.data, li.w);
            let text: String = ctc::greedy(&lp, t, model.nc, &model.charset).iter().map(|g| g.ch).collect();
            let buf: Vec<u8> = li.data.iter().map(|v| (255.0 - v * 255.0).round() as u8).collect();
            image::GrayImage::from_raw(li.w as u32, LINE_H as u32, buf)
                .unwrap()
                .save(out.join(format!("{stem}_{n}.png")))
                .unwrap();
            writeln!(
                rows,
                "{{\"i\":{n},\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"slope\":{},\"curve\":{},\"model\":{:?}}}",
                piece.x, piece.y, piece.w, piece.h, piece.slope, piece.curve, text
            )
            .unwrap();
            n += 1;
        }
    }
    eprintln!("{stem}: {n} lines");
}
