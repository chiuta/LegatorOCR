//! Detection of two-page spreads and of the gutter between the pages.

use crate::BitImage;

/// Returns the x coordinate at which to cut a two-page spread, or `None`
/// when the image looks like a single page.
pub fn find_gutter(ink: &BitImage) -> Option<u32> {
    let (w, h) = (ink.w as usize, ink.h as usize);
    if (w as f32) < 1.15 * h as f32 {
        return None; // portrait or near-square: single page
    }
    let mut col = vec![0f32; w];
    for y in 0..h {
        for x in 0..w {
            col[x] += ink.data[y * w + x] as f32;
        }
    }
    let r = (w / 150).max(2);
    let smooth: Vec<f32> = (0..w)
        .map(|x| {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            col[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect();
    let (lo, hi) = (w * 35 / 100, w * 65 / 100);
    let (mut bx, mut bv) = (lo, f32::MAX);
    for x in lo..hi {
        if smooth[x] < bv {
            bv = smooth[x];
            bx = x;
        }
    }
    // Typical ink density of the two halves' content areas.
    let mut side: Vec<f32> = smooth[w / 12..w * 35 / 100]
        .iter()
        .chain(smooth[w * 65 / 100..w * 11 / 12].iter())
        .copied()
        .filter(|&v| v > 0.0)
        .collect();
    if side.len() < 10 {
        return None;
    }
    side.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = side[side.len() / 2];
    if bv > 0.25 * med {
        return None;
    }
    // Centre the cut in the valley (columns almost as empty as the minimum).
    let thr = bv + 0.1 * med;
    let (mut a, mut b) = (bx, bx);
    while a > lo && smooth[a - 1] <= thr {
        a -= 1;
    }
    while b + 1 < hi && smooth[b + 1] <= thr {
        b += 1;
    }
    let cut = ((a + b) / 2) as u32;
    // Both sides must hold content.
    let left: f32 = col[..cut as usize].iter().sum();
    let right: f32 = col[cut as usize..].iter().sum();
    let tot = left + right;
    if tot <= 0.0 || left / tot < 0.15 || right / tot < 0.15 {
        return None;
    }
    Some(cut)
}
