//! `--embed-project` / `--extract-project`: the PDF keeps the originals and the project, and
//! processing them again gives the same pages (synthetic page images; the real-page check is
//! `scripts/embed-roundtrip.sh`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .args(args)
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A JPEG that declares 150 dpi (so the run detects a resolution the project must remember)
/// and a PNG with text-like bars, slightly rotated bars on the second.
fn make_inputs(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let mut a = image::GrayImage::from_pixel(600, 800, image::Luma([240]));
    for row in 0..14u32 {
        for y in (60 + row * 40)..(72 + row * 40) {
            for x in 50..(300 + (row * 31) % 250) {
                a.put_pixel(x, y, image::Luma([20]));
            }
        }
    }
    let mut f = std::fs::File::create(dir.join("a.jpg")).unwrap();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 90);
    enc.set_pixel_density(image::codecs::jpeg::PixelDensity::dpi(150));
    enc.encode_image(&a).unwrap();
    let mut b = image::GrayImage::from_pixel(500, 700, image::Luma([255]));
    for row in 0..10u32 {
        for y in (50 + row * 50)..(60 + row * 50) {
            for x in 40..(200 + row * 20) {
                b.put_pixel(x, y + x / 60, image::Luma([0]));
            }
        }
    }
    b.save(dir.join("b.png")).unwrap();
}

fn pages(dir: &Path) -> Vec<Vec<u8>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    v.sort();
    v.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

const PROJECT: &str = "legatorocr-project 1\nmode = bw\n\n[sheet]\npath = a.jpg\nrotate = 1\nskew = 0.5\nadjust = 0,0,2.5,0\n\n[sheet]\npath = b.png\n";

#[test]
fn the_pdf_returns_its_originals_and_processing_them_again_gives_the_same_pdf() {
    let base = std::env::temp_dir().join(format!("lgr_embed_cli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let one = base.join("one");
    make_inputs(&one);
    std::fs::write(one.join("in.lgr"), PROJECT).unwrap();
    std::fs::create_dir_all(one.join("plain")).unwrap();
    std::fs::create_dir_all(one.join("emb")).unwrap();
    // run 1: plain PDF and the same with the project embedded
    ok(&one, &["--project", "in.lgr", "--pdf", "plain/o.pdf", "-o", "plain/pages"]);
    let log = ok(&one, &["--project", "in.lgr", "--pdf", "emb/o.pdf", "-o", "emb/pages", "--embed-project"]);
    assert!(log.contains("--embed-project: 2 original file(s)"), "{log}");
    let plain = std::fs::read(one.join("plain/o.pdf")).unwrap();
    let emb = std::fs::read(one.join("emb/o.pdf")).unwrap();
    let orig = std::fs::metadata(one.join("a.jpg")).unwrap().len() + std::fs::metadata(one.join("b.png")).unwrap().len();
    assert!(emb.len() as u64 >= plain.len() as u64 + orig, "originals must be inside");
    assert!(!String::from_utf8_lossy(&plain).contains("/EmbeddedFiles"), "off by default");
    // extract and process again
    let x = base.join("extracted");
    ok(&base, &["--extract-project", one.join("emb/o.pdf").to_str().unwrap(), x.to_str().unwrap()]);
    for (n, src) in [("001_a.jpg", "a.jpg"), ("002_b.png", "b.png")] {
        assert_eq!(std::fs::read(x.join(n)).unwrap(), std::fs::read(one.join(src)).unwrap(), "{n} not byte for byte");
    }
    std::fs::create_dir_all(x.join("again")).unwrap();
    ok(&x, &["--project", "project.lgr", "--pdf", "again/o.pdf", "-o", "again/pages"]);
    assert_eq!(pages(&one.join("emb/pages")), pages(&x.join("again/pages")), "page images differ");
    assert!(pages(&x.join("again/pages")).len() >= 2);
    assert!(std::fs::read(x.join("again/o.pdf")).unwrap() == plain, "re-processed PDF differs from the first plain PDF");
    // a second extraction into the same folder is refused
    assert!(!run(&base, &["--extract-project", one.join("emb/o.pdf").to_str().unwrap(), x.to_str().unwrap()]).status.success());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn detected_resolution_is_kept_without_a_project_and_pdfa_needs_part_3() {
    let base = std::env::temp_dir().join(format!("lgr_embed_cli2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    make_inputs(&base);
    std::fs::create_dir_all(base.join("d0")).unwrap();
    // inputs given on the command line (not a project): the JPEG's 150 dpi comes from its header
    ok(&base, &["a.jpg", "b.png", "--mode", "bw", "--pdf", "d0/o.pdf", "-o", "p0"]);
    ok(&base, &["a.jpg", "b.png", "--mode", "bw", "--pdf", "e.pdf", "-o", "p1", "--embed-project"]);
    ok(&base, &["--extract-project", "e.pdf", "x"]);
    std::fs::create_dir_all(base.join("x/d2")).unwrap();
    ok(&base.join("x"), &["--project", "project.lgr", "--pdf", "d2/o.pdf", "-o", "../p2"]);
    assert_eq!(pages(&base.join("p0")), pages(&base.join("p2")), "resolution detected from the JPEG was lost");
    assert!(std::fs::read(base.join("d0/o.pdf")).unwrap() == std::fs::read(base.join("x/d2/o.pdf")).unwrap());
    // PDF/A-2 cannot carry these files; PDF/A-3 can
    let o = run(&base, &["a.jpg", "--pdf", "f.pdf", "--pdfa", "--embed-project"]);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("--pdfa-part 3"));
    ok(&base, &["a.jpg", "--pdf", "g.pdf", "--pdfa", "--pdfa-part", "3", "--embed-project", "-o", "p3"]);
    let g = std::fs::read(base.join("g.pdf")).unwrap();
    let s = String::from_utf8_lossy(&g);
    assert!(s.contains("<pdfaid:part>3</pdfaid:part>") && s.contains("/AFRelationship /Source") && s.contains("/AF ["));
    let _ = std::fs::remove_dir_all(&base);
}
