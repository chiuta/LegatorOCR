//! Page-curl correction. Near a book's spine the text lines bend. We follow every
//! text line across the page in narrow vertical strips (peaks of the row profile,
//! chained from strip to strip), measure how far each line sits from its own mean
//! height, and resample the page vertically so that every line becomes straight.
//! Between lines the correction is interpolated linearly; above the first and below
//! the last it is held constant. Nothing is done unless several long lines show a
//! clear, plausible bend.

use crate::{binarize, transform};
use image::{imageops, Rgb, RgbImage};
use rayon::prelude::*;

struct Chain {
    /// (strip index, y) in analysis pixels.
    pts: Vec<(usize, f32)>,
}

/// Peaks of a smoothed row profile: (y, strength), sub-pixel via parabola.
fn peaks(profile: &[f32], min_gap: usize, floor: f32) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::new();
    for y in 1..profile.len().saturating_sub(1) {
        let v = profile[y];
        if v >= floor && v > profile[y - 1] && v >= profile[y + 1] {
            let (a, b, c) = (profile[y - 1], v, profile[y + 1]);
            let d = a - 2.0 * b + c;
            let off = if d.abs() > 1e-6 {
                0.5 * (a - c) / d
            } else {
                0.0
            };
            let yy = y as f32 + off.clamp(-0.5, 0.5);
            match out.last_mut() {
                Some(last) if yy - last.0 < min_gap as f32 => {
                    if v > last.1 {
                        *last = (yy, v);
                    }
                }
                _ => out.push((yy, v)),
            }
        }
    }
    out
}

