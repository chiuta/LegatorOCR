//! Content-box detection: cluster ink into blocks, merge neighbours, drop
//! specks and scanner-edge shadows, return the bounding box of what is left.

use crate::{BitImage, Rect};

/// `ink` with long straight runs removed, but only in the outer tenth of the image.
fn edge_cleaned(ink: &BitImage, dpi: u32) -> BitImage {
    let long = crate::orient::without_long_runs(ink, (dpi as usize * 3 / 4).max(40));
    let (w, h) = (ink.w as usize, ink.h as usize);
    let (mx, my) = (w / 10, h / 10);
    let mut out = ink.clone();
    for y in 0..h {
        for x in 0..w {
            if x < mx || x + mx >= w || y < my || y + my >= h {
                out.data[y * w + x] = long.data[y * w + x];
            }
        }
    }
    out
}

pub fn detect_content(ink: &BitImage, dpi: u32) -> Option<Rect> {
    // Scanner-edge bars sit a few pixels from the text and would otherwise join
    // it into one component that is then rejected as a shadow, taking text along.
    // Only within a tenth of the page width/height from the border: a large dark photo in the
    // middle of the page has long runs too and must stay part of the content.
    let cleaned = edge_cleaned(ink, dpi);
    let ink = &cleaned;
    let b = (dpi / 50).max(4) as usize; // block edge in px
    let (w, h) = (ink.w as usize, ink.h as usize);
    let (gw, gh) = (w.div_ceil(b), h.div_ceil(b));
    let mut mass = vec![0u32; gw * gh];
    for y in 0..h {
        for x in 0..w {
            if ink.data[y * w + x] != 0 {
                mass[(y / b) * gw + x / b] += 1;
            }
        }
    }
    let total: u64 = mass.iter().map(|&m| m as u64).sum();
    if total == 0 {
        return None;
    }
    let min_block = 2u32;
    let active: Vec<bool> = mass.iter().map(|&m| m >= min_block).collect();
    // Dilate by `r` blocks so words/lines of one text body join up.
    let r = ((dpi as usize) / 100).clamp(2, 6);
    let mut dil = vec![false; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            if !active[gy * gw + gx] {
                continue;
            }
            for yy in gy.saturating_sub(r)..(gy + r + 1).min(gh) {
                for xx in gx.saturating_sub(r)..(gx + r + 1).min(gw) {
                    dil[yy * gw + xx] = true;
                }
            }
        }
    }
    // Label 8-connected components of the dilated grid.
    let mut label = vec![u32::MAX; gw * gh];
    struct Comp {
        blocks: Vec<usize>,
        mass: u64,
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
    }
    let mut comps: Vec<Comp> = Vec::new();
    for s in 0..gw * gh {
        if !dil[s] || label[s] != u32::MAX {
            continue;
        }
        let id = comps.len() as u32;
        let mut c = Comp {
            blocks: vec![],
            mass: 0,
            x0: usize::MAX,
            y0: usize::MAX,
            x1: 0,
            y1: 0,
        };
        let mut st = vec![s];
        label[s] = id;
        while let Some(p) = st.pop() {
            let (px, py) = (p % gw, p / gw);
            if active[p] {
                c.blocks.push(p);
                c.mass += mass[p] as u64;
            }
            c.x0 = c.x0.min(px);
            c.y0 = c.y0.min(py);
            c.x1 = c.x1.max(px);
            c.y1 = c.y1.max(py);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (px as i64 + dx, py as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let q = ny as usize * gw + nx as usize;
                    if dil[q] && label[q] == u32::MAX {
                        label[q] = id;
                        st.push(q);
                    }
                }
            }
        }
        comps.push(c);
    }
    // Absolute size threshold (≈ 60 ink px at 300 dpi): keeps page numbers and
    // footnote marks, rejects dust. A threshold relative to total ink would
    // wrongly drop small-but-real content on text-heavy pages.
    let scale = dpi as f64 / 300.0;
    let min_mass = ((scale * scale * 60.0) as u64).max(20);
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    for c in &comps {
        if c.mass < min_mass || c.blocks.is_empty() {
            continue;
        }
        // Scanner-edge shadow: touches the border and is a long thin strip.
        let (cw, ch) = (c.x1 - c.x0 + 1, c.y1 - c.y0 + 1);
        let touches = c.x0 == 0 || c.y0 == 0 || c.x1 == gw - 1 || c.y1 == gh - 1;
        if touches && (cw * 8 < ch || ch * 8 < cw) {
            continue;
        }
        // Dark frame or L-shaped corner shadow (strips along two edges): its
        // bounding box is large but almost empty. Real content that reaches
        // the border is filled much more densely.
        if touches && (c.blocks.len() as f64) < 0.12 * (cw * ch) as f64 {
            continue;
        }
        // After deskewing, the scanner-bed wedge no longer touches the image border:
        // it sits a few pixels inside as a long, solid, very thin strip hugging an
        // edge. Text lines are as thin but mostly gaps; the strip is mostly ink.
        let near = c.x0 < gw / 12 || c.y0 < gh / 12 || c.x1 + gw / 12 >= gw || c.y1 + gh / 12 >= gh;
        let long = (cw * 5 >= gw * 3 && ch * 20 < cw) || (ch * 5 >= gh * 3 && cw * 20 < ch);
        if near && long && c.mass as f64 > 0.3 * (cw * ch * b * b) as f64 {
            continue;
        }
        // Small speck cut by the image border: the tip of a shadow wedge.
        if touches && cw * 25 < gw && ch * 25 < gh {
            continue;
        }
        // The four wedges around a deskewed page form an almost empty frame.
        if near
            && cw * 20 >= gw * 17
            && ch * 20 >= gh * 17
            && (c.blocks.len() as f64) < 0.12 * (cw * ch) as f64
        {
            continue;
        }
        for &p in &c.blocks {
            let (bx, by) = ((p % gw) * b, (p / gw) * b);
            // Tighten to real ink inside the block.
            for y in by..(by + b).min(h) {
                for x in bx..(bx + b).min(w) {
                    if ink.data[y * w + x] != 0 {
                        bbox = Some(match bbox {
                            None => (x, y, x, y),
                            Some((a, bb, cc, d)) => (a.min(x), bb.min(y), cc.max(x), d.max(y)),
                        });
                    }
                }
            }
        }
    }
    bbox.map(|(x0, y0, x1, y1)| Rect {
        x: x0 as u32,
        y: y0 as u32,
        w: (x1 - x0 + 1) as u32,
        h: (y1 - y0 + 1) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_block(img: &mut BitImage, x0: u32, y0: u32, w: u32, h: u32) {
        let mut y = y0;
        while y + 14 < y0 + h {
            for x in (x0..x0 + w).step_by(40) {
                for yy in y..y + 14 {
                    for xx in x..x + 28 {
                        img.set(xx, yy, true);
                    }
                }
            }
            y += 36;
        }
    }

    #[test]
    fn dark_corner_frame_is_not_content() {
        let mut img = BitImage::new(1200, 1600);
        text_block(&mut img, 300, 400, 600, 800);
        // L-shaped scanner shadow along the left and bottom edges.
        for y in 0..1600 {
            for x in 0..30 {
                img.set(x, y, true);
            }
        }
        for y in 1560..1600 {
            for x in 0..1200 {
                img.set(x, y, true);
            }
        }
        let r = detect_content(&img, 300).expect("text found");
        assert!(
            r.x >= 290 && r.y >= 390 && r.x + r.w <= 910 && r.y + r.h <= 1210,
            "{r:?}"
        );
    }

    #[test]
    fn dense_text_touching_the_border_is_kept() {
        let mut img = BitImage::new(1200, 1600);
        text_block(&mut img, 0, 0, 1200, 1600);
        let r = detect_content(&img, 300).unwrap();
        assert!(r.w > 1100 && r.h > 1500, "{r:?}");
    }
}

#[cfg(test)]
mod edge_bar_tests {
    use super::*;

    #[test]
    fn text_next_to_a_scanner_edge_bar_is_not_dropped_with_it() {
        let (w, h) = (600usize, 800usize);
        let mut ink = BitImage::new(w as u32, h as u32);
        for y in 0..h {
            for x in 0..30 {
                ink.data[y * w + x] = 1; // solid bar on the left edge
            }
        }
        for line in 0..10 {
            let y0 = 100 + line * 40;
            for y in y0..y0 + 14 {
                for x in (45..400).step_by(3) {
                    ink.data[y * w + x] = 1;
                    ink.data[y * w + x + 1] = 1;
                }
            }
        }
        let r = detect_content(&ink, 300).expect("content");
        assert!(r.y <= 105 && r.y + r.h >= 470, "text lost: {r:?}");
        assert!(r.x >= 30, "bar included: {r:?}");
    }

    #[test]
    fn a_big_dark_picture_in_the_middle_stays_in_the_content_box() {
        let (w, h) = (1200usize, 1600usize);
        let mut ink = BitImage::new(w as u32, h as u32);
        for y in 500..1100 {
            for x in 300..900 {
                ink.data[y * w + x] = 1; // 600×600 px solid block at 300 dpi: runs of 2 inches
            }
        }
        let r = detect_content(&ink, 300).expect("content");
        assert!(
            r.x <= 300 && r.y <= 500 && r.x + r.w >= 900 && r.y + r.h >= 1100,
            "{r:?}"
        );
    }
}
