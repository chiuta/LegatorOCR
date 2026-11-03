//! With SOURCE_DATE_EPOCH set, the same inputs and options give the same PDF bytes,
//! also from another folder (roadmap 5.3).

use std::process::Command;

fn make(dir: &std::path::Path, extra: &[&str]) -> Vec<u8> {
    std::fs::create_dir_all(dir).unwrap();
    let mut img = image::GrayImage::from_pixel(500, 400, image::Luma([255]));
    for y in 80..92 {
        for x in 50..400 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(dir.join("a.png")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .args(["a.png", "a.png", "--dpi", "300", "--mode", "bw", "--pdf", "o.pdf", "-o", "pages"])
        .args(extra)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::read(dir.join("o.pdf")).unwrap()
}

#[test]
fn same_inputs_same_bytes() {
    let base = std::env::temp_dir().join(format!("lgr_repro_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for extra in [&[][..], &["--pdfa-part", "2", "--tagged"][..], &["--provenance"][..]] {
        let a = make(&base.join("one"), extra);
        let b = make(&base.join("two"), extra);
        assert!(a == b, "PDF bytes differ for {extra:?}");
    }
    let _ = std::fs::remove_dir_all(&base);
}