/// Straighten text lines that curve (book spine, bent sheet). Returns `None` when
/// the page has no measurable, plausible curvature.
pub fn dewarp(img: &RgbImage, dpi: u32, fill: Rgb<u8>) -> Option<RgbImage> {
    let f = (dpi / 150).max(1);
    let small = transform::downscale_gray(&imageops::grayscale(img), f);
    let eff = (dpi / f).max(36) as usize;
    let (w, h) = (small.width() as usize, small.height() as usize);
    let ink = binarize::sauvola(&small, (eff / 8).max(15) as u32, 0.3);

    let sw = (eff / 4).max(12);
    let ns = w / sw;
    if ns < 6 || h < 8 * eff / 10 {
        return None;
    }
    let smooth_r = (eff / 40).max(1);
    let min_gap = (eff / 10).max(4);

    // --- row profiles per strip, peaks, chaining -------------------------------
    let mut chains: Vec<Chain> = Vec::new();
    let mut active: Vec<usize> = Vec::new(); // chains that reached the previous strip
    for s in 0..ns {
        let mut prof = vec![0f32; h];
        for (y, p) in prof.iter_mut().enumerate() {
            let row = &ink.data[y * w + s * sw..y * w + (s + 1) * sw];
            *p = row.iter().filter(|&&v| v != 0).count() as f32;
        }
        let mut sm = vec![0f32; h];
        for y in 0..h {
            let (a, b) = (y.saturating_sub(smooth_r), (y + smooth_r + 1).min(h));
            sm[y] = prof[a..b].iter().sum::<f32>() / (b - a) as f32;
        }
        let mx = sm.iter().cloned().fold(0.0, f32::max);
        if mx < 2.0 {
            active.clear();
            continue;
        }
        let pk = peaks(&sm, min_gap, 0.35 * mx);
        let tol = (eff as f32 / 8.0).max(4.0);
        let mut used = vec![false; pk.len()];
        let mut next_active = Vec::new();
        for &ci in &active {
            let last = chains[ci].pts.last().unwrap().1;
            let best = pk
                .iter()
                .enumerate()
                .filter(|(i, _)| !used[*i])
                .map(|(i, p)| (i, (p.0 - last).abs()))
                .filter(|(_, d)| *d <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, _)) = best {
                used[i] = true;
                chains[ci].pts.push((s, pk[i].0));
                next_active.push(ci);
            }
        }
        for (i, p) in pk.iter().enumerate() {
            if !used[i] {
                chains.push(Chain {
                    pts: vec![(s, p.0)],
                });
                next_active.push(chains.len() - 1);
            }
        }
        active = next_active;
    }
    // Keep only long chains.
    let need = (ns / 2).max(5);
    let chains: Vec<Chain> = chains.into_iter().filter(|c| c.pts.len() >= need).collect();
    if chains.len() < 5 {
        return None;
    }

    // --- per-chain offset curves over all strips ------------------------------------
    // target = mean y of the chain; offset = actual − target (analysis px)
    let mut lines: Vec<(f32, Vec<f32>)> = Vec::new(); // (target, offset per strip)
    for c in &chains {
        let mean = c.pts.iter().map(|p| p.1).sum::<f32>() / c.pts.len() as f32;
        let mut off = vec![f32::NAN; ns];
        for &(s, y) in &c.pts {
            off[s] = y - mean;
        }
        // fill gaps: hold at the ends, interpolate inside
        let first = off.iter().position(|v| !v.is_nan()).unwrap();
        let last = off.iter().rposition(|v| !v.is_nan()).unwrap();
        for s in 0..first {
            off[s] = off[first];
        }
        for s in last + 1..ns {
            off[s] = off[last];
        }
        let mut s = first;
        while s < last {
            if off[s + 1].is_nan() {
                let e = (s + 1..=last).find(|&k| !off[k].is_nan()).unwrap();
                for k in s + 1..e {
                    let t = (k - s) as f32 / (e - s) as f32;
                    off[k] = off[s] * (1.0 - t) + off[e] * t;
                }
                s = e;
            } else {
                s += 1;
            }
        }
        // light smoothing along x (3-tap, twice) to suppress peak jitter
        for _ in 0..2 {
            let src = off.clone();
            for k in 0..ns {
                let (a, b) = (src[k.saturating_sub(1)], src[(k + 1).min(ns - 1)]);
                off[k] = 0.25 * a + 0.5 * src[k] + 0.25 * b;
            }
        }
        lines.push((mean, off));
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    // drop lines that nearly coincide (would make the interpolation singular)
    lines.dedup_by(|b, a| b.0 - a.0 < min_gap as f32);

    // Plausibility: a real bend is smooth and moderate.
    let max_off = lines
        .iter()
        .flat_map(|l| l.1.iter())
        .fold(0f32, |m, v| m.max(v.abs()));
    let range = |l: &(f32, Vec<f32>)| {
        l.1.iter().cloned().fold(f32::MIN, f32::max) - l.1.iter().cloned().fold(f32::MAX, f32::min)
    };
    let bent = lines.iter().filter(|l| range(l) >= 1.5).count();
    if max_off < 1.5 || max_off > 0.05 * h as f32 || bent * 3 < lines.len() || lines.len() < 5 {
        return None;
    }

    // --- full-resolution offset table per line ---------------------------------------
    let fw = img.width() as usize;
    let scale = f as f32;
    let table: Vec<(f32, Vec<f32>)> = lines
        .iter()
        .map(|(t, off)| {
            let row: Vec<f32> = (0..fw)
                .map(|x| {
                    let xs = (x as f32 / scale / sw as f32 - 0.5).clamp(0.0, (ns - 1) as f32);
                    let (i0, fr) = (xs.floor() as usize, xs.fract());
                    let i1 = (i0 + 1).min(ns - 1);
                    (off[i0] * (1.0 - fr) + off[i1] * fr) * scale
                })
                .collect();
            (t * scale, row)
        })
        .collect();

    Some(resample(img, &table, fill))
}

/// Resample the page so that each guide line (target row in pixels, offset of its actual
/// position at every column) becomes straight; between guides the correction is interpolated.
fn resample(img: &RgbImage, table: &[(f32, Vec<f32>)], fill: Rgb<u8>) -> RgbImage {
    let fw = img.width() as usize;
    let fh = img.height() as usize;
    let mut out = RgbImage::from_pixel(fw as u32, fh as u32, fill);
    let src = img.as_raw();
    out.as_mut()
        .par_chunks_mut(fw * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let yf = y as f32;
            // which pair of lines surrounds this row?
            let k = table.partition_point(|l| l.0 <= yf);
            let (a, b, t) = if k == 0 {
                (0, 0, 0.0)
            } else if k >= table.len() {
                (table.len() - 1, table.len() - 1, 0.0)
            } else {
                (
                    k - 1,
                    k,
                    (yf - table[k - 1].0) / (table[k].0 - table[k - 1].0),
                )
            };
            for x in 0..fw {
                let off = table[a].1[x] * (1.0 - t) + table[b].1[x] * t;
                let sy = yf + off;
                if sy < 0.0 || sy > (fh - 1) as f32 {
                    continue; // stays `fill`
                }
                let (y0, fr) = (sy.floor() as usize, sy.fract());
                let y1 = (y0 + 1).min(fh - 1);
                for c in 0..3 {
                    let p0 = src[(y0 * fw + x) * 3 + c] as f32;
                    let p1 = src[(y1 * fw + x) * 3 + c] as f32;
                    row[x * 3 + c] = (p0 + (p1 - p0) * fr + 0.5) as u8;
                }
            }
        });
    out
}

