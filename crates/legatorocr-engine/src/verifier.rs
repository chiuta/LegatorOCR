//! Document verifier (roadmap 7.5): checks that can be done on a picture of a document without
//! knowing what is genuine. Each result is a **hint for a person to look at**, never a verdict
//! of forgery or authenticity.
//!
//! - QR codes: decoded with the `rqrr` crate (payload text; a verifier-URL still has to be
//!   checked by the person with the issuer).
//! - Copy-move: blocks of the picture that appear again elsewhere with the same offset (a patch
//!   copied inside one picture). Repeated letters of a text page are common and can give false
//!   alarms; the measured rate is in the roadmap.
//!
//! Error-level analysis (ELA) was tried and removed: on a page spliced into a JPEG it did not
//! point at the splice and it flagged blocks of an untouched original (see ROADMAP 7.5).

use image::GrayImage;
use std::collections::HashMap;

/// Decoded QR payloads.
pub fn qr_codes(g: &GrayImage) -> Vec<String> {
    qr_read(g).into_iter().map(|(t, _)| t).collect()
}

/// Decoded QR payloads with their bounding boxes (x0, y0, x1, y1).
pub fn qr_read(g: &GrayImage) -> Vec<(String, (i32, i32, i32, i32))> {
    let mut p = rqrr::PreparedImage::prepare_from_greyscale(g.width() as usize, g.height() as usize, |x, y| g.get_pixel(x as u32, y as u32)[0]);
    p.detect_grids()
        .into_iter()
        .filter_map(|grid| {
            let (_, text) = grid.decode().ok()?;
            let xs = grid.bounds.iter().map(|b| b.x);
            let ys = grid.bounds.iter().map(|b| b.y);
            Some((text, (xs.clone().min()?, ys.clone().min()?, xs.max()?, ys.max()?)))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyMove {
    /// Offset between the two copies, in pixels.
    pub dx: i32,
    pub dy: i32,
    /// Number of matching blocks that agree on this offset.
    pub blocks: usize,
    /// Bounding box (x0, y0, x1, y1) of the source copy.
    pub bbox: (u32, u32, u32, u32),
}

const BLOCK: u32 = 16;
const STRIDE: u32 = 4;

/// Hash keys of a block: 4×4 cell means quantised in steps of 32 grey levels, once as is and once
/// shifted by half a step, so a cell near a step boundary still agrees in one of the two keys
/// after the grey levels moved by re-compression. Low-contrast blocks carry no evidence.
fn block_keys(g: &GrayImage, x: u32, y: u32) -> Option<[u64; 2]> {
    let mut cells = [0u32; 16];
    let (mut s, mut s2) = (0u64, 0u64);
    for j in 0..BLOCK {
        for i in 0..BLOCK {
            let v = g.get_pixel(x + i, y + j)[0] as u32;
            cells[((j / 4) * 4 + i / 4) as usize] += v;
            s += v as u64;
            s2 += (v * v) as u64;
        }
    }
    let n = (BLOCK * BLOCK) as f64;
    let var = s2 as f64 / n - (s as f64 / n).powi(2);
    if var < 100.0 {
        return None;
    }
    let mut k = [0u64; 2];
    for c in cells {
        let m = c / 16;
        k[0] = k[0].wrapping_mul(31).wrapping_add((m / 32) as u64);
        k[1] = k[1].wrapping_mul(31).wrapping_add(((m + 16) / 32) as u64);
    }
    Some(k)
}

/// Copy-move candidates with at least `min_blocks` agreeing blocks and a distance of at least `min_dist` px.
pub fn copy_move(g: &GrayImage, min_blocks: usize, min_dist: i32) -> Vec<CopyMove> {
    let (w, h) = g.dimensions();
    if w < BLOCK * 2 || h < BLOCK * 2 {
        return Vec::new();
    }
    let mut buckets: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    let mut y = 0;
    while y + BLOCK <= h {
        let mut x = 0;
        while x + BLOCK <= w {
            if let Some(keys) = block_keys(g, x, y) {
                for (variant, k) in keys.into_iter().enumerate() {
                    let v = buckets.entry(k ^ ((variant as u64) << 63)).or_default();
                    if v.len() < 12 {
                        v.push((x, y));
                    }
                }
            }
            x += STRIDE;
        }
        y += STRIDE;
    }
    let mut votes: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
    for v in buckets.values() {
        for (i, a) in v.iter().enumerate() {
            for b in &v[i + 1..] {
                let (mut dx, mut dy) = (b.0 as i32 - a.0 as i32, b.1 as i32 - a.1 as i32);
                if dx * dx + dy * dy < min_dist * min_dist {
                    continue;
                }
                let mut src = *a;
                // canonical direction so both orders vote for the same offset
                if dy < 0 || (dy == 0 && dx < 0) {
                    dx = -dx;
                    dy = -dy;
                    src = *b;
                }
                votes.entry((dx, dy)).or_default().push(src);
            }
        }
    }
    // a periodic pattern (rules, a QR code, columns of text) repeats at multiples of its period
    let counts: HashMap<(i32, i32), usize> = votes.iter().map(|(k, v)| (*k, v.len())).collect();
    let mut out: Vec<CopyMove> = votes
        .into_iter()
        .filter(|(_, v)| v.len() >= min_blocks)
        .filter(|((dx, dy), v)| {
            let twice = counts.get(&(2 * dx, 2 * dy)).copied().unwrap_or(0);
            twice * 3 < v.len()
        })
        .filter(|(_, v)| {
            // a pasted patch fills its bounding box; scattered coincidences do not
            let x0 = v.iter().map(|p| p.0).min().unwrap();
            let y0 = v.iter().map(|p| p.1).min().unwrap();
            let x1 = v.iter().map(|p| p.0).max().unwrap() + BLOCK;
            let y1 = v.iter().map(|p| p.1).max().unwrap() + BLOCK;
            let area = ((x1 - x0) * (y1 - y0)) as f64;
            let covered = (v.len() as f64) * (STRIDE * STRIDE) as f64;
            covered / area > 0.5
        })
        .map(|((dx, dy), v)| {
            let x0 = v.iter().map(|p| p.0).min().unwrap();
            let y0 = v.iter().map(|p| p.1).min().unwrap();
            let x1 = v.iter().map(|p| p.0).max().unwrap() + BLOCK;
            let y1 = v.iter().map(|p| p.1).max().unwrap() + BLOCK;
            CopyMove { dx, dy, blocks: v.len(), bbox: (x0, y0, x1, y1) }
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.blocks));
    out.truncate(5);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(w: u32, h: u32, seed: u64) -> GrayImage {
        let mut s = seed;
        let mut g = GrayImage::new(w, h);
        for p in g.pixels_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            p[0] = (s >> 56) as u8;
        }
        g
    }

    #[test]
    fn copied_patch_is_found_and_plain_noise_is_not() {
        let mut g = texture(300, 200, 1);
        assert!(copy_move(&g, 15, 24).is_empty(), "independent noise has no copy");
        // copy a 64×48 patch from (20,20) to (180,100)
        for j in 0..48 {
            for i in 0..64 {
                let v = *g.get_pixel(20 + i, 20 + j);
                g.put_pixel(180 + i, 100 + j, v);
            }
        }
        let found = copy_move(&g, 15, 24);
        assert!(!found.is_empty());
        assert_eq!((found[0].dx, found[0].dy), (160, 80));
    }

    #[test]
    fn qr_code_is_read_and_noise_is_not() {
        // fixture made with python-qrcode: https://example.org/verifica/ABC-123 on a grey page
        let img = image::load_from_memory(include_bytes!("../tests/data/qr.png")).unwrap().to_luma8();
        assert_eq!(qr_codes(&img), vec!["https://example.org/verifica/ABC-123".to_string()]);
        assert!(qr_codes(&texture(120, 120, 3)).is_empty());
    }
}

/// A round mark (the outline of a circular stamp or seal) found by [`round_marks`].
#[derive(Debug, Clone, PartialEq)]
pub struct RoundMark {
    /// Centre and radius in pixels of the picture given.
    pub cx: u32,
    pub cy: u32,
    pub radius: u32,
    /// Share of the circle that is ink (0..1).
    pub coverage: f32,
}

/// Find outlines of round stamps and seals (radius 6..18 mm) by testing circles against the
/// ink of the picture, working on a reduced copy (about 75 dpi). A circle counts when at least
/// `min_coverage` of its outline is ink and no gap is longer than a quarter of it. Shape,
/// not colour: a coloured page does not matter. A hint only: a large letter "O", a round logo
/// or a coin photo can match.
pub fn round_marks(g: &GrayImage, dpi: u32, min_coverage: f32) -> Vec<RoundMark> {
    let f = (dpi.max(75) / 75).max(1);
    let (w, h) = (g.width() / f, g.height() / f);
    if w < 64 || h < 64 {
        return Vec::new();
    }
    // background level: the 95th percentile of the grey values; ink is clearly darker
    let mut hist = [0u32; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = g.pixels().len() as u32;
    let (mut acc, mut bg) = (0u32, 255u8);
    for (v, c) in hist.iter().enumerate() {
        acc += c;
        if acc >= total / 100 * 95 {
            bg = v as u8;
            break;
        }
    }
    let thr = (bg as f32 * 0.78) as u8;
    let mut ink = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            'blk: for dy in 0..f {
                for dx in 0..f {
                    if g.get_pixel(x * f + dx, y * f + dy)[0] < thr {
                        ink[(y * w + x) as usize] = true;
                        break 'blk;
                    }
                }
            }
        }
    }
    let at = |x: i32, y: i32| x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && ink[(y as u32 * w + x as u32) as usize];
    // 75 dpi: 1 mm = 2.95 px
    let (r_min, r_max) = (18i32, 54i32);
    let mut found: Vec<RoundMark> = Vec::new();
    let mut r = r_min;
    while r <= r_max {
        let n = ((2.0 * std::f32::consts::PI * r as f32) as usize).clamp(48, 360);
        let dirs: Vec<(f32, f32)> = (0..n).map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            (a.cos(), a.sin())
        }).collect();
        let mut cy = r;
        while cy < h as i32 - r {
            let mut cx = r;
            while cx < w as i32 - r {
                // quick reject: the four compass points must be near ink
                let near = |x: i32, y: i32| (-1..=1).any(|d| at(x + d, y) || at(x, y + d));
                if near(cx + r, cy) && near(cx - r, cy) && near(cx, cy + r) && near(cx, cy - r) {
                    let mut hit = vec![false; n];
                    let mut cnt = 0usize;
                    for (k, (c, s)) in dirs.iter().enumerate() {
                        let (x, y) = (cx as f32 + c * r as f32, cy as f32 + s * r as f32);
                        let (xi, yi) = (x.round() as i32, y.round() as i32);
                        // ink on the circle itself or one pixel to either side along the radius
                        let on = at(xi, yi) || at((x + c).round() as i32, (y + s).round() as i32) || at((x - c).round() as i32, (y - s).round() as i32);
                        hit[k] = on;
                        cnt += on as usize;
                    }
                    let cov = cnt as f32 / n as f32;
                    // a ring has little ink just inside and just outside it; a filled blob, a
                    // dark band or dense text has ink on both sides
                    let side = |off: f32| {
                        let k = dirs.iter().filter(|(c, s)| {
                            let (x, y) = (cx as f32 + c * (r as f32 + off), cy as f32 + s * (r as f32 + off));
                            at(x.round() as i32, y.round() as i32)
                        });
                        k.count() as f32 / n as f32
                    };
                    let (inside, outside) = if cov >= min_coverage { (side(-3.0), side(3.0)) } else { (1.0, 1.0) };
                    if cov >= min_coverage && inside.min(outside) <= 0.3 && inside.max(outside) <= 0.5 {
                        // longest gap (circular)
                        let (mut best, mut run) = (0usize, 0usize);
                        for k in 0..2 * n {
                            if hit[k % n] {
                                run = 0;
                            } else {
                                run += 1;
                                best = best.max(run.min(n));
                            }
                        }
                        if best * 4 <= n {
                            found.push(RoundMark { cx: cx as u32 * f, cy: cy as u32 * f, radius: r as u32 * f, coverage: cov });
                        }
                    }
                }
                cx += 2;
            }
            cy += 2;
        }
        r += 2;
    }
    // keep the best circle of each cluster
    found.sort_by(|a, b| b.coverage.total_cmp(&a.coverage));
    let mut kept: Vec<RoundMark> = Vec::new();
    for m in found {
        let d = |k: &RoundMark| (((k.cx as f32 - m.cx as f32).powi(2) + (k.cy as f32 - m.cy as f32).powi(2)).sqrt(), (k.radius as f32 - m.radius as f32).abs());
        if !kept.iter().any(|k| {
            let (dc, dr) = d(k);
            dc < (k.radius.max(m.radius) as f32) * 0.6 && dr < k.radius.max(m.radius) as f32 * 0.5
        }) {
            kept.push(m);
        }
    }
    kept
}

