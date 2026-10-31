//! Automatic picture-zone detection for mixed pages (text + photographs/halftones).
//!
//! Idea: printed text is *bimodal* — almost every pixel is paper or solid ink and
//! the grey in between lives only on glyph edges. A photograph or halftone is
//! continuous-tone: a large share of its pixels are mid-grey. So tile the page,
//! measure each tile's share of mid-tones against the local paper level, close the
//! result morphologically and keep only blobs big enough to be a picture.

use crate::Rect;
use image::GrayImage;

/// One detected picture zone (full-resolution pixel coordinates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub rect: Rect,
}

/// Find picture zones. `sensitivity` 0–1 (higher = more areas count as pictures).
pub fn detect(gray: &GrayImage, dpi: u32, sensitivity: f32) -> Vec<Zone> {
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    // Analyse on a copy of at most ~150 dpi.
    let f = (dpi / 150).max(1) as usize;
    let small = crate::transform::downscale_gray(gray, f as u32);
    let (sw, sh) = (small.width() as usize, small.height() as usize);
    let eff = dpi as f32 / f as f32;
    let tile = ((eff / 12.0).round() as usize).clamp(6, 24); // ≈ 1/12 inch
    let (gw, gh) = (sw.div_ceil(tile), sh.div_ceil(tile));
    if gw < 3 || gh < 3 {
        return Vec::new();
    }
    // Paper level: bright percentile of the whole page (robust to a dark scanner bed).
    let mut hist = [0u64; 256];
    for &p in small.as_raw() {
        hist[p as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let (mut acc, mut paper) = (0u64, 255usize);
    for v in (0..256).rev() {
        acc += hist[v];
        if acc * 5 >= total {
            paper = v;
            break;
        }
    }
    let paper = paper.max(60) as f32;
    let sens = sensitivity.clamp(0.0, 1.0);
    let mid_needed = 0.50 - 0.25 * sens; // share of mid-tone pixels that marks a picture tile
    let raw = small.as_raw();
    let mut pic = vec![false; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let (x0, y0) = (gx * tile, gy * tile);
            let (x1, y1) = ((x0 + tile).min(sw), (y0 + tile).min(sh));
            let (mut mid, mut n, mut paperish, mut dark) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for &p in &raw[y * sw + x0..y * sw + x1] {
                    let r = p as f32 / paper;
                    if r > 0.88 {
                        paperish += 1;
                    } else if r > 0.18 {
                        mid += 1;
                    } else {
                        dark += 1;
                    }
                    n += 1;
                }
            }
            let n = n.max(1) as f32;
            // Mostly mid-tone, or dark-and-grainy with little paper: a picture tile.
            // (A solid dark tile — the black part of a photo — also counts; the blob-size
            // test below keeps rules and bold headings from becoming pictures.)
            pic[gy * gw + gx] = (mid as f32 / n >= mid_needed && (paperish as f32 / n) < 0.55) || dark as f32 / n >= 0.7;
        }
    }
    // Close gaps (radius 1 tile) so a picture with bright patches stays one blob.
    let dil = |m: &[bool], r: usize| -> Vec<bool> {
        let mut o = vec![false; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                if m[gy * gw + gx] {
                    for yy in gy.saturating_sub(r)..(gy + r + 1).min(gh) {
                        for xx in gx.saturating_sub(r)..(gx + r + 1).min(gw) {
                            o[yy * gw + xx] = true;
                        }
                    }
                }
            }
        }
        o
    };
    let ero = |m: &[bool], r: usize| -> Vec<bool> {
        let inv: Vec<bool> = m.iter().map(|&b| !b).collect();
        dil(&inv, r).iter().map(|&b| !b).collect()
    };
    let closed = ero(&dil(&pic, 1), 1);
    // Connected components (4-neighbour) → bounding boxes.
    let mut label = vec![false; gw * gh];
    let mut zones = Vec::new();
    // A picture is at least ≈ 0.5 × 0.5 inch.
    let min_tiles = ((eff * 0.5 / tile as f32).ceil() as usize).max(2);
    for s in 0..gw * gh {
        if !closed[s] || label[s] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (usize::MAX, usize::MAX, 0usize, 0usize, 0usize);
        let mut st = vec![s];
        label[s] = true;
        while let Some(p) = st.pop() {
            let (px, py) = (p % gw, p / gw);
            count += 1;
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px);
            y1 = y1.max(py);
            let mut push = |q: usize| {
                if closed[q] && !label[q] {
                    label[q] = true;
                    st.push(q);
                }
            };
            if px > 0 { push(p - 1) }
            if px + 1 < gw { push(p + 1) }
            if py > 0 { push(p - gw) }
            if py + 1 < gh { push(p + gw) }
        }
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        // Big enough, reasonably filled (not a thin frame/rule), not a whole-page smear.
        if bw >= min_tiles && bh >= min_tiles && count * 100 >= bw * bh * 45 {
            let scale = f * tile;
            let rx0 = (x0 * scale).min(w);
            let ry0 = (y0 * scale).min(h);
            let rx1 = ((x1 + 1) * scale).min(w);
            let ry1 = ((y1 + 1) * scale).min(h);
            if rx1 > rx0 && ry1 > ry0 {
                zones.push(Zone { rect: Rect { x: rx0 as u32, y: ry0 as u32, w: (rx1 - rx0) as u32, h: (ry1 - ry0) as u32 } });
            }
        }
    }
    zones
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    /// Text lines on paper with one smooth "photo" block.
    fn page() -> (GrayImage, Rect) {
        let (w, h) = (900u32, 1200u32);
        let photo = Rect { x: 450, y: 300, w: 330, h: 380 };
        let mut g = GrayImage::from_pixel(w, h, Luma([236]));
        for y in (100..1100).step_by(34) {
            for x in 100..800u32 {
                if (x / 14) % 2 == 0 && (y + x / 7) % 34 < 12 && !(photo.y - 10..photo.y + photo.h + 10).contains(&y) {
                    g.put_pixel(x, y, Luma([30]));
                    g.put_pixel(x, y + 1, Luma([30]));
                }
            }
        }
        for y in photo.y..photo.y + photo.h {
            for x in photo.x..photo.x + photo.w {
                let v = 40.0 + 150.0 * (((x - photo.x) as f32 / 40.0).sin() * 0.5 + 0.5) * ((y - photo.y) as f32 / photo.h as f32);
                g.put_pixel(x, y, Luma([v as u8]));
            }
        }
        (g, photo)
    }

    #[test]
    fn finds_the_photo_and_not_the_text() {
        let (g, photo) = page();
        let z = detect(&g, 150, 0.5);
        assert_eq!(z.len(), 1, "{z:?}");
        let r = z[0].rect;
        let (ix0, iy0) = (r.x.max(photo.x), r.y.max(photo.y));
        let (ix1, iy1) = ((r.x + r.w).min(photo.x + photo.w), (r.y + r.h).min(photo.y + photo.h));
        let inter = ((ix1.saturating_sub(ix0)) * (iy1.saturating_sub(iy0))) as f32;
        assert!(inter / (photo.w * photo.h) as f32 > 0.85, "photo coverage {r:?}");
        assert!(r.w < photo.w + 120 && r.h < photo.h + 120, "zone too large: {r:?}");
    }

    #[test]
    fn plain_text_page_has_no_zones() {
        let (mut g, photo) = page();
        for y in photo.y..photo.y + photo.h {
            for x in photo.x..photo.x + photo.w {
                g.put_pixel(x, y, Luma([236]));
            }
        }
        assert!(detect(&g, 150, 0.5).is_empty());
        assert!(detect(&GrayImage::from_pixel(10, 10, Luma([200])), 300, 0.5).is_empty());
    }

    #[test]
    fn mostly_black_photo_is_still_a_picture() {
        // A dark photo: solid black with a few bright specks, under lines of text.
        let (w, h) = (900u32, 1200u32);
        let mut g = GrayImage::from_pixel(w, h, image::Luma([235]));
        for y in 500..800 {
            for x in 200..700 {
                g.put_pixel(x, y, image::Luma([if (x * 31 + y * 17) % 97 == 0 { 200 } else { 8 }]));
            }
        }
        for line in 0..8 {
            for x in (100..800).step_by(17) {
                for yy in 0..10 {
                    for xx in 0..9 {
                        g.put_pixel(x + xx, 100 + line * 40 + yy, image::Luma([30]));
                    }
                }
            }
        }
        let zones = detect(&g, 150, 0.5);
        assert_eq!(zones.len(), 1, "{zones:?}");
        let r = zones[0].rect;
        assert!(r.x <= 260 && r.y <= 560 && r.x + r.w >= 640 && r.y + r.h >= 740, "{r:?}");
        assert!(r.y > 300, "text lines must not be swallowed: {r:?}");
    }
}
