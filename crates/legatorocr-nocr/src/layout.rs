//! Page → text lines: binarisation (with polarity and ruling removal), recursive X-Y cut on ink
//! projections, long-line splitting and line normalisation.
//!
//! Works on pages that were already straightened by the pipeline (small residual skew is
//! tolerated because lines are cut at whitespace valleys of the projection, not at fixed rows).

use image::GrayImage;
use legatorocr_core::{binarize, BitImage, PageImage};

/// A text line: its box on the page (pixels).
#[derive(Debug, Clone)]
pub struct Line {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The page as the engine sees it: grey image (dark text on light paper), ink mask, letter height.
pub struct Prepared {
    pub gray: GrayImage,
    pub ink: BitImage,
    /// Median height of letter-like components (0 = nothing found).
    pub med: u32,
}

pub fn to_ink(page: &PageImage, dpi: u32) -> (GrayImage, BitImage) {
    match page {
        PageImage::Bw(b) => (b.to_gray(), b.clone()),
        _ => {
            let g = page.to_gray();
            let ink = threshold_ink(&g, dpi);
            (g, ink)
        }
    }
}

fn threshold_ink(g: &GrayImage, dpi: u32) -> BitImage {
    let smooth = median3(g);
    let win = ((dpi.clamp(72, 1200) / 6) | 1).clamp(15, 151);
    binarize::sauvola(&smooth, win, 0.25)
}

/// Binarise, fix polarity (white-on-black pages are inverted), measure the letter height and
/// strip ruling lines (underlines, table borders) that would glue lines together.
pub fn prepare(page: &PageImage, dpi: u32) -> Prepared {
    let (mut gray, mut ink) = to_ink(page, dpi);
    let total = (ink.w as u64 * ink.h as u64).max(1);
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let mut acc = 0u64;
    let median = hist
        .iter()
        .position(|&c| {
            acc += c;
            acc * 2 >= total
        })
        .unwrap_or(255);
    // dark paper (median grey below ~40 %) or more "ink" than paper: light text on a dark page
    if median < 100 || ink.count_ink() * 100 / total > 50 {
        // more "ink" than paper: the page is light-on-dark
        for p in gray.pixels_mut() {
            p.0[0] = 255 - p.0[0];
        }
        ink = threshold_ink(&gray, dpi);
    }
    let med = median_component_height(&ink);
    if med >= 6 {
        remove_rules(&mut ink, &mut gray, med);
    }
    Prepared { gray, ink, med }
}

/// 3×3 median filter: removes single-pixel scanner noise before thresholding, keeps edges.
fn median3(g: &GrayImage) -> GrayImage {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let src = g.as_raw();
    let mut out = src.clone();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let mut v = [0u8; 9];
            let mut k = 0;
            for dy in 0..3 {
                for dx in 0..3 {
                    v[k] = src[(y + dy - 1) * w + x + dx - 1];
                    k += 1;
                }
            }
            v.select_nth_unstable(4);
            out[y * w + x] = v[4];
        }
    }
    GrayImage::from_raw(g.width(), g.height(), out).expect("same size")
}

fn find(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        p[x as usize] = p[p[x as usize] as usize];
        x = p[x as usize];
    }
    x
}

