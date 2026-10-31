//! Page → text lines: binarisation, recursive X-Y cut on ink projections, line normalisation.
//!
//! Works on pages that were already straightened by the pipeline (small residual skew is
//! tolerated because lines are cut at whitespace valleys of the projection, not at fixed rows).

use image::GrayImage;
use legatorocr_core::{binarize, BitImage, PageImage};

/// A text line: its box on the page (pixels) and the ink mask of that box.
#[derive(Debug, Clone)]
pub struct Line {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn to_ink(page: &PageImage, dpi: u32) -> (GrayImage, BitImage) {
    match page {
        PageImage::Bw(b) => (b.to_gray(), b.clone()),
        _ => {
            let g = page.to_gray();
            let win = ((dpi.max(72) / 6) | 1).clamp(15, 151);
            let ink = binarize::sauvola(&g, win, 0.25);
            (g, ink)
        }
    }
}

/// Median height of the connected components that look like letters (8-connectivity).
pub fn median_component_height(ink: &BitImage) -> u32 {
    let (w, h) = (ink.w as usize, ink.h as usize);
    if w == 0 || h == 0 {
        return 0;
    }
    let mut parent: Vec<u32> = Vec::new();
    let mut lab = vec![0u32; w * h];
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    parent.push(0);
    for y in 0..h {
        for x in 0..w {
            if ink.data[y * w + x] == 0 {
                continue;
            }
            let mut m;
            let mut nb = [0u32; 4];
            let mut n = 0;
            if x > 0 && lab[y * w + x - 1] != 0 {
                nb[n] = lab[y * w + x - 1];
                n += 1;
            }
            if y > 0 {
                for dx in [-1i64, 0, 1] {
                    let xx = x as i64 + dx;
                    if xx >= 0 && (xx as usize) < w && lab[(y - 1) * w + xx as usize] != 0 {
                        nb[n] = lab[(y - 1) * w + xx as usize];
                        n += 1;
                    }
                }
            }
            if n == 0 {
                m = parent.len() as u32;
                parent.push(m);
            } else {
                let r0 = find(&mut parent, nb[0]);
                m = r0;
                for &o in &nb[1..n] {
                    let r = find(&mut parent, o);
                    if r != m {
                        let (a, b) = (m.min(r), m.max(r));
                        parent[b as usize] = a;
                        m = a;
                    }
                }
            }
            lab[y * w + x] = m;
        }
    }
    // bounding boxes per root
    let nlab = parent.len();
    let mut y0 = vec![u32::MAX; nlab];
    let mut y1 = vec![0u32; nlab];
    for y in 0..h {
        for x in 0..w {
            let l = lab[y * w + x];
            if l != 0 {
                let r = find(&mut parent, l) as usize;
                y0[r] = y0[r].min(y as u32);
                y1[r] = y1[r].max(y as u32 + 1);
            }
        }
    }
    let mut hs: Vec<u32> = (1..nlab)
        .filter(|&i| parent[i] as usize == i && y1[i] > y0[i])
        .map(|i| y1[i] - y0[i])
        .filter(|&v| v >= 4 && (v as usize) < h / 4)
        .collect();
    if hs.is_empty() {
        return 0;
    }
    hs.sort_unstable();
    hs[hs.len() / 2]
}

struct Prof<'a> {
    ink: &'a BitImage,
}

impl Prof<'_> {
    fn rows(&self, r: (u32, u32, u32, u32)) -> Vec<u32> {
        let (x0, y0, x1, y1) = r;
        (y0..y1)
            .map(|y| {
                let s = y as usize * self.ink.w as usize;
                self.ink.data[s + x0 as usize..s + x1 as usize].iter().map(|&b| b as u32).sum()
            })
            .collect()
    }
    fn cols(&self, r: (u32, u32, u32, u32)) -> Vec<u32> {
        let (x0, y0, x1, y1) = r;
        let mut v = vec![0u32; (x1 - x0) as usize];
        for y in y0..y1 {
            let s = y as usize * self.ink.w as usize;
            for (i, &b) in self.ink.data[s + x0 as usize..s + x1 as usize].iter().enumerate() {
                v[i] += b as u32;
            }
        }
        v
    }
    /// Shrink a region to its ink bounding box.
    fn trim(&self, r: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
        let rows = self.rows(r);
        let cols = self.cols(r);
        let a = rows.iter().position(|&v| v > 0)?;
        let b = rows.iter().rposition(|&v| v > 0)?;
        let c = cols.iter().position(|&v| v > 0)?;
        let d = cols.iter().rposition(|&v| v > 0)?;
        Some((r.0 + c as u32, r.1 + a as u32, r.0 + d as u32 + 1, r.1 + b as u32 + 1))
    }
}

