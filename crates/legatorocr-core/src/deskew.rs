//! Skew estimation by maximising the sharpness of the horizontal projection
//! profile of the ink (text lines produce sharp peaks when level).

use crate::BitImage;

/// Returns the angle (radians) by which the page must be rotated clockwise to
/// level the text lines, or `None` when the page has too little ink to judge.
pub fn detect_skew(ink: &BitImage, max_deg: f32) -> Option<f32> {
    // Scanner-bed shadows and page edges are axis-aligned even when the page is not: leave
    // them out so only the text decides.
    let cleaned = crate::orient::without_long_runs(ink, 150);
    let ink = &cleaned;
    let (ang, prom, share) = profile_skew(ink, max_deg)?;
    // A page with only a line or two of ink (a title, a folio) cannot be judged: stay put.
    if share < MIN_INK_SHARE {
        return None;
    }
    // Line drawings, perspective views and photographs have no dominant line direction:
    // their best angle barely stands out from the others and is not a page tilt.
    if prom < MIN_PROMINENCE {
        return None;
    }
    // Cross-check with the vertical structure (columns, margins, drawing edges): a rigid
    // rotation of the scan tilts both. When both axes give strong evidence and disagree,
    // the content is not a plain rotated page (oblique drawing, hatching): do not rotate.
    if prom >= STRONG_PROMINENCE {
        let mut t = BitImage::new(ink.h, ink.w);
        let (w, h) = (ink.w as usize, ink.h as usize);
        for y in 0..h {
            for x in 0..w {
                if ink.data[y * w + x] != 0 {
                    t.data[x * h + y] = 1;
                }
            }
        }
        if let Some((va, vprom, _)) = profile_skew(&t, max_deg) {
            if vprom >= STRONG_PROMINENCE {
                // Transposing mirrors the angle's sign.
                let v = -va;
                if (ang - v).abs() > 0.4f32.to_radians() {
                    return Some(0.0);
                }
                let (wh, wv) = (prom as f32, vprom as f32);
                return Some((ang * wh + v * wv) / (wh + wv));
            }
        }
    }
    Some(ang)
}

/// Fewer inked pixels than this share of the page: too little to judge the skew.
const MIN_INK_SHARE: f64 = 0.008;
/// Below this peak prominence there is no trustworthy text orientation (typical text pages
/// score 15–60, drawings and half-tones 1.4–5.6): leave the page alone.
const MIN_PROMINENCE: f64 = 8.0;
/// Peak-to-median ratio of the sharpness curve over all tried angles that counts as a
/// clear, well-defined orientation (typical text pages score 10–60).
const STRONG_PROMINENCE: f64 = 10.0;