/// Height of a typical letter: the median over components that are not dust, dots or pictures.
/// Run-length connected components (8-connectivity), so memory follows the number of ink runs
/// rather than the number of pixels.
pub fn median_component_height(ink: &BitImage) -> u32 {
    let (w, h) = (ink.w as usize, ink.h as usize);
    if w == 0 || h == 0 {
        return 0;
    }
    let mut parent: Vec<u32> = Vec::new();
    let mut ymin: Vec<u32> = Vec::new();
    let mut ymax: Vec<u32> = Vec::new();
    let mut prev: Vec<(u32, u32, u32)> = Vec::new(); // (x0, x1 exclusive, run id)
    let mut cur: Vec<(u32, u32, u32)> = Vec::new();
    const MAX_RUNS: usize = 24_000_000;
    for y in 0..h {
        cur.clear();
        let row = &ink.data[y * w..(y + 1) * w];
        let mut x = 0;
        while x < w {
            if row[x] == 0 {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && row[x] != 0 {
                x += 1;
            }
            if parent.len() >= MAX_RUNS {
                continue; // pathological noise: stop adding components
            }
            let id = parent.len() as u32;
            parent.push(id);
            ymin.push(y as u32);
            ymax.push(y as u32 + 1);
            cur.push((x0 as u32, x as u32, id));
        }
        // merge with overlapping runs of the previous row (sorted two-pointer sweep)
        let mut j = 0;
        for &(a, b, id) in &cur {
            while j < prev.len() && prev[j].1 < a {
                j += 1;
            }
            let mut k = j;
            while k < prev.len() && prev[k].0 <= b {
                let (ra, rb) = (find(&mut parent, id), find(&mut parent, prev[k].2));
                if ra != rb {
                    let (lo, hi) = (ra.min(rb), ra.max(rb));
                    parent[hi as usize] = lo;
                    ymin[lo as usize] = ymin[lo as usize].min(ymin[hi as usize]);
                    ymax[lo as usize] = ymax[lo as usize].max(ymax[hi as usize]);
                }
                k += 1;
            }
            // the last overlapping run may also touch the next run of this row
            j = k.saturating_sub(1);
        }
        for &(_, _, id) in &cur {
            let root = find(&mut parent, id) as usize;
            ymax[root] = ymax[root].max(y as u32 + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let cap = (h as u32 / 4).max(64);
    let mut hs: Vec<u32> = (0..parent.len())
        .filter(|&i| parent[i] as usize == i)
        .map(|i| ymax[i] - ymin[i])
        .filter(|&v| v >= 4 && v < cap)
        .collect();
    if hs.is_empty() {
        return 0;
    }
    hs.sort_unstable();
    // ignore dots, commas and dust: components far smaller than the upper-quartile letter
    let p75 = hs[hs.len() * 3 / 4];
    let letters: Vec<u32> = hs.iter().copied().filter(|&v| v * 100 >= p75 * 45).collect();
    letters[letters.len() / 2]
}

/// Erase horizontal runs longer than `10·med` and vertical runs longer than `6·med`: ruled lines,
/// underlines and table borders. The grey image gets the paper level in their place.
fn remove_rules(ink: &mut BitImage, gray: &mut GrayImage, med: u32) {
    let (w, h) = (ink.w as usize, ink.h as usize);
    let (hmin, vmin) = (med as usize * 10, med as usize * 6);
    let mut hist = [0u32; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let mut acc = 0u64;
    let mut paper = 255u8;
    for (v, &c) in hist.iter().enumerate() {
        acc += c as u64;
        if acc * 100 >= (w as u64 * h as u64) * 85 {
            paper = v as u8;
            break;
        }
    }
    let mut clear = vec![false; w * h];
    let mut any = false;
    for y in 0..h {
        let row = &ink.data[y * w..(y + 1) * w];
        let mut x = 0;
        while x < w {
            if row[x] == 0 {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && row[x] != 0 {
                x += 1;
            }
            if x - x0 >= hmin {
                clear[y * w + x0..y * w + x].fill(true);
                any = true;
            }
        }
    }
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if ink.data[y * w + x] == 0 {
                y += 1;
                continue;
            }
            let y0 = y;
            while y < h && ink.data[y * w + x] != 0 {
                y += 1;
            }
            if y - y0 >= vmin {
                for yy in y0..y {
                    clear[yy * w + x] = true;
                }
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    // widen by one pixel so the anti-aliased edge of a rule goes too
    let snapshot = clear.clone();
    for y in 0..h {
        for x in 0..w {
            if snapshot[y * w + x] {
                for (dy, dx) in [(0i64, 1i64), (0, -1), (1, 0), (-1, 0)] {
                    let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                    if yy >= 0 && xx >= 0 && (yy as usize) < h && (xx as usize) < w {
                        clear[yy as usize * w + xx as usize] = true;
                    }
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if clear[y * w + x] {
                ink.data[y * w + x] = 0;
                gray.put_pixel(x as u32, y as u32, image::Luma([paper]));
            }
        }
    }
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

/// All maximal runs of "empty" entries (value ≤ `noise`) strictly inside `v`, as (start, len).
fn gaps(v: &[u32], noise: u32) -> Vec<(usize, usize)> {
    let (Some(first), Some(last)) =
        (v.iter().position(|&x| x > noise), v.iter().rposition(|&x| x > noise))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let (mut start, mut len) = (0usize, 0usize);
    for i in first..=last {
        if v[i] <= noise {
            if len == 0 {
                start = i;
            }
            len += 1;
        } else {
            if len > 0 {
                out.push((start, len));
            }
            len = 0;
        }
    }
    out
}

fn widest(g: &[(usize, usize)]) -> Option<(usize, usize)> {
    g.iter().copied().max_by_key(|&(s, l)| (l, std::cmp::Reverse(s)))
}

/// Find the text lines of a page in reading order.
pub fn find_lines(ink: &BitImage, med_h: u32) -> Vec<Line> {
    let med = med_h.max(6);
    let p = Prof { ink };
    let mut out = Vec::new();
    cut(&p, (0, 0, ink.w, ink.h), med, 0, &mut out);
    attach_floating_marks(&mut out, med);
    // speckle: boxes that hold almost no ink are scanner dust, not text
    out.retain(|l| {
        let n: u64 = p.rows((l.x, l.y, l.x + l.w, l.y + l.h)).iter().map(|&v| v as u64).sum();
        n * 12 > med as u64 * med as u64
    });
    out
}

/// Accents over capitals, dots and the like float above the line body with a blank row between
/// them; the X-Y cut then makes them separate "lines". Glue each such thin piece to the line it
/// belongs to (close vertically, overlapping horizontally) and drop leftovers that are mere specks.
fn attach_floating_marks(lines: &mut Vec<Line>, med: u32) {
    let thin = |l: &Line| l.h * 2 < med;
    let n = lines.len();
    let mut gone = vec![false; n];
    for i in 0..n {
        if !thin(&lines[i]) {
            continue;
        }
        let t = lines[i].clone();
        let mut target: Option<(usize, u32)> = None;
        for (j, l) in lines.iter().enumerate() {
            if j == i || gone[j] || thin(l) {
                continue;
            }
            let vgap = if t.y + t.h <= l.y {
                l.y - (t.y + t.h)
            } else if l.y + l.h <= t.y {
                t.y - (l.y + l.h)
            } else {
                0
            };
            let ox = (t.x + t.w).min(l.x + l.w).saturating_sub(t.x.max(l.x));
            if vgap * 5 <= med * 2 && ox * 2 >= t.w && target.is_none_or(|(_, g)| vgap < g) {
                target = Some((j, vgap));
            }
        }
        match target {
            Some((j, _)) => {
                let l = &mut lines[j];
                let (x0, y0) = (l.x.min(t.x), l.y.min(t.y));
                let (x1, y1) = ((l.x + l.w).max(t.x + t.w), (l.y + l.h).max(t.y + t.h));
                *l = Line { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
                gone[i] = true;
            }
            None if t.w < med * 3 => gone[i] = true, // an isolated speck
            None => {}
        }
    }
    let mut k = 0;
    lines.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
}

type Region = (u32, u32, u32, u32);

fn cut(p: &Prof, r: Region, med: u32, depth: u32, out: &mut Vec<Line>) {
    let Some(r) = p.trim(r) else { return };
    let (x0, y0, x1, y1) = r;
    let (w, h) = (x1 - x0, y1 - y0);
    if w < 2 || h < 2 {
        return;
    }
    // too tiny to be a letter or a mark
    if w * 6 < med && h * 6 < med {
        return;
    }
    if h <= med * 2 + med / 2 {
        out.push(Line { x: x0, y: y0, w, h }); // one text line (thin pieces are attached later)
        return;
    }
    if depth > 60 {
        if h <= med * 4 {
            out.push(Line { x: x0, y: y0, w, h });
        }
        return;
    }
    let rows = p.rows(r);
    let cols = p.cols(r);
    // a little scanner noise must not bridge a gap
    let (noise_r, noise_c) = (w / 400, h / 400);
    let hgaps = gaps(&rows, noise_r);
    let vg = widest(&gaps(&cols, noise_c)).filter(|&(_, l)| l as u32 * 10 >= med * 18);
    let hg = widest(&hgaps);
    // a whitespace column running through several lines is a gutter: read it column by column
    // unless a blank row band is clearly bigger (a heading above two columns)
    if let (Some((vs, vl)), true) = (vg, h > med * 3) {
        if hg.is_none_or(|(_, hl)| vl * 2 >= hl) {
            let xs = x0 + vs as u32;
            cut(p, (x0, y0, xs, y1), med, depth + 1, out);
            cut(p, (xs + vl as u32, y0, x1, y1), med, depth + 1, out);
            return;
        }
    }
    if !hgaps.is_empty() {
        // split at every blank band at once, so the depth does not grow with the line count
        let mut start = y0;
        for &(s, l) in &hgaps {
            let ys = y0 + s as u32;
            cut(p, (x0, start, x1, ys), med, depth + 1, out);
            start = ys + l as u32;
        }
        cut(p, (x0, start, x1, y1), med, depth + 1, out);
        return;
    }
    if let Some((vs, vl)) = vg {
        let xs = x0 + vs as u32;
        cut(p, (x0, y0, xs, y1), med, depth + 1, out);
        cut(p, (xs + vl as u32, y0, x1, y1), med, depth + 1, out);
        return;
    }
    // no whitespace at all: touching lines (heavy skew) or a picture
    if h <= med * 8 {
        if let Some(c) = weakest_row(&rows, med) {
            let ys = y0 + c as u32;
            cut(p, (x0, y0, x1, ys), med, depth + 1, out);
            cut(p, (x0, ys, x1, y1), med, depth + 1, out);
        } else if h <= med * 4 {
            out.push(Line { x: x0, y: y0, w, h });
        }
    }
}

/// Row with the least ink within the middle band of a tall block, if it is a real valley.
fn weakest_row(rows: &[u32], med: u32) -> Option<usize> {
    let n = rows.len();
    let lo = (med as usize).min(n / 2);
    let hi = n.saturating_sub(lo).max(lo + 1);
    let (i, &v) = rows[lo..hi].iter().enumerate().min_by_key(|&(_, v)| *v)?;
    let mean = rows.iter().sum::<u32>() as f32 / n as f32;
    if (v as f32) < mean * 0.35 {
        Some(lo + i)
    } else {
        None
    }
}

/// Cut a line whose normalised width would exceed `max_w` recogniser pixels at word gaps
/// (the model was trained on lines up to ≈ 2300 px; very long lines are also slow and fragile).
pub fn split_long(ink: &BitImage, line: &Line, max_w: u32) -> Vec<Line> {
    let scale = (line.h as f32 * 1.24 / 32.0).max(0.01);
    let max_src = (max_w as f32 * scale) as u32;
    if line.w <= max_src || max_src < 64 {
        return vec![line.clone()];
    }
    let p = Prof { ink };
    let cols = p.cols((line.x, line.y, line.x + line.w, line.y + line.h));
    let mut out = Vec::new();
    let mut start = 0usize;
    while line.w as usize - start > max_src as usize {
        let (lo, hi) = (start + max_src as usize * 6 / 10, start + max_src as usize);
        // widest blank column run inside the window; fall back to a hard cut
        let (mut best, mut best_len, mut run_start, mut run) = (hi, 0usize, 0usize, 0usize);
        for x in lo..hi.min(cols.len()) {
            if cols[x] == 0 {
                if run == 0 {
                    run_start = x;
                }
                run += 1;
                if run > best_len {
                    best_len = run;
                    best = run_start + run / 2;
                }
            } else {
                run = 0;
            }
        }
        out.push(Line { x: line.x + start as u32, y: line.y, w: (best - start) as u32, h: line.h });
        start = best;
    }
    out.push(Line { x: line.x + start as u32, y: line.y, w: line.w - start as u32, h: line.h });
    out
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

/// Widest normalised line the recogniser is given (longer lines must be split first).
pub const MAX_LINE_W: usize = 2600;

pub fn extract(gray: &GrayImage, line: &Line, line_h: usize, invert: bool) -> Option<LineImage> {
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
    let pix = |x: u32, y: u32| -> f32 {
        let v = gray.get_pixel(x, y).0[0];
        (if invert { 255 - v } else { v }) as f32
    };
    // contrast: paper = high percentile, ink = low percentile
    let mut hist = [0u32; 256];
    for y in y0..y1 {
        for x in x0..x1 {
            hist[pix(x, y) as usize] += 1;
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
    if nw > MAX_LINE_W {
        return None;
    }
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
                    acc += pix(x0 + xx as u32, y0 + yy as u32) * wx * wy;
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
