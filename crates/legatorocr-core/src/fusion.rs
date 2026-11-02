//! Multi-frame fusion for phone photos (roadmap 7.6): several shots of the same page are
//! aligned to the first by a translation (coarse search on a reduced copy, then refined at full
//! size) and combined by a per-pixel median, which removes sensor noise and the odd glare
//! highlight that appears in one frame only.
//!
//! Limits: translation only (a phone held in the hand also rotates and changes perspective a
//! little; frames that do not align well are left out and reported), and the frames must show the
//! same page at the same scale.

use image::{GrayImage, RgbImage};

/// Gray copy reduced by an integer factor (box average).
fn reduce(g: &GrayImage, f: u32) -> GrayImage {
    let (w, h) = (g.width() / f, g.height() / f);
    let mut o = GrayImage::new(w.max(1), h.max(1));
    for y in 0..h {
        for x in 0..w {
            let mut s = 0u32;
            for dy in 0..f {
                for dx in 0..f {
                    s += g.get_pixel(x * f + dx, y * f + dy)[0] as u32;
                }
            }
            o.put_pixel(x, y, image::Luma([(s / (f * f)) as u8]));
        }
    }
    o
}

/// Mean squared difference of `b` shifted by (dx, dy) against `a` over their overlap, sampled on
/// a grid of `step`; `None` when the overlap is under half of `a`.
fn msd(a: &GrayImage, b: &GrayImage, dx: i32, dy: i32, step: u32) -> Option<f64> {
    let (w, h) = (a.width() as i32, a.height() as i32);
    let (x0, x1) = (0.max(dx), w.min(b.width() as i32 + dx));
    let (y0, y1) = (0.max(dy), h.min(b.height() as i32 + dy));
    if (x1 - x0) * 2 < w || (y1 - y0) * 2 < h {
        return None;
    }
    let (mut s, mut n) = (0f64, 0u64);
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let d = a.get_pixel(x as u32, y as u32)[0] as f64 - b.get_pixel((x - dx) as u32, (y - dy) as u32)[0] as f64;
            s += d * d;
            n += 1;
            x += step as i32;
        }
        y += step as i32;
    }
    (n > 0).then(|| s / n as f64)
}

/// Shift (dx, dy) such that frame `b` moved by it matches `a`, and the residual error.
pub fn align(a: &GrayImage, b: &GrayImage, max_shift: u32) -> Option<(i32, i32, f64)> {
    const F: u32 = 4;
    let (ra, rb) = (reduce(a, F), reduce(b, F));
    let r = (max_shift / F + 1) as i32;
    let mut best: Option<(f64, i32, i32)> = None;
    for dy in -r..=r {
        for dx in -r..=r {
            if let Some(e) = msd(&ra, &rb, dx, dy, 1) {
                if best.is_none_or(|(be, _, _)| e < be) {
                    best = Some((e, dx, dy));
                }
            }
        }
    }
    let (_, cx, cy) = best?;
    // refine at full size around the coarse answer
    let (bx, by) = (cx * F as i32, cy * F as i32);
    let mut fine: Option<(f64, i32, i32)> = None;
    for dy in (by - F as i32)..=(by + F as i32) {
        for dx in (bx - F as i32)..=(bx + F as i32) {
            if let Some(e) = msd(a, b, dx, dy, 3) {
                if fine.is_none_or(|(be, _, _)| e < be) {
                    fine = Some((e, dx, dy));
                }
            }
        }
    }
    fine.map(|(e, x, y)| (x, y, e))
}

pub struct Fused {
    pub image: RgbImage,
    /// Per input frame: the shift applied, or `None` when it was left out.
    pub shifts: Vec<Option<(i32, i32)>>,
}

fn gray_of(img: &RgbImage) -> GrayImage {
    image::DynamicImage::ImageRgb8(img.clone()).to_luma8()
}

