#![allow(clippy::manual_is_multiple_of, clippy::field_reassign_with_default, clippy::type_complexity)]
//! Metamorphic / property tests: transformations whose effect on the result is known
//! exactly, run over many random text-like pages. They catch *quality* regressions that
//! crash-only fuzzing cannot (wrong angle, drifting crop, unstable thresholds).

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
}

/// A text-like page at 150 dpi: lines of word-shaped bars, random margins and pitch.
fn text_page(r: &mut Rng, w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb([240, 236, 222]));
    let (mx, my) = (w / 8 + r.below(30) as u32, h / 9 + r.below(30) as u32);
    let pitch = 22 + r.below(14) as u32;
    let mut y = my;
    while y + 14 < h - my {
        let mut x = mx;
        while x < w - mx - 40 {
            let word = 20 + r.below(60) as u32;
            for xx in x..(x + word).min(w - mx) {
                if (xx / 3) % 3 != 0 {
                    for dy in 0..11 {
                        img.put_pixel(xx, y + dy, Rgb([28, 26, 24]));
                    }
                }
            }
            x += word + 12;
        }
        y += pitch;
    }
    img
}

fn base() -> Settings {
    Settings { dpi: 150, split: SplitMode::Off, auto_orient: false, margins_mm: [4.0; 4], ..Settings::default() }
}

fn one(img: &RgbImage, s: &Settings) -> Page {
    let mut p = process_page(&DynamicImage::ImageRgb8(img.clone()), s);
    assert_eq!(p.len(), 1);
    p.remove(0)
}

#[test]
fn deskew_recovers_the_applied_angle() {
    let mut r = Rng(0x1234_5678_9ABC_DEF1);
    let mut worst = 0f32;
    for i in 0..24 {
        let page = text_page(&mut r, 900, 1200);
        let angle = (r.below(101) as f32 - 50.0) / 10.0; // −5.0 … +5.0°
        // Rotating the page by `angle` (counter-clockwise for positive in rotate_cw's sign) …
        let skewed = transform::rotate_cw(&page, angle.to_radians(), Rgb([240, 236, 222]));
        let p = one(&skewed, &Settings { max_skew_deg: 7.0, ..base() });
        // … must be detected as −angle (the correction), within 0.25°.
        let err = (p.skew_deg + angle).abs();
        worst = worst.max(err);
        assert!(err < 0.25, "page {i}: applied {angle:.2}°, corrected {:.2}° (error {err:.2}°)", p.skew_deg);
    }
    eprintln!("worst deskew error {worst:.3}°");
}

#[test]
fn processing_is_nearly_idempotent() {
    let mut r = Rng(0xDEAD_BEEF_0000_0001);
    for i in 0..12 {
        let page = text_page(&mut r, 900, 1200);
        let skewed = transform::rotate_cw(&page, ((r.below(61) as f32 - 30.0) / 10.0).to_radians(), Rgb([240, 236, 222]));
        let s = base();
        let first = one(&skewed, &s);
        let PageImage::Rgb(img1) = &first.image else { panic!() };
        let second = one(img1, &s);
        assert!(second.skew_deg.abs() < 0.12, "page {i}: second pass still rotates {:.2}°", second.skew_deg);
        let (dw, dh) = (first.image.width().abs_diff(second.image.width()), first.image.height().abs_diff(second.image.height()));
        assert!(dw <= 6 && dh <= 6, "page {i}: size drifted by {dw}×{dh} px on re-processing");
    }
}

#[test]
fn margins_are_exact_and_content_is_never_cut() {
    let mut r = Rng(0xABCD_EF01_2345_6789);
    for i in 0..16 {
        let page = text_page(&mut r, 800, 1100);
        let mm = 3.0 + r.below(20) as f32;
        let s = Settings { deskew: false, margins_mm: [mm; 4], mode: OutputMode::BlackWhite, ..base() };
        let p = one(&page, &s);
        let PageImage::Bw(b) = &p.image else { panic!() };
        // ink bounding box
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..b.h {
            for x in 0..b.w {
                if b.get(x, y) {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        let want = (mm / 25.4 * 150.0).round() as i64;
        for (name, got) in [("left", x0 as i64), ("top", y0 as i64), ("right", (b.w - 1 - x1) as i64), ("bottom", (b.h - 1 - y1) as i64)] {
            assert!((got - want).abs() <= 4, "page {i}: {name} margin {got}px, wanted {want}px ({mm} mm)");
        }
    }
}

#[test]
fn quarter_turns_are_detected_regardless_of_direction() {
    let mut r = Rng(0x0F0F_F0F0_1234_4321);
    for i in 0..12 {
        let page = text_page(&mut r, 900, 1200);
        let upright = one(&page, &Settings { auto_orient: true, ..base() });
        for q in [1u8, 3] {
            let turned = transform::rot90(&page, q);
            let fixed = one(&turned, &Settings { auto_orient: true, ..base() });
            // an upright page and its 90°/270° version must produce pages of the same shape
            let (dw, dh) = (upright.image.width().abs_diff(fixed.image.width()), upright.image.height().abs_diff(fixed.image.height()));
            assert!(dw <= 8 && dh <= 8, "page {i} q={q}: {}x{} vs {}x{}", upright.image.width(), upright.image.height(), fixed.image.width(), fixed.image.height());
        }
    }
}

#[test]
fn black_white_is_invariant_to_lighting_changes() {
    let mut r = Rng(0x5555_AAAA_5555_AAAA);
    for i in 0..10 {
        let page = text_page(&mut r, 700, 900);
        // multiply by a smooth vignette: text stays darker than paper everywhere
        let dim = RgbImage::from_fn(700, 900, |x, y| {
            let p = page.get_pixel(x, y);
            let k = 1.0 - 0.35 * (((x as f32 / 700.0 - 0.5).powi(2) + (y as f32 / 900.0 - 0.5).powi(2)) * 2.0);
            Rgb([(p[0] as f32 * k) as u8, (p[1] as f32 * k) as u8, (p[2] as f32 * k) as u8])
        });
        let s = Settings { deskew: false, mode: OutputMode::BlackWhite, margins_mm: [2.0; 4], ..base() };
        let (a, b) = (one(&page, &s), one(&dim, &s));
        let (PageImage::Bw(a), PageImage::Bw(b)) = (&a.image, &b.image) else { panic!() };
        let (ia, ib) = (a.count_ink() as f64, b.count_ink() as f64);
        assert!((ia - ib).abs() / ia.max(1.0) < 0.12, "page {i}: ink {ia} vs {ib} under vignette");
        assert!(ib > 100.0, "page {i}: text vanished under vignette");
    }
}
