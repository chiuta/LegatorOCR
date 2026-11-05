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

/// Copy of `ink` without solid bars: any horizontal or vertical run of ink longer than
/// `max_run` pixels (scanner shadows, page edges, frames, rules) is cleared. Text has short
/// runs in both directions, so only borders and rules disappear.
pub fn without_long_runs(ink: &BitImage, max_run: usize) -> BitImage {
    let (w, h) = (ink.w as usize, ink.h as usize);
    let mut out = BitImage::new(ink.w, ink.h);
    out.data.copy_from_slice(&ink.data);
    let clear_runs =
        |out: &mut BitImage, lines: usize, len: usize, at: &dyn Fn(usize, usize) -> usize| {
            for a in 0..lines {
                let mut start = 0;
                while start < len {
                    if ink.data[at(a, start)] == 0 {
                        start += 1;
                        continue;
                    }
                    let mut end = start;
                    while end < len && ink.data[at(a, end)] != 0 {
                        end += 1;
                    }
                    if end - start > max_run {
                        for i in start..end {
                            out.data[at(a, i)] = 0;
                        }
                    }
                    start = end;
                }
            }
        };
    clear_runs(&mut out, h, w, &|y, x| y * w + x);
    clear_runs(&mut out, w, h, &|x, y| y * w + x);
    out
}

/// `true` when text lines run vertically (page lies on its side).
/// Cannot distinguish 90° from 270° or detect 180° (use OCR/OSD for that).
pub fn needs_quarter_turn(ink: &BitImage, dpi: u32) -> bool {
    if ink.count_ink() < 500 {
        return false;
    }
    let cleaned = without_long_runs(ink, (dpi as usize * 3 / 4).max(40));
    let ink = &cleaned;
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
    // Besides beating the rows, the columns must show real line structure: pages that are
    // mostly photograph with a caption have weak band energy on both axes and are left alone
    // (measured on 76 + 498 book pages: upright text pages score ec/er ≤ 0.9, the same pages
    // turned on their side 2.5–17 with ec ≥ 7e-4; a photo page scored 4 with ec 5e-4).
    // Weak evidence (columns only slightly stronger than rows, energy near the floor) is what a
    // picture with vertical ornaments produces: measured on an ornament photo page, ec 7.9e-4,
    // ratio 2.8 (turned wrongly by 90 degrees); real sideways text pages are far from both
    // limits (ratio 33 on a text page turned by 90 degrees). Such borderline pages are left as
    // they are; orientation by OCR (OSD) still sees them.
    let strong = ec > STRONG_BAND_ENERGY || ec > MIN_RATIO_WEAK * er;
    ec > 2.0 * er && ec > MIN_BAND_ENERGY && strong
}

/// Above this column energy the ratio to the rows alone decides.
const STRONG_BAND_ENERGY: f64 = 2e-3;
/// Below it, the columns must beat the rows by at least this factor.
const MIN_RATIO_WEAK: f64 = 4.0;

/// Smallest normalised band energy that counts as text lines.
const MIN_BAND_ENERGY: f64 = 6e-4;

#[cfg(test)]
mod tests {
    use super::*;

    /// Text-like page: dashes in horizontal lines, optionally with solid scanner-edge bars.
    fn page(bars: bool, slope: f32) -> BitImage {
        let (w, h) = (600u32, 800u32);
        let mut b = BitImage::new(w, h);
        for line in 0..28u32 {
            for x0 in (80..520u32).step_by(14) {
                if (x0 / 14 + line) % 5 == 4 {
                    continue; // word gaps
                }
                for dx in 0..9 {
                    let x = x0 + dx;
                    let y = 90 + line * 22 + (slope * (x as f32 - 300.0)) as u32;
                    for dy in 0..5 {
                        b.data[((y + dy) * w + x) as usize] = 1;
                    }
                }
            }
        }
        if bars {
            for y in 0..h {
                for x in (0..45).chain(w - 25..w) {
                    b.data[(y * w + x) as usize] = 1;
                }
            }
        }
        b
    }

    #[test]
    fn solid_scanner_edge_bars_do_not_turn_the_page_on_its_side() {
        assert!(!needs_quarter_turn(&page(false, 0.0), 150));
        assert!(!needs_quarter_turn(&page(true, 0.0), 150));
    }

    #[test]
    fn page_turned_on_its_side_is_still_detected_with_strong_evidence() {
        // text page rotated by 90 degrees: columns carry all the line structure
        let up = page(false, 0.0);
        let (w, h) = (up.w as usize, up.h as usize);
        let mut side = BitImage::new(up.h, up.w);
        for y in 0..h {
            for x in 0..w {
                side.data[x * h + (h - 1 - y)] = up.data[y * w + x];
            }
        }
        assert!(needs_quarter_turn(&side, 150));
    }

    #[test]
    fn long_runs_are_removed_text_is_not() {
        let p = page(true, 0.0);
        let c = without_long_runs(&p, 112);
        assert_eq!(c.count_ink(), page(false, 0.0).count_ink());
    }
}
