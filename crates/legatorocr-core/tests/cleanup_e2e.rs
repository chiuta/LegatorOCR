//! End-to-end checks of the clean-up stages through `process_page`.
use image::{DynamicImage, Rgb, RgbImage};
use legatorocr_core::{
    process_page, Dropout, OutputMode, PageImage, Settings, SplitMode, Threshold,
};

fn text_page(ink: [u8; 3], paper: [u8; 3]) -> RgbImage {
    let mut img = RgbImage::from_pixel(1200, 1600, Rgb(paper));
    for line in 0..20u32 {
        for y in 0..14 {
            for x in 0..700 {
                if (x / 9) % 3 != 2 {
                    img.put_pixel(200 + x, 300 + line * 40 + y, Rgb(ink));
                }
            }
        }
    }
    img
}

fn base() -> Settings {
    Settings {
        dpi: 200,
        split: SplitMode::Off,
        auto_orient: false,
        deskew: false,
        mode: OutputMode::BlackWhite,
        ..Settings::default()
    }
}

fn ink_count(p: &PageImage) -> u64 {
    match p {
        PageImage::Bw(b) => b.count_ink(),
        _ => panic!("expected 1-bit page"),
    }
}

fn ink_fraction(p: &PageImage) -> f64 {
    match p {
        PageImage::Bw(b) => b.count_ink() as f64 / (b.w as f64 * b.h as f64),
        _ => panic!("expected 1-bit page"),
    }
}

#[test]
fn negative_page_comes_out_like_the_positive() {
    let pos = process_page(
        &DynamicImage::ImageRgb8(text_page([10, 10, 10], [240, 240, 240])),
        &base(),
    );
    let neg_img = text_page([240, 240, 240], [10, 10, 10]);
    let off = process_page(&DynamicImage::ImageRgb8(neg_img.clone()), &base());
    let on = process_page(
        &DynamicImage::ImageRgb8(neg_img),
        &Settings {
            auto_invert: true,
            ..base()
        },
    );
    let (fp, fon) = (ink_fraction(&pos[0].image), ink_fraction(&on[0].image));
    assert!(
        (fp - fon).abs() < 0.02,
        "inverted negative ≈ positive: {fp} vs {fon}"
    );
    assert!(
        off[0].blank || ink_fraction(&off[0].image) != fon,
        "without --invert the result differs"
    );
}

#[test]
fn red_dropout_removes_red_rules_from_the_output() {
    let mut img = text_page([10, 10, 10], [245, 245, 245]);
    for y in (100..1500).step_by(60) {
        for x in 50..1150 {
            img.put_pixel(x, y, Rgb([225, 40, 40]));
        }
    }
    let keep = process_page(&DynamicImage::ImageRgb8(img.clone()), &base());
    let drop = process_page(
        &DynamicImage::ImageRgb8(img),
        &Settings {
            dropout: Dropout::Red,
            ..base()
        },
    );
    assert!(
        ink_count(&keep[0].image) > ink_count(&drop[0].image) + 15_000,
        "red rules were removed"
    );
}

#[test]
fn ruled_lines_are_removed_in_bw() {
    let mut img = text_page([10, 10, 10], [245, 245, 245]);
    for y in [200u32, 1400] {
        for x in 100..1100 {
            img.put_pixel(x, y, Rgb([0, 0, 0]));
        }
    }
    let keep = process_page(&DynamicImage::ImageRgb8(img.clone()), &base());
    let gone = process_page(
        &DynamicImage::ImageRgb8(img),
        &Settings {
            remove_lines: true,
            ..base()
        },
    );
    assert!(
        ink_count(&keep[0].image) > ink_count(&gone[0].image) + 1_500,
        "rules were erased"
    );
}

#[test]
fn every_threshold_including_auto_produces_ink_on_text() {
    for t in Threshold::ALL {
        let p = process_page(
            &DynamicImage::ImageRgb8(text_page([10, 10, 10], [240, 240, 240])),
            &Settings {
                threshold: t,
                ..base()
            },
        );
        assert!(ink_fraction(&p[0].image) > 0.05, "{}", t.name());
    }
}

#[test]
fn sharpen_and_levels_keep_geometry() {
    let img = DynamicImage::ImageRgb8(text_page([90, 90, 90], [200, 200, 200]));
    let a = process_page(
        &img,
        &Settings {
            mode: OutputMode::Color,
            ..base()
        },
    );
    let b = process_page(
        &img,
        &Settings {
            mode: OutputMode::Color,
            auto_levels: true,
            sharpen: 1.0,
            ..base()
        },
    );
    assert_eq!(a[0].image.width(), b[0].image.width());
    assert_eq!(a[0].image.height(), b[0].image.height());
}

#[test]
fn erase_zones_remove_ink_in_page_and_sheet_coordinates() {
    let src = DynamicImage::ImageRgb8(text_page([10, 10, 10], [240, 240, 240]));
    let plain = process_page(&src, &base());
    let full = ink_count(&plain[0].image);
    // 200 dpi: the first 10 text lines sit at y = 300..700 px = 38..89 mm, x = 25..114 mm
    let zone = vec![[20.0, 36.0, 100.0, 50.0]]; // covers lines 0..~10
    for sheet in [false, true] {
        let mut s = base();
        if sheet {
            s.sheet_erase_zones = zone.clone();
        } else {
            s.erase_zones = zone.clone();
        }
        let out = process_page(&src, &s);
        let less = ink_count(&out[0].image);
        assert!(less < full * 3 / 4, "sheet={sheet}: {less} vs {full}");
        assert!(
            less > full / 8,
            "sheet={sheet}: the other lines must survive ({less} vs {full})"
        );
    }
}
