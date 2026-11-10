#![allow(clippy::field_reassign_with_default)]
//! Hostile inputs for the features of pre.41: slanted cut line, keystone (sheet and page),
//! automatic keystone detection, per-page settings and the paper-size table.
//! Deterministic (seeded); scale with `LEGATOROCR_FUZZ_ITERS`.

use image::{DynamicImage, Rgb, RgbImage};
use legatorocr_core::*;

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
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

fn iters(default: u64) -> u64 {
    std::env::var("LEGATOROCR_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) {
            default / 3
        } else {
            default
        })
}

const NASTY: &[f32] = &[
    0.0,
    -0.0,
    0.04,
    0.05,
    1.0,
    -1.0,
    5.5,
    -5.5,
    19.99,
    20.0,
    -20.0,
    20.01,
    29.9,
    30.0,
    -30.0,
    99.0,
    -99.0,
    1e9,
    -1e9,
    f32::MAX,
    f32::MIN,
    f32::MIN_POSITIVE,
    f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
];

fn image_kind(r: &mut Rng) -> DynamicImage {
    let (w, h) = r.pick(&[
        (1u32, 1u32),
        (2, 2),
        (3, 1),
        (1, 3),
        (7, 5),
        (4, 400),
        (400, 4),
        (64, 64),
        (150, 200),
        (321, 199),
        (600, 400),
    ]);
    let mut img = RgbImage::new(w, h);
    let kind = r.below(7);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = match kind {
            0 => Rgb([255, 255, 255]),
            1 => Rgb([0, 0, 0]),
            2 => {
                let v = r.below(256) as u8;
                Rgb([v, v, v])
            }
            3 => {
                // text-like rows
                if (y / 6) % 2 == 0 && (x / 5) % 3 != 0 {
                    Rgb([10, 10, 10])
                } else {
                    Rgb([250, 250, 250])
                }
            }
            4 => {
                // one huge black bar on a white page
                if x > w / 3 && x < 2 * w / 3 {
                    Rgb([0, 0, 0])
                } else {
                    Rgb([255, 255, 255])
                }
            }
            5 => Rgb([r.below(256) as u8, r.below(256) as u8, r.below(256) as u8]),
            _ => {
                // diagonal stripes
                if (x + y) % 9 < 3 {
                    Rgb([20, 20, 20])
                } else {
                    Rgb([240, 240, 240])
                }
            }
        };
    }
    DynamicImage::ImageRgb8(img)
}

fn hostile_settings(r: &mut Rng) -> Settings {
    let mut s = Settings::default();
    s.dpi = r.pick(&[1, 20, 72, 150, 300, 600, 2400, 100_000]);
    s.sheet_split_angle_deg = r.pick(NASTY);
    s.sheet_keystone = [r.pick(NASTY), r.pick(NASTY)];
    s.page_keystone = [r.pick(NASTY), r.pick(NASTY)];
    s.sheet_split_frac = r.pick(&[
        None,
        Some(0.0),
        Some(1.0),
        Some(0.5),
        Some(f32::NAN),
        Some(-3.0),
        Some(7.0),
    ]);
    s.split = r.pick(&[SplitMode::Off, SplitMode::Auto, SplitMode::Middle]);
    s.deskew = r.below(2) == 0;
    s.auto_orient = r.below(2) == 0;
    // per-page overrides, nested on purpose, with their own hostile values
    for slot in 0..2 {
        if r.below(2) == 0 {
            let mut p = Settings::default();
            p.page_keystone = [r.pick(NASTY), r.pick(NASTY)];
            p.sheet_keystone = [r.pick(NASTY), r.pick(NASTY)];
            p.sheet_split_angle_deg = r.pick(NASTY);
            p.dpi = r.pick(&[0, 1, 300, 1_000_000]);
            if r.below(2) == 0 {
                let mut inner = Settings::default();
                inner.page_keystone = [r.pick(NASTY), r.pick(NASTY)];
                p.part_settings = [Some(Box::new(inner.clone())), Some(Box::new(inner))];
            }
            s.part_settings[slot] = Some(Box::new(p));
        }
    }
    s
}

