#![allow(clippy::manual_is_multiple_of, clippy::field_reassign_with_default, clippy::type_complexity)]
//! Fuzzing of everything that reads untrusted bytes: image files (valid ones mutated
//! byte by byte, truncated, extended), project files, Tesseract output, file names.
//! Scale: `LEGATOROCR_FUZZ_ITERS=5000 LEGATOROCR_FUZZ_SEED=3 cargo test --release -p legatorocr-engine --test fuzz_inputs`

use legatorocr_engine::{files, load_image, ocr, project::Project};
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
    std::env::var("LEGATOROCR_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn seed() -> u64 {
    std::env::var("LEGATOROCR_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0xFEED_FACE_1234_5678) | 1
}
fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr-fuzz-{tag}-{}-{}", std::process::id(), seed()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Mutate: flip bytes, truncate, splice garbage, duplicate chunks, zero runs.
fn mutate(src: &[u8], r: &mut Rng) -> Vec<u8> {
    let mut v = src.to_vec();
    for _ in 0..1 + r.below(6) {
        if v.is_empty() {
            v.push(r.below(256) as u8);
            continue;
        }
        let i = r.below(v.len() as u64) as usize;
        match r.below(8) {
            0 => v[i] = r.below(256) as u8,
            1 => v[i] ^= 1 << r.below(8),
            2 => v.truncate(i),
            3 => {
                let n = r.below(64) as usize;
                for _ in 0..n {
                    v.insert(i.min(v.len()), r.below(256) as u8);
                }
            }
            4 => {
                let e = (i + r.below(200) as usize).min(v.len());
                for b in &mut v[i..e] {
                    *b = 0;
                }
            }
            5 => {
                let e = (i + r.below(200) as usize).min(v.len());
                for b in &mut v[i..e] {
                    *b = 0xFF;
                }
            }
            6 => {
                let e = (i + r.below(100) as usize).min(v.len());
                let chunk = v[i..e].to_vec();
                v.splice(i..i, chunk);
            }
            _ => {
                // write an extreme 32-bit value (sizes, offsets, counts)
                if i + 4 <= v.len() {
                    let x = [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x0100_0000, 65_536][r.below(6) as usize];
                    v[i..i + 4].copy_from_slice(&x.to_le_bytes());
                }
            }
        }
    }
    v
}

fn seeds_images(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    use image::{codecs::jpeg::JpegEncoder, ImageFormat, RgbImage};
    let img = RgbImage::from_fn(97, 61, |x, y| image::Rgb([(x * 2) as u8, (y * 4) as u8, ((x ^ y) * 3) as u8]));
    let mut out = Vec::new();
    for (name, fmt) in [("png", ImageFormat::Png), ("bmp", ImageFormat::Bmp), ("tif", ImageFormat::Tiff)] {
        let mut c = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img.clone()).write_to(&mut c, fmt).unwrap();
        out.push((name.to_string(), c.into_inner()));
    }
    let mut j = Vec::new();
    JpegEncoder::new_with_quality(&mut j, 80).encode(img.as_raw(), 97, 61, image::ExtendedColorType::Rgb8).unwrap();
    out.push(("jpg".into(), j));
    // multi-page TIFF (gray + rgb) and a G4 TIFF
    {
        use tiff::encoder::{colortype, TiffEncoder};
        let p = dir.join("seed_multi.tif");
        let mut e = TiffEncoder::new(std::fs::File::create(&p).unwrap()).unwrap();
        e.write_image::<colortype::Gray8>(30, 20, &vec![99u8; 600]).unwrap();
        e.write_image::<colortype::RGB8>(11, 9, &vec![7u8; 297]).unwrap();
        drop(e);
        out.push(("tif".into(), std::fs::read(&p).unwrap()));
    }
    let mut b = legatorocr_core::BitImage::new(120, 40);
    for i in 0..120 {
        b.set(i, (i / 3) % 40, true);
    }
    out.push(("tif".into(), legatorocr_pdf::g4::tiff_g4(&b, 300)));
    out
}

#[test]
fn corrupted_image_files_never_panic_or_explode() {
    let dir = tmp("img");
    let seeds = seeds_images(&dir);
    let mut r = Rng(seed());
    let n = iters(1500);
    let (mut ok, mut err) = (0u32, 0u32);
    for it in 0..n {
        let (ext, src) = &seeds[r.below(seeds.len() as u64) as usize];
        let data = mutate(src, &mut r);
        // sometimes lie about the extension
        let ext = if r.below(5) == 0 { ["png", "jpg", "tif", "bmp", "gif", "webp", "dat"][r.below(7) as usize] } else { ext.as_str() };
        let p = dir.join(format!("f{}.{ext}", it % 8));
        std::fs::write(&p, &data).unwrap();
        // The call must return (Ok or Err) without panicking, hanging or allocating absurdly.
        match load_image(&p) {
            Ok(img) => {
                ok += 1;
                assert!(img.width() >= 1 && img.height() >= 1);
                assert!((img.width() as u64) * (img.height() as u64) <= 400_000_000, "it {it}: {}x{}", img.width(), img.height());
            }
            Err(_) => err += 1,
        }
        let _ = files::detect_dpi(&p);
        let _ = files::tiff_frame_count(&p);
        let framed = dir.join(format!("f{}.{ext}#{}", it % 8, 1 + r.below(5)));
        let _ = load_image(&framed);
    }
    eprintln!("images: {ok} decoded, {err} rejected of {n}");
    let _ = std::fs::remove_dir_all(dir);
}