#[cfg(test)]
mod round_tests {
    use super::*;

    fn text_page() -> GrayImage {
        // lines of dark "words" on white, 300 dpi A5-ish
        let mut s = 99u32;
        let mut g = GrayImage::from_pixel(1240, 1750, image::Luma([250]));
        for line in 0..18 {
            let y = 120 + line * 84;
            let mut x = 100;
            while x < 1100 {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                let wlen = 30 + (s >> 24) % 120;
                for xx in x..(x + wlen).min(1140) {
                    for yy in y..y + 24 {
                        if ((xx / 7 + yy / 5) % 3) != 0 {
                            g.put_pixel(xx, yy, image::Luma([40]));
                        }
                    }
                }
                x += wlen + 22;
            }
        }
        g
    }

    fn ring(g: &mut GrayImage, cx: f32, cy: f32, r: f32, t: f32, ink: u8) {
        for y in 0..g.height() {
            for x in 0..g.width() {
                let d = (((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() - r).abs();
                if d < t {
                    let p = g.get_pixel_mut(x, y);
                    p[0] = p[0].min(ink);
                }
            }
        }
    }

    #[test]
    fn a_ring_stamp_over_text_is_found_and_plain_text_gives_none() {
        let plain = text_page();
        let none = round_marks(&plain, 300, 0.85);
        assert!(none.is_empty(), "false alarms on plain text: {none:?}");
        let mut st = plain.clone();
        ring(&mut st, 700.0, 900.0, 170.0, 4.0, 110); // faded ink: lighter than text
        ring(&mut st, 700.0, 900.0, 140.0, 2.5, 110);
        let marks = round_marks(&st, 300, 0.85);
        assert!(marks.iter().any(|m| m.cx.abs_diff(700) < 20 && m.cy.abs_diff(900) < 20 && m.radius.abs_diff(170) < 20), "{marks:?}");
    }
}

/// Double-quantisation test on the DCT coefficients of a JPEG's luma (Popescu & Farid style).
/// A picture saved at quality A and later again at quality B leaves a comb pattern in the
/// histogram of its low-frequency coefficients; one saved once has a smooth histogram.
#[derive(Debug, Clone)]
pub struct DoubleQuant {
    /// Mean of `spikiness` over the five coefficients.
    pub score: f64,
    /// True when `score` is above [`DOUBLE_QUANT_THRESHOLD`].
    pub likely_double: bool,
    /// "Spikiness" of the histogram for the coefficients (0,1) (1,0) (1,1) (0,2) (2,0):
    /// mean absolute second difference over the mean count; higher = more comb-like.
    pub spikiness: [f64; 5],
    /// Quantisation step of those coefficients in the file's own table.
    pub steps: [u16; 5],
}

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Luma quantisation table (natural order) of a JPEG file.
fn jpeg_luma_table(b: &[u8]) -> Option<[u16; 64]> {
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let m = b[i + 1];
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            i += 2;
            continue;
        }
        let len = ((b[i + 2] as usize) << 8) | b[i + 3] as usize;
        if m == 0xDB {
            let mut p = i + 4;
            let end = (i + 2 + len).min(b.len());
            while p < end {
                let (prec, id) = (b[p] >> 4, b[p] & 15);
                p += 1;
                let sz = if prec == 0 { 64 } else { 128 };
                if p + sz > end {
                    return None;
                }
                if id == 0 {
                    let mut t = [0u16; 64];
                    for k in 0..64 {
                        t[ZIGZAG[k]] = if prec == 0 { b[p + k] as u16 } else { ((b[p + 2 * k] as u16) << 8) | b[p + 2 * k + 1] as u16 };
                    }
                    return Some(t);
                }
                p += sz;
            }
        }
        if m == 0xDA {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Chosen on 42 files as the lowest score with no false alarm on single saves, then checked
/// on 78 other files (ROADMAP 7.5b).
pub const DOUBLE_QUANT_THRESHOLD: f64 = 5.0;

pub fn jpeg_double_quant(bytes: &[u8]) -> Option<DoubleQuant> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let q = jpeg_luma_table(bytes)?;
    let g = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).ok()?.to_luma8();
    let (bw, bh) = (g.width() / 8, g.height() / 8);
    if bw < 8 || bh < 8 {
        return None;
    }
    let modes: [(usize, usize); 5] = [(0, 1), (1, 0), (1, 1), (0, 2), (2, 0)]; // (u = column freq, v = row freq)
    let mut cos = [[0f32; 8]; 8];
    for (k, row) in cos.iter_mut().enumerate() {
        for (n, c) in row.iter_mut().enumerate() {
            *c = ((2 * n + 1) as f32 * k as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    let scale = |k: usize| if k == 0 { (1.0f32 / 8.0).sqrt() } else { 0.5 };
    const K: i32 = 16;
    let mut hist = vec![[0u32; (2 * K + 1) as usize]; 5];
    let raw = g.as_raw();
    let w = g.width() as usize;
    for by in 0..bh as usize {
        for bx in 0..bw as usize {
            let mut blk = [0f32; 64];
            for y in 0..8 {
                for x in 0..8 {
                    blk[y * 8 + x] = raw[(by * 8 + y) * w + bx * 8 + x] as f32 - 128.0;
                }
            }
            for (mi, &(u, v)) in modes.iter().enumerate() {
                let mut s = 0f32;
                for y in 0..8 {
                    for x in 0..8 {
                        s += blk[y * 8 + x] * cos[u][x] * cos[v][y];
                    }
                }
                let c = s * scale(u) * scale(v) / q[v * 8 + u] as f32;
                let k = c.round() as i32;
                if (-K..=K).contains(&k) {
                    hist[mi][(k + K) as usize] += 1;
                }
            }
        }
    }
    let mut spikiness = [0f64; 5];
    for (mi, h) in hist.iter().enumerate() {
        // Take log counts on bins 2..=14 of each side (bins with at least 30 blocks), remove a
        // smooth quadratic trend, and look for a periodic ripple: the largest spectral line over
        // the mean spectral power. A comb from double quantisation is periodic; the curvature of
        // an ordinary histogram is not.
        let mut score = 0f64;
        for side in [1i32, -1] {
            let ks: Vec<i32> = (2..=14).filter(|k| h[(side * k + K) as usize] >= 30).collect();
            if ks.len() < 8 {
                continue;
            }
            let ys: Vec<f64> = ks.iter().map(|k| (h[(side * k + K) as usize] as f64).ln()).collect();
            // least-squares quadratic
            let n = ks.len();
            let (mut sx, mut sx2, mut sx3, mut sx4, mut sy, mut sxy, mut sx2y) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
            for (k, y) in ks.iter().zip(&ys) {
                let x = *k as f64 / 14.0;
                sx += x;
                sx2 += x * x;
                sx3 += x * x * x;
                sx4 += x * x * x * x;
                sy += y;
                sxy += x * y;
                sx2y += x * x * y;
            }
            // normal equations for y = c0 + c1 x + c2 x^2
            let m = [[n as f64, sx, sx2], [sx, sx2, sx3], [sx2, sx3, sx4]];
            let rhs = [sy, sxy, sx2y];
            let det = |m: &[[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            let d = det(&m);
            if d.abs() < 1e-12 {
                continue;
            }
            let mut c = [0f64; 3];
            for j in 0..3 {
                let mut mj = m;
                for i in 0..3 {
                    mj[i][j] = rhs[i];
                }
                c[j] = det(&mj) / d;
            }
            let res: Vec<f64> = ks.iter().zip(&ys).map(|(k, y)| {
                let x = *k as f64 / 14.0;
                y - (c[0] + c[1] * x + c[2] * x * x)
            }).collect();
            let mut powers = Vec::new();
            for step in 1..=40 {
                let f = step as f64 / 40.0 * 0.5; // cycles per bin, up to 0.5
                let (mut re, mut im) = (0f64, 0f64);
                for (k, r) in ks.iter().zip(&res) {
                    let ph = std::f64::consts::TAU * f * *k as f64;
                    re += r * ph.cos();
                    im += r * ph.sin();
                }
                powers.push(re * re + im * im);
            }
            let mean = powers.iter().sum::<f64>() / powers.len() as f64;
            let peak = powers.iter().cloned().fold(0.0, f64::max);
            // amplitude of the ripple matters as much as its sharpness
            let rms = (res.iter().map(|r| r * r).sum::<f64>() / res.len() as f64).sqrt();
            if mean > 0.0 {
                score = score.max(peak / mean * rms);
            }
        }
        spikiness[mi] = score;
    }
    let score = spikiness.iter().sum::<f64>() / 5.0;
    Some(DoubleQuant { score, likely_double: score > DOUBLE_QUANT_THRESHOLD, spikiness, steps: modes.map(|(u, v)| q[v * 8 + u]) })
}

#[cfg(test)]
mod double_quant_tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;

    fn photo_like() -> image::GrayImage {
        // blocks of different brightness, waves and noise, so that low-frequency coefficients
        // spread over many quantisation bins as in a photographed page
        let mut s = 7u32;
        image::GrayImage::from_fn(1024, 1024, |x, y| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = ((s >> 24) as f32 - 128.0) * 0.25;
            let base = if (x / 13 + y / 11) % 2 == 0 { 70.0 } else { 175.0 };
            let v = base + 30.0 * ((x as f32 * 0.21).sin() + (y as f32 * 0.13).cos()) + n;
            image::Luma([v.clamp(0.0, 255.0) as u8])
        })
    }
    fn save(g: &image::GrayImage, q: u8) -> Vec<u8> {
        let mut b = Vec::new();
        JpegEncoder::new_with_quality(&mut b, q).encode(g.as_raw(), g.width(), g.height(), image::ExtendedColorType::L8).unwrap();
        b
    }

    #[test]
    fn a_double_saved_jpeg_scores_higher_than_a_single_save() {
        let g = photo_like();
        let single = save(&g, 90);
        let first = image::load_from_memory(&save(&g, 55)).unwrap().to_luma8();
        let double = save(&first, 90);
        let (s, d) = (jpeg_double_quant(&single).unwrap(), jpeg_double_quant(&double).unwrap());
        eprintln!("single {:.2} double {:.2}", s.score, d.score);
        assert!(d.score > s.score * 2.0, "single {} double {}", s.score, d.score);
        assert!(jpeg_double_quant(b"not a jpeg").is_none());
    }
}
