//! Content-box detection: cluster ink into blocks, merge neighbours, drop
//! specks and scanner-edge shadows, return the bounding box of what is left.

use crate::{BitImage, Rect};

pub fn detect_content(ink: &BitImage, dpi: u32) -> Option<Rect> {
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
    struct Comp { blocks: Vec<usize>, mass: u64, x0: usize, y0: usize, x1: usize, y1: usize }
    let mut comps: Vec<Comp> = Vec::new();
    for s in 0..gw * gh {
        if !dil[s] || label[s] != u32::MAX {
            continue;
        }
        let id = comps.len() as u32;
        let mut c = Comp { blocks: vec![], mass: 0, x0: usize::MAX, y0: usize::MAX, x1: 0, y1: 0 };
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
