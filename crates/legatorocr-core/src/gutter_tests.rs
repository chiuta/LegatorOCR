//! Synthetic regressions: text near the spine clipped by the content box, and a dark strip /
//! gutter shadow at the page margin surviving in the output.

use crate::pipeline::testutil::{text_page, to_rgb};
use crate::pipeline::{process_page, OutputMode, Page, PageImage, Settings, SplitMode};
use crate::BitImage;
use image::{DynamicImage, GrayImage, Luma};

const W: u32 = 1200;
const H: u32 = 1600;
const TEXT_X0: u32 = 70; // text starts close to the spine (left edge)
const TEXT_X1: u32 = 1000;

fn page() -> GrayImage {
    text_page(W, H, TEXT_X0, TEXT_X1)
}

/// Fold shadow: everything darkens linearly toward x = 0 over `width` px, down to `edge`
/// grey for paper at the border (multiplicative, so text darkens too, like a real photo).
fn gutter_shadow(g: &mut GrayImage, width: u32, edge: f32) {
    for y in 0..g.height() {
        for x in 0..width.min(g.width()) {
            let t = x as f32 / width as f32;
            let k = edge / 235.0 + (1.0 - edge / 235.0) * t;
            let p = g.get_pixel_mut(x, y);
            p.0[0] = (p.0[0] as f32 * k) as u8;
        }
    }
}

fn run(g: &GrayImage, mode: OutputMode) -> Page {
    let s = Settings {
        mode,
        dpi: 300,
        margins_mm: [0.0; 4],
        deskew: false,
        auto_orient: false,
        split: SplitMode::Off,
        ..Settings::default()
    };
    let src = DynamicImage::ImageRgb8(to_rgb(g));
    process_page(&src, &s).remove(0)
}

fn bw_of(p: &Page) -> &BitImage {
    match &p.image {
        PageImage::Bw(b) => b,
        _ => panic!("bw expected"),
    }
}