/// (angle in radians, peak prominence over the median angle, ink share of the page).
fn profile_skew(ink: &BitImage, max_deg: f32) -> Option<(f32, f64, f64)> {
    // tan() explodes towards 90°: bound the search (and survive NaN) so the
    // profile table can never be absurdly large.
    let max_deg = if max_deg.is_finite() {
        max_deg.clamp(0.0, 30.0)
    } else {
        0.0
    };
    let total = ink.count_ink();
    // Too little ink to judge, or so much that the page is a solid area (a
    // black scan, a photo) rather than text lines.
    if total < 200 || total * 2 > ink.w as u64 * ink.h as u64 {
        return None;
    }
    // Collect (a possibly sub-sampled set of) ink coordinates.
    let stride = ((total / 400_000) as usize).max(1);
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity((total as usize / stride) + 1);
    let mut n = 0usize;
    for y in 0..ink.h {
        let row = &ink.data[(y as usize * ink.w as usize)..((y as usize + 1) * ink.w as usize)];
        for (x, &v) in row.iter().enumerate() {
            if v != 0 {
                if n.is_multiple_of(stride) {
                    pts.push((x as f32, y as f32));
                }
                n += 1;
            }
        }
    }
    let score = |deg: f32| -> f64 {
        let t = deg.to_radians().tan();
        // A line rising to the right by `deg` has y + x·tan(deg) = const.
        let off = (ink.w as f32 * t.abs()).ceil() as i64 + 1;
        let bins = (ink.h as i64 + 2 * off + 2) as usize;
        let mut prof = vec![0u32; bins];
        for &(x, y) in &pts {
            let b = (y + x * t).round() as i64 + off;
            prof[b as usize] += 1;
        }
        prof.windows(2)
            .map(|p| {
                let d = p[1] as f64 - p[0] as f64;
                d * d
            })
            .sum()
    };
    let search = |lo: f32, hi: f32, step: f32| -> (f32, f64) {
        let mut best = (0.0f32, f64::MIN);
        let mut a = lo;
        while a <= hi + 1e-6 {
            let s = score(a);
            if s > best.1 {
                best = (a, s);
            }
            a += step;
        }
        best
    };
    let mut all = Vec::new();
    {
        let mut a = -max_deg;
        while a <= max_deg + 1e-6 {
            all.push(score(a));
            a += 0.5;
        }
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = all[all.len() / 2].max(1.0);
    let (c, _) = search(-max_deg, max_deg, 0.5);
    let (f, best) = search(c - 0.5, c + 0.5, 0.05);
    let (f, best) = {
        let (g, s) = search(f - 0.05, f + 0.05, 0.01);
        if s >= best {
            (g, s)
        } else {
            (f, best)
        }
    };
    // Require a real gain over the unrotated profile, otherwise stay put.
    let base = score(0.0);
    let share = total as f64 / (ink.w as f64 * ink.h as f64);
    if best < base * 1.02 {
        return Some((0.0, best / med, share));
    }
    Some((f.to_radians(), best / med, share))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thick line from (x0,y0) to (x1,y1).
    fn line(img: &mut BitImage, x0: f32, y0: f32, x1: f32, y1: f32) {
        let n = ((x1 - x0).abs().max((y1 - y0).abs()) as usize).max(1) * 2;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
            for d in 0..3 {
                let (xx, yy) = (x as i64, y as i64 + d);
                if xx >= 0 && yy >= 0 && (xx as u32) < img.w && (yy as u32) < img.h {
                    img.set(xx as u32, yy as u32, true);
                }
            }
        }
    }

    /// A page of text-like rows and column rules, rotated by `deg` clockwise-positive.
    fn grid(deg_h: f32, deg_v: f32) -> BitImage {
        let (w, h) = (900u32, 1200u32);
        let mut img = BitImage::new(w, h);
        let (th, tv) = (deg_h.to_radians().tan(), deg_v.to_radians().tan());
        for r in 0..40 {
            let y = 80.0 + r as f32 * 26.0;
            line(&mut img, 60.0, y, 840.0, y + 780.0 * th);
        }
        for c in 0..6 {
            let x = 60.0 + c as f32 * 156.0;
            line(&mut img, x, 60.0, x + 1100.0 * tv, 1140.0);
        }
        img
    }

    #[test]
    fn rotated_page_is_measured_from_both_axes() {
        // Rows rise to the right by 1.5°; a clockwise-leaning column rule means the
        // same rigid rotation (x grows with y by tan(1.5°)).
        let a = detect_skew(&grid(-1.5, 1.5), 7.0).unwrap().to_degrees();
        assert!((a.abs() - 1.5).abs() < 0.2, "{a}");
    }

    #[test]
    fn conflicting_axes_leave_the_page_alone() {
        // Rows tilted 2° but the column rules perfectly upright: not a rotated page.
        let a = detect_skew(&grid(-2.0, 0.0), 7.0).unwrap();
        assert!(a.abs() < 1e-6, "{a}");
    }

    #[test]
    fn a_page_with_a_single_line_is_not_judged() {
        let mut img = BitImage::new(900, 1200);
        line(&mut img, 300.0, 100.0, 600.0, 112.0);
        assert_eq!(detect_skew(&img, 7.0), None);
    }

    #[test]
    fn axis_aligned_edge_bars_do_not_hide_the_skew() {
        // Text tilted by 2 degrees plus scanner-edge bars that stay vertical.
        let (w, h) = (600u32, 800u32);
        let mut b = BitImage::new(w, h);
        let t = (2.0f32).to_radians().tan();
        for line in 0..26u32 {
            for x in 80..520u32 {
                if (x / 14 + line) % 5 == 4 {
                    continue;
                }
                let y = (90 + line * 24) as f32 + t * (x as f32 - 300.0);
                for dy in 0..5 {
                    b.data[((y as u32 + dy) * w + x) as usize] = 1;
                }
            }
        }
        for y in 0..h {
            for x in (0..45).chain(w - 25..w) {
                b.data[(y * w + x) as usize] = 1;
            }
        }
        let a = detect_skew(&b, 7.0).unwrap().to_degrees();
        assert!((a + 2.0).abs() < 0.4, "got {a}"); // the correction undoes the tilt
    }
}
