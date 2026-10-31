#![allow(
    clippy::manual_is_multiple_of,
    clippy::field_reassign_with_default,
    clippy::type_complexity
)]
//! Extreme-variety fuzzing of the whole core pipeline and every building block.
//!
//! Deterministic (seeded) so failures reproduce. Scale with
//! `LEGATOROCR_FUZZ_ITERS=2000 LEGATOROCR_FUZZ_SEED=7 cargo test --release -p legatorocr-core --test fuzz_extreme`.
//! Default iteration counts are small enough for CI.

use image::{DynamicImage, Luma, Rgb, RgbImage, Rgba, RgbaImage};
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
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

fn trace(s: &str) {
    if std::env::var_os("LEGATOROCR_FUZZ_TRACE").is_some() {
        eprintln!("  > {s}");
    }
}

fn iters(default: u64) -> u64 {
    // debug builds (the CI default) are ~10x slower: fewer iterations, same code paths
    let d = if cfg!(debug_assertions) {
        (default / 4).max(10)
    } else {
        default
    };
    std::env::var("LEGATOROCR_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}
fn seed() -> u64 {
    std::env::var("LEGATOROCR_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_1234_ABCD_EF01)
        | 1
}

/// One of many hostile image kinds.
fn gen_image(r: &mut Rng, max_side: u32) -> (RgbImage, &'static str) {
    // dimensions: tiny, degenerate lines, extreme aspect, ordinary
    let (w, h) = match r.below(8) {
        0 => (1 + r.below(8) as u32, 1 + r.below(8) as u32),
        1 => (1, 1 + r.below(max_side as u64) as u32),
        2 => (1 + r.below(max_side as u64) as u32, 1),
        3 => (
            2 + r.below(5) as u32,
            200 + r.below(max_side as u64 * 2) as u32,
        ),
        4 => (
            200 + r.below(max_side as u64 * 2) as u32,
            2 + r.below(5) as u32,
        ),
        _ => (
            16 + r.below(max_side as u64) as u32,
            16 + r.below(max_side as u64) as u32,
        ),
    };
    let paper = r.pick(&[255u8, 240, 200, 128, 30, 0]);
    let ink = r.pick(&[0u8, 20, 60, 255, 128]);
    let mut img = RgbImage::from_pixel(
        w,
        h,
        Rgb([paper, paper, paper.saturating_sub(r.below(30) as u8)]),
    );
    let kind = r.below(16);
    let name = match kind {
        0 => "paper",
        1 => "noise",
        2 => "text-bars",
        3 => "stripes-h",
        4 => "stripes-v",
        5 => "checker",
        6 => "gradient",
        7 => "salt-pepper",
        8 => "black-frame",
        9 => "two-columns",
        10 => "photo-block",
        11 => "diagonal",
        12 => "circles",
        13 => "one-pixel",
        14 => "extreme-contrast-noise",
        _ => "colour-blocks",
    };
    let put = |img: &mut RgbImage, x: u32, y: u32, v: [u8; 3]| {
        if x < img.width() && y < img.height() {
            img.put_pixel(x, y, Rgb(v));
        }
    };
    match kind {
        1 => {
            for p in img.pixels_mut() {
                *p = Rgb([r.below(256) as u8, r.below(256) as u8, r.below(256) as u8]);
            }
        }
        2 => {
            let pitch = 8 + r.below(60) as u32;
            let bar = 3 + r.below(pitch as u64 / 2 + 1) as u32;
            let (mx, my) = (w / 10, h / 10);
            let skew = (r.below(9) as f32 - 4.0) / 100.0;
            let mut y = my;
            while y + bar < h.saturating_sub(my) {
                for x in (mx..w.saturating_sub(mx)).step_by(1 + r.below(4) as usize) {
                    if (x / (3 + r.below(5) as u32)) % 2 == 0 {
                        let yy = y as f32 + skew * (x as f32 - w as f32 / 2.0);
                        for dy in 0..bar {
                            put(
                                &mut img,
                                x,
                                (yy as i64 + dy as i64).max(0) as u32,
                                [ink, ink, ink],
                            );
                        }
                    }
                }
                y += pitch;
            }
        }
        3 => (0..h)
            .filter(|y| (y / (1 + r.below(6) as u32)) % 2 == 0)
            .collect::<Vec<_>>()
            .iter()
            .for_each(|&y| {
                for x in 0..w {
                    put(&mut img, x, y, [ink; 3]);
                }
            }),
        4 => (0..w)
            .filter(|x| x % (2 + r.below(9) as u32) == 0)
            .collect::<Vec<_>>()
            .iter()
            .for_each(|&x| {
                for y in 0..h {
                    put(&mut img, x, y, [ink; 3]);
                }
            }),
        5 => {
            let c = 1 + r.below(30) as u32;
            for y in 0..h {
                for x in 0..w {
                    if (x / c + y / c) % 2 == 0 {
                        put(&mut img, x, y, [ink; 3]);
                    }
                }
            }
        }
        6 => {
            for y in 0..h {
                for x in 0..w {
                    let v = ((x as u64 * 255) / w.max(1) as u64) as u8;
                    put(&mut img, x, y, [v, 255 - v, v / 2]);
                }
            }
        }
        7 => {
            for _ in 0..(w as u64 * h as u64 / (3 + r.below(200))) {
                let (x, y) = (r.below(w as u64) as u32, r.below(h as u64) as u32);
                put(&mut img, x, y, [ink; 3]);
            }
        }
        8 => {
            let t = 1 + r.below(40) as u32;
            for y in 0..h {
                for x in 0..w {
                    if x < t || y < t || x + t >= w || y + t >= h {
                        put(&mut img, x, y, [ink; 3]);
                    }
                }
            }
        }
        9 => {
            let mid = w / 2;
            for y in (h / 10..h - h / 10).step_by(11) {
                for x in (w / 12..w - w / 12).step_by(7) {
                    if x.abs_diff(mid) > 6 + r.below(20) as u32 {
                        for dy in 0..5 {
                            put(&mut img, x, y + dy, [ink; 3]);
                        }
                    }
                }
            }
        }
        10 => {
            let (x0, y0) = (r.below(w as u64) as u32, r.below(h as u64) as u32);
            for y in y0..(y0 + h / 2).min(h) {
                for x in x0..(x0 + w / 2).min(w) {
                    let v = r.below(256) as u8;
                    put(&mut img, x, y, [v, v / 2, 255 - v]);
                }
            }
        }
        11 => {
            for i in 0..w.max(h) * 2 {
                put(&mut img, i % w, (i / 2) % h, [ink; 3]);
            }
        }
        12 => {
            for _ in 0..1 + r.below(8) {
                let (cx, cy, rad) = (
                    r.below(w as u64) as i64,
                    r.below(h as u64) as i64,
                    1 + r.below(60) as i64,
                );
                for y in (cy - rad).max(0)..(cy + rad).min(h as i64) {
                    for x in (cx - rad).max(0)..(cx + rad).min(w as i64) {
                        if (x - cx).pow(2) + (y - cy).pow(2) <= rad * rad {
                            put(&mut img, x as u32, y as u32, [ink; 3]);
                        }
                    }
                }
            }
        }
        13 => put(
            &mut img,
            r.below(w as u64) as u32,
            r.below(h as u64) as u32,
            [ink; 3],
        ),
        14 => {
            for p in img.pixels_mut() {
                let v = if r.chance(50) { 0 } else { 255 };
                *p = Rgb([v, v, v]);
            }
        }
        15 => {
            for _ in 0..1 + r.below(30) {
                let (x0, y0) = (r.below(w as u64) as u32, r.below(h as u64) as u32);
                let (bw, bh) = (
                    1 + r.below(w as u64 / 2 + 1) as u32,
                    1 + r.below(h as u64 / 2 + 1) as u32,
                );
                let c = [r.below(256) as u8, r.below(256) as u8, r.below(256) as u8];
                for y in y0..(y0 + bh).min(h) {
                    for x in x0..(x0 + bw).min(w) {
                        put(&mut img, x, y, c);
                    }
                }
            }
        }
        _ => {}
    }
    (img, name)
}

fn gen_settings(r: &mut Rng) -> Settings {
    let wild_f = |r: &mut Rng, lo: f32, hi: f32| -> f32 {
        match r.below(14) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => -1e30,
            4 => 1e30,
            5 => 0.0,
            6 => -0.0,
            _ => lo + (hi - lo) * (r.below(10_000) as f32 / 10_000.0),
        }
    };
    let mut s = Settings::default();
    s.dpi = match r.below(12) {
        0 => 0,
        1 => 1,
        2 => u32::MAX,
        3 => 2400,
        _ => r.pick(&[36, 72, 96, 150, 200, 300, 400, 600]),
    };
    s.rotate_quarters = r.below(256) as u8;
    s.auto_orient = r.chance(50);
    s.split = r.pick(&[SplitMode::Off, SplitMode::Auto, SplitMode::Middle]);
    s.deskew = r.chance(60);
    s.manual_skew_deg = r.chance(25).then(|| wild_f(r, -60.0, 60.0));
    s.max_skew_deg = wild_f(r, 0.0, 20.0);
    s.dewarp = r.chance(40);
    for m in s.margins_mm.iter_mut() {
        *m = wild_f(r, 0.0, 60.0);
    }
    for a in s.content_adjust_mm.iter_mut() {
        *a = wild_f(r, -60.0, 60.0);
    }
    s.page_size_mm = r
        .chance(40)
        .then(|| (wild_f(r, 0.0, 500.0), wild_f(r, 0.0, 500.0)));
    s.align = (
        r.pick(&[HAlign::Left, HAlign::Center, HAlign::Right]),
        r.pick(&[VAlign::Top, VAlign::Center, VAlign::Bottom]),
    );
    s.mode = r.pick(&[
        OutputMode::Color,
        OutputMode::Gray,
        OutputMode::BlackWhite,
        OutputMode::Mixed,
        OutputMode::Hybrid,
    ]);
    s.threshold = r.pick(&Threshold::ALL);
    s.sauvola_k = wild_f(r, 0.0, 1.0);
    s.despeckle = r.chance(50);
    s.normalize_bg = r.chance(50);
    s.output_dpi = r
        .chance(40)
        .then(|| r.pick(&[0, 1, 35, 36, 50, 150, 300, 5000, u32::MAX]));
    s.picture_sensitivity = wild_f(r, 0.0, 1.0);
    s.hybrid_bg_dpi = r.pick(&[0, 1, 36, 100, 300, 100_000, u32::MAX]);
    s.auto_invert = r.chance(40);
    s.auto_levels = r.chance(40);
    s.sharpen = wild_f(r, -1.0, 3.0);
    s.punch_holes = r.chance(40);
    s.fingers = r.chance(40);
    s.remove_lines = r.chance(40);
    s.erase_zones = (0..r.below(4))
        .map(|_| {
            [
                wild_f(r, -50.0, 400.0),
                wild_f(r, -50.0, 400.0),
                wild_f(r, -5.0, 400.0),
                wild_f(r, -5.0, 400.0),
            ]
        })
        .collect();
    s.dropout = r.pick(&legatorocr_core::Dropout::ALL);
    s.despeckle_strength = wild_f(r, 0.0, 6.0);
    s.show_through = r.chance(40);
    s.descreen = r.chance(40);
    s.perspective = r.chance(40);
    s
}

fn check_page(p: &Page, ctx: &str) {
    let (w, h) = (p.image.width(), p.image.height());
    assert!(w >= 1 && h >= 1, "{ctx}: empty page");
    assert!(w <= 40_000 && h <= 40_000, "{ctx}: absurd page {w}x{h}");
    assert!((36..=2400).contains(&p.dpi), "{ctx}: dpi {}", p.dpi);
    assert!(p.skew_deg.is_finite(), "{ctx}: skew");
    match &p.image {
        PageImage::Rgb(i) => assert_eq!(i.as_raw().len(), (w * h * 3) as usize),
        PageImage::Gray(i) => assert_eq!(i.as_raw().len(), (w * h) as usize),
        PageImage::Bw(b) => assert_eq!(b.data.len(), (w * h) as usize),
    }
    if let Some(l) = &p.layers {
        assert_eq!((l.mask.w, l.mask.h), (w, h), "{ctx}: mask size");
        for pa in &l.patches {
            assert!(
                pa.x + pa.img.width() <= w && pa.y + pa.img.height() <= h,
                "{ctx}: patch outside page"
            );
        }
        let flat = l.compose();
        assert_eq!(flat.dimensions(), (w, h), "{ctx}: compose size");
    }
}

fn digest(p: &Page) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut feed = |b: &[u8]| {
        for &x in b {
            h = (h ^ x as u64).wrapping_mul(0x100000001b3);
        }
    };
    match &p.image {
        PageImage::Rgb(i) => feed(i.as_raw()),
        PageImage::Gray(i) => feed(i.as_raw()),
        PageImage::Bw(b) => feed(&b.data),
    }
    feed(&p.image.width().to_le_bytes());
    feed(&p.image.height().to_le_bytes());
    h
}

#[test]
fn pipeline_survives_everything_and_is_deterministic() {
    let mut r = Rng(seed());
    let n = iters(150);
    for it in 0..n {
        let (img, name) = gen_image(&mut r, 700);
        let s = gen_settings(&mut r);
        let ctx = format!(
            "iter {it} image={name} {}x{} mode={:?} dpi={} thr={:?}",
            img.width(),
            img.height(),
            s.mode,
            s.dpi,
            s.threshold
        );
        // Convert to a few input representations the loaders can produce.
        let dynimg = match r.below(4) {
            0 => DynamicImage::ImageRgb8(img.clone()),
            1 => DynamicImage::ImageLuma8(image::imageops::grayscale(&img)),
            2 => DynamicImage::ImageRgba8(RgbaImage::from_fn(img.width(), img.height(), |x, y| {
                let p = img.get_pixel(x, y);
                Rgba([p[0], p[1], p[2], 255])
            })),
            _ => DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(
                img.width(),
                img.height(),
                |x, y| Luma([(img.get_pixel(x, y)[0] as u16) << 8]),
            )),
        };
        if std::env::var_os("LEGATOROCR_FUZZ_VERBOSE").is_some() {
            eprintln!("{ctx} settings={s:?}");
        }
        let pages = process_page(&dynimg, &s);
        assert!(
            !pages.is_empty() && pages.len() <= 2,
            "{ctx}: {} pages",
            pages.len()
        );
        for p in &pages {
            check_page(p, &ctx);
        }
        // measure_page must agree with process_page on the number of pages
        let m = measure_page(&dynimg, &s);
        assert_eq!(m.len(), pages.len(), "{ctx}: measure/process page count");
        for sz in m.into_iter().flatten() {
            assert!(
                sz.0.is_finite() && sz.1.is_finite() && sz.0 > 0.0 && sz.1 > 0.0,
                "{ctx}: size {sz:?}"
            );
        }
        // determinism (rayon must not introduce ordering effects)
        if it % 5 == 0 {
            let again = process_page(&dynimg, &s);
            assert_eq!(pages.len(), again.len(), "{ctx}");
            for (a, b) in pages.iter().zip(&again) {
                assert_eq!(digest(a), digest(b), "{ctx}: non-deterministic output");
            }
        }
    }
}