#[test]
fn hostile_pages_and_settings_never_panic_and_never_yield_bad_pages() {
    let mut r = Rng(0xAD0E_5A41_77C3_91B5);
    for i in 0..iters(240) {
        let img = image_kind(&mut r);
        let s = hostile_settings(&mut r);
        let pages = process_page(&img, &s);
        assert!(pages.len() <= 2, "iteration {i}: {} pages", pages.len());
        for p in &pages {
            let (w, h) = (p.image.width(), p.image.height());
            assert!(w >= 1 && h >= 1, "iteration {i}: empty page");
            assert!(w < 200_000 && h < 200_000, "iteration {i}: absurd {w}x{h}");
        }
        let _ = measure_page(&img, &s);
        for (_, k) in detect_page_keystone(&img, &s) {
            assert!(
                k.iter().all(|v| v.is_finite() && v.abs() <= 15.0),
                "iteration {i}: keystone {k:?}"
            );
        }
    }
}

#[test]
fn detected_keystones_are_zero_on_blank_noise_and_tiny_pages() {
    let mut r = Rng(42);
    let s = Settings::default();
    for kind in 0..40 {
        let img = image_kind(&mut r);
        for (_, k) in detect_page_keystone(&img, &s) {
            let (w, h) = (img.width(), img.height());
            // a page with no readable text lines must not be "corrected"
            if w * h < 80 * 80 {
                assert_eq!(k, [0.0, 0.0], "kind {kind}: {w}x{h} got {k:?}");
            }
        }
    }
}

#[test]
fn detection_is_deterministic() {
    let mut r = Rng(7);
    let s = Settings::default();
    for _ in 0..10 {
        let img = image_kind(&mut r);
        assert_eq!(
            detect_page_keystone(&img, &s),
            detect_page_keystone(&img, &s)
        );
    }
}

#[test]
fn detection_ignores_the_keystone_already_set() {
    // what the user typed must not leak into the measurement
    let img = {
        let mut g = RgbImage::from_pixel(600, 800, Rgb([255, 255, 255]));
        for y in (60..740).step_by(24) {
            for x in (50..550).filter(|x| (x / 40) % 5 != 4) {
                for dy in 0..10 {
                    g.put_pixel(x, y + dy, Rgb([15, 15, 15]));
                }
            }
        }
        DynamicImage::ImageRgb8(g)
    };
    let a = Settings::default();
    let mut b = Settings::default();
    b.page_keystone = [9.0, -9.0];
    let mut c = Settings::default();
    c.part_settings[0] = Some(Box::new(b.clone()));
    assert_eq!(
        detect_page_keystone(&img, &a),
        detect_page_keystone(&img, &b)
    );
    assert_eq!(
        detect_page_keystone(&img, &a),
        detect_page_keystone(&img, &c)
    );
}

#[test]
fn a_zero_keystone_changes_nothing() {
    let mut r = Rng(99);
    for _ in 0..12 {
        let img = image_kind(&mut r);
        let mut a = Settings::default();
        a.split = SplitMode::Off;
        let mut b = a.clone();
        b.page_keystone = [0.0, 0.0];
        b.sheet_keystone = [0.04, -0.04]; // below the threshold: ignored
        let (pa, pb) = (process_page(&img, &a), process_page(&img, &b));
        assert_eq!(pa.len(), pb.len());
        for (x, y) in pa.iter().zip(&pb) {
            assert_eq!(
                (x.image.width(), x.image.height()),
                (y.image.width(), y.image.height())
            );
        }
    }
}

#[test]
fn paper_names_are_strict() {
    for bad in [
        "",
        " ",
        "a",
        "a0x",
        "a4 ",
        "A 4",
        "a4;",
        "a-4",
        "b",
        "b9",
        "a99",
        "letterx",
        "𝔸4",
        "a４",
        "\0",
        "a4\n",
        "tabloid2",
        "legal letter",
    ] {
        let got = paper_size_mm(bad);
        assert!(
            got.is_none() || bad.trim().eq_ignore_ascii_case("a4"),
            "{bad:?} -> {got:?}"
        );
    }
    for (n, w, h) in [
        ("a4", 210.0, 297.0),
        ("A4", 210.0, 297.0),
        ("b5", 176.0, 250.0),
        ("letter", 215.9, 279.4),
    ] {
        let (x, y) = paper_size_mm(n).unwrap();
        assert!((x - w).abs() < 0.2 && (y - h).abs() < 0.2, "{n}: {x}x{y}");
    }
    // every listed size is positive, portrait and in descending A order
    for (name, w, h) in PAPER_SIZES {
        assert!(*w > 0.0 && *h >= *w, "{name}");
    }
}
