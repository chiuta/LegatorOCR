//! Adversarial inputs: the engine must never panic and never return nonsense geometry.
use legatorocr_core::{BitImage, PageImage};
use legatorocr_nocr::{recognize, Model, NativeOptions};

fn model() -> &'static Model {
    legatorocr_nocr::embedded_model().expect("model compiled in")
}

fn lcg(seed: &mut u64) -> u32 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) as u32
}

fn check(words: &[legatorocr_core::Word], w: u32, h: u32) {
    for x in words {
        assert!(x.w >= 1 && x.h >= 1, "empty box {x:?}");
        assert!(
            x.x + x.w <= w + 4 && x.y + x.h <= h + 4,
            "box outside {w}x{h}: {x:?}"
        );
        assert!(!x.text.is_empty());
        assert!(x.conf.is_finite());
    }
}

#[test]
fn odd_sizes_and_noise_never_panic() {
    let mut seed = 7u64;
    let sizes = [
        (1, 1),
        (2, 2),
        (3, 50),
        (50, 3),
        (1, 500),
        (500, 1),
        (31, 31),
        (64, 64),
        (333, 17),
        (17, 333),
        (200, 200),
        (1000, 40),
    ];
    for &(w, h) in &sizes {
        for kind in 0..4 {
            let g = image::GrayImage::from_fn(w, h, |x, y| {
                image::Luma([match kind {
                    0 => 255,
                    1 => 0,
                    2 => (lcg(&mut seed) & 0xff) as u8,
                    _ => {
                        if (x / 3 + y / 5) % 2 == 0 {
                            0
                        } else {
                            255
                        }
                    }
                }])
            });
            let words = recognize(
                &PageImage::Gray(g),
                300,
                model(),
                &NativeOptions {
                    lang: "ro+en".into(),
                    ..Default::default()
                },
            );
            check(&words, w, h);
        }
    }
}

#[test]
fn bilevel_and_rgb_inputs_and_odd_dpi() {
    let mut b = BitImage::new(120, 80);
    for y in 20..40 {
        for x in 10..110 {
            b.set(x, y, (x / 4) % 2 == 0);
        }
    }
    for dpi in [0u32, 1, 72, 300, 2400, u32::MAX] {
        let words = recognize(
            &PageImage::Bw(b.clone()),
            dpi,
            model(),
            &NativeOptions::default(),
        );
        check(&words, 120, 80);
    }
    let rgb = image::RgbImage::from_fn(90, 60, |x, y| {
        image::Rgb([(x * 3) as u8, (y * 4) as u8, 128])
    });
    check(
        &recognize(
            &PageImage::Rgb(rgb),
            300,
            model(),
            &NativeOptions::default(),
        ),
        90,
        60,
    );
}

#[test]
fn full_black_and_full_white_pages_give_nothing_or_bounded_output() {
    for v in [0u8, 255] {
        let g = image::GrayImage::from_pixel(600, 800, image::Luma([v]));
        let words = recognize(&PageImage::Gray(g), 300, model(), &NativeOptions::default());
        assert!(
            words.len() < 20,
            "constant page {v} gave {} words",
            words.len()
        );
    }
}