const GOOD_PROJECT: &str = "legatorocr-project 1\ndpi = 300\nauto_orient = true\nsplit = auto\ndeskew = true\nmax_skew = 7\ndewarp = true\nmargins = 5,5,5,5\ncontent_adjust = 0,0,0,0\npage_size = 148,210\nalign = center,center\nmode = hybrid\nthreshold = wolf\nsensitivity = 0.3\ndespeckle = true\nnormalize = true\noutput_dpi = 0\npicture_sensitivity = 0.5\nhybrid_dpi = 100\npdf_quality = 80\n\n[sheet]\npath = a/b.png\nrotate = 1\nskew = 1.5\nadjust = 1,2,3,4\n\n[sheet]\npath = /abs/c.tif#2\nsplit = off\n";

#[test]
fn project_parser_survives_mutations_and_noise() {
    let mut r = Rng(seed() ^ 0x77);
    let n = iters(20_000);
    let base = GOOD_PROJECT.as_bytes();
    let mut parsed = 0;
    for it in 0..n {
        let text = if r.below(6) == 0 {
            // pure noise, including invalid UTF-8 (lossy)
            let len = r.below(400) as usize;
            let v: Vec<u8> = (0..len).map(|_| r.below(256) as u8).collect();
            String::from_utf8_lossy(&v).into_owned()
        } else {
            String::from_utf8_lossy(&mutate(base, &mut r)).into_owned()
        };
        if let Ok(p) = Project::parse(&text, Some(std::path::Path::new("/proj"))) {
            parsed += 1;
            // Whatever was accepted must be fully sanitised and re-serialisable.
            let s = &p.settings;
            assert!((36..=2400).contains(&s.dpi), "it {it}");
            assert!(s.margins_mm.iter().all(|m| m.is_finite() && *m >= 0.0), "it {it}");
            assert!(s.sauvola_k.is_finite() && (0.0..=1.0).contains(&s.sauvola_k));
            let again = Project::parse(&p.to_text(None), None).unwrap_or_else(|e| panic!("it {it}: re-parse failed: {e:#}\n{}", p.to_text(None)));
            assert_eq!(again.sheets.len(), p.sheets.len(), "it {it}");
        }
    }
    eprintln!("projects: {parsed} accepted of {n}");
    assert!(Project::parse(GOOD_PROJECT, None).is_ok());
}

#[test]
fn tesseract_output_parsers_survive_garbage() {
    let mut r = Rng(seed() ^ 0x99);
    let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t10\t20\t30\t12\t96.5\tBună\n";
    for _ in 0..iters(20_000) {
        let s = String::from_utf8_lossy(&mutate(tsv.as_bytes(), &mut r)).into_owned();
        for w in ocr::parse_tsv(&s, r.below(120) as f32 - 10.0) {
            assert!(w.conf.is_finite() || w.conf.is_nan() || w.conf.is_infinite());
        }
        let osd = "Page number: 0\nOrientation in degrees: 270\nRotate: 90\nOrientation confidence: 7.31\nScript: Latin\n";
        let o = String::from_utf8_lossy(&mutate(osd.as_bytes(), &mut r)).into_owned();
        if let Some(d) = ocr::parse_osd(&o) {
            assert!(matches!(d, 0 | 90 | 180 | 270));
        }
        let lang: String = (0..r.below(12)).map(|_| char::from_u32(r.below(0x250) as u32).unwrap_or('x')).collect();
        let _ = ocr::valid_lang(&lang);
    }
    // known bad language strings must stay rejected
    for bad in ["", "--psm", "a;b", "a b", "$(x)", "ron+", "+", "../x", "a\nb", "a\0b"] {
        assert!(!ocr::valid_lang(bad), "{bad:?}");
    }
}

#[test]
fn file_name_helpers_survive_weird_names() {
    let mut r = Rng(seed() ^ 0x55);
    let alphabet: Vec<char> = "ab01#.-_ /\\é日\u{0}\n tif png%".chars().collect();
    for _ in 0..iters(30_000) {
        let a: String = (0..r.below(24)).map(|_| alphabet[r.below(alphabet.len() as u64) as usize]).collect();
        let b: String = (0..r.below(24)).map(|_| alphabet[r.below(alphabet.len() as u64) as usize]).collect();
        let (x, y) = (files::natural_cmp(&a, &b), files::natural_cmp(&b, &a));
        assert_eq!(x, y.reverse(), "natural_cmp must be antisymmetric: {a:?} {b:?}");
        assert_eq!(files::natural_cmp(&a, &a), std::cmp::Ordering::Equal);
        let _ = files::is_image(std::path::Path::new(&a));
        let _ = files::split_frame(std::path::Path::new(&a));
    }
    // transitivity on a sample
    let mut v: Vec<String> = (0..200).map(|_| (0..r.below(10)).map(|_| alphabet[r.below(12) as usize]).collect()).collect();
    v.sort_by(|a, b| files::natural_cmp(a, b));
    for w in v.windows(2) {
        assert_ne!(files::natural_cmp(&w[0], &w[1]), std::cmp::Ordering::Greater);
    }
}
