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
    side.sort_by(|a, b| a.total_cmp(b));
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
    if tot <= 0.0 {
        return None;
    }
    if left / tot < 0.15 || right / tot < 0.15 {
        // One half is (almost) blank — a chapter end, a verso. Still a spread when the
        // other half holds a page-sized body and the sparse half holds only a small mark.
        let extent = |c: &[f32]| -> usize {
            match (
                c.iter().position(|&v| v > 0.0),
                c.iter().rposition(|&v| v > 0.0),
            ) {
                (Some(a), Some(b)) => b - a + 1,
                _ => 0,
            }
        };
        let (ext_l, ext_r) = (extent(&col[..cut as usize]), extent(&col[cut as usize..]));
        let (big, small) = if left > right {
            (ext_l, ext_r)
        } else {
            (ext_r, ext_l)
        };
        if big * 100 < w * 30 || small * 100 > w * 20 {
            return None;
        }
    }
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(img: &mut BitImage, x0: u32, x1: u32) {
        for y in (200..1500).step_by(30) {
            for x in x0..x1 {
                if (x / 25) % 2 == 0 {
                    for yy in y..y + 12 {
                        img.set(x, yy, true);
                    }
                }
            }
        }
    }

    #[test]
    fn spread_with_empty_right_half_is_still_split() {
        let mut img = BitImage::new(2400, 1700);
        body(&mut img, 150, 1000);
        for y in 300..330 {
            for x in 1500..1650 {
                img.set(x, y, true);
            }
        }
        let cut = find_gutter(&img).expect("split");
        assert!((1000..1500).contains(&cut), "{cut}");
    }

    #[test]
    fn wide_single_page_is_not_split() {
        let mut img = BitImage::new(2400, 1700);
        body(&mut img, 150, 2250);
        assert_eq!(find_gutter(&img), None);
    }

    #[test]
    fn normal_spread_is_split_in_the_middle() {
        let mut img = BitImage::new(2400, 1700);
        body(&mut img, 150, 1100);
        body(&mut img, 1300, 2250);
        let cut = find_gutter(&img).unwrap();
        assert!((1100..1300).contains(&cut));
    }
}
