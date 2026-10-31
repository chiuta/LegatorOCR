//! Coarse text-orientation test (0° vs a quarter turn) from projection profiles.
//!
//! Horizontal text lines make the *row* profile oscillate at the line pitch.
//! Large structures (page gutter, margins, a spread's two pages) also move the
//! profile but at much lower frequency, so each profile is band-passed around
//! the line-pitch scale before comparing rows against columns.

use crate::BitImage;

/// Box-filter `p` with half-width `r` (prefix sums, edges clamp).
fn box_smooth(p: &[f64], r: usize) -> Vec<f64> {
    let n = p.len();
    let mut pre = vec![0.0; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + p[i];
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(r), (i + r + 1).min(n));
            (pre[b] - pre[a]) / (b - a) as f64
        })
        .collect()
}

/// Mean squared difference between a fine and a coarse smoothing of the profile.
fn band_energy(profile: &[f64], dpi: u32) -> f64 {
    let fine = box_smooth(profile, (dpi / 60).max(2) as usize);
    let coarse = box_smooth(profile, (dpi / 4).max(8) as usize);
    fine.iter()
        .zip(&coarse)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        / profile.len().max(1) as f64
}

/// `true` when text lines run vertically (page lies on its side).
/// Cannot distinguish 90° from 270° or detect 180° (use OCR/OSD for that).
pub fn needs_quarter_turn(ink: &BitImage, dpi: u32) -> bool {
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
    let er = band_energy(&rows, dpi) / (w as f64 * w as f64);
    let ec = band_energy(&cols, dpi) / (h as f64 * h as f64);
    ec > 2.0 * er
}