#[test]
fn building_blocks_survive_hostile_input() {
    let mut r = Rng(seed() ^ 0xABCDEF);
    let n = iters(300);
    for it in 0..n {
        let (img, name) = gen_image(&mut r, 500);
        let gray = image::imageops::grayscale(&img);
        let ctx = format!("iter {it} {name} {}x{}", img.width(), img.height());
        let dpi = r.pick(&[1u32, 36, 72, 150, 300, 600, 2400]);
        if std::env::var_os("LEGATOROCR_FUZZ_VERBOSE").is_some() {
            eprintln!("blocks {ctx} dpi={dpi}");
        }
        // binarisers, all window/k extremes
        let win = r.pick(&[0u32, 1, 2, 3, 15, 51, 101, 1001, 100_000, u32::MAX]);
        let k = r.pick(&[f32::NAN, -5.0, 0.0, 0.3, 1.0, 1e9, f32::INFINITY]);
        for m in Threshold::ALL {
            let b = binarize::threshold(&gray, m, win, k);
            assert_eq!((b.w, b.h), gray.dimensions(), "{ctx}: threshold {m:?}");
        }
        let b = binarize::sauvola(&gray, win, k);
        trace("let b = binarize::sauvola(&gray, win, k);");
        let _ = (
            content::detect_content(&b, dpi),
            split::find_gutter(&b),
            deskew::detect_skew(&b, r.pick(&[0.0, 1.0, 7.0, 90.0, f32::NAN])),
            orient::needs_quarter_turn(&b, dpi),
        );
        trace("let _ = (content::detect_content(&b, dpi), split::find_gutte");
        // geometry helpers
        let ang = r.pick(&[
            0.0f32,
            0.01,
            1.2,
            -44.0,
            89.0,
            180.0,
            f32::NAN,
            f32::INFINITY,
            1e20,
        ]);
        let _ = transform::rotate_cw(&img, ang, Rgb([255; 3]));
        trace("let _ = transform::rotate_cw(&img, ang, Rgb([255; 3]));");
        let _ = transform::rot90(&img, r.below(300) as u8);
        trace("let _ = transform::rot90(&img, r.below(300) as u8);");
        let _ = transform::estimate_background(&img);
        trace("let _ = transform::estimate_background(&img);");
        let _ = transform::crop_pad(
            &img,
            r.below(2000) as i64 - 1000,
            r.below(2000) as i64 - 1000,
            r.below(1500) as u32,
            r.below(1500) as u32,
            Rgb([9; 3]),
        );
        trace("let _ = transform::crop_pad(&img, r.below(2000) as i64 - 100");
        let _ = transform::downscale_gray(&gray, r.pick(&[0u32, 1, 2, 7, 1000, u32::MAX]));
        trace("let _ = transform::downscale_gray(&gray, r.pick(&[0u32, 1, 2");
        let _ = transform::normalize_background(&img, dpi, Rgb([255; 3]));
        trace("let _ = transform::normalize_background(&img, dpi, Rgb([255;");
        // zones, dewarp
        let _ = pictures::detect(&gray, dpi, r.pick(&[f32::NAN, -1.0, 0.0, 0.5, 1.0, 9.0]));
        trace("let _ = pictures::detect(&gray, dpi, r.pick(&[f32::NAN, -1.0");
        let _ = dewarp::dewarp(&img, dpi, Rgb([255; 3]));
        trace("let _ = dewarp::dewarp(&img, dpi, Rgb([255; 3]));");
        // bit-image ops with out-of-range arguments
        let mut b2 = BitImage::new(img.width(), img.height());
        let mut dst = BitImage::new(1 + r.below(300) as u32, 1 + r.below(300) as u32);
        dst.paste_region(
            &b,
            (
                r.below(2000) as i64 - 1000,
                r.below(2000) as i64 - 1000,
                r.below(3000) as u32,
                r.below(3000) as u32,
            ),
            (r.below(2000) as i64 - 1000, r.below(2000) as i64 - 1000),
        );
        b2.despeckle(r.pick(&[0u32, 1, 5, 1000, u32::MAX]));
        let w = b.w;
        let (a, c) = (r.below(w as u64 + 5) as u32, r.below(w as u64 + 5) as u32);
        let _ = b.crop_x(a.min(w), c.min(w).max(a.min(w)));
        trace("let _ = b.crop_x(a.min(w), c.min(w).max(a.min(w)));");
        // layers with random content
        let layers = layers::build_background(
            &img,
            &layers::dilate(&b),
            r.pick(&[1u32, 2, 3, 7, 50, 5000]),
        );
        trace("let layers = layers::build_background(&img, &layers::dilate(");
        assert!(
            layers.width() >= 1 && layers.height() >= 1,
            "{ctx}: bg empty"
        );
        let _ = layers::ink_colour(&img, &b);
        trace("let _ = layers::ink_colour(&img, &b);");
    }
}
