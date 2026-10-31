//! Skew estimation by maximising the sharpness of the horizontal projection
//! profile of the ink (text lines produce sharp peaks when level).

use crate::BitImage;

/// Returns the angle (radians) by which the page must be rotated clockwise to
/// level the text lines, or `None` when the page has too little ink to judge.
pub fn detect_skew(ink: &BitImage, max_deg: f32) -> Option<f32> {
    // tan() explodes towards 90°: bound the search (and survive NaN) so the
    // profile table can never be absurdly large.
    let max_deg = if max_deg.is_finite() { max_deg.clamp(0.0, 30.0) } else { 0.0 };
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
        prof.windows(2).map(|p| {
            let d = p[1] as f64 - p[0] as f64;
            d * d
        }).sum()
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
    let (c, _) = search(-max_deg, max_deg, 0.5);
    let (f, best) = search(c - 0.5, c + 0.5, 0.05);
    let (f, best) = {
        let (g, s) = search(f - 0.05, f + 0.05, 0.01);
        if s >= best { (g, s) } else { (f, best) }
    };
    // Require a real gain over the unrotated profile, otherwise stay put.
    let base = score(0.0);
    if best < base * 1.02 {
        return Some(0.0);
    }
    Some(f.to_radians())
}