/// Straighten the page along lines the user marked: each guide is a polyline of points
/// `(x, y)` in fractions of the image width and height that should become a horizontal text
/// line. Needs at least one guide with two points; returns `None` for unusable input
/// (non-finite or out-of-range points, a guide that is already straight).
pub fn dewarp_guides(
    img: &RgbImage,
    guides: &[Vec<(f32, f32)>],
    fill: Rgb<u8>,
) -> Option<RgbImage> {
    let (fw, fh) = (img.width() as usize, img.height() as usize);
    if fw < 8 || fh < 8 {
        return None;
    }
    // (mean height, per-column offset, first x, last x)
    let mut table: Vec<(f32, Vec<f32>, usize, usize)> = Vec::new();
    let mut bent = false;
    for g in guides.iter().take(64) {
        let mut pts: Vec<(f32, f32)> = g
            .iter()
            .filter(|p| p.0.is_finite() && p.1.is_finite())
            .map(|p| {
                (
                    p.0.clamp(0.0, 1.0) * (fw - 1) as f32,
                    p.1.clamp(0.0, 1.0) * (fh - 1) as f32,
                )
            })
            .collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|b, a| (b.0 - a.0).abs() < 1.0);
        if pts.len() < 2 {
            continue;
        }
        // a guide has to run mostly across the page; a (nearly) vertical stroke is a slip
        let (x0, x1) = (pts[0].0, pts[pts.len() - 1].0);
        if x1 - x0 < 0.3 * fw as f32 {
            continue;
        }
        let mean = pts.iter().map(|p| p.1).sum::<f32>() / pts.len() as f32;
        let row: Vec<f32> = (0..fw)
            .map(|x| {
                let x = x as f32;
                let k = pts.partition_point(|p| p.0 <= x);
                let y = if k == 0 {
                    pts[0].1
                } else if k >= pts.len() {
                    pts[pts.len() - 1].1
                } else {
                    let (p, q) = (pts[k - 1], pts[k]);
                    p.1 + (q.1 - p.1) * (x - p.0) / (q.0 - p.0)
                };
                y - mean
            })
            .collect();
        let range = row.iter().cloned().fold(f32::MIN, f32::max)
            - row.iter().cloned().fold(f32::MAX, f32::min);
        // like the automatic estimate: a bend of more than 5 % of the height is not a text line
        let worst = row.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if worst > 0.05 * fh as f32 {
            continue;
        }
        bent |= range >= 1.0;
        table.push((
            mean,
            row,
            x0.max(0.0) as usize,
            (x1.max(0.0) as usize).min(fw - 1),
        ));
    }
    if !bent {
        return None;
    }
    table.sort_by(|a, b| a.0.total_cmp(&b.0));
    // guides on (almost) the same row would make the interpolation singular: two halves of
    // one line are joined (each column takes the guide that covers it), not one dropped
    let mut merged: Vec<(f32, Vec<f32>, usize, usize)> = Vec::new();
    for g in table {
        match merged.last_mut() {
            Some(m) if (g.0 - m.0).abs() < 4.0 => {
                for x in 0..fw {
                    let in_m = x >= m.2 && x <= m.3;
                    let in_g = x >= g.2 && x <= g.3;
                    if !in_m && in_g {
                        m.1[x] = g.1[x];
                    }
                }
                m.0 = (m.0 + g.0) * 0.5;
                m.2 = m.2.min(g.2);
                m.3 = m.3.max(g.3);
            }
            _ => merged.push(g),
        }
    }
    let table: Vec<(f32, Vec<f32>)> = merged.into_iter().map(|m| (m.0, m.1)).collect();
    Some(resample(img, &table, fill))
}

