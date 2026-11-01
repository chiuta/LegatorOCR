use legatorocr_core::{BitImage, PageImage};
use legatorocr_nocr::{ctc, layout, lexicon, Model, NativeOptions};

/// Build an LNM1 file with deterministic pseudo-random weights (shape-correct, meaningless).
fn tiny_model(hid: usize, charset: &str) -> Vec<u8> {
    let mut seed = 12345u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.2
    };
    let nc = charset.chars().count() + 1;
    let c = [4usize, 4, 4, 4];
    let mut t: Vec<(String, Vec<usize>)> = Vec::new();
    let mut cin = 1;
    for (i, &co) in c.iter().enumerate() {
        t.push((format!("conv{i}.w"), vec![co, cin, 3, 3]));
        t.push((format!("conv{i}.b"), vec![co]));
        cin = co;
    }
    for (name, nin) in [("gru1", c[3] * 2), ("gru2", 2 * hid)] {
        for d in ["f", "r"] {
            t.push((format!("{name}.{d}.weight_ih"), vec![3 * hid, nin]));
            t.push((format!("{name}.{d}.weight_hh"), vec![3 * hid, hid]));
            t.push((format!("{name}.{d}.bias_ih"), vec![3 * hid]));
            t.push((format!("{name}.{d}.bias_hh"), vec![3 * hid]));
        }
    }
    t.push(("fc.w".into(), vec![nc, 2 * hid]));
    t.push(("fc.b".into(), vec![nc]));
    let mut b = b"LNM1".to_vec();
    for v in [32u32, hid as u32, 4, charset.len() as u32] {
        b.extend(v.to_le_bytes());
    }
    b.extend(charset.as_bytes());
    b.extend((t.len() as u32).to_le_bytes());
    for (name, dims) in t {
        b.extend((name.len() as u32).to_le_bytes());
        b.extend(name.as_bytes());
        b.extend((dims.len() as u32).to_le_bytes());
        let mut n = 1;
        for d in &dims {
            b.extend((*d as u32).to_le_bytes());
            n *= d;
        }
        for _ in 0..n {
            b.extend(rnd().to_le_bytes());
        }
    }
    b
}

