use image::{imageops, Rgb, RgbImage};
use rayon::prelude::*;

/// Rotate by `turns` × 90° clockwise.
pub fn rot90(img: &RgbImage, turns: u8) -> RgbImage {
    match turns % 4 {
        1 => imageops::rotate90(img),
        2 => imageops::rotate180(img),
        3 => imageops::rotate270(img),
        _ => img.clone(),
    }
}

/// Estimate the paper colour: mean of the brightest ~10 % of sampled pixels.
pub fn estimate_background(img: &RgbImage) -> Rgb<u8> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Rgb([255, 255, 255]);
    }
    let step = (((w as u64 * h as u64) / 200_000).max(1) as f64)
        .sqrt()
        .ceil() as u32;
    let luma = |p: &Rgb<u8>| (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
    let mut hist = [0u64; 256];
    let mut n = 0u64;
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            hist[luma(img.get_pixel(x, y)) as usize] += 1;
            n += 1;
        }
    }
    let (mut acc, mut cut) = (0u64, 255usize);
    for l in (0..256).rev() {
        acc += hist[l];
        if acc * 10 >= n {
            cut = l;
            break;
        }
    }
    let (mut r, mut g, mut b, mut c) = (0u64, 0u64, 0u64, 0u64);
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let p = img.get_pixel(x, y);
            if luma(p) as usize >= cut {
                r += p[0] as u64;
                g += p[1] as u64;
                b += p[2] as u64;
                c += 1;
            }
        }
    }
    if c == 0 {
        return Rgb([255, 255, 255]);
    }
    Rgb([(r / c) as u8, (g / c) as u8, (b / c) as u8])
}

