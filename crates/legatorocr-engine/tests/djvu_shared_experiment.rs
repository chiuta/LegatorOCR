//! Experiment for roadmap 6.6 (shared symbol dictionary, thumbnails): size of a bundled
//! bilevel DjVu made page by page against one made with a shared dictionary.
//! Ignored by default; run with LGR_BW_DIR=<folder of 1-bit-looking PNGs>.
use djvu_rs::Bitmap;

fn load(dir: &str) -> Vec<Bitmap> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| {
            let g = image::open(f).unwrap().to_luma8();
            let mut bm = Bitmap::new(g.width(), g.height());
            for (x, y, p) in g.enumerate_pixels() {
                if p[0] < 128 {
                    bm.set_black(x, y);
                }
            }
            bm
        })
        .collect()
}

#[test]
#[ignore]
fn shared_dictionary_size() {
    let dir = std::env::var("LGR_BW_DIR").expect("LGR_BW_DIR");
    let pages = load(&dir);
    let per_page: Vec<Vec<u8>> = pages
        .iter()
        .map(|b| {
            djvu_rs::djvu_encode::PageEncoder::from_bitmap(b)
                .with_dpi(300)
                .encode()
                .unwrap()
        })
        .collect();
    let refs: Vec<&[u8]> = per_page.iter().map(|p| p.as_slice()).collect();
    let plain = djvu_rs::djvm::merge(&refs).unwrap();
    println!("pages {}  page by page {} bytes", pages.len(), plain.len());
    for th in [2usize, 3, 5] {
        for thumbs in [false, true] {
            let b = djvu_rs::jb2_encode::encode_djvm_bundle_jb2_with_thumbnails(
                &pages, th, 300, thumbs,
            );
            println!(
                "shared threshold {th} thumbnails {thumbs}: {} bytes ({:+.1} %)",
                b.len(),
                100.0 * (b.len() as f64 / plain.len() as f64 - 1.0)
            );
        }
    }
}