#[test]
fn model_loads_and_forward_is_a_distribution() {
    let m = Model::from_bytes(&tiny_model(8, " abc")).unwrap();
    assert_eq!(m.nc, 5);
    for w in [4usize, 7, 33, 100] {
        let img: Vec<f32> = (0..32 * w).map(|i| ((i * 7) % 11) as f32 / 10.0).collect();
        let (t, lp) = m.forward(&img, w);
        assert_eq!(t, w.div_ceil(4));
        assert_eq!(lp.len(), t * 5);
        for x in 0..t {
            let s: f32 = lp[x * 5..(x + 1) * 5].iter().map(|v| v.exp()).sum();
            assert!((s - 1.0).abs() < 1e-3, "column {x} sums to {s}");
        }
        assert!(lp.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn corrupt_models_are_rejected_not_panicking() {
    let good = tiny_model(8, " abc");
    assert!(Model::from_bytes(b"").is_err());
    assert!(Model::from_bytes(b"XXXX").is_err());
    for cut in [5usize, 20, 50, good.len() / 2, good.len() - 1] {
        assert!(Model::from_bytes(&good[..cut]).is_err(), "cut at {cut}");
    }
    // every single-byte corruption of the header region must be Err or a valid model, never a panic
    for i in 0..80.min(good.len()) {
        let mut b = good.clone();
        b[i] ^= 0xFF;
        let _ = Model::from_bytes(&b);
    }
}

#[test]
fn ctc_collapses_repeats_and_blanks() {
    let cs: Vec<char> = "ab".chars().collect();
    // classes: 0 blank, 1 'a', 2 'b'; sequence a a blank a b b
    let seq = [1usize, 1, 0, 1, 2, 2];
    let nc = 3;
    let mut lp = vec![-20.0f32; seq.len() * nc];
    for (x, &k) in seq.iter().enumerate() {
        lp[x * nc + k] = -0.01;
    }
    let g = ctc::greedy(&lp, seq.len(), nc, &cs);
    let s: String = g.iter().map(|g| g.ch).collect();
    assert_eq!(s, "aab");
    assert_eq!((g[0].c0, g[0].c1), (0, 1));
}

fn ink_with_lines(rows: &[(u32, u32)]) -> BitImage {
    let mut b = BitImage::new(400, 300);
    for &(y, h) in rows {
        for yy in y..y + h {
            for x in 20..380 {
                if (x / 6) % 3 != 0 {
                    b.set(x, yy, true);
                }
            }
        }
    }
    b
}

#[test]
fn finds_lines_in_order() {
    let ink = ink_with_lines(&[(20, 20), (60, 20), (100, 20)]);
    let lines = layout::find_lines(&ink, 20);
    assert_eq!(lines.len(), 3);
    assert!(lines.windows(2).all(|w| w[0].y < w[1].y));
    assert!(lines.iter().all(|l| l.h == 20 && l.w >= 350));
}

#[test]
fn floating_accent_joins_its_line() {
    // a 3-px mark two rows above a 20-px line must not become a line of its own
    let mut ink = ink_with_lines(&[(60, 20)]);
    for y in 53..56 {
        for x in 100..108 {
            ink.set(x, y, true);
        }
    }
    let lines = layout::find_lines(&ink, 20);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].y <= 53);
}

#[test]
fn two_columns_are_read_column_by_column() {
    let mut b = BitImage::new(500, 300);
    for col in [(20u32, 220u32), (280, 480)] {
        for y in [20u32, 60, 100, 140] {
            for yy in y..y + 20 {
                for x in col.0..col.1 {
                    if (x / 5) % 3 != 0 {
                        b.set(x, yy, true);
                    }
                }
            }
        }
    }
    let lines = layout::find_lines(&b, 20);
    assert_eq!(lines.len(), 8);
    assert!(lines[..4].iter().all(|l| l.x < 100) && lines[4..].iter().all(|l| l.x > 200));
}

#[test]
fn speckle_and_blank_pages_give_no_lines() {
    let mut b = BitImage::new(300, 300);
    assert!(layout::find_lines(&b, 12).is_empty());
    for (x, y) in [(10, 10), (50, 90), (200, 200), (201, 200)] {
        b.set(x, y, true);
    }
    assert!(layout::find_lines(&b, 12).is_empty());
}

#[test]
fn blank_page_recognises_nothing() {
    let m = Model::from_bytes(&tiny_model(8, " abc")).unwrap();
    let page = PageImage::Gray(image::GrayImage::from_pixel(200, 200, image::Luma([250])));
    assert!(legatorocr_nocr::recognize(&page, 300, &m, &NativeOptions::default()).is_empty());
}

#[cfg(feature = "lexicon")]
#[test]
fn lexicon_codes_and_repair() {
    assert_eq!(lexicon::codes("ron+eng+xxx"), vec!["ro", "en"]);
    assert_eq!(lexicon::codes("hu+HU"), vec!["hu"]);
    assert!(
        lexicon::all_supported("ron+eng")
            && !lexicon::all_supported("ron+rus")
            && !lexicon::all_supported("")
    );
    let en = lexicon::Lexicon::load("en").unwrap();
    let ro = lexicon::Lexicon::load("ro").unwrap();
    assert!(en.contains("The") && en.contains("people") && !en.contains("peopie"));
    let mk = |s: &str, alt_at: usize, alt: char, p: f32| -> Vec<ctc::Glyph> {
        s.chars()
            .enumerate()
            .map(|(i, ch)| ctc::Glyph {
                ch,
                c0: i,
                c1: i,
                p: if i == alt_at { p } else { 0.97 },
                alt: (i == alt_at).then_some((alt, 0.4)),
            })
            .collect()
    };
    let fix = |lx: &[&lexicon::Lexicon], t: &str, at: usize, alt: char, p: f32, first: bool| {
        let g = mk(t, at, alt, p);
        let r: Vec<&ctc::Glyph> = g.iter().collect();
        lexicon::repair(lx, t, &r, first)
    };
    assert_eq!(
        fix(&[en], "peopie", 4, 'l', 0.5, false).as_deref(),
        Some("people")
    );
    assert_eq!(
        fix(&[en], "„peopie,", 5, 'l', 0.5, false).as_deref(),
        Some("„people,")
    );
    // a known word, a confident glyph, a capitalised name mid-line, and an upper/lower swap are left alone
    assert_eq!(fix(&[en], "people", 4, 'x', 0.5, false), None);
    assert_eq!(fix(&[en], "peopie", 4, 'l', 0.95, false), None);
    assert_eq!(fix(&[en], "Peopie", 4, 'l', 0.5, false), None);
    assert_eq!(fix(&[en], "peopie", 4, 'L', 0.5, false), None);
    // multi-language: a correct English word is not "repaired" through the Romanian list
    assert!(lexicon::known(&[ro, en], "table"));
    assert_eq!(fix(&[ro, en], "table", 3, 'x', 0.5, false), None);
}

// ---- adversarial layout cases found by the review ----

#[test]
fn dense_page_with_many_lines_loses_none() {
    // 120 lines at uniform spacing: a depth-limited cut used to swallow everything after ~40
    let rows: Vec<(u32, u32)> = (0..120).map(|i| (10 + i * 12, 8)).collect();
    let mut b = BitImage::new(400, 10 + 120 * 12 + 10);
    for &(y, h) in &rows {
        for yy in y..y + h {
            for x in 20..380 {
                if (x / 5) % 3 != 0 {
                    b.set(x, yy, true);
                }
            }
        }
    }
    let lines = layout::find_lines(&b, 8);
    assert_eq!(lines.len(), 120);
    assert!(lines.iter().all(|l| l.h == 8));
}

#[test]
fn ruled_table_rows_stay_separate_lines() {
    // text rows inside a frame with vertical borders: borders must not weld the rows together
    let mut g = image::GrayImage::from_pixel(400, 300, image::Luma([240]));
    for y in 20..280 {
        for x in [30u32, 31, 32, 368, 369, 370] {
            g.put_pixel(x, y, image::Luma([20]));
        }
    }
    for row in 0..5 {
        for yy in 0..14 {
            for x in 50..350 {
                if (x / 4 + row) % 3 != 0 {
                    g.put_pixel(x, 40 + row * 45 + yy, image::Luma([20]));
                }
            }
        }
    }
    let prep = layout::prepare(&PageImage::Gray(g), 150);
    assert!(prep.med > 0);
    let lines = layout::find_lines(&prep.ink, prep.med);
    assert_eq!(lines.len(), 5, "{lines:?}");
}

#[test]
fn light_on_dark_page_is_inverted() {
    let mut g = image::GrayImage::from_pixel(300, 120, image::Luma([15]));
    for row in 0..2 {
        for yy in 0..16 {
            for x in 20..280 {
                if (x / 4) % 3 != 0 {
                    g.put_pixel(x, 20 + row * 50 + yy, image::Luma([235]));
                }
            }
        }
    }
    let prep = layout::prepare(&PageImage::Gray(g), 150);
    assert!(
        prep.gray.get_pixel(0, 0).0[0] > 200,
        "paper must be light after inversion"
    );
    assert_eq!(layout::find_lines(&prep.ink, prep.med).len(), 2);
}

#[test]
fn long_lines_are_split_at_gaps_and_cover_the_line() {
    let mut b = BitImage::new(6000, 60);
    let mut x = 10;
    while x < 5900 {
        for xx in x..x + 40 {
            for y in 10..40 {
                if (xx / 3) % 2 == 0 {
                    b.set(xx, y, true);
                }
            }
        }
        x += 60; // 20-px word gaps
    }
    let line = layout::Line {
        x: 10,
        y: 10,
        w: 5900,
        h: 30,
    };
    let parts = layout::split_long(&b, &line, 600);
    assert!(parts.len() > 1);
    assert_eq!(parts[0].x, 10);
    let end = parts.last().map(|l| l.x + l.w).unwrap();
    assert_eq!(end, 5910);
    assert!(
        parts.windows(2).all(|w| w[0].x + w[0].w == w[1].x),
        "parts must tile the line"
    );
    assert!(parts.iter().all(|l| l.w <= 600 * 30 * 124 / 100 / 32 + 1));
}

#[test]
fn median_height_ignores_dust_and_survives_short_images() {
    // one cropped text line only 40 px high: letters (28 px) must still count
    let mut b = BitImage::new(300, 40);
    for x in (10..290).step_by(14) {
        for xx in x..x + 8 {
            for y in 6..34 {
                b.set(xx, y, true);
            }
        }
    }
    assert_eq!(layout::median_component_height(&b), 28);
    // add dust: many 2-px specks must not drag the median down
    for i in 0..60u32 {
        b.set(3 + (i * 5) % 290, 1 + (i * 7) % 3, true);
    }
    assert_eq!(layout::median_component_height(&b), 28);
}

#[test]
fn huge_blank_page_does_not_allocate_per_pixel_labels() {
    // 9000 × 9000 empty page: the run-based labelling needs next to no memory and returns fast
    let b = BitImage::new(9000, 9000);
    assert_eq!(layout::median_component_height(&b), 0);
}

#[test]
fn tilted_lines_are_levelled_and_boxes_map_back() {
    // seven text-like bands tilted by 2.4° (one pixel row drops every 24 columns)
    let (w, h) = (900u32, 700u32);
    let th = 2.4f32.to_radians();
    let (sn, cs) = th.sin_cos();
    let mut g = image::GrayImage::from_pixel(w, h, image::Luma([245]));
    // rigid rotation of an upright page: bands of 16 px every 70 px, broken into 5-px "letters"
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - 450.0, y as f32 - 350.0);
            let u = dx * cs + dy * sn + 450.0;
            let v = -dx * sn + dy * cs + 350.0;
            if (60.0..840.0).contains(&u) && !(u as u32 / 5).is_multiple_of(3) {
                let r = (v - 90.0).rem_euclid(70.0);
                if (0.0..16.0).contains(&r) && (90.0..580.0).contains(&v) {
                    g.put_pixel(x, y, image::Luma([25]));
                }
            }
        }
    }
    let prep = layout::prepare(&PageImage::Gray(g), 300);
    let deg = prep.skew.to_degrees();
    assert!((deg.abs() - 2.4).abs() < 0.5, "estimated {deg}°");
    assert_eq!(layout::find_lines(&prep.ink, prep.med).len(), 7);
    // a box on the levelled page maps back inside the original page, near the same spot
    let (x, y, bw, bh) = layout::unrotate_box((450, 300, 40, 20), prep.skew, w, h);
    assert!(
        (430..470).contains(&x) && (280..320).contains(&y) && bw >= 40 && bh >= 20,
        "{x},{y},{bw},{bh}"
    );
    // an untilted page stays untouched
    let flat = layout::prepare(
        &PageImage::Gray(image::GrayImage::from_fn(w, h, |x, y| {
            image::Luma([
                if (y / 70) % 2 == 0 && (y % 70) < 16 && (x / 5) % 3 != 0 && x > 60 && x < 840 {
                    25
                } else {
                    245
                },
            ])
        })),
        300,
    );
    assert_eq!(flat.skew, 0.0);
}
