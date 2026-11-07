//! Page → text lines: binarisation (with polarity and ruling removal), recursive X-Y cut on ink
//! projections, long-line splitting and line normalisation.
//!
//! Works on pages that were already straightened by the pipeline (small residual skew is
//! tolerated because lines are cut at whitespace valleys of the projection, not at fixed rows).

use image::GrayImage;
use legatorocr_core::{binarize, BitImage, PageImage};

/// A text line: its box on the page (pixels). A line cut out of a locally tilted block carries a
/// `slope` (rows of drift per column): `y` is then the top of the line at its horizontal centre.
#[derive(Debug, Clone, Default)]
pub struct Line {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub slope: f32,
    /// Second-order drift (rows per column squared), measured from the line's horizontal centre.
    pub curve: f32,
}

impl Line {
    /// Rows the line's centre has drifted from its middle at page column `x`.
    pub fn drift(&self, x: f32) -> f32 {
        let u = x - (self.x as f32 + self.w as f32 / 2.0);
        self.slope * u + self.curve * u * u
    }
}

/// The page as the engine sees it: grey image (dark text on light paper), ink mask, letter height.
pub struct Prepared {
    pub gray: GrayImage,
    pub ink: BitImage,
    /// Median height of letter-like components (0 = nothing found).
    pub med: u32,
    /// Clockwise angle (radians) the page was turned to level its text lines; 0 = untouched.
    /// Boxes found on `gray`/`ink` must go through [`unrotate_box`] to match the input page.
    pub skew: f32,
}

/// Skew below this is left alone (the line cut tolerates it).
const MIN_SKEW: f32 = 0.3 * std::f32::consts::PI / 180.0;

/// Turn a grey image clockwise by `rad` about its centre (bilinear, paper-coloured corners).
fn rotate_gray(g: &GrayImage, rad: f32, fill: u8) -> GrayImage {
    let (w, h) = (g.width() as i64, g.height() as i64);
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let (s, c) = rad.sin_cos();
    let mut out = GrayImage::from_pixel(g.width(), g.height(), image::Luma([fill]));
    for y in 0..h {
        for x in 0..w {
            // source = inverse rotation of the output position
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let sx = cx + dx * c + dy * s;
            let sy = cy - dx * s + dy * c;
            if sx < 0.0 || sy < 0.0 || sx > w as f32 - 1.0 || sy > h as f32 - 1.0 {
                continue;
            }
            let (x0, y0) = (sx.floor() as i64, sy.floor() as i64);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let px = |xx: i64, yy: i64| g.get_pixel(xx as u32, yy as u32).0[0] as f32;
            let v = px(x0, y0) * (1.0 - fx) * (1.0 - fy)
                + px(x1, y0) * fx * (1.0 - fy)
                + px(x0, y1) * (1.0 - fx) * fy
                + px(x1, y1) * fx * fy;
            out.put_pixel(
                x as u32,
                y as u32,
                image::Luma([v.round().clamp(0.0, 255.0) as u8]),
            );
        }
    }
    out
}

/// Map a box from the levelled page back to the page that was given (`w`×`h` pixels).
pub fn unrotate_box(b: (u32, u32, u32, u32), rad: f32, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, (h as f32 - 1.0) / 2.0);
    let (s, c) = rad.sin_cos();
    let (x0, y0, x1, y1) = (
        b.0 as f32,
        b.1 as f32,
        (b.0 + b.2) as f32,
        (b.1 + b.3) as f32,
    );
    let mut lo = (f32::MAX, f32::MAX);
    let mut hi = (f32::MIN, f32::MIN);
    for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        // the levelling turned the page by +rad; undo it
        let (dx, dy) = (px - cx, py - cy);
        let (ox, oy) = (cx + dx * c + dy * s, cy - dx * s + dy * c);
        lo = (lo.0.min(ox), lo.1.min(oy));
        hi = (hi.0.max(ox), hi.1.max(oy));
    }
    let x = lo.0.max(0.0).floor() as u32;
    let y = lo.1.max(0.0).floor() as u32;
    (
        x,
        y,
        (hi.0.ceil().max(0.0) as u32).saturating_sub(x).max(1),
        (hi.1.ceil().max(0.0) as u32).saturating_sub(y).max(1),
    )
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
    // dark paper (median grey below 50 %) or more "ink" than paper: light text on a dark page
    if median < 128 || ink.count_ink() * 100 / total > 50 {
        // more "ink" than paper: the page is light-on-dark
        for p in gray.pixels_mut() {
            p.0[0] = 255 - p.0[0];
        }
        ink = threshold_ink(&gray, dpi);
    }
    let mut med = median_component_height(&ink);
    if med >= 6 {
        remove_rules(&mut ink, &mut gray, med);
    }
    let mut skew = 0.0;
    if med >= 6 {
        if let Some(a) = legatorocr_core::deskew::detect_skew(&ink, 6.0) {
            if a.abs() > MIN_SKEW {
                // level the lines so the cut sees horizontal rows; paper level = a light percentile
                let mut hist = [0u64; 256];
                for p in gray.pixels() {
                    hist[p.0[0] as usize] += 1;
                }
                let (mut acc, mut paper) = (0u64, 255u8);
                for (v, &c) in hist.iter().enumerate() {
                    acc += c;
                    if acc * 100 >= total * 85 {
                        paper = v as u8;
                        break;
                    }
                }
                gray = rotate_gray(&gray, a, paper);
                ink = threshold_ink(&gray, dpi);
                med = median_component_height(&ink).max(1);
                remove_rules(&mut ink, &mut gray, med);
                skew = a;
            }
        }
    }
    Prepared {
        gray,
        ink,
        med,
        skew,
    }
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
    let letters: Vec<u32> = hs
        .iter()
        .copied()
        .filter(|&v| v * 100 >= p75 * 45)
        .collect();
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
    med: u32,
}