/// `x,y x,y …;x,y x,y …` (fractions 0–1) → guides. `None` on any malformed, non-finite or
/// out-of-range point, a guide with fewer than two points, or more than 64 guides.
pub fn parse_guides(s: &str) -> Option<Vec<Vec<(f32, f32)>>> {
    let mut out = Vec::new();
    for g in s.split(';').map(str::trim).filter(|g| !g.is_empty()) {
        let mut pts = Vec::new();
        for p in g.split_whitespace() {
            let (x, y) = p.split_once(',')?;
            let (x, y): (f32, f32) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return None;
            }
            pts.push((x, y));
        }
        if pts.len() < 2 || pts.len() > 256 || out.len() >= 64 {
            return None;
        }
        out.push(pts);
    }
    (out.len() <= 64).then_some(out)
}

pub fn guides_to_string(g: &[Vec<(f32, f32)>]) -> String {
    let ok = |p: &(f32, f32)| (0.0..=1.0).contains(&p.0) && (0.0..=1.0).contains(&p.1);
    g.iter()
        .take(64)
        .filter(|l| (2..=256).contains(&l.len()) && l.iter().all(ok))
        .map(|l| {
            l.iter()
                .map(|p| format!("{},{}", p.0, p.1))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Robust quadratic `a + b·t + c·t²` through `(t, e)` points (t in 0–1): least squares, then
/// three rounds of Tukey reweighting around the current fit. Returns the coefficients and the
/// share of points within `tol` of the final curve.
fn robust_quadratic(pts: &[(f32, f32)], tol: f32) -> Option<([f64; 3], f32)> {
    if pts.len() < 6 {
        return None;
    }
    let mut wt = vec![1.0f64; pts.len()];
    let mut coef = [0.0f64; 3];
    for round in 0..4 {
        // normal equations 3x3
        let (mut m, mut v) = ([[0.0f64; 3]; 3], [0.0f64; 3]);
        for (&(t, e), &w) in pts.iter().zip(&wt) {
            let b = [1.0, t as f64, (t * t) as f64];
            for r in 0..3 {
                for c in 0..3 {
                    m[r][c] += w * b[r] * b[c];
                }
                v[r] += w * b[r] * e as f64;
            }
        }
        // solve by Gauss-Jordan with partial pivoting
        for c in 0..3 {
            let p = (c..3).max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs()))?;
            if m[p][c].abs() < 1e-9 {
                return None;
            }
            m.swap(c, p);
            v.swap(c, p);
            for r in 0..3 {
                if r != c {
                    let f = m[r][c] / m[c][c];
                    for k in c..3 {
                        m[r][k] -= f * m[c][k];
                    }
                    v[r] -= f * v[c];
                }
            }
        }
        coef = [v[0] / m[0][0], v[1] / m[1][1], v[2] / m[2][2]];
        if round < 3 {
            let res: Vec<f64> = pts
                .iter()
                .map(|&(t, e)| {
                    (e as f64 - (coef[0] + coef[1] * t as f64 + coef[2] * (t * t) as f64)).abs()
                })
                .collect();
            let mut sorted = res.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let cut = (6.0 * sorted[sorted.len() / 2]).max(tol as f64 * 2.0);
            for (w, r) in wt.iter_mut().zip(&res) {
                let u = r / cut;
                *w = if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 };
            }
        }
    }
    let inl = pts
        .iter()
        .filter(|&&(t, e)| {
            (e as f64 - (coef[0] + coef[1] * t as f64 + coef[2] * (t * t) as f64)).abs()
                <= tol as f64
        })
        .count();
    Some((coef, inl as f32 / pts.len() as f32))
}

