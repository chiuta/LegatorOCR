//! Developer tool: `dump_ink IMAGE OUT.png [dpi]` writes the binarised ink the layout stage works on.
use legatorocr_core::PageImage;
use legatorocr_nocr::layout;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let img = image::open(&a[1]).expect("image").to_luma8();
    let dpi = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);
    let prep = layout::prepare(&PageImage::Gray(img), dpi);
    let (w, h) = (prep.ink.w, prep.ink.h);
    let mut o = image::GrayImage::from_pixel(w, h, image::Luma([255]));
    for y in 0..h {
        for x in 0..w {
            if prep.ink.get(x, y) {
                o.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    o.save(&a[2]).unwrap();
}