/// Rotate clockwise by `rad` (visual sense, image y axis pointing down),
/// enlarging the canvas so nothing is clipped. Bilinear interpolation;
/// uncovered area is filled with `fill`.
pub fn rotate_cw(img: &RgbImage, rad: f32, fill: Rgb<u8>) -> RgbImage {
    let (w, h) = img.dimensions();
    let (s, c) = (rad.sin(), rad.cos());
    let nw = ((w as f32 * c.abs() + h as f32 * s.abs()).ceil() as u32).max(1);
    let nh = ((w as f32 * s.abs() + h as f32 * c.abs()).ceil() as u32).max(1);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (ncx, ncy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    let mut out = vec![0u8; (nw as usize) * (nh as usize) * 3];
    let src = img.as_raw();
    out.par_chunks_mut(nw as usize * 3)
        .enumerate()
        .for_each(|(yd, row)| {
            for xd in 0..nw as usize {
                let dx = xd as f32 + 0.5 - ncx;
                let dy = yd as f32 + 0.5 - ncy;
                let xs = cx + dx * c + dy * s - 0.5;
                let ys = cy - dx * s + dy * c - 0.5;
                let px = &mut row[xd * 3..xd * 3 + 3];
                if xs < -1.0 || ys < -1.0 || xs > w as f32 || ys > h as f32 {
                    px.copy_from_slice(&fill.0);
                    continue;
                }
                let (x0, y0) = (xs.floor(), ys.floor());
                let (fx, fy) = (xs - x0, ys - y0);
                let sample = |xi: i64, yi: i64, ch: usize| -> f32 {
                    if xi < 0 || yi < 0 || xi >= w as i64 || yi >= h as i64 {
                        fill.0[ch] as f32
                    } else {
                        src[(yi as usize * w as usize + xi as usize) * 3 + ch] as f32
                    }
                };
                for ch in 0..3 {
                    let (xi, yi) = (x0 as i64, y0 as i64);
                    let v = sample(xi, yi, ch) * (1.0 - fx) * (1.0 - fy)
                        + sample(xi + 1, yi, ch) * fx * (1.0 - fy)
                        + sample(xi, yi + 1, ch) * (1.0 - fx) * fy
                        + sample(xi + 1, yi + 1, ch) * fx * fy;
                    px[ch] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        });
    RgbImage::from_raw(nw, nh, out).expect("size matches")
}

/// Cut the window `(x0,y0,w,h)` (may extend outside the image) out of `img`,
/// padding with `fill`.
pub fn crop_pad(img: &RgbImage, x0: i64, y0: i64, w: u32, h: u32, fill: Rgb<u8>) -> RgbImage {
    let mut out = RgbImage::from_pixel(w, h, fill);
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let sx0 = x0.max(0);
    let sx1 = (x0 + w as i64).min(iw);
    if sx1 <= sx0 {
        return out;
    }
    let len = (sx1 - sx0) as usize * 3;
    for dy in 0..h as i64 {
        let sy = y0 + dy;
        if sy < 0 || sy >= ih {
            continue;
        }
        let s = (sy as usize * iw as usize + sx0 as usize) * 3;
        let d = (dy as usize * w as usize + (sx0 - x0) as usize) * 3;
        out.as_mut()[d..d + len].copy_from_slice(&img.as_raw()[s..s + len]);
    }
    out
}

/// Box-average downscale by an integer factor (`f >= 1`).
pub fn downscale_rgb(img: &RgbImage, f: u32) -> RgbImage {
    if f <= 1 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let (nw, nh) = (w.div_ceil(f).max(1), h.div_ceil(f).max(1));
    let src = img.as_raw();
    let mut out = vec![0u8; nw as usize * nh as usize * 3];
    out.par_chunks_mut(nw as usize * 3)
        .enumerate()
        .for_each(|(oy, row)| {
            let (y0, y1) = (oy as u32 * f, ((oy as u32 + 1) * f).min(h));
            for ox in 0..nw as usize {
                let (x0, x1) = (ox as u32 * f, ((ox as u32 + 1) * f).min(w));
                let mut acc = [0u32; 3];
                for y in y0..y1 {
                    let base = (y as usize * w as usize + x0 as usize) * 3;
                    for i in 0..(x1 - x0) as usize {
                        for c in 0..3 {
                            acc[c] += src[base + i * 3 + c] as u32;
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)).max(1);
                for c in 0..3 {
                    row[ox * 3 + c] = ((acc[c] + n / 2) / n) as u8;
                }
            }
        });
    RgbImage::from_raw(nw, nh, out).expect("size matches")
}

/// Box-average downscale of a grayscale image by an integer factor.
pub fn downscale_gray(g: &image::GrayImage, f: u32) -> image::GrayImage {
    if f <= 1 {
        return g.clone();
    }
    let (w, h) = g.dimensions();
    let (nw, nh) = (w.div_ceil(f).max(1), h.div_ceil(f).max(1));
    let src = g.as_raw();
    let mut out = vec![0u8; nw as usize * nh as usize];
    out.par_chunks_mut(nw as usize)
        .enumerate()
        .for_each(|(oy, row)| {
            let (y0, y1) = (oy as u32 * f, ((oy as u32 + 1) * f).min(h));
            for (ox, o) in row.iter_mut().enumerate() {
                let (x0, x1) = (ox as u32 * f, ((ox as u32 + 1) * f).min(w));
                let mut acc = 0u32;
                for y in y0..y1 {
                    let base = y as usize * w as usize;
                    acc += src[base + x0 as usize..base + x1 as usize]
                        .iter()
                        .map(|&p| p as u32)
                        .sum::<u32>();
                }
                let n = ((y1 - y0) * (x1 - x0)).max(1);
                *o = ((acc + n / 2) / n) as u8;
            }
        });
    image::GrayImage::from_raw(nw, nh, out).expect("size matches")
}

/// 1-D max filter, radius `r` (clamped at the borders).
fn max_filter_1d(v: &[f32], r: usize) -> Vec<f32> {
    (0..v.len())
        .map(|i| {
            v[i.saturating_sub(r)..(i + r + 1).min(v.len())]
                .iter()
                .fold(0f32, |a, &b| a.max(b))
        })
        .collect()
}

/// 1-D box blur, radius `r`.
fn box_blur_1d(v: &[f32], r: usize) -> Vec<f32> {
    let mut pre = vec![0f64; v.len() + 1];
    for (i, &x) in v.iter().enumerate() {
        pre[i + 1] = pre[i] + x as f64;
    }
    (0..v.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(r), (i + r + 1).min(v.len()));
            ((pre[b] - pre[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Apply a separable 1-D filter to every row, then every column, of a `w`×`h` plane.
fn separable(plane: &mut [f32], w: usize, h: usize, f: impl Fn(&[f32]) -> Vec<f32>) {
    for y in 0..h {
        let r = f(&plane[y * w..(y + 1) * w]);
        plane[y * w..(y + 1) * w].copy_from_slice(&r);
    }
    let mut col = vec![0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = plane[y * w + x];
        }
        let r = f(&col);
        for y in 0..h {
            plane[y * w + x] = r[y];
        }
    }
}

/// Even out uneven lighting (shadow gradients, darkened corners, yellowed edges):
/// the paper level is estimated on a ≈60 dpi copy (max filter wipes out text, a wide
/// blur smooths the result), then every pixel is scaled so that paper becomes
/// `target`. Cheap — the heavy lifting happens on a few hundred-thousand pixels.
pub fn normalize_background(img: &RgbImage, dpi: u32, target: Rgb<u8>) -> RgbImage {
    let (w, h) = img.dimensions();
    if w < 8 || h < 8 {
        return img.clone();
    }
    let n = (dpi / 60).max(1);
    let small = downscale_rgb(img, n);
    let (sw, sh) = (small.width() as usize, small.height() as usize);
    let eff = dpi as f32 / n as f32;
    let (rmax, rblur) = (
        ((eff * 0.07).ceil() as usize).max(2),
        ((eff * 0.08).ceil() as usize).max(3),
    );
    let mut planes: Vec<Vec<f32>> = (0..3)
        .map(|c| {
            small
                .as_raw()
                .iter()
                .skip(c)
                .step_by(3)
                .map(|&v| v as f32)
                .collect()
        })
        .collect();
    for p in planes.iter_mut() {
        separable(p, sw, sh, |v| max_filter_1d(v, rmax));
        separable(p, sw, sh, |v| box_blur_1d(v, rblur));
        separable(p, sw, sh, |v| box_blur_1d(v, rblur / 2 + 1));
    }
    let src = img.as_raw();
    let mut out = vec![0u8; src.len()];
    let (fx, fy) = (1.0 / n as f32, 1.0 / n as f32);
    out.par_chunks_mut(w as usize * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let sy = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
            let (y0, ty) = (sy.floor() as usize, sy - sy.floor());
            let y1 = (y0 + 1).min(sh - 1);
            for x in 0..w as usize {
                let sx = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
                let (x0, tx) = (sx.floor() as usize, sx - sx.floor());
                let x1 = (x0 + 1).min(sw - 1);
                for c in 0..3 {
                    let p = &planes[c];
                    let bg = p[y0 * sw + x0] * (1.0 - tx) * (1.0 - ty)
                        + p[y0 * sw + x1] * tx * (1.0 - ty)
                        + p[y1 * sw + x0] * (1.0 - tx) * ty
                        + p[y1 * sw + x1] * tx * ty;
                    let v = src[(y * w as usize + x) * 3 + c] as f32 * target.0[c] as f32
                        / bg.max(24.0);
                    row[x * 3 + c] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        });
    RgbImage::from_raw(w, h, out).expect("size matches")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_averages_and_keeps_size_rules() {
        let img = RgbImage::from_fn(5, 3, |x, _| Rgb([(x * 50) as u8, 0, 255]));
        let d = downscale_rgb(&img, 2);
        assert_eq!(d.dimensions(), (3, 2));
        assert_eq!(d.get_pixel(0, 0).0, [25, 0, 255]);
        let g = image::GrayImage::from_fn(4, 4, |x, y| image::Luma([(x + y * 4) as u8 * 10]));
        assert_eq!(downscale_gray(&g, 2).dimensions(), (2, 2));
        assert_eq!(downscale_gray(&g, 1).dimensions(), (4, 4));
    }

    #[test]
    fn normalisation_flattens_a_lighting_gradient_and_keeps_text_dark() {
        let (w, h) = (600u32, 800u32);
        let img = RgbImage::from_fn(w, h, |x, y| {
            let light = 1.0 - 0.5 * (x as f32 / w as f32) - 0.2 * (y as f32 / h as f32);
            let text =
                (y / 30) % 2 == 0 && (y % 30) < 8 && (80..520).contains(&x) && (x / 10) % 2 == 0;
            let v = if text { 40.0 } else { 235.0 };
            Rgb([
                (v * light) as u8,
                (v * light) as u8,
                (v * light * 0.9) as u8,
            ])
        });
        let out = normalize_background(&img, 150, Rgb([240, 240, 240]));
        // Paper far apart (blank rows between lines) ends up at the same level.
        let a = out.get_pixel(30, 400)[0] as i32;
        let b = out.get_pixel(570, 790)[0] as i32;
        assert!((a - b).abs() <= 10, "paper levels {a} vs {b}");
        assert!(a >= 220, "paper not whitened: {a}");
        // Text pixels stay clearly darker than paper.
        let t = out.get_pixel(100, 4)[0] as i32;
        assert!(t < a - 100, "text {t} vs paper {a}");
    }
}
