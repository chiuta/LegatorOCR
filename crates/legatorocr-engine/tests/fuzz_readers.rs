#![allow(clippy::manual_is_multiple_of)]
//! Fuzzing of the readers added for archives, PDF text and full-text search: valid inputs
//! mutated byte by byte must never panic, hang or allocate without bound.
//! Scale: `LEGATOROCR_FUZZ_ITERS=5000 LEGATOROCR_FUZZ_SEED=3 cargo test --release -p legatorocr-engine --test fuzz_readers`

use legatorocr_core::{BitImage, Page, PageImage, Word};
use legatorocr_engine::{archive, docx, search, zipmini};
use legatorocr_pdf::{reader::PdfDoc, Compress, PdfWriter};
use std::path::PathBuf;

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
fn iters(d: u64) -> u64 {
    std::env::var("LEGATOROCR_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}
fn seed() -> u64 {
    std::env::var("LEGATOROCR_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0xC0FF_EE00_1234_5678u64)
        | 1
}
fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr-fuzzr-{tag}-{}-{}", std::process::id(), seed()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mutate(src: &[u8], r: &mut Rng) -> Vec<u8> {
    let mut v = src.to_vec();
    for _ in 0..1 + r.below(6) {
        if v.is_empty() {
            v.push(r.below(256) as u8);
            continue;
        }
        let i = r.below(v.len() as u64) as usize;
        match r.below(7) {
            0 => v[i] = r.below(256) as u8,
            1 => v[i] ^= 1 << r.below(8),
            2 => v.truncate(i),
            3 => {
                for _ in 0..r.below(32) {
                    v.insert(i.min(v.len()), r.below(256) as u8);
                }
            }
            4 => {
                let n = (r.below(64) as usize).min(v.len() - i);
                v[i..i + n].fill(0);
            }
            5 => {
                let n = (r.below(16) as usize).min(v.len() - i);
                v[i..i + n].fill(0xFF);
            }
            _ => {
                let n = (r.below(128) as usize).min(v.len() - i);
                let chunk = v[i..i + n].to_vec();
                let at = r.below(v.len() as u64) as usize;
                v.splice(at..at, chunk);
            }
        }
    }
    v
}

fn tar_entry(name: &str, body: &[u8]) -> Vec<u8> {
    let mut h = [0u8; 512];
    h[..name.len().min(100)].copy_from_slice(&name.as_bytes()[..name.len().min(100)]);
    h[100..107].copy_from_slice(b"0000644");
    h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
    h[156] = b'0';
    h[257..262].copy_from_slice(b"ustar");
    let sum: u32 = h.iter().map(|&b| b as u32).sum::<u32>() + 8 * 32;
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut out = h.to_vec();
    out.extend_from_slice(body);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

fn sample_pdf() -> Vec<u8> {
    let mut bw = BitImage::new(60, 40);
    for i in 0..40 {
        bw.set(i, i % 40, true);
    }
    let page = Page {
        image: PageImage::Bw(bw),
        dpi: 300,
        skew_deg: 0.0,
        blank: false,
        split_index: None,
        content_box: None,
        layers: None,
    };
    let words = vec![
        Word {
            text: "Bucureşti".into(),
            x: 2,
            y: 2,
            w: 30,
            h: 8,
            conf: 90.0,
            line: 0,
        },
        Word {
            text: "Legator".into(),
            x: 2,
            y: 14,
            w: 30,
            h: 8,
            conf: 90.0,
            line: 1,
        },
    ];
    let mut w = PdfWriter::new(Compress::Flate);
    w.add_page(&page, &words);
    w.add_page(&page, &words);
    w.finish()
}

#[test]
fn corrupted_zip_never_panics() {
    let mut r = Rng(seed());
    let seed_zip = zipmini::write(&[
        ("a.txt", b"hello hello hello hello hello"),
        ("dir/b.png", &[0u8; 300]),
        ("c.hocr", b"<span>x</span>"),
    ]);
    for _ in 0..iters(20_000) {
        let z = mutate(&seed_zip, &mut r);
        if let Some(list) = zipmini::list(&z) {
            for e in list.iter().take(100) {
                let _ = zipmini::read(&z, e);
            }
        }
        // pure noise
        let n: Vec<u8> = (0..r.below(200)).map(|_| r.below(256) as u8).collect();
        let _ = zipmini::list(&n);
    }
}

#[test]
fn corrupted_tar_and_tgz_never_panic() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let mut r = Rng(seed() ^ 0x77);
    let mut tar = tar_entry("one.txt", b"first");
    tar.extend(tar_entry("sub/two.png", &[7u8; 700]));
    tar.extend(vec![0u8; 1024]);
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&tar).unwrap();
    let tgz = gz.finish().unwrap();
    let dir = tmp("tar");
    let n = iters(3000).min(6000);
    for i in 0..n {
        let (name, bytes) = if i % 2 == 0 {
            ("f.tar", mutate(&tar, &mut r))
        } else {
            ("f.tgz", mutate(&tgz, &mut r))
        };
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        let _ = archive::extract(&p, &|_| true);
        if i % 50 == 0 {
            archive::cleanup();
        }
    }
    archive::cleanup();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_pdf_text_extraction_never_panics() {
    let mut r = Rng(seed() ^ 0x99);
    let pdf = sample_pdf();
    // the pristine file must give the text back
    let doc = PdfDoc::open(pdf.clone()).expect("own pdf opens");
    assert!(doc.page_text(0).unwrap().contains("Legator"));
    for _ in 0..iters(5000) {
        let m = mutate(&pdf, &mut r);
        if let Ok(d) = PdfDoc::open(m) {
            for p in 0..d.page_count().min(5) {
                let _ = d.page_text(p);
            }
        }
    }
}

#[test]
fn search_and_markup_stripping_survive_corrupt_files() {
    let mut r = Rng(seed() ^ 0x1234);
    let dir = tmp("search");
    let pdf = sample_pdf();
    let zip = zipmini::write(&[
        ("x.txt", b"Legator found here"),
        ("y.html", b"<p>Lega&#x74;or"),
    ]);
    let hocr = b"<div class='ocr_page'><span class='ocrx_word'>Lega&amp;tor &#9999999999; &#xZZ;</span></div>".to_vec();
    let docx_bytes = docx::from_pages(&["Legator\nsecond line".to_string()], "t");
    let seeds: [(&str, &[u8]); 5] = [
        ("a.pdf", &pdf),
        ("b.zip", &zip),
        ("c.hocr", &hocr),
        ("d.docx", &docx_bytes),
        ("e.txt", b"Legator\r\nlegator\n\xff\xfe broken utf8"),
    ];
    let opts = [
        search::SearchOptions {
            regex: false,
            exact: false,
            max_hits: 0,
        },
        search::SearchOptions {
            regex: true,
            exact: true,
            max_hits: 5,
        },
    ];
    for i in 0..iters(2000).min(4000) {
        let (name, bytes) = seeds[(i % 5) as usize];
        std::fs::write(dir.join(name), mutate(bytes, &mut r)).unwrap();
        let o = &opts[(i % 2) as usize];
        let q = if o.regex { "Leg(a|x)+tor" } else { "legator" };
        let _ = search::search(std::slice::from_ref(&dir), q, o, &mut |_| {});
        if i % 100 == 0 {
            archive::cleanup();
        }
    }
    archive::cleanup();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_regexes_are_bounded() {
    // The regex engine is linear-time; these must finish at once even on long input.
    let dir = tmp("redos");
    std::fs::write(dir.join("long.txt"), "a".repeat(2_000_000)).unwrap();
    let o = search::SearchOptions {
        regex: true,
        exact: false,
        max_hits: 0,
    };
    let t = std::time::Instant::now();
    for q in ["(a+)+$", "(a|aa)+b", "(.*)*x", "((a*)*)*b"] {
        let _ = search::search(std::slice::from_ref(&dir), q, &o, &mut |_| {});
    }
    assert!(t.elapsed().as_secs() < 30, "regex search too slow");
    let _ = std::fs::remove_dir_all(&dir);
}