/// Longest run of "empty" entries (value ≤ `noise`) strictly inside `v`, as (start, len).
fn widest_gap(v: &[u32], noise: u32) -> Option<(usize, usize)> {
    let (mut best, mut start, mut len) = (None::<(usize, usize)>, 0usize, 0usize);
    let first = v.iter().position(|&x| x > noise)?;
    let last = v.iter().rposition(|&x| x > noise)?;
    for i in first..=last {
        if v[i] <= noise {
            if len == 0 {
                start = i;
            }
            len += 1;
        } else {
            if len > 0 && best.is_none_or(|(_, l)| len > l) {
                best = Some((start, len));
            }
            len = 0;
        }
    }
    best
}

/// Find the text lines of a page in reading order.
pub fn find_lines(ink: &BitImage, med_h: u32) -> Vec<Line> {
    let med = med_h.max(6);
    let p = Prof { ink };
    let mut out = Vec::new();
    let root = (0, 0, ink.w, ink.h);
    cut(&p, root, med, 0, &mut out);
    out
}

fn cut(p: &Prof, r: (u32, u32, u32, u32), med: u32, depth: u32, out: &mut Vec<Line>) {
    let Some(r) = p.trim(r) else { return };
    let (x0, y0, x1, y1) = r;
    let (w, h) = (x1 - x0, y1 - y0);
    // ignore specks
    if w < 3 || h < 3 {
        return;
    }
    if depth > 40 || h <= med * 2 + med / 2 && w <= med * 3 {
        if h >= med / 3 {
            out.push(Line { x: x0, y: y0, w, h });
        }
        return;
    }
    let rows = p.rows(r);
    let cols = p.cols(r);
    // a gutter must be clearly wider than a word gap; line gaps clearly thinner than letters
    let vg = widest_gap(&cols, 0).filter(|&(_, l)| l as u32 * 10 >= med * 18);
    let hg = widest_gap(&rows, 0).filter(|&(_, l)| l as u32 >= 1);
    // very tall regions without any row gap are not text lines (pictures/stamps): keep as one
    match (hg, vg) {
        (Some((s, _)), None) => split_h(p, r, s, med, depth, out),
        (None, Some((s, l))) => split_v(p, r, s, l, med, depth, out),
        (Some((hs, hl)), Some((vs, vl))) => {
            // prefer the cut that separates more (relative to the letter size)
            if vl as u32 * 3 > hl as u32 * 8 && h > med * 3 {
                split_v(p, r, vs, vl, med, depth, out);
            } else {
                split_h(p, r, hs, med, depth, out);
            }
        }
        (None, None) => {
            if h >= med / 3 && h <= med * 8 {
                out.push(Line { x: x0, y: y0, w, h });
            } else if h > med * 8 {
                // a block with no whitespace row (picture or heavy skew): try the weakest row
                if let Some(c) = weakest_row(&rows, med) {
                    let ys = y0 + c as u32;
                    cut(p, (x0, y0, x1, ys), med, depth + 1, out);
                    cut(p, (x0, ys, x1, y1), med, depth + 1, out);
                }
            }
        }
    }
}

fn split_h(p: &Prof, r: (u32, u32, u32, u32), gap_start: usize, med: u32, depth: u32, out: &mut Vec<Line>) {
    // cut at the middle of the gap found; the gap run itself carries no ink
    let ys = r.1 + gap_start as u32;
    cut(p, (r.0, r.1, r.2, ys), med, depth + 1, out);
    // skip empty rows to the next ink
    let rows = p.rows((r.0, ys, r.2, r.3));
    let skip = rows.iter().position(|&v| v > 0).unwrap_or(rows.len()) as u32;
    cut(p, (r.0, ys + skip, r.2, r.3), med, depth + 1, out);
}

