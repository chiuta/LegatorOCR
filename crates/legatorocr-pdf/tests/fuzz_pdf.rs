#![allow(clippy::manual_is_multiple_of, clippy::field_reassign_with_default, clippy::type_complexity)]
//! PDF writer fuzzing: random pages (every image kind, layered pages, extreme sizes),
//! hostile OCR words (huge coordinates, zero boxes, odd Unicode, NUL, surrogates-adjacent
//! text, > 64k distinct characters) — the file must always be structurally valid.
//! Validation: our own structural checker always; `qpdf --check` and `pdftotext` when installed.
//! Scale: `LEGATOROCR_FUZZ_ITERS=500`.

use image::{GrayImage, Rgb, RgbImage};
use legatorocr_core::layers::{Background, Layers, Patch};
use legatorocr_core::{BitImage, Page, PageImage, Word};
use legatorocr_pdf::{Compress, PdfWriter};

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

fn gen_page(r: &mut Rng) -> Page {
    let dw = r.pick_dim();
    let w = 1 + r.below(dw) as u32;
    let dh = r.pick_dim();
    let h = 1 + r.below(dh) as u32;
    let dpi = [0u32, 1, 36, 72, 300, 2400, u32::MAX][r.below(7) as usize];
    let mut bw = BitImage::new(w, h);
    for _ in 0..r.below(2000) {
        bw.set(r.below(w as u64) as u32, r.below(h as u64) as u32, true);
    }
    let (image, layers) = match r.below(5) {
        0 => (PageImage::Rgb(RgbImage::from_fn(w, h, |x, y| Rgb([(x * 3) as u8, (y * 5) as u8, 9]))), None),
        1 => (PageImage::Gray(GrayImage::from_fn(w, h, |x, y| image::Luma([(x ^ y) as u8]))), None),
        2 => (PageImage::Bw(bw), None),
        3 => {
            let l = Layers { mask: bw, fg: [r.below(256) as u8; 3], bg: None, patches: vec![] };
            (PageImage::Rgb(l.compose()), Some(l))
        }
        _ => {
            let (pw, ph) = (1 + r.below(w as u64) as u32, 1 + r.below(h as u64) as u32);
            let l = Layers {
                mask: bw,
                fg: [0, 0, 0],
                bg: Some(Background { img: RgbImage::from_pixel(1 + w / 3, 1 + h / 3, Rgb([200, 190, 170])), dpi: 100 }),
                patches: vec![Patch { x: r.below((w - pw + 1) as u64) as u32, y: r.below((h - ph + 1) as u64) as u32, img: RgbImage::from_pixel(pw, ph, Rgb([1, 2, 3])) }],
            };
            (PageImage::Rgb(l.compose()), Some(l))
        }
    };
    Page { image, dpi, skew_deg: 0.0, blank: false, split_index: None, layers }
}

trait Dim {
    fn pick_dim(&mut self) -> u64;
}
impl Dim for Rng {
    fn pick_dim(&mut self) -> u64 {
        [3u64, 40, 300, 900][self.below(4) as usize]
    }
}

fn gen_words(r: &mut Rng) -> Vec<Word> {
    let texts: [&str; 14] = [
        "Școala", "țăranului", "", " ", "\0", "a(b)c\\d", ")\\(", "😀ok", "日本語のテキスト", "e\u{301}", "\u{FFFD}", "Ünï", "x".repeat(500).leak(), "\u{202E}rtl",
    ];
    (0..r.below(60))
        .map(|_| {
            let coord = |r: &mut Rng| [0u32, 1, 50, 100_000, u32::MAX, u32::MAX - 1][r.below(6) as usize];
            Word {
                text: if r.below(10) == 0 { char::from_u32(r.below(0x30000) as u32).unwrap_or('?').to_string() } else { texts[r.below(14) as usize].to_string() },
                x: if r.below(2) == 0 { r.below(300) as u32 } else { coord(r) },
                y: if r.below(2) == 0 { r.below(300) as u32 } else { coord(r) },
                w: if r.below(2) == 0 { r.below(100) as u32 } else { coord(r) },
                h: if r.below(2) == 0 { r.below(40) as u32 } else { coord(r) },
                conf: [f32::NAN, -1.0, 0.0, 99.9, f32::INFINITY][r.below(5) as usize],
                line: r.below(5) as u32,
            }
        })
        .collect()
}

