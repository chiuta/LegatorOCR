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

#[test]
fn lexicon_codes_and_repair() {
    assert_eq!(lexicon::codes("ron+eng+xxx"), vec!["ro", "en"]);
    assert_eq!(lexicon::codes("hu+HU"), vec!["hu"]);
    let en = lexicon::Lexicon::load("en").unwrap();
    assert!(en.contains("The") && en.contains("people"));
    assert!(!en.contains("peopie"));
    let mk = |s: &str, alt_at: usize, alt: char| -> Vec<ctc::Glyph> {
        s.chars()
            .enumerate()
            .map(|(i, ch)| ctc::Glyph {
                ch,
                c0: i,
                c1: i,
                p: 0.5,
                alt: (i == alt_at).then_some((alt, 0.4)),
            })
            .collect()
    };
    let g = mk("peopie", 4, 'l');
    let refs: Vec<&ctc::Glyph> = g.iter().collect();
    assert_eq!(en.repair("peopie", &refs).as_deref(), Some("people"));
    // punctuation around the word is kept; a known word is never touched
    let g = mk("„peopie,", 5, 'l');
    let refs: Vec<&ctc::Glyph> = g.iter().collect();
    assert_eq!(en.repair("„peopie,", &refs).as_deref(), Some("„people,"));
    let g = mk("people", 4, 'x');
    let refs: Vec<&ctc::Glyph> = g.iter().collect();
    assert_eq!(en.repair("people", &refs), None);
}
