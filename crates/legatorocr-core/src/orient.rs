//! Coarse text-orientation test (0° vs a quarter turn) from projection profiles.

use crate::BitImage;

fn variance(v: &[f64]) -> f64 {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n
}

/// `true` when text lines run vertically (page lies on its side), i.e. the
/// column profile is markedly "peakier" than the row profile.
/// Cannot distinguish 90° from 270° or detect 180° (use OCR/OSD for that).
pub fn needs_quarter_turn(ink: &BitImage) -> bool {
    if ink.count_ink() < 500 {
        return false;
    }
    let (w, h) = (ink.w as usize, ink.h as usize);
    let mut rows = vec![0f64; h];
    let mut cols = vec![0f64; w];
    for y in 0..h {
        for x in 0..w {
            if ink.data[y * w + x] != 0 {
                rows[y] += 1.0;
                cols[x] += 1.0;
            }
        }
    }
    // Normalise by length so landscape/portrait shape does not bias the test.
    let vr = variance(&rows) / (w as f64 * w as f64);
    let vc = variance(&cols) / (h as f64 * h as f64);
    vc > 2.0 * vr
}