/// Fuse `frames` (the first is the reference). A frame whose best alignment still differs from
/// the reference by more than `reject_msd` (mean squared gray difference) is left out.
pub fn fuse(frames: &[RgbImage], max_shift: u32, reject_msd: f64) -> Option<Fused> {
    let first = frames.first()?;
    let ga = gray_of(first);
    let mut used: Vec<(usize, i32, i32)> = vec![(0, 0, 0)];
    let mut shifts = vec![None; frames.len()];
    shifts[0] = Some((0, 0));
    for (i, f) in frames.iter().enumerate().skip(1) {
        if f.dimensions() != first.dimensions() {
            continue;
        }
        if let Some((dx, dy, e)) = align(&ga, &gray_of(f), max_shift) {
            if e <= reject_msd {
                used.push((i, dx, dy));
                shifts[i] = Some((dx, dy));
            }
        }
    }
    let (w, h) = first.dimensions();
    let mut out = RgbImage::new(w, h);
    let mut buf: Vec<[u8; 3]> = Vec::with_capacity(used.len());
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            buf.clear();
            for &(i, dx, dy) in &used {
                let (sx, sy) = (x - dx, y - dy);
                if sx >= 0 && sy >= 0 && sx < w as i32 && sy < h as i32 {
                    buf.push(frames[i].get_pixel(sx as u32, sy as u32).0);
                }
            }
            let mut px = [0u8; 3];
            for c in 0..3 {
                let mut v: Vec<u8> = buf.iter().map(|p| p[c]).collect();
                v.sort_unstable();
                px[c] = match v.len() {
                    0 => first.get_pixel(x as u32, y as u32)[c],
                    n if n % 2 == 1 => v[n / 2],
                    n => ((v[n / 2 - 1] as u16 + v[n / 2] as u16) / 2) as u8,
                };
            }
            out.put_pixel(x as u32, y as u32, image::Rgb(px));
        }
    }
    Some(Fused { image: out, shifts })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random in [0,1).
    fn rnd(state: &mut u64) -> f64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*state >> 33) as f64) / (1u64 << 31) as f64
    }

    fn page() -> RgbImage {
        let mut im = RgbImage::from_pixel(240, 320, image::Rgb([235, 232, 225]));
        let mut s = 7u64;
        for _ in 0..140 {
            let (x, y) = ((rnd(&mut s) * 200.0) as u32 + 10, (rnd(&mut s) * 290.0) as u32 + 10);
            let (w, h) = ((rnd(&mut s) * 14.0) as u32 + 3, (rnd(&mut s) * 5.0) as u32 + 2);
            for yy in y..(y + h).min(320) {
                for xx in x..(x + w).min(240) {
                    im.put_pixel(xx, yy, image::Rgb([30, 30, 35]));
                }
            }
        }
        im
    }

    fn noisy(base: &RgbImage, dx: i32, dy: i32, seed: u64, sigma: f64) -> RgbImage {
        let mut s = seed;
        let (w, h) = base.dimensions();
        let mut o = RgbImage::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (sx, sy) = ((x - dx).clamp(0, w as i32 - 1), (y - dy).clamp(0, h as i32 - 1));
                let p = base.get_pixel(sx as u32, sy as u32);
                let n = (rnd(&mut s) + rnd(&mut s) + rnd(&mut s) - 1.5) * 2.0 * sigma;
                o.put_pixel(x as u32, y as u32, image::Rgb([0, 1, 2].map(|c| (p[c] as f64 + n).clamp(0.0, 255.0) as u8)));
            }
        }
        o
    }

    fn mse(a: &RgbImage, b: &RgbImage) -> f64 {
        a.pixels().zip(b.pixels()).map(|(p, q)| (0..3).map(|c| (p[c] as f64 - q[c] as f64).powi(2)).sum::<f64>()).sum::<f64>() / (a.width() * a.height() * 3) as f64
    }

    #[test]
    fn shifts_are_found_and_noise_drops() {
        let truth = page();
        let shifts = [(0, 0), (5, -3), (-7, 4), (2, 6), (-4, -5)];
        let frames: Vec<RgbImage> = shifts.iter().enumerate().map(|(i, &(dx, dy))| noisy(&truth, dx, dy, 100 + i as u64, 30.0)).collect();
        let fused = fuse(&frames, 12, 1500.0).unwrap();
        for (i, &(dx, dy)) in shifts.iter().enumerate() {
            // frame i is the reference moved by (dx, dy); aligning it back needs the opposite shift
            assert_eq!(fused.shifts[i], Some((-dx, -dy)), "frame {i}");
        }
        // compare in the area every frame covers
        let crop = |im: &RgbImage| image::imageops::crop_imm(im, 12, 12, 216, 296).to_image();
        let (single, many) = (mse(&crop(&frames[0]), &crop(&truth)), mse(&crop(&fused.image), &crop(&truth)));
        eprintln!("MSE vs truth: one frame {single:.1}, fused {many:.1}");
        assert!(many < single * 0.7, "fusion should cut the noise: {single} -> {many}");
    }

    #[test]
    fn a_different_page_is_left_out() {
        let a = page();
        let b = RgbImage::from_pixel(240, 320, image::Rgb([10, 200, 10]));
        let f = fuse(&[a, b], 12, 1500.0).unwrap();
        assert_eq!(f.shifts[1], None);
    }
}