impl Prof<'_> {
    fn rows(&self, r: (u32, u32, u32, u32)) -> Vec<u32> {
        let (x0, y0, x1, y1) = r;
        (y0..y1)
            .map(|y| {
                let s = y as usize * self.ink.w as usize;
                self.ink.data[s + x0 as usize..s + x1 as usize]
                    .iter()
                    .map(|&b| b as u32)
                    .sum()
            })
            .collect()
    }
    fn cols(&self, r: (u32, u32, u32, u32)) -> Vec<u32> {
        let (x0, y0, x1, y1) = r;
        let mut v = vec![0u32; (x1 - x0) as usize];
        for y in y0..y1 {
            let s = y as usize * self.ink.w as usize;
            for (i, &b) in self.ink.data[s + x0 as usize..s + x1 as usize]
                .iter()
                .enumerate()
            {
                v[i] += b as u32;
            }
        }
        v
    }
    /// Does a line of text cross the strip `xs..xs+len` of region `r`?
    fn crossed(&self, r: (u32, u32, u32, u32), xs: u32, len: u32, min_run: u32) -> bool {
        !self.crossings(r, xs, len, min_run).is_empty()
    }
    /// Row ranges (start, end exclusive, absolute) of the runs of at least `min_run` rows in which
    /// the ink is in several pieces along the row: letters of a line of text, not dust or a smudge.
    fn crossings(
        &self,
        r: (u32, u32, u32, u32),
        xs: u32,
        len: u32,
        min_run: u32,
    ) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut start: Option<u32> = None;
        let need = (len / self.med.max(1)).clamp(2, 3);
        for y in r.1..=r.3 {
            let ink = y < r.3 && {
                let s = y as usize * self.ink.w as usize + xs as usize;
                // separate pieces of ink along the row: letters, where dust is a single speck
                let row = &self.ink.data[s..s + len as usize];
                let mut pieces = 0u32;
                let mut prev = 0u8;
                for &b in row {
                    if b != 0 && prev == 0 {
                        pieces += 1;
                    }
                    prev = b;
                }
                pieces >= need
            };
            match (ink, start) {
                (true, None) => start = Some(y),
                (false, Some(a)) => {
                    if y - a >= min_run {
                        out.push((a, y));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        out
    }
    /// Shrink a region to its ink bounding box.
    fn trim(&self, r: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
        let rows = self.rows(r);
        let cols = self.cols(r);
        let a = rows.iter().position(|&v| v > 0)?;
        let b = rows.iter().rposition(|&v| v > 0)?;
        let c = cols.iter().position(|&v| v > 0)?;
        let d = cols.iter().rposition(|&v| v > 0)?;
        Some((
            r.0 + c as u32,
            r.1 + a as u32,
            r.0 + d as u32 + 1,
            r.1 + b as u32 + 1,
        ))
    }
}

/// All maximal runs of "empty" entries (value ≤ `noise`) strictly inside `v`, as (start, len).
fn gaps(v: &[u32], noise: u32) -> Vec<(usize, usize)> {
    let (Some(first), Some(last)) = (
        v.iter().position(|&x| x > noise),
        v.iter().rposition(|&x| x > noise),
    ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let (mut start, mut len) = (0usize, 0usize);
    #[allow(clippy::needless_range_loop)]
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

/// Do the bands between these gaps have the height of text lines (not slices of display letters)?
fn looks_like_text_lines(g: &[(usize, usize)], len: usize, med: u32) -> bool {
    if g.is_empty() {
        return false;
    }
    let mut start = 0usize;
    let (mut ok, mut all) = (0u32, 0u32);
    for &(s, l) in g.iter().chain(std::iter::once(&(len, 0))) {
        let h = (s - start) as u32;
        if h * 2 >= med {
            all += 1;
            if h * 10 >= med * 6 && h * 10 <= med * 26 {
                ok += 1;
            }
        }
        start = s + l;
    }
    all >= 2 && ok * 10 >= all * 7
}

fn widest(g: &[(usize, usize)]) -> Option<(usize, usize)> {
    g.iter()
        .copied()
        .max_by_key(|&(s, l)| (l, std::cmp::Reverse(s)))
}

/// Find the text lines of a page in reading order.
pub fn find_lines(ink: &BitImage, med_h: u32) -> Vec<Line> {
    let med = med_h.max(6);
    let p = Prof { ink, med };
    let mut out = Vec::new();
    cut(&p, (0, 0, ink.w, ink.h), med, 0, &mut out);
    recover_above(&p, &mut out, med);
    attach_floating_marks(&mut out, med);
    drop_duplicates(&mut out);
    // speckle: boxes that hold almost no ink are scanner dust, not text
    out.retain(|l| {
        let n: u64 = p
            .rows((l.x, l.y, l.x + l.w, l.y + l.h))
            .iter()
            .map(|&v| v as u64)
            .sum();
        n * 12 > med as u64 * med as u64
    });
    out
}

/// A heading set larger than the body is cut in two overlapping bands: join the bands that
/// overlap (union at most 4 letter heights tall); of pieces that cannot be joined keep the larger.
/// Result sorted by position.
fn join_bands(v: Vec<Line>, med: u32) -> Vec<Line> {
    let mut v = v;
    v.sort_by_key(|f| std::cmp::Reverse(f.w as u64 * f.h as u64));
    let mut chosen: Vec<Line> = Vec::new();
    for f in v {
        let hit = chosen
            .iter()
            .position(|o| f.x < o.x + o.w && o.x < f.x + f.w && f.y < o.y + o.h && o.y < f.y + f.h);
        match hit {
            None => chosen.push(f),
            Some(i) => {
                let o = &chosen[i];
                let (x0, y0) = (o.x.min(f.x), o.y.min(f.y));
                let (x1, y1) = ((o.x + o.w).max(f.x + f.w), (o.y + o.h).max(f.y + f.h));
                if y1 - y0 <= med * 4 {
                    chosen[i] = Line {
                        x: x0,
                        y: y0,
                        w: x1 - x0,
                        h: y1 - y0,
                        slope: 0.0,
                        curve: 0.0,
                    };
                }
            }
        }
    }
    chosen.sort_by_key(|f| (f.y, f.x));
    chosen
}

/// A block of text sitting beside or over a picture may fail to be cut at all, while the
/// column below it is found. Take the column that holds the most wide lines of one left edge and
/// cut the strip above its first line again, over that column's width only; keep the pieces that
/// have the height and width of text lines and overlap nothing found already.
fn recover_above(p: &Prof, out: &mut Vec<Line>, med: u32) {
    let wide: Vec<Line> = out
        .iter()
        .filter(|l| l.w >= med * 15 && l.h * 10 >= med * 6 && l.h * 10 <= med * 30)
        .cloned()
        .collect();
    let mut best: Option<(usize, u32, u32, u32)> = None; // lines, left, right, first y
    for a in &wide {
        let group: Vec<&Line> = wide
            .iter()
            .filter(|b| b.x.abs_diff(a.x) <= med * 2 && b.w.abs_diff(a.w) <= med * 6)
            .collect();
        let n = group.len();
        if n >= 3 && best.is_none_or(|b| n > b.0) {
            let l = group.iter().map(|g| g.x).min().unwrap();
            let r = group.iter().map(|g| g.x + g.w).max().unwrap();
            let y = group.iter().map(|g| g.y).min().unwrap();
            best = Some((n, l, r, y));
        }
    }
    let Some((_, l, r, y)) = best else { return };
    let (xa, xb) = (l.saturating_sub(med), (r + med).min(p.ink.w));
    if y < med * 3 {
        return;
    }
    let mut found = Vec::new();
    let mut keep = Vec::new();
    cut(p, (xa, 0, xb, y), med, 0, &mut found);
    for f in found {
        let fits = f.w >= med * 8 && f.h * 10 >= med * 6 && f.h * 10 <= med * 30;
        let free = !out
            .iter()
            .any(|o| f.x < o.x + o.w && o.x < f.x + f.w && f.y < o.y + o.h && o.y < f.y + f.h);
        if fits && free {
            keep.push(f);
        }
    }
    let keep = join_bands(keep, med);
    // reading order: the strip lies above the column, so its lines go before the column's first
    let at = out
        .iter()
        .position(|o| o.y == y && o.x >= l && o.x <= r)
        .unwrap_or(out.len());
    let mut top = keep.iter().map(|k| k.y).min().map_or(y, |t| t.min(y));
    out.splice(at..at, keep);
    // The strip as a whole may still hold a picture edge that hides a heading: go up in short
    // windows from the top line found, a few times, while each window yields lines. A line cut by
    // the window's upper edge is not trusted.
    for _ in 0..4 {
        let wtop = top.saturating_sub(med * 8);
        if top - wtop < med * 2 {
            break;
        }
        let mut found = Vec::new();
        cut(p, (xa, wtop, xb, top), med, 0, &mut found);
        let pre: Vec<Line> = found
            .into_iter()
            .filter(|f| {
                let fits = f.w >= med * 8 && f.h * 10 >= med * 6 && f.h * 10 <= med * 30;
                let free = !out.iter().any(|o| {
                    f.x < o.x + o.w && o.x < f.x + f.w && f.y < o.y + o.h && o.y < f.y + f.h
                });
                fits && free && (wtop == 0 || f.y > wtop + 1)
            })
            .collect();
        let pre = join_bands(pre, med);
        let Some(t) = pre.iter().map(|f| f.y).min() else {
            break;
        };
        top = t;
        out.splice(at..at, pre);
    }
}

/// Two lines that cover mostly the same area are one line read twice (a curved block cut by two
/// routes): keep the first.
fn drop_duplicates(lines: &mut Vec<Line>) {
    let n = lines.len();
    let mut gone = vec![false; n];
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| lines[i].y);
    for a in 0..n {
        let i = order[a];
        if gone[i] {
            continue;
        }
        for &j in &order[a + 1..] {
            if lines[j].y > lines[i].y + lines[i].h {
                break;
            }
            if gone[j] {
                continue;
            }
            let (li, lj) = (&lines[i], &lines[j]);
            let ox = (li.x + li.w)
                .min(lj.x + lj.w)
                .saturating_sub(li.x.max(lj.x)) as u64;
            let oy = (li.y + li.h)
                .min(lj.y + lj.h)
                .saturating_sub(li.y.max(lj.y)) as u64;
            let small = (li.w as u64 * li.h as u64)
                .min(lj.w as u64 * lj.h as u64)
                .max(1);
            if ox * oy * 10 > small * 6 {
                gone[j] = true;
            }
        }
    }
    let mut k = 0;
    lines.retain(|_| {
        k += 1;
        !gone[k - 1]
    });
}

/// Accents over capitals, dots and the like float above the line body with a blank row between
/// them; the X-Y cut then makes them separate "lines". Glue each such thin piece to the line it
/// belongs to (close vertically, overlapping horizontally) and drop leftovers that are mere specks.
fn attach_floating_marks(lines: &mut Vec<Line>, med: u32) {
    let thin = |l: &Line| l.h * 2 < med;
    let n = lines.len();
    let mut gone = vec![false; n];
    // Only a thin piece close above or below a body line can join it, so the candidates are found
    // by position (the lines of a page are sorted by y) instead of testing every pair: a dusty
    // scan has tens of thousands of specks and the pairwise test took minutes.
    let mut order: Vec<usize> = (0..n).filter(|&j| !thin(&lines[j])).collect();
    order.sort_by_key(|&j| lines[j].y);
    // the sort keys are frozen: a line that grows by a join must not unsort the search
    let ys: Vec<u32> = order.iter().map(|&j| lines[j].y).collect();
    let max_h = order.iter().map(|&j| lines[j].h).max().unwrap_or(0);
    let reach = med * 2; // vertical reach of a piece (0.4 med) plus slack for lines grown by a join
    for i in 0..n {
        if !thin(&lines[i]) {
            continue;
        }
        let t = lines[i].clone();
        let mut target: Option<(usize, u32)> = None;
        let lo = t.y.saturating_sub(reach + max_h);
        let hi = t.y + t.h + reach;
        let a = ys.partition_point(|&y| y < lo);
        let b = ys.partition_point(|&y| y <= hi);
        for &j in &order[a..b.max(a)] {
            let l = &lines[j];
            if j == i || gone[j] {
                continue;
            }
            let vgap = if t.y + t.h <= l.y {
                l.y - (t.y + t.h)
            } else {
                t.y.saturating_sub(l.y + l.h)
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
                *l = Line {
                    x: x0,
                    y: y0,
                    w: x1 - x0,
                    h: y1 - y0,
                    slope: 0.0,
                    curve: 0.0,
                };
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
        out.push(Line {
            x: x0,
            y: y0,
            w,
            h,
            slope: 0.0,
            curve: 0.0,
        }); // one text line (thin pieces are attached later)
        return;
    }
    if depth > 60 {
        if h <= med * 4 {
            out.push(Line {
                x: x0,
                y: y0,
                w,
                h,
                slope: 0.0,
                curve: 0.0,
            });
        }
        return;
    }
    let rows = p.rows(r);
    let cols = p.cols(r);
    // a little scanner noise must not bridge a gap
    // (scan dust in a gutter is ink too: allow about one speck per 60 rows of the region)
    let (noise_r, noise_c) = (w / 400, (h / 60).max(1));
    let mut hgaps = gaps(&rows, noise_r);
    // a tall block without a single blank row is usually text lines whose ascenders and descenders
    // are bridged by a slight skew or scanner noise: accept a little more ink in a gap before
    // giving up (a block that stays one piece is cut at its weakest row or dropped as a picture)
    if hgaps.is_empty() && h > med * 3 {
        for k in [3u32, 6, 12] {
            let g = gaps(&rows, noise_r * k + k);
            if looks_like_text_lines(&g, rows.len(), med) {
                hgaps = g;
                break;
            }
        }
    }
    // a gutter is at least 1.8 line heights wide; a narrower one (tightly set columns, 0.9 to
    // 1.8) counts only in a tall block with real text on both sides, where a river of white
    // between the words of a justified paragraph is unlikely to run all the way down
    let mut vgaps = gaps(&cols, noise_c);
    // Columns with a heading or a footer line running across the gutter: cut the lines that cross
    // it off first (at the blank bands around them), then read the rest column by column.
    if h > med * 3 {
        let wide = |l: usize| l as u32 * 10 >= med * 18;
        if let Some((s, l)) = widest(&vgaps).filter(|&(_, l)| wide(l)) {
            let runs = p.crossings(
                r,
                x0 + (s + l / 4) as u32,
                (l / 2).max(1) as u32,
                (med / 2).max(4),
            );
            let rows_cross: u32 = runs.iter().map(|&(a, b)| b - a).sum();
            if !runs.is_empty() && rows_cross * 6 <= h {
                let bands: Vec<(u32, u32)> = hgaps
                    .iter()
                    .map(|&(s, l)| (y0 + s as u32, y0 + (s + l) as u32))
                    .collect();
                let mut cuts: Vec<u32> = Vec::new();
                let mut ok = true;
                for &(a, b) in &runs {
                    // a blank band right above the crossing line and right below it
                    let up = bands
                        .iter()
                        .rev()
                        .find(|&&(_, e)| e <= a)
                        .map(|&(bs, be)| (bs + be) / 2);
                    let dn = bands
                        .iter()
                        .find(|&&(bs, _)| bs >= b)
                        .map(|&(bs, be)| (bs + be) / 2);
                    if a > y0 + med && up.is_none() {
                        ok = false;
                    }
                    if b + med < y1 && dn.is_none() {
                        ok = false;
                    }
                    cuts.extend(up);
                    cuts.extend(dn);
                }
                cuts.sort_unstable();
                cuts.dedup();
                if ok && !cuts.is_empty() {
                    let mut start = y0;
                    for c in cuts {
                        cut(p, (x0, start, x1, c), med, depth + 1, out);
                        start = c;
                    }
                    cut(p, (x0, start, x1, y1), med, depth + 1, out);
                    return;
                }
            }
        }
    }
    // a line of text that crosses a candidate gutter (the tolerance above is taller than a line on
    // a whole page) makes it no gutter
    vgaps.retain(|&(s, l)| {
        !p.crossed(
            r,
            x0 + (s + l / 4) as u32,
            (l / 2).max(1) as u32,
            (med / 2).max(4),
        )
    });
    let vg = widest(&vgaps).filter(|&(s, l)| {
        let l32 = l as u32;
        l32 * 10 >= med * 18
            || (l32 * 10 >= med * 9
                && h > med * 8
                && s as u32 * 5 >= w
                && (w - s as u32 - l32) * 5 >= w)
    });
    let hg = widest(&hgaps);
    // a whitespace column running through several lines is a gutter: read it column by column
    // unless a blank row band is clearly bigger (a heading above two columns)
    // Blocks of text laid out in rows of columns (stanzas side by side, a table of paragraphs): two
    // or more blank bands of 4+ letter heights across the whole region cannot come from columns of
    // running text, whose paragraph breaks do not line up. Read such a region row by row.
    let rows_of_columns = hgaps.iter().filter(|&&(_, l)| l as u32 >= med * 4).count() >= 2;
    if let (Some((vs, vl)), true) = (vg, h > med * 3) {
        if !rows_of_columns && hg.is_none_or(|(_, hl)| vl * 2 >= hl) {
            let xs = x0 + vs as u32;
            cut(p, (x0, y0, xs, y1), med, depth + 1, out);
            cut(p, (xs + vl as u32, y0, x1, y1), med, depth + 1, out);
            return;
        }
    }
    if !hgaps.is_empty() {
        // Split first only at the big blank bands (a picture, a heading, a paragraph break): the
        // blocks between them may hold two columns whose lines happen to sit at the same height,
        // and cutting those into full-width lines would glue the columns together. A block with no
        // big band left falls through to the cut at every band.
        let big: Vec<(usize, usize)> = hgaps
            .iter()
            .copied()
            .filter(|&(_, l)| l as u32 * 10 >= med * 15)
            .collect();
        let hgaps = if big.is_empty() || big.len() == hgaps.len() || h <= med * 3 {
            hgaps
        } else {
            big
        };
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
            return;
        }
    }
    // curved or tilted lines whose rows overlap: follow the letters instead
    if h > med * 2 + med / 2 && h <= med * 24 && chain_cut(p, r, med, out) {
        return;
    }
    if h <= med * 4 {
        out.push(Line {
            x: x0,
            y: y0,
            w,
            h,
            slope: 0.0,
            curve: 0.0,
        });
    }
}

/// Letters of a block that cannot be cut by blank rows (curved or tilted lines whose rows overlap,
/// as near the spine of a bound book) are chained left to right into lines: each letter joins the
/// chain whose recent trend predicts its height. A chain becomes a line with a slope and a
/// curvature (a parabola fitted to the letter centres) that the recogniser input follows.
fn chain_cut(p: &Prof, r: Region, med: u32, out: &mut Vec<Line>) -> bool {
    let (x0, y0, x1, y1) = r;
    let (w, h) = (x1 - x0, y1 - y0);
    if w < med * 8 || h < med * 2 || (w as u64) * (h as u64) > 4_000_000 {
        return false;
    }
    // connected components (8-neighbourhood) of the block
    let ink = p.ink;
    let mut lab = vec![0u32; (w * h) as usize];
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new(); // x0,y0,x1,y1 (inclusive)
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            if lab[(sy * w + sx) as usize] != 0 || !ink.get(x0 + sx, y0 + sy) {
                continue;
            }
            let id = boxes.len() as u32 + 1;
            let mut b = (sx, sy, sx, sy);
            lab[(sy * w + sx) as usize] = id;
            stack.push((sx, sy));
            while let Some((cx, cy)) = stack.pop() {
                b = (b.0.min(cx), b.1.min(cy), b.2.max(cx), b.3.max(cy));
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let (nx, ny) = (nx as u32, ny as u32);
                        if lab[(ny * w + nx) as usize] == 0 && ink.get(x0 + nx, y0 + ny) {
                            lab[(ny * w + nx) as usize] = id;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            boxes.push(b);
        }
    }
    // letter-like components, left to right
    let mut cs: Vec<(f32, f32, f32)> = boxes // centre x, centre y, width
        .iter()
        .filter(|b| {
            let (bw, bh) = (b.2 - b.0 + 1, b.3 - b.1 + 1);
            bh * 4 >= med && bh <= med * 3 && bw <= med * 6
        })
        .map(|b| {
            (
                (b.0 + b.2) as f32 / 2.0,
                (b.1 + b.3) as f32 / 2.0,
                (b.2 - b.0 + 1) as f32,
            )
        })
        .collect();
    cs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let medf = med as f32;
    let mut chains: Vec<Vec<(f32, f32)>> = Vec::new();
    for &(cx, cy, _) in &cs {
        let mut best: Option<(usize, f32)> = None;
        for (k, ch) in chains.iter().enumerate() {
            let last = ch[ch.len() - 1];
            if cx - last.0 > medf * 8.0 {
                continue;
            }
            // height predicted by the trend of the last few letters
            let n = ch.len().min(8);
            let tail = &ch[ch.len() - n..];
            let pred = if n >= 4 {
                let (mx, my) = (
                    tail.iter().map(|t| t.0).sum::<f32>() / n as f32,
                    tail.iter().map(|t| t.1).sum::<f32>() / n as f32,
                );
                let (sxy, sxx) = tail.iter().fold((0.0, 0.0), |a, t| {
                    (a.0 + (t.0 - mx) * (t.1 - my), a.1 + (t.0 - mx) * (t.0 - mx))
                });
                let slope = if sxx > 1.0 {
                    (sxy / sxx).clamp(-0.12, 0.12)
                } else {
                    0.0
                };
                my + slope * (cx - mx)
            } else {
                last.1
            };
            let d = (cy - pred).abs();
            if d < medf * 0.75 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((k, d));
            }
        }
        match best {
            Some((k, _)) => chains[k].push((cx, cy)),
            None => chains.push(vec![(cx, cy)]),
        }
    }
    let mut made = 0;
    let lh = (medf * 2.0).round() as u32;
    let xc_page = |c: &[(f32, f32)]| (c[0].0 + c[c.len() - 1].0) / 2.0;
    let mut found: Vec<Line> = Vec::new();
    for c in chains
        .iter()
        .filter(|c| c.len() >= 6 && c[c.len() - 1].0 - c[0].0 >= medf * 6.0)
    {
        // parabola through the centres, relative to the middle: y = a + b u + c u²
        let xc = xc_page(c);
        let fit = |pts: &[(f32, f32)]| -> Option<(f32, f32, f32)> {
            let n = pts.len() as f64;
            let (mut s1, mut s2, mut s3, mut s4, mut t0, mut t1, mut t2) =
                (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
            for &(x, y) in pts {
                let (u, y) = ((x - xc) as f64, y as f64);
                s1 += u;
                s2 += u * u;
                s3 += u * u * u;
                s4 += u * u * u * u;
                t0 += y;
                t1 += u * y;
                t2 += u * u * y;
            }
            // normal equations of the 3x3 system, solved with Cramer's rule
            let det3 = |m: [[f64; 3]; 3]| {
                m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                    - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                    + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
            };
            let m = [[n, s1, s2], [s1, s2, s3], [s2, s3, s4]];
            let det = det3(m);
            if det.abs() < 1e-6 {
                return None;
            }
            let col = |k: usize| {
                let mut q = m;
                for (row, t) in q.iter_mut().zip([t0, t1, t2]) {
                    row[k] = t;
                }
                det3(q) / det
            };
            let (a, b, cc) = (col(0), col(1), col(2));
            Some((a as f32, b as f32, cc as f32))
        };
        let Some(mut f) = fit(c) else { continue };
        // robust fit: drop letters far from the curve (ascenders, descenders, letters of a
        // neighbouring line that slipped in) and fit again, a few times
        let mut kept: Vec<(f32, f32)> = c.to_vec();
        for _ in 0..4 {
            let res = |&(x, y): &(f32, f32)| {
                let u = x - xc;
                (y - (f.0 + f.1 * u + f.2 * u * u)).abs()
            };
            let mut r: Vec<f32> = kept.iter().map(res).collect();
            r.sort_by(|a, b| a.total_cmp(b));
            let limit = (r[r.len() / 2] * 2.5).clamp(medf * 0.35, medf * 0.7);
            let next: Vec<(f32, f32)> = c.iter().copied().filter(|p| res(p) <= limit).collect();
            if next.len() < 6 {
                break;
            }
            match fit(&next) {
                Some(g) => {
                    f = g;
                    kept = next;
                }
                None => break,
            }
        }
        // a chain that does not follow a smooth curve mixed letters of neighbouring lines
        let resid = kept
            .iter()
            .map(|&(x, y)| {
                let u = x - xc;
                (y - (f.0 + f.1 * u + f.2 * u * u)).abs()
            })
            .sum::<f32>()
            / kept.len().max(1) as f32;
        if kept.len() * 10 < c.len() * 7 || resid > medf * 0.45 || f.1.abs() > 0.12 {
            continue;
        }
        let first = c[0].0;
        let last = c[c.len() - 1].0;
        let top = y0 as f32 + f.0 - lh as f32 / 2.0;
        // the box stays inside the page
        let lx = (x0 as f32 + first - medf).max(0.0) as u32;
        let lw = ((last - first + 2.0 * medf) as u32).min(ink.w.saturating_sub(lx));
        let ly = (top.max(0.0) as u32).min(ink.h.saturating_sub(lh));
        if lw < 2 {
            continue;
        }
        found.push(Line {
            x: lx,
            y: ly,
            w: lw,
            h: lh.min(ink.h),
            slope: f.1,
            curve: f.2,
        });
        made += 1;
    }
    found.sort_by_key(|l| l.y);
    out.extend(found);
    made >= 2
}

/// Row with the least ink within the middle band of a tall block, if it is a real valley.
fn weakest_row(rows: &[u32], med: u32) -> Option<usize> {
    let n = rows.len();
    let lo = (med as usize).min(n / 2);
    let hi = n.saturating_sub(lo).max(lo + 1);
    let (i, &v) = rows[lo..hi].iter().enumerate().min_by_key(|&(_, v)| *v)?;
    let mean = rows.iter().sum::<u32>() as f32 / n as f32;
    if (v as f32) < mean * 0.25 {
        Some(lo + i)
    } else {
        None
    }
}

/// The part of `line` that starts `start` pixels in and is `w` wide; a tilted line keeps its slope
/// and its `y` is moved to the part's own centre.
fn piece(line: &Line, start: u32, w: u32) -> Line {
    let shift = line.drift(line.x as f32 + start as f32 + w as f32 / 2.0);
    Line {
        x: line.x + start,
        y: (line.y as f32 + shift).round().max(0.0) as u32,
        w,
        h: line.h,
        slope: line.slope
            + 2.0 * line.curve * (start as f32 + w as f32 / 2.0 - line.w as f32 / 2.0),
        curve: line.curve,
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
    let p = Prof { ink, med: 1 };
    let cols = p.cols((line.x, line.y, line.x + line.w, line.y + line.h));
    let mut out = Vec::new();
    let mut start = 0usize;
    while line.w as usize - start > max_src as usize {
        let (lo, hi) = (start + max_src as usize * 6 / 10, start + max_src as usize);
        // widest blank column run inside the window; fall back to a hard cut
        let (mut best, mut best_len, mut run_start, mut run) = (hi, 0usize, 0usize, 0usize);
        #[allow(clippy::needless_range_loop)]
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
        out.push(piece(line, start as u32, (best - start) as u32));
        start = best;
    }
    out.push(piece(line, start as u32, line.w - start as u32));
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
    // a tilted line is read straight: the rows are taken along its slope
    let (gw, gh) = (gray.width() as i64, gray.height() as i64);
    let pix = |x: u32, y: u32| -> f32 {
        let y = if line.slope == 0.0 && line.curve == 0.0 {
            y
        } else {
            (y as i64 + line.drift(x as f32).round() as i64).clamp(0, gh - 1) as u32
        };
        let x = (x as i64).min(gw - 1) as u32;
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
    Some(LineImage {
        data,
        w: nw,
        scale,
        x: x0,
        y: y0,
        h: (y1 - y0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(ink: &mut BitImage, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                ink.data[(y * ink.w + x) as usize] = 1;
            }
        }
    }

    #[test]
    fn text_above_a_column_that_could_not_be_cut_is_recovered() {
        let med = 10;
        let mut ink = BitImage::new(600, 400);
        // a column of five text lines, 300 px wide, left edge 100, lines 20 px apart
        let lines: Vec<(u32, u32)> = (0..5).map(|k| (200 + k * 20, 0)).collect();
        for &(y, _) in &lines {
            for k in 0..30 {
                fill(&mut ink, 100 + k * 10, y, 106 + k * 10, y + 10);
            }
        }
        // three more lines above it, and a dark frame edge running down the right side that
        // touches them, so no blank row crosses the page there
        for k in 0..3u32 {
            for j in 0..30 {
                fill(
                    &mut ink,
                    100 + j * 10,
                    60 + k * 20,
                    106 + j * 10,
                    70 + k * 20,
                );
            }
        }
        fill(&mut ink, 560, 0, 566, 400);
        let mut out = vec![];
        for &(y, _) in &lines {
            out.push(Line {
                x: 100,
                y,
                w: 296,
                h: 10,
                slope: 0.0,
                curve: 0.0,
            });
        }
        let p = Prof { ink: &ink, med };
        recover_above(&p, &mut out, med);
        assert_eq!(out.len(), 8, "the three lines above were added");
        assert!(
            out[..3].iter().all(|l| l.y < 200),
            "and placed before the column"
        );
        assert!(
            out.iter().all(|l| l.x < 400),
            "the frame edge is not a line"
        );
    }

    #[test]
    fn a_heading_larger_than_the_body_is_one_line() {
        let med = 10;
        let mut ink = BitImage::new(700, 500);
        // body column: four lines (10 px tall, 20 px apart), left edge 100, 400 px wide
        let mut out = vec![];
        for k in 0..4u32 {
            let y = 300 + k * 20;
            for j in 0..40 {
                fill(&mut ink, 100 + j * 10, y, 106 + j * 10, y + 10);
            }
            out.push(Line {
                x: 100,
                y,
                w: 396,
                h: 10,
                slope: 0.0,
                curve: 0.0,
            });
        }
        // a heading 34 px tall (3.4 x med): the left half sits 12 px lower than the right half,
        // so the rows of the two halves overlap and no blank row separates them from the body's
        // frame edge; a dark edge touches it on the right
        for j in 0..12 {
            fill(&mut ink, 150 + j * 14, 150 + 12, 160 + j * 14, 150 + 34);
        }
        for j in 12..24 {
            fill(&mut ink, 150 + j * 14, 150, 160 + j * 14, 150 + 22);
        }
        fill(&mut ink, 560, 0, 566, 500);
        let p = Prof { ink: &ink, med };
        recover_above(&p, &mut out, med);
        let heads: Vec<&Line> = out.iter().filter(|l| l.y < 250).collect();
        assert_eq!(heads.len(), 1, "{heads:?}");
        assert!(
            heads[0].h >= 30,
            "the heading keeps its full height: {heads:?}"
        );
        assert_eq!(
            out.iter().position(|l| l.y < 250),
            Some(0),
            "before the column"
        );
    }
}
