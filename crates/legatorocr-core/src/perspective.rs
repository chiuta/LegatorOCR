//! Perspective correction for pages photographed at an angle (phone shots): the page
//! is found as the large bright quadrilateral on a darker background, and warped to a
//! rectangle with a homography. Deliberately conservative: when the page does not look
//! like a clean four-cornered shape, or is already nearly rectangular, nothing happens
//! (small rotations are the deskew stage's job).

use crate::binarize::otsu_threshold;
use image::{imageops, GrayImage, Rgb, RgbImage};
use rayon::prelude::*;

type Pt = (f32, f32);

/// Corners (TL, TR, BR, BL) of the page in `img` coordinates, if a plausible quad is found.
pub fn find_page_quad(img: &RgbImage) -> Option<[Pt; 4]> {
    let (w, h) = img.dimensions();
    if w < 64 || h < 64 {
        return None;
    }
    let f = (w.max(h) as f32 / 480.0).max(1.0);
    let (sw, sh) = (
        ((w as f32 / f) as u32).max(8),
        ((h as f32 / f) as u32).max(8),
    );
    let small: GrayImage = imageops::resize(
        &imageops::grayscale(img),
        sw,
        sh,
        imageops::FilterType::Triangle,
    );
    let t = otsu_threshold(&small);
    let (sw, sh) = (sw as usize, sh as usize);
    let mask: Vec<bool> = small.as_raw().iter().map(|&v| v > t).collect();
    let bright = mask.iter().filter(|&&m| m).count() as f32 / mask.len() as f32;
    if !(0.2..=0.97).contains(&bright) {
        return None;
    }
    // largest 4-connected bright component
    let mut seen = vec![false; mask.len()];
    let mut best: Vec<usize> = Vec::new();
    let mut stack = Vec::new();
    for s in 0..mask.len() {
        if !mask[s] || seen[s] {
            continue;
        }
        let mut comp = Vec::new();
        seen[s] = true;
        stack.push(s);
        while let Some(p) = stack.pop() {
            comp.push(p);
            let (x, y) = (p % sw, p / sw);
            let mut push = |q: usize| {
                if mask[q] && !seen[q] {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if x > 0 {
                push(p - 1);
            }
            if x + 1 < sw {
                push(p + 1);
            }
            if y > 0 {
                push(p - sw);
            }
            if y + 1 < sh {
                push(p + sw);
            }
        }
        if comp.len() > best.len() {
            best = comp;
        }
    }
    if (best.len() as f32) < 0.3 * mask.len() as f32 {
        return None;
    }
    // corners = extreme points along the diagonals
    let (mut tl, mut tr, mut br, mut bl) = (
        (0i32, i32::MAX),
        (0i32, i32::MIN),
        (0i32, i32::MIN),
        (0i32, i32::MAX),
    );
    let mut pts = [(0f32, 0f32); 4];
    for &p in &best {
        let (x, y) = ((p % sw) as i32, (p / sw) as i32);
        let (sum, dif) = (x + y, x - y);
        if sum < tl.1 {
            tl = (p as i32, sum);
            pts[0] = (x as f32, y as f32);
        }
        if dif > tr.1 {
            tr = (p as i32, dif);
            pts[1] = (x as f32, y as f32);
        }
        if sum > br.1 {
            br = (p as i32, sum);
            pts[2] = (x as f32, y as f32);
        }
        if dif < bl.1 {
            bl = (p as i32, dif);
            pts[3] = (x as f32, y as f32);
        }
    }
    // shoelace area and convexity
    let area = |q: &[Pt; 4]| {
        (0..4)
            .map(|i| q[i].0 * q[(i + 1) % 4].1 - q[(i + 1) % 4].0 * q[i].1)
            .sum::<f32>()
            .abs()
            / 2.0
    };
    let a = area(&pts);
    // text holes do not matter: measure the component by its row spans
    let mut span = vec![(usize::MAX, 0usize); sh];
    for &p in &best {
        let (x, y) = (p % sw, p / sw);
        span[y].0 = span[y].0.min(x);
        span[y].1 = span[y].1.max(x);
    }
    let filled: usize = span
        .iter()
        .filter(|s| s.0 != usize::MAX)
        .map(|s| s.1 - s.0 + 1)
        .sum();
    if a < 0.3 * (sw * sh) as f32 || (filled as f32) < 0.8 * a {
        return None; // not a clean quadrilateral (ragged blob, several objects)
    }
    let cross = |o: Pt, p: Pt, q: Pt| (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0);
    let signs: Vec<bool> = (0..4)
        .map(|i| cross(pts[i], pts[(i + 1) % 4], pts[(i + 2) % 4]) > 0.0)
        .collect();
    if signs.iter().any(|&s| s != signs[0]) {
        return None;
    }
    let q = pts.map(|(x, y)| (x * f, y * f));
    // already (nearly) a rectangle? then leave it to deskew
    let d = |a: Pt, b: Pt| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let (top, bottom, left, right) = (d(q[0], q[1]), d(q[3], q[2]), d(q[0], q[3]), d(q[1], q[2]));
    let ratio = |a: f32, b: f32| a.max(b) / a.min(b).max(1.0);
    let keystone = ratio(top, bottom).max(ratio(left, right));
    let tilt = |a: Pt, b: Pt| (b.1 - a.1).atan2(b.0 - a.0).to_degrees().abs();
    let off_axis = tilt(q[0], q[1])
        .max(tilt(q[3], q[2]))
        .max(90.0 - tilt(q[0], q[3]).min(180.0 - tilt(q[0], q[3])));
    if keystone < 1.04 && off_axis < 1.0 {
        return None;
    }
    Some(q)
}

/// Warp the quad to a rectangle of the average edge lengths. `None` when no page quad is
/// found or the result would be unreasonably large/small.
pub fn correct(img: &RgbImage) -> Option<RgbImage> {
    let q = find_page_quad(img)?;
    let d = |a: Pt, b: Pt| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let ow = ((d(q[0], q[1]) + d(q[3], q[2])) / 2.0).round();
    let oh = ((d(q[0], q[3]) + d(q[1], q[2])) / 2.0).round();
    if !(32.0..=16_000.0).contains(&ow)
        || !(32.0..=16_000.0).contains(&oh)
        || ow as f64 * oh as f64 > 150e6
    {
        return None;
    }
    let (ow, oh) = (ow as u32, oh as u32);
    // unit square → quad (projective mapping, closed form)
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q;
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let den = dx1 * dy2 - dx2 * dy1;
    if den.abs() < 1e-6 {
        return None;
    }
    let g = (dx3 * dy2 - dx2 * dy3) / den;
    let hh = (dx1 * dy3 - dx3 * dy1) / den;
    let (a, b, c) = (x1 - x0 + g * x1, x3 - x0 + hh * x3, x0);
    let (dd, e, ff) = (y1 - y0 + g * y1, y3 - y0 + hh * y3, y0);
    let (w, h) = img.dimensions();
    let src = img.as_raw();
    let mut out = RgbImage::new(ow, oh);
    out.par_chunks_mut(ow as usize * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let v = (y as f32 + 0.5) / oh as f32;
            for x in 0..ow as usize {
                let u = (x as f32 + 0.5) / ow as f32;
                let den = g * u + hh * v + 1.0;
                let (sx, sy) = (
                    (a * u + b * v + c) / den - 0.5,
                    (dd * u + e * v + ff) / den - 0.5,
                );
                let (sx, sy) = (sx.clamp(0.0, (w - 1) as f32), sy.clamp(0.0, (h - 1) as f32));
                let (ix, iy) = (sx.floor() as usize, sy.floor() as usize);
                let (fx, fy) = (sx - ix as f32, sy - iy as f32);
                let (ix1, iy1) = ((ix + 1).min(w as usize - 1), (iy + 1).min(h as usize - 1));
                for ch in 0..3 {
                    let p = |xx: usize, yy: usize| src[(yy * w as usize + xx) * 3 + ch] as f32;
                    let top = p(ix, iy) * (1.0 - fx) + p(ix1, iy) * fx;
                    let bot = p(ix, iy1) * (1.0 - fx) + p(ix1, iy1) * fx;
                    row[x * 3 + ch] = (top * (1.0 - fy) + bot * fy).round() as u8;
                }
            }
        });
    let _ = Rgb([0u8; 3]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white page with text-like bars, photographed as a trapezoid on a dark desk.
    fn photo(quad: [Pt; 4]) -> RgbImage {
        let (w, h) = (900u32, 1200u32);
        let mut img = RgbImage::from_pixel(w, h, Rgb([40, 35, 30]));
        let inside = |x: f32, y: f32| {
            (0..4).all(|i| {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0) >= 0.0
            })
        };
        for y in 0..h {
            for x in 0..w {
                if inside(x as f32, y as f32) {
                    let (fx, fy) = (x as f32, y as f32);
                    let deep = inside(fx - 40.0, fy)
                        && inside(fx + 40.0, fy)
                        && inside(fx, fy - 40.0)
                        && inside(fx, fy + 40.0);
                    let bar = deep && (y / 30) % 3 == 0 && (x / 10) % 7 != 0;
                    img.put_pixel(
                        x,
                        y,
                        if bar {
                            Rgb([30, 30, 30])
                        } else {
                            Rgb([235, 232, 225])
                        },
                    );
                }
            }
        }
        img
    }

    #[test]
    fn keystoned_page_becomes_a_rectangle() {
        let q = [
            (150.0, 100.0),
            (750.0, 160.0),
            (820.0, 1100.0),
            (80.0, 1050.0),
        ];
        let img = photo(q);
        let found = find_page_quad(&img).expect("quad");
        for (f, t) in found.iter().zip(q.iter()) {
            assert!(
                (f.0 - t.0).abs() < 25.0 && (f.1 - t.1).abs() < 25.0,
                "{f:?} vs {t:?}"
            );
        }
        let out = correct(&img).expect("warp");
        assert!(out.width() > 500 && out.height() > 800);
        // corners of the result are paper, not desk
        for (x, y) in [
            (6, 6),
            (out.width() - 7, 6),
            (6, out.height() - 7),
            (out.width() - 7, out.height() - 7),
        ] {
            assert!(
                out.get_pixel(x, y).0[0] > 150,
                "corner {x},{y} = {:?}",
                out.get_pixel(x, y)
            );
        }
    }

    #[test]
    fn upright_page_and_full_frame_scans_are_left_alone() {
        let straight = photo([
            (100.0, 100.0),
            (800.0, 100.0),
            (800.0, 1100.0),
            (100.0, 1100.0),
        ]);
        assert!(correct(&straight).is_none());
        let full = RgbImage::from_pixel(600, 800, Rgb([240, 240, 240]));
        assert!(correct(&full).is_none());
        let dark = RgbImage::from_pixel(600, 800, Rgb([20, 20, 20]));
        assert!(correct(&dark).is_none());
        assert!(correct(&RgbImage::new(10, 10)).is_none());
    }
}