fn longest_vertical_run(b: &BitImage) -> u32 {
    let mut best = 0;
    for x in 0..b.w {
        let mut run = 0;
        for y in 0..b.h {
            if b.get(x, y) {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    best
}

#[test]
fn text_in_the_gutter_shadow_is_not_clipped() {
    let clean = page();
    let mut g = clean.clone();
    gutter_shadow(&mut g, 160, 60.0);
    let reference = run(&clean, OutputMode::BlackWhite);
    let shaded = run(&g, OutputMode::BlackWhite);
    let (rb, sb) = (reference.content_box.unwrap(), shaded.content_box.unwrap());
    assert!(
        sb[2] + 8 >= rb[2],
        "gutter text clipped: clean box {rb:?}, shaded box {sb:?}"
    );
    let kept = bw_of(&shaded).count_ink() as f64 / bw_of(&reference).count_ink() as f64;
    assert!(kept > 0.93, "ink kept {kept}");
}

#[test]
fn dark_strip_at_the_page_edge_beside_the_text_is_not_ink() {
    let mut g = page();
    for y in 0..H {
        for x in 0..24 {
            g.put_pixel(x, y, Luma([18]));
        }
        for x in 24..44 {
            let t = (x - 24) as f32 / 20.0;
            g.put_pixel(x, y, Luma([(18.0 + t * 217.0) as u8]));
        }
    }
    let p = run(&g, OutputMode::BlackWhite);
    let run_len = longest_vertical_run(bw_of(&p));
    assert!(
        run_len < 120,
        "a dark strip survives: vertical run {run_len}"
    );
    let r = p.content_box.unwrap();
    assert!(r[2] < (TEXT_X1 - TEXT_X0) + 40, "box too wide: {r:?}");
}

#[test]
fn dark_strip_and_gutter_shadow_together_keep_all_text_and_drop_the_strip() {
    let clean = page();
    let mut g = clean.clone();
    gutter_shadow(&mut g, 160, 90.0);
    for y in 0..H {
        for x in 0..16 {
            g.put_pixel(x, y, Luma([12]));
        }
    }
    let reference = run(&clean, OutputMode::BlackWhite);
    let p = run(&g, OutputMode::BlackWhite);
    assert!(longest_vertical_run(bw_of(&p)) < 120, "strip survives");
    let kept = bw_of(&p).count_ink() as f64 / bw_of(&reference).count_ink() as f64;
    assert!(kept > 0.93, "text lost: kept {kept}");
    let (rb, sb) = (reference.content_box.unwrap(), p.content_box.unwrap());
    assert!(sb[2] + 8 >= rb[2], "clipped: {rb:?} vs {sb:?}");
}

#[test]
fn gray_output_has_no_dark_column_band_from_the_edge_strip() {
    let mut g = page();
    for y in 0..H {
        for x in 0..24 {
            g.put_pixel(x, y, Luma([18]));
        }
    }
    let p = run(&g, OutputMode::Gray);
    let gray = p.image.to_gray();
    let dark_cols = (0..gray.width())
        .filter(|&x| {
            (0..gray.height())
                .filter(|&y| gray.get_pixel(x, y).0[0] < 60)
                .count() as u32
                > gray.height() * 9 / 10
        })
        .count();
    assert_eq!(dark_cols, 0, "a dark column band survives");
}

/// Marginal column of short lines (line numbers, a gloss) hugging the cut edge of the page.
fn with_margin_column(g: &mut GrayImage, x0: u32, x1: u32, y0: u32, y1: u32) {
    let mut y = y0;
    while y + 14 < y1 {
        for yy in y..y + 14 {
            for xx in x0..x1 {
                g.put_pixel(xx, yy, Luma([30]));
            }
        }
        y += 36;
    }
}

#[test]
fn a_marginal_column_hugging_the_cut_edge_is_kept() {
    // body well inside; a column of line numbers 30 px wide touching the left border
    let mut g = text_page(W, H, 260, 1000);
    with_margin_column(&mut g, 0, 30, 200, 1400);
    let p = run(&g, OutputMode::BlackWhite);
    let b = bw_of(&p);
    // the column must still be there: ink in the left 40 px of the output
    let left: u64 = (0..b.h)
        .map(|y| (0..40).filter(|&x| b.get(x, y)).count() as u64)
        .sum();
    assert!(
        left > 5000,
        "marginal column lost: {left} ink px, box {:?}",
        p.content_box
    );
}

#[test]
fn a_short_text_line_touching_the_cut_edge_is_kept() {
    // a one-line note (a folio, a catchword) cut right at its first letters
    let mut g = text_page(W, H, 260, 1000);
    with_margin_column(&mut g, 0, 40, 1420, 1456);
    for x in (0..40).step_by(12) {
        for y in 1425..1439 {
            g.put_pixel(x, y, Luma([30]));
        }
    }
    let p = run(&g, OutputMode::BlackWhite);
    let r = p.content_box.unwrap();
    assert!(r[3] > 1230, "folio at the border dropped: {r:?}");
}

/// Darken everything within `strip` px of a border to `edge` grey, then fade back to paper over
/// `fade` px. `side`: 0 left, 1 right, 2 top, 3 bottom.
fn border_shadow(g: &mut GrayImage, side: u8, strip: u32, fade: u32, edge: u8) {
    let (w, h) = g.dimensions();
    for y in 0..h {
        for x in 0..w {
            let d = match side {
                0 => x,
                1 => w - 1 - x,
                2 => y,
                _ => h - 1 - y,
            };
            if d < strip + fade {
                let k0 = edge as f32 / 235.0;
                let k = if d < strip {
                    k0
                } else {
                    k0 + (1.0 - k0) * (d - strip) as f32 / fade as f32
                };
                let p = g.get_pixel_mut(x, y);
                p.0[0] = (p.0[0] as f32 * k) as u8;
            }
        }
    }
}

#[test]
fn a_wide_dark_band_at_any_border_does_not_widen_the_content_box() {
    // 150-190 px of a 1200x1600 page is 12-15 % of the width: deeper than the outer tenth
    for (side, strip) in [(0u8, 150u32), (1, 150), (2, 175), (3, 175)] {
        let clean = text_page(W, H, 260, 940);
        let reference = run(&clean, OutputMode::BlackWhite);
        let mut g = clean.clone();
        border_shadow(&mut g, side, strip, 20, 25);
        let p = run(&g, OutputMode::BlackWhite);
        let (rw, rh) = (reference.image.width(), reference.image.height());
        assert!(
            p.image.width() <= rw + 12 && p.image.height() <= rh + 12,
            "side {side}: band kept, page {}x{} instead of {rw}x{rh}",
            p.image.width(),
            p.image.height()
        );
        let kept = bw_of(&p).count_ink() as f64 / bw_of(&reference).count_ink() as f64;
        assert!((0.95..1.05).contains(&kept), "side {side}: ink kept {kept}");
    }
}

#[test]
fn a_wide_band_beside_text_that_starts_right_after_it_keeps_every_line() {
    let clean = text_page(W, H, 200, 1000);
    let reference = run(&clean, OutputMode::BlackWhite);
    let mut g = clean.clone();
    border_shadow(&mut g, 0, 150, 10, 18);
    let p = run(&g, OutputMode::BlackWhite);
    let kept = bw_of(&p).count_ink() as f64 / bw_of(&reference).count_ink() as f64;
    assert!((0.97..1.04).contains(&kept), "ink kept {kept}");
    assert!(p.image.width() <= reference.image.width() + 12, "band kept");
}
