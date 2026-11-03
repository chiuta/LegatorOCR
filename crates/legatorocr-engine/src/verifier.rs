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
