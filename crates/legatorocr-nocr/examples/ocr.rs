//! `cargo run --release -p legatorocr-nocr --example ocr -- MODEL.lnm IMAGE [dpi]` prints the
//! recognised text line by line (developer tool for checking models).
use legatorocr_core::PageImage;
use legatorocr_nocr::{recognize, Model, NativeOptions};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = Model::from_bytes(&std::fs::read(&a[1]).expect("model")).expect("valid model");
    let img = image::open(&a[2]).expect("image").to_luma8();
    let dpi = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);
    if std::env::var_os("NOCR_DEBUG").is_some() {
        let (_, ink) = legatorocr_nocr::layout::to_ink(&PageImage::Gray(img.clone()), dpi);
        let med = legatorocr_nocr::layout::median_component_height(&ink);
        let lines = legatorocr_nocr::layout::find_lines(&ink, med);
        eprintln!("median component height {med}, {} lines", lines.len());
        for l in lines.iter().take(12) {
            let mut n = 0u32;
            for y in l.y..l.y + l.h { for x in l.x..l.x + l.w { n += ink.get(x, y) as u32; } }
            eprintln!("  line x{} y{} w{} h{} ink{}", l.x, l.y, l.w, l.h, n);
        }
    }
    if std::env::var_os("NOCR_ORIENT").is_some() {
        let lang = std::env::var("NOCR_LANG").unwrap_or_default();
        let t = std::time::Instant::now();
        println!("{:?} in {:?}", legatorocr_nocr::detect_orientation(&PageImage::Gray(img.clone()), dpi, &model, &lang), t.elapsed());
        return;
    }
    let t = std::time::Instant::now();
    let words = recognize(&PageImage::Gray(img), dpi, &model, &NativeOptions { lang: std::env::var("NOCR_LANG").unwrap_or_default(), ..Default::default() });
    let mut line = u32::MAX;
    for w in &words {
        if w.line != line {
            if line != u32::MAX {
                println!();
            }
            line = w.line;
        }
        print!("{} ", w.text);
    }
    println!();
    eprintln!("{} words in {:?}", words.len(), t.elapsed());
}