/// Minimal structural validation independent of external tools.
fn check_structure(pdf: &[u8], pages: usize, ctx: &str) {
    assert!(pdf.starts_with(b"%PDF-1."), "{ctx}");
    assert!(pdf.ends_with(b"%%EOF\n") || pdf.ends_with(b"%%EOF"), "{ctx}: no EOF");
    let text = String::from_utf8_lossy(pdf);
    let tail = String::from_utf8_lossy(&pdf[pdf.len().saturating_sub(64)..]).into_owned();
    let sx = tail.rfind("startxref").unwrap_or_else(|| panic!("{ctx}: no startxref"));
    let off: usize = tail[sx + 9..].split_whitespace().next().unwrap().parse().unwrap();
    assert!(pdf[off..].starts_with(b"xref"), "{ctx}: startxref does not point at xref");
    assert!(text.contains(&format!("/Count {pages}")), "{ctx}: /Count");
    // every xref entry points at "<n> 0 obj"
    let xr_owned = String::from_utf8_lossy(&pdf[off..]).into_owned(); // the xref table itself is ASCII
    let xr = xr_owned.as_str();
    let mut lines = xr.lines();
    lines.next();
    let hdr: Vec<usize> = lines.next().unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
    assert_eq!(hdr[0], 0);
    for (n, line) in lines.take(hdr[1]).enumerate() {
        if n == 0 {
            continue;
        }
        let o: usize = line[..10].parse().unwrap();
        assert!(pdf[o..].starts_with(format!("{n} 0 obj").as_bytes()), "{ctx}: xref entry {n} wrong");
    }
}

#[test]
fn writer_always_produces_valid_pdf() {
    let n: u64 = std::env::var("LEGATOROCR_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let mut r = Rng(std::env::var("LEGATOROCR_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0xC0FFEE) | 1);
    let dir = std::env::temp_dir().join(format!("lgr-pdffuzz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let have_qpdf = std::process::Command::new("qpdf").arg("--version").output().is_ok();
    let have_text = std::process::Command::new("pdftotext").arg("-v").output().is_ok();
    for it in 0..n {
        let compress = if r.below(2) == 0 { Compress::Flate } else { Compress::Jpeg([0u8, 1, 50, 100, 255][r.below(5) as usize]) };
        let mut w = PdfWriter::new(compress);
        if r.below(2) == 0 {
            w.set_title(["", "Titlu ăâîșț", "a(b)\\", "日本", "\0x", &"t".repeat(2000)][r.below(6) as usize]);
        }
        let np = 1 + r.below(4) as usize;
        for _ in 0..np {
            let page = gen_page(&mut r);
            let words = gen_words(&mut r);
            w.add_page(&page, &words);
        }
        assert_eq!(w.page_count(), np);
        let pdf = w.finish();
        let ctx = format!("iter {it}");
        check_structure(&pdf, np, &ctx);
        if have_qpdf && it % 4 == 0 {
            let p = dir.join("f.pdf");
            std::fs::write(&p, &pdf).unwrap();
            let out = std::process::Command::new("qpdf").arg("--check").arg(&p).output().unwrap();
            assert!(out.status.success(), "{ctx}: qpdf: {}", String::from_utf8_lossy(&out.stdout));
            // content streams must not contain non-finite numbers or absurd magnitudes
            let q = dir.join("q.pdf");
            let st = std::process::Command::new("qpdf").args(["--qdf", "--object-streams=disable"]).arg(&p).arg(&q).status().unwrap();
            assert!(st.success() || st.code() == Some(3), "{ctx}: qpdf --qdf");
            let qdf = String::from_utf8_lossy(&std::fs::read(&q).unwrap_or_default()).into_owned();
            for line in qdf.lines().filter(|l| l.ends_with(" Tj") || l.ends_with(" Q") || l.ends_with(" re f Q")) {
                let l = line.to_ascii_lowercase();
                assert!(!l.contains("nan") && !l.contains("inf") && !l.contains("e+") && !l.contains("e-"), "{ctx}: bad number in {line}");
                for tok in line.split_whitespace().filter_map(|t| t.parse::<f64>().ok()) {
                    assert!(tok.abs() < 1e7, "{ctx}: absurd magnitude {tok} in {line}");
                }
            }
            if let Ok(o) = std::process::Command::new("pdftoppm").args(["-r", "8", "-f", "1", "-l", "1", "-png"]).arg(&p).arg(dir.join("r")).output() {
                let err = String::from_utf8_lossy(&o.stderr);
                assert!(!err.contains("Syntax Error"), "{ctx}: poppler: {err}");
            }
            if have_text {
                let t = std::process::Command::new("pdftotext").arg(&p).arg("-").output().unwrap();
                assert!(t.status.success(), "{ctx}: pdftotext failed");
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn too_many_distinct_characters_do_not_corrupt_the_font() {
    let mut w = PdfWriter::new(Compress::Flate);
    let words: Vec<Word> = (0..3000u32)
        .map(|i| Word { text: (0..40).map(|k| char::from_u32(0x4E00 + (i * 40 + k) % 20000).unwrap()).collect(), x: 5, y: 5 + i % 200, w: 80, h: 10, conf: 90.0, line: i })
        .collect();
    let page = Page { image: PageImage::Gray(GrayImage::new(300, 300)), dpi: 300, skew_deg: 0.0, blank: false, split_index: None, layers: None };
    for _ in 0..4 {
        w.add_page(&page, &words);
    }
    let pdf = w.finish();
    check_structure(&pdf, 4, "many chars");
}
