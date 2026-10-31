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
    let fh = img.height() as usize;
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
}
