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
    let step = (((w as u64 * h as u64) / 200_000).max(1) as f64).sqrt().ceil() as u32;
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
    out.par_chunks_mut(nw as usize * 3).enumerate().for_each(|(yd, row)| {
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