/// Marginal dewarping: straighten a justified text block whose *left margin* is bent (the other
/// direction of [`dewarp`], which straightens bent text lines). Every text line's left edge is
/// measured, a robust parabola is fitted through them, and each pixel row is moved sideways so the
/// parabola becomes a vertical line. Applied only when at least 8 lines agree on a smooth curve
/// that moves the margin between 0.8 % and 8 % of the page width; ragged or indented text, and a
/// margin that is already straight, leave the page alone (`None`).
pub fn straighten_margins(img: &RgbImage, dpi: u32, fill: Rgb<u8>) -> Option<RgbImage> {
    let f = (dpi / 150).max(1);
    let small = transform::downscale_gray(&imageops::grayscale(img), f);
    let eff = (dpi / f).max(36) as usize;
    let (w, h) = (small.width() as usize, small.height() as usize);
    if w < 100 || h < 100 {
        return None;
    }
    let ink = binarize::sauvola(&small, (eff / 8).max(15) as u32, 0.3);
    // rows with enough ink form text lines
    let rowink: Vec<usize> = (0..h)
        .map(|y| (0..w).filter(|&x| ink.get(x as u32, y as u32)).count())
        .collect();
    let floor = (w / 80).max(3);
    let mut pts: Vec<(f32, f32)> = Vec::new(); // (y / h, left edge in small px)
    let mut y = 0;
    while y < h {
        if rowink[y] < floor {
            y += 1;
            continue;
        }
        let y0 = y;
        while y < h && rowink[y] >= floor {
            y += 1;
        }
        if y - y0 < (eff / 40).max(2) || y - y0 > eff / 3 {
            continue; // a speck or a picture, not a text line
        }
        let mut lefts: Vec<usize> = (y0..y)
            .filter_map(|yy| (0..w).find(|&x| ink.get(x as u32, yy as u32)))
            .collect();
        lefts.sort_unstable();
        pts.push((
            (y0 + y) as f32 / 2.0 / h as f32,
            lefts[lefts.len() / 2] as f32,
        ));
    }
    if pts.len() < 8 {
        return None;
    }
    let tol = (w as f32 * 0.004).max(1.5);
    let (c, inliers) = robust_quadratic(&pts, tol)?;
    if inliers < 0.6 {
        return None;
    }
    let at = |t: f64| c[0] + c[1] * t + c[2] * t * t;
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for k in 0..=20 {
        let v = at(k as f64 / 20.0);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let swing = (hi - lo) / w as f64;
    if !(0.008..=0.08).contains(&swing) {
        return None;
    }
    // is the curve really a curve of the margin, or only the first lines of a short block?
    // the fitted line must also describe most of the page height
    let (tmin, tmax) = pts
        .iter()
        .fold((1.0f32, 0.0f32), |a, p| (a.0.min(p.0), a.1.max(p.0)));
    if tmax - tmin < 0.5 {
        return None;
    }
    let (iw, ih) = (img.width() as usize, img.height() as usize);
    let reference = at(0.5);
    let mut out = RgbImage::from_pixel(iw as u32, ih as u32, fill);
    for yy in 0..ih {
        let t = (yy as f64 / ih as f64).clamp(tmin as f64, tmax as f64);
        let shift = ((reference - at(t)) * f as f64).round() as i64; // + = move right
        for xx in 0..iw {
            let sx = xx as i64 - shift;
            if (0..iw as i64).contains(&sx) {
                out.put_pixel(xx as u32, yy as u32, *img.get_pixel(sx as u32, yy as u32));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines of text-like bars whose height follows `bend(x)`.
    fn page(w: u32, h: u32, bend: impl Fn(f32) -> f32) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([238, 238, 238]));
        let mut y0 = 120.0;
        while y0 < h as f32 - 120.0 {
            for x in (60..w - 60).step_by(21) {
                let dy = bend(x as f32 / w as f32);
                for yy in 0..13 {
                    for xx in 0..14 {
                        let y = (y0 + dy) as u32 + yy;
                        if y < h {
                            img.put_pixel(x + xx, y, Rgb([25, 25, 25]));
                        }
                    }
                }
            }
            y0 += 46.0;
        }
        img
    }

    /// Sharpness of the row profile: higher when lines are straight.
    fn sharp(img: &RgbImage) -> f64 {
        let g = imageops::grayscale(img);
        let rows: Vec<f64> = (0..g.height())
            .map(|y| {
                (0..g.width())
                    .filter(|&x| g.get_pixel(x, y)[0] < 100)
                    .count() as f64
            })
            .collect();
        rows.windows(2).map(|p| (p[1] - p[0]).powi(2)).sum()
    }

    #[test]
    fn straight_page_is_left_alone() {
        assert!(dewarp(&page(900, 1200, |_| 0.0), 150, Rgb([238; 3])).is_none());
    }

    #[test]
    fn curved_lines_become_straight() {
        let img = page(900, 1200, |u| {
            26.0 * (u - 0.5) * (u - 0.5) * 4.0 - 8.0 * (u - 0.5)
        });
        let before = sharp(&img);
        let flat = dewarp(&img, 150, Rgb([238; 3])).expect("bend detected");
        assert_eq!(flat.dimensions(), img.dimensions());
        let after = sharp(&flat);
        assert!(after > before * 1.4, "sharpness {before} → {after}");
    }

    #[test]
    fn user_guides_straighten_a_bend_and_bad_input_is_ignored() {
        let bend = |u: f32| 26.0 * (u - 0.5) * (u - 0.5) * 4.0 - 8.0 * (u - 0.5);
        let img = page(900, 1200, bend);
        // two guides following the first and the third line (y0 = 120, 212)
        let guide = |y0: f32| -> Vec<(f32, f32)> {
            (0..=10)
                .map(|i| {
                    let u = i as f32 / 10.0;
                    (u, (y0 + 6.0 + bend(u)) / 1200.0)
                })
                .collect()
        };
        let before = sharp(&img);
        let flat = dewarp_guides(&img, &[guide(120.0), guide(212.0)], Rgb([238; 3])).unwrap();
        assert_eq!(flat.dimensions(), img.dimensions());
        assert!(sharp(&flat) > before * 1.2, "{before} → {}", sharp(&flat));
        let straight = vec![(0.0, 0.5), (1.0, 0.5)];
        assert!(dewarp_guides(&img, &[straight], Rgb([238; 3])).is_none());
        assert!(dewarp_guides(&img, &[], Rgb([238; 3])).is_none());
        let junk = vec![(f32::NAN, 0.5), (2.0, f32::INFINITY), (-3.0, 9.0)];
        let _ = dewarp_guides(&img, &[junk], Rgb([238; 3]));
    }

    #[test]
    fn vertical_strokes_are_ignored_and_halves_of_one_line_are_joined() {
        let bend = |u: f32| 26.0 * (u - 0.5) * (u - 0.5) * 4.0 - 8.0 * (u - 0.5);
        let img = page(900, 1200, bend);
        // a near-vertical slip is not a text line
        let slip = vec![(0.5, 0.1), (0.505, 0.9)];
        assert!(dewarp_guides(&img, &[slip], Rgb([238; 3])).is_none());
        // left and right half of the same line, with equal means, act together
        let half = |a: i32, b: i32| -> Vec<(f32, f32)> {
            (a..=b)
                .map(|i| {
                    let u = i as f32 / 10.0;
                    (u, (126.0 + bend(u)) / 1200.0)
                })
                .collect()
        };
        let both = dewarp_guides(&img, &[half(0, 5), half(5, 10)], Rgb([238; 3])).unwrap();
        let left = dewarp_guides(&img, &[half(0, 5)], Rgb([238; 3]));
        // the left half alone cannot reach the 30 % rule differently, but joined it must
        // straighten at least as well as either half on its own
        let s_both = sharp(&both);
        assert!(s_both > sharp(&img) * 1.1, "{} → {s_both}", sharp(&img));
        if let Some(l) = left {
            assert!(s_both >= sharp(&l) * 0.99);
        }
    }

    #[test]
    fn serialised_guides_always_parse_back() {
        let g = vec![
            vec![(0.1, 0.2), (0.9, 0.2)],
            vec![(0.5, f32::NAN), (1.0, 0.5)],
            vec![(0.5, 0.5)],
            vec![],
            vec![(0.0, 1.0000001), (1.0, 0.5)],
        ];
        let t = guides_to_string(&g);
        assert_eq!(parse_guides(&t).unwrap().len(), 1, "{t}");
    }

    #[test]
    fn guide_text_round_trips_and_rejects_junk() {
        let g = parse_guides("0,0.5 0.5,0.52 1,0.5; 0,0.8 1,0.8").unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(parse_guides(&guides_to_string(&g)).unwrap(), g);
        assert!(parse_guides("0,0.5").is_none());
        assert!(parse_guides("0,0.5 1.5,0.5").is_none());
        assert!(parse_guides("0,a 1,0.5").is_none());
        assert!(parse_guides("0,nan 1,0.5").is_none());
        assert_eq!(parse_guides("").unwrap().len(), 0);
    }

    #[test]
    fn blank_and_noise_pages_do_not_panic() {
        let blank = RgbImage::from_pixel(600, 800, Rgb([240; 3]));
        assert!(dewarp(&blank, 150, Rgb([240; 3])).is_none());
        let mut s = 12345u64;
        let noise = RgbImage::from_fn(500, 700, |_, _| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let v = (s % 256) as u8;
            Rgb([v, v, v])
        });
        let _ = dewarp(&noise, 150, Rgb([240; 3]));
        let tiny = RgbImage::from_pixel(20, 20, Rgb([0; 3]));
        assert!(dewarp(&tiny, 150, Rgb([0; 3])).is_none());
    }

    fn margin_page(left: impl Fn(u32) -> u32, ragged: bool) -> RgbImage {
        let (w, h) = (900u32, 1200u32);
        let mut img = RgbImage::from_pixel(w, h, Rgb([240, 240, 240]));
        let mut seed = 5u32;
        for (n, y) in (120..1080).step_by(36).enumerate() {
            let x0 = left(y) + if ragged && n % 3 == 0 { 40 } else { 0 };
            for yy in y..y + 12 {
                for x in x0..x0 + 640 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    if ((x - x0) / 5) % 3 != 2 || (seed >> 28) == 0 {
                        img.put_pixel(x, yy, Rgb([25, 25, 25]));
                    }
                }
            }
        }
        img
    }

    fn left_edges(img: &RgbImage) -> Vec<u32> {
        let mut v = Vec::new();
        for (n, y) in (120..1080).step_by(36).enumerate() {
            let _ = n;
            let probe = y + 6;
            if let Some(x) = (0..img.width()).find(|&x| img.get_pixel(x, probe).0[0] < 120) {
                v.push(x);
            }
        }
        v
    }

    #[test]
    fn a_bent_left_margin_is_straightened() {
        // margin bows 24 px to the right in the middle of the page (parabola)
        let bent = margin_page(
            |y| 100 + (24.0 * (1.0 - ((y as f32 - 600.0) / 480.0).powi(2))).max(0.0) as u32,
            false,
        );
        let before = left_edges(&bent);
        let spread = |v: &[u32]| v.iter().max().unwrap() - v.iter().min().unwrap();
        assert!(spread(&before) >= 18);
        let fixed = straighten_margins(&bent, 150, Rgb([240, 240, 240])).expect("curve found");
        let after = left_edges(&fixed);
        assert!(
            spread(&after) <= 4,
            "spread {} -> {}\n{:?}\n{:?}",
            spread(&before),
            spread(&after),
            before,
            after
        );
    }

    #[test]
    fn a_straight_or_ragged_margin_is_left_alone() {
        let straight = margin_page(|_| 100, false);
        assert!(straight_none(&straight));
        // every third line indented: no smooth curve
        let ragged = margin_page(|_| 100, true);
        assert!(straight_none(&ragged));
    }

    fn straight_none(img: &RgbImage) -> bool {
        straighten_margins(img, 150, Rgb([240, 240, 240])).is_none()
    }

    #[test]
    fn robust_quadratic_ignores_outliers() {
        let mut pts: Vec<(f32, f32)> = (0..30)
            .map(|k| {
                let t = k as f32 / 29.0;
                (t, 50.0 + 10.0 * t - 30.0 * t * t)
            })
            .collect();
        pts[3].1 += 80.0;
        pts[17].1 -= 60.0;
        let (c, inl) = robust_quadratic(&pts, 1.0).unwrap();
        assert!(
            (c[2] + 30.0).abs() < 1.0 && (c[0] - 50.0).abs() < 1.0,
            "{c:?}"
        );
        assert!(inl > 0.9);
        assert!(robust_quadratic(&pts[..3], 1.0).is_none());
    }
}