fn split_v(p: &Prof, r: (u32, u32, u32, u32), s: usize, l: usize, med: u32, depth: u32, out: &mut Vec<Line>) {
    let xs = r.0 + s as u32;
    cut(p, (r.0, r.1, xs, r.3), med, depth + 1, out);
    cut(p, (xs + l as u32, r.1, r.2, r.3), med, depth + 1, out);
}

/// Row with the least ink within the middle band of a tall block, if it is a real valley.
fn weakest_row(rows: &[u32], med: u32) -> Option<usize> {
    let n = rows.len();
    let lo = (med as usize * 2).min(n / 2);
    let hi = n.saturating_sub(lo).max(lo + 1);
    let (i, &v) = rows[lo..hi].iter().enumerate().min_by_key(|&(_, v)| *v)?;
    let mean = rows.iter().sum::<u32>() as f32 / n as f32;
    if (v as f32) < mean * 0.35 {
        Some(lo + i)
    } else {
        None
    }
}

/// A line cut out of the page as the recogniser's input: `LINE_H` rows, ink = 1.0, plus the scale
/// that maps recogniser pixels back to page pixels.
pub struct LineImage {
    pub data: Vec<f32>,
    pub w: usize,
    pub scale: f32,
    pub x: u32,
    pub y: u32,
    pub h: u32,
}

pub fn extract(gray: &GrayImage, line: &Line, line_h: usize) -> Option<LineImage> {
    // pad like the training data does (a fraction of the line height on every side)
    let pad_y = (line.h as f32 * 0.12).round() as i64;
    let pad_x = (line.h as f32 * 0.3).round() as i64;
    let x0 = (line.x as i64 - pad_x).max(0) as u32;
    let y0 = (line.y as i64 - pad_y).max(0) as u32;
    let x1 = ((line.x + line.w) as i64 + pad_x).min(gray.width() as i64) as u32;
    let y1 = ((line.y + line.h) as i64 + pad_y).min(gray.height() as i64) as u32;
    if x1 <= x0 + 2 || y1 <= y0 + 2 {
        return None;
    }
    let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
    // contrast: paper = high percentile, ink = low percentile
    let mut hist = [0u32; 256];
    for y in y0..y1 {
        for x in x0..x1 {
            hist[gray.get_pixel(x, y).0[0] as usize] += 1;
        }
    }
    let total = (cw * ch) as u32;
    let pct = |q: f32| -> f32 {
        let target = (total as f32 * q) as u32;
        let mut acc = 0;
        for (v, &c) in hist.iter().enumerate() {
            acc += c;
            if acc > target {
                return v as f32;
            }
        }
        255.0
    };
    let (ink_v, paper_v) = (pct(0.02), pct(0.85));
    let range = (paper_v - ink_v).max(40.0);
    let scale = ch as f32 / line_h as f32;
    let nw = ((cw as f32 / scale).round() as usize).max(8);
    let mut data = vec![0.0f32; line_h * nw];
    // area-average resampling (box filter over the source footprint) keeps thin strokes
    for oy in 0..line_h {
        let sy0 = oy as f32 * scale;
        let sy1 = sy0 + scale;
        for ox in 0..nw {
            let sx0 = ox as f32 * scale;
            let sx1 = (sx0 + scale).min(cw as f32);
            let (mut acc, mut wsum) = (0.0f32, 0.0f32);
            let mut yy = sy0.floor() as usize;
            while (yy as f32) < sy1 && yy < ch {
                let wy = (sy1.min(yy as f32 + 1.0) - sy0.max(yy as f32)).max(0.0);
                let mut xx = sx0.floor() as usize;
                while (xx as f32) < sx1 && xx < cw {
                    let wx = (sx1.min(xx as f32 + 1.0) - sx0.max(xx as f32)).max(0.0);
                    let v = gray.get_pixel(x0 + xx as u32, y0 + yy as u32).0[0] as f32;
                    acc += v * wx * wy;
                    wsum += wx * wy;
                    xx += 1;
                }
                yy += 1;
            }
            let v = if wsum > 0.0 { acc / wsum } else { paper_v };
            data[oy * nw + ox] = ((paper_v - v) / range).clamp(0.0, 1.0);
        }
    }
    Some(LineImage { data, w: nw, scale, x: x0, y: y0, h: (y1 - y0) })
}
