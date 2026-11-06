//! Page clean-up stages that go beyond geometry: negative scans, contrast stretch,
//! sharpening, punch holes, fingers at the page edge, ruled lines and colour
//! drop-out. All are small, self-contained image operations written from the
//! problem statement (what a hole / finger / ruled line looks like), not from any
//! other program's code.

use crate::BitImage;
use image::{Rgb, RgbImage};

/// Colour removed by [`dropout`] (form lines, stamps, review marks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dropout {
    #[default]
    Off,
    Red,
    Green,
    Blue,
}

impl Dropout {
    pub const ALL: [Dropout; 4] = [Dropout::Off, Dropout::Red, Dropout::Green, Dropout::Blue];
    pub fn name(self) -> &'static str {
        match self {
            Dropout::Off => "off",
            Dropout::Red => "red",
            Dropout::Green => "green",
            Dropout::Blue => "blue",
        }
    }
    pub fn from_name(s: &str) -> Option<Dropout> {
        Dropout::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s))
    }
}

fn luma(p: &[u8]) -> u32 {
    (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000
}

fn luma_hist(img: &RgbImage) -> [u64; 256] {
    let mut h = [0u64; 256];
    for p in img.as_raw().as_chunks::<3>().0 {
        h[luma(p) as usize] += 1;
    }
    h
}

/// Value below which `frac` of the pixels lie.
fn percentile(h: &[u64; 256], frac: f64) -> u8 {
    let total: u64 = h.iter().sum();
    let target = (total as f64 * frac) as u64;
    let mut acc = 0;
    for (v, &n) in h.iter().enumerate() {
        acc += n;
        if acc > target {
            return v as u8;
        }
    }
    255
}

/// A negative (light text on a dark page) is inverted. Detected by a dark median
/// together with a bright tail; ordinary dark photos have no bright text-like tail
/// covering enough of the page to move the 97th percentile. Returns whether it inverted.
pub fn invert_if_negative(img: &mut RgbImage) -> bool {
    if img.width() == 0 || img.height() == 0 {
        return false;
    }
    let h = luma_hist(img);
    // Light text on a dark page: dark median, a bright tail, and the bright part stays a small
    // share (text). A light sheet that fills a third of a black scanner lid has a dark median
    // too, but its bright share is over a third — that is a page on a dark bed, not a negative.
    if percentile(&h, 0.5) < 90 && percentile(&h, 0.97) > 150 && percentile(&h, 0.67) < 120 {
        for v in img.as_mut() {
            *v = 255 - *v;
        }
        true
    } else {
        false
    }
}

/// Stretch contrast so the 0.5 % darkest pixels become black and the 0.5 % lightest
/// white. One gain for all channels (hue is kept). Skipped for flat pages.
pub fn auto_levels(img: &mut RgbImage) {
    if img.width() == 0 || img.height() == 0 {
        return;
    }
    let h = luma_hist(img);
    let (lo, hi) = (percentile(&h, 0.005) as i32, percentile(&h, 0.995) as i32);
    if hi - lo < 40 || (lo <= 2 && hi >= 253) {
        return;
    }
    let lut: Vec<u8> = (0..256)
        .map(|v| (((v - lo) * 255) / (hi - lo)).clamp(0, 255) as u8)
        .collect();
    for v in img.as_mut() {
        *v = lut[*v as usize];
    }
}

/// Blur every channel with two box passes per axis (≈ Gaussian), radius `r` px.
///
/// Works in horizontal bands (in parallel) that overlap by the filter's vertical reach (2·r
/// rows), so memory is proportional to a band, not to the whole page (the old version held two
/// `f32` copies of the image: 24 bytes per pixel), and the result is identical to blurring the
/// whole image at once.
fn blur(img: &RgbImage, r: usize) -> RgbImage {
    use rayon::prelude::*;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return img.clone();
    }
    const BAND: usize = 256;
    let margin = 2 * r;
    let src = img.as_raw();
    let starts: Vec<usize> = (0..h).step_by(BAND).collect();
    let bands: Vec<Vec<u8>> = starts
        .par_iter()
        .map(|&y0| {
            let y1 = (y0 + BAND).min(h);
            let (s, e) = (y0.saturating_sub(margin), (y1 + margin).min(h));
            let hb = e - s;
            let mut cur: Vec<f32> = src[s * w * 3..e * w * 3]
                .iter()
                .map(|&v| v as f32)
                .collect();
            let mut tmp = vec![0f32; cur.len()];
            let mut col = vec![0f32; w * 3];
            for _ in 0..2 {
                // horizontal: sliding window along each row
                for y in 0..hb {
                    for c in 0..3 {
                        let at = |x: usize| (y * w + x) * 3 + c;
                        let (mut sum, mut lo, mut hi) = (0f32, 0usize, 0usize);
                        for x in 0..w {
                            let (nlo, nhi) = (x.saturating_sub(r), (x + r + 1).min(w));
                            while hi < nhi {
                                sum += cur[at(hi)];
                                hi += 1;
                            }
                            while lo < nlo {
                                sum -= cur[at(lo)];
                                lo += 1;
                            }
                            tmp[at(x)] = sum / (hi - lo) as f32;
                        }
                    }
                }
                // vertical: running per-column sums, whole rows added and dropped (sequential memory)
                col.iter_mut().for_each(|v| *v = 0.0);
                let (mut lo, mut hi) = (0usize, 0usize);
                for y in 0..hb {
                    let (nlo, nhi) = (y.saturating_sub(r), (y + r + 1).min(hb));
                    while hi < nhi {
                        for (c, v) in col.iter_mut().zip(&tmp[hi * w * 3..(hi + 1) * w * 3]) {
                            *c += v;
                        }
                        hi += 1;
                    }
                    while lo < nlo {
                        for (c, v) in col.iter_mut().zip(&tmp[lo * w * 3..(lo + 1) * w * 3]) {
                            *c -= v;
                        }
                        lo += 1;
                    }
                    let n = (hi - lo) as f32;
                    for (o, c) in cur[y * w * 3..(y + 1) * w * 3].iter_mut().zip(&col) {
                        *o = c / n;
                    }
                }
            }
            cur[(y0 - s) * w * 3..(y1 - s) * w * 3]
                .iter()
                .map(|v| v.round().clamp(0.0, 255.0) as u8)
                .collect()
        })
        .collect();
    RgbImage::from_raw(w as u32, h as u32, bands.concat()).expect("size matches")
}

/// Unsharp mask: `out = img + amount·(img − blur(img, ≈0.3 mm))`. `amount` ≤ 0 → unchanged.
pub fn sharpen(img: &RgbImage, dpi: u32, amount: f32) -> RgbImage {
    if amount.is_nan() || amount <= 0.0 || img.width() < 3 || img.height() < 3 {
        return img.clone();
    }
    let r = ((0.3 / 25.4 * dpi as f32).round() as usize).max(1);
    let b = blur(img, r);
    let mut out = img.clone();
    for ((o, &s), &bl) in out.as_mut().iter_mut().zip(img.as_raw()).zip(b.as_raw()) {
        *o = (s as f32 + amount * (s as f32 - bl as f32))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// Replace pixels that are strongly of the chosen primary with `paper`.
pub fn dropout(img: &mut RgbImage, which: Dropout, paper: Rgb<u8>) {
    if which == Dropout::Off {
        return;
    }
    for p in img.pixels_mut() {
        let [r, g, b] = p.0.map(|v| v as i32);
        let hit = match which {
            Dropout::Red => r > 110 && r - g.max(b) > 50,
            Dropout::Green => g > 110 && g - r.max(b) > 40,
            Dropout::Blue => b > 110 && b - r.max(g) > 50,
            Dropout::Off => false,
        };
        if hit {
            *p = paper;
        }
    }
}

/// Remove show-through (print from the other side of thin paper): the lighting is
/// levelled first (`target` becomes the paper colour), then every pixel that is
/// only slightly darker than the paper (≥ 60 % of its local brightness, fading in up
/// to 88 %) is pulled towards paper. Real ink is far darker and untouched.
pub fn show_through(img: &RgbImage, dpi: u32, paper: Rgb<u8>) -> RgbImage {
    if img.width() < 9 || img.height() < 9 {
        return img.clone();
    }
    let mut out = crate::transform::normalize_background(img, dpi, paper);
    let pl = luma(&paper.0).max(1) as f32;
    for p in out.pixels_mut() {
        let v = luma(&p.0) as f32 / pl;
        let t = ((v - 0.60) / 0.28).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        for c in 0..3 {
            p.0[c] = (p.0[c] as f32 + (paper.0[c] as f32 - p.0[c] as f32) * t)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Smooth halftone screens (printed photographs) without touching text: tiles of
/// ≈ 2 mm that are mostly mid-tone *and* busy at the pixel scale are replaced by a
/// lightly blurred copy. Returns how many tiles were smoothed.
pub fn descreen(img: &mut RgbImage, dpi: u32) -> usize {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let tile = ((2.0 / 25.4 * dpi as f32).round() as usize).max(6);
    if w < tile * 2 || h < tile * 2 {
        return 0;
    }
    let g: Vec<u8> = img
        .as_raw()
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| luma(p) as u8)
        .collect();
    let (tw, th) = (w / tile, h / tile);
    let mut flagged = vec![false; tw * th];
    for ty in 0..th {
        for tx in 0..tw {
            let (mut mid, mut busy, mut n) = (0u32, 0u32, 0u32);
            for y in ty * tile..(ty + 1) * tile {
                for x in tx * tile..(tx + 1) * tile {
                    let v = g[y * w + x];
                    mid += (60..=200).contains(&v) as u32;
                    if x + 1 < w {
                        busy += (v as i32 - g[y * w + x + 1] as i32).unsigned_abs();
                    }
                    n += 1;
                }
            }
            flagged[ty * tw + tx] = mid * 2 >= n && busy / n >= 12;
        }
    }
    let count = flagged.iter().filter(|&&f| f).count();
    if count == 0 {
        return 0;
    }
    let r = ((0.12 / 25.4 * dpi as f32).round() as usize).max(1);
    let soft = blur(img, r);
    for ty in 0..th {
        for tx in 0..tw {
            if !flagged[ty * tw + tx] {
                continue;
            }
            for y in ty * tile..(ty + 1) * tile {
                for x in tx * tile..(tx + 1) * tile {
                    img.put_pixel(x as u32, y as u32, *soft.get_pixel(x as u32, y as u32));
                }
            }
        }
    }
    count
}

// ---------------------------------------------------------------------------
// connected components

#[derive(Debug, Clone, Copy)]
pub(crate) struct Comp {
    pub(crate) x0: u32,
    pub(crate) y0: u32,
    pub(crate) x1: u32, // inclusive
    pub(crate) y1: u32,
    pub(crate) area: u32,
}

/// 8-connected labelling of a `w×h` mask. Labels start at 1 (0 = background).
pub(crate) fn label(mask: &[u8], w: usize, h: usize) -> (Vec<u32>, Vec<Comp>) {
    let mut lab = vec![0u32; w * h];
    let mut comps: Vec<Comp> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 || lab[start] != 0 {
            continue;
        }
        let id = comps.len() as u32 + 1;
        let mut c = Comp {
            x0: u32::MAX,
            y0: u32::MAX,
            x1: 0,
            y1: 0,
            area: 0,
        };
        lab[start] = id;
        stack.push(start);
        while let Some(p) = stack.pop() {
            let (px, py) = (p % w, p / w);
            c.x0 = c.x0.min(px as u32);
            c.x1 = c.x1.max(px as u32);
            c.y0 = c.y0.min(py as u32);
            c.y1 = c.y1.max(py as u32);
            c.area += 1;
            for ny in py.saturating_sub(1)..(py + 2).min(h) {
                for nx in px.saturating_sub(1)..(px + 2).min(w) {
                    let q = ny * w + nx;
                    if mask[q] != 0 && lab[q] == 0 {
                        lab[q] = id;
                        stack.push(q);
                    }
                }
            }
        }
        comps.push(c);
    }
    (lab, comps)
}

fn small_gray(img: &RgbImage, f: u32) -> (Vec<u8>, usize, usize) {
    let s = crate::transform::downscale_rgb(img, f);
    let (w, h) = (s.width() as usize, s.height() as usize);
    let g = s
        .as_raw()
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| luma(p) as u8)
        .collect();
    (g, w, h)
}

/// Mean colour of the pixels on a ring around a box (skipping dark/hole pixels).
fn ring_colour(img: &RgbImage, x0: i64, y0: i64, x1: i64, y1: i64, margin: i64) -> Option<Rgb<u8>> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (mut acc, mut n) = ([0u64; 3], 0u64);
    let mut take = |x: i64, y: i64| {
        if x >= 0 && y >= 0 && x < w && y < h {
            let p = img.get_pixel(x as u32, y as u32).0;
            if luma(&p) >= 110 {
                for k in 0..3 {
                    acc[k] += p[k] as u64;
                }
                n += 1;
            }
        }
    };
    for x in (x0 - margin)..=(x1 + margin) {
        for m in 1..=margin.min(3) {
            take(x, y0 - margin + m - 1);
            take(x, y1 + margin - m + 1);
        }
    }
    for y in y0..=y1 {
        for m in 1..=margin.min(3) {
            take(x0 - margin + m - 1, y);
            take(x1 + margin - m + 1, y);
        }
    }
    (n >= 8).then(|| Rgb([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8]))
}

/// Erase round binder/punch holes near the page edge: dark, roughly circular blobs
/// 3–10 mm across (scanner backing showing through) are filled with the colour of
/// the paper around them.
pub fn remove_punch_holes(img: &mut RgbImage, dpi: u32) -> usize {
    if img.width() < 16 || img.height() < 16 {
        return 0;
    }
    let f = (dpi / 100).max(1);
    let (g, w, h) = small_gray(img, f);
    let sdpi = dpi as f32 / f as f32;
    let px = |mm: f32| mm / 25.4 * sdpi;
    let mask: Vec<u8> = g.iter().map(|&v| (v < 70) as u8).collect();
    let (_, comps) = label(&mask, w, h);
    let near = px(25.0);
    let mut n = 0;
    for c in comps {
        let (bw, bh) = ((c.x1 - c.x0 + 1) as f32, (c.y1 - c.y0 + 1) as f32);
        if bw < px(3.0) || bh < px(3.0) || bw > px(10.0) || bh > px(10.0) {
            continue;
        }
        let aspect = bw / bh;
        let fill = c.area as f32 / (bw * bh);
        if !(0.7..=1.4).contains(&aspect) || !(0.6..=0.95).contains(&fill) {
            continue;
        }
        let d_edge = (c.x0 as f32)
            .min(c.y0 as f32)
            .min((w as u32 - 1 - c.x1) as f32)
            .min((h as u32 - 1 - c.y1) as f32);
        if d_edge > near {
            continue;
        }
        // full-resolution box, grown by 1 mm
        let grow = (1.0 / 25.4 * dpi as f32).round() as i64 + f as i64;
        let (x0, y0) = (c.x0 as i64 * f as i64 - grow, c.y0 as i64 * f as i64 - grow);
        let (x1, y1) = (
            (c.x1 as i64 + 1) * f as i64 + grow,
            (c.y1 as i64 + 1) * f as i64 + grow,
        );
        let ring = (2.0 / 25.4 * dpi as f32).round() as i64 + 2;
        let Some(col) = ring_colour(img, x0, y0, x1, y1, ring) else {
            continue;
        };
        let (cx, cy) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
        let (rx, ry) = ((x1 - x0) as f32 / 2.0, (y1 - y0) as f32 / 2.0);
        for y in y0.max(0)..y1.min(img.height() as i64) {
            for x in x0.max(0)..x1.min(img.width() as i64) {
                let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
                if dx * dx + dy * dy <= 1.0 {
                    img.put_pixel(x as u32, y as u32, col);
                }
            }
        }
        n += 1;
    }
    n
}

/// Paint rectangles (mm from the top-left: x, y, width, height) with `paper`.
pub fn erase_zones(img: &mut RgbImage, dpi: u32, zones: &[[f32; 4]], paper: Rgb<u8>) {
    let k = dpi as f32 / 25.4;
    let (w, h) = (img.width() as f32, img.height() as f32);
    for z in zones.iter().take(64) {
        let (x0, y0) = ((z[0] * k).max(0.0).min(w), (z[1] * k).max(0.0).min(h));
        let (x1, y1) = (
            ((z[0] + z[2]) * k).max(0.0).min(w),
            ((z[1] + z[3]) * k).max(0.0).min(h),
        );
        if !(x1 > x0 && y1 > y0) {
            continue;
        }
        for y in y0.round() as u32..(y1.round() as u32).min(img.height()) {
            for x in x0.round() as u32..(x1.round() as u32).min(img.width()) {
                img.put_pixel(x, y, paper);
            }
        }
    }
}

/// `x,y,w,h;x,y,w,h` → zones (None on any malformed or non-finite entry).
pub fn parse_zones(s: &str) -> Option<Vec<[f32; 4]>> {
    let mut out = Vec::new();
    for part in s.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let v: Vec<f32> = part
            .split(',')
            .map(|x| x.trim().parse().ok())
            .collect::<Option<_>>()?;
        if v.len() != 4 || v.iter().any(|x| !x.is_finite()) || v[2] <= 0.0 || v[3] <= 0.0 {
            return None;
        }
        out.push([v[0], v[1], v[2], v[3]]);
    }
    (out.len() <= 64).then_some(out)
}

pub fn zones_to_string(z: &[[f32; 4]]) -> String {
    z.iter()
        .map(|r| format!("{},{},{},{}", r[0], r[1], r[2], r[3]))
        .collect::<Vec<_>>()
        .join(";")
}

fn ycbcr(p: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    (
        y,
        128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b,
        128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b,
    )
}

/// Morphological opening (erode then dilate, square window of radius `r`) of a 0/1 mask.
fn open_mask(m: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let pass = |src: &[u8], keep_all: bool| -> Vec<u8> {
        // separable min (keep_all) or max (!keep_all) filter
        let mut tmp = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
                let row = &src[y * w + a..=y * w + b];
                tmp[y * w + x] = if keep_all {
                    (row.len() == 2 * r + 1 && row.iter().all(|&v| v != 0)) as u8
                } else {
                    row.iter().any(|&v| v != 0) as u8
                };
            }
        }
        let mut out = vec![0u8; w * h];
        for x in 0..w {
            for y in 0..h {
                let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
                let col = (a..=b).map(|yy| tmp[yy * w + x]);
                out[y * w + x] = if keep_all {
                    (b - a == 2 * r && col.clone().all(|v| v != 0)) as u8
                } else {
                    col.clone().any(|v| v != 0) as u8
                };
            }
        }
        out
    };
    pass(&pass(m, true), false)
}

/// Square window max (`max`) or min filter of radius `r` over a `w×h` byte plane; the window is
/// cut at the border (so an erosion does not eat in from the image edge).
fn box_filter(src: &[u8], w: usize, h: usize, r: usize, max: bool) -> Vec<u8> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    // Two 1-D passes; the second runs on the transpose so both walk memory linearly.
    let t = transpose(&slide(src, w, h, r, max), w, h);
    transpose(&slide(&t, h, w, r, max), h, w)
}

fn transpose(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = src[y * w + x];
        }
    }
    out
}

/// Max/min over the window `[x-r, x+r]` (cut at the row ends) of every row, in O(w) per row
/// with a monotonic queue.
fn slide(src: &[u8], w: usize, h: usize, r: usize, max: bool) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    let mut q: Vec<usize> = Vec::with_capacity(w.min(2 * r + 2));
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let o = &mut out[y * w..(y + 1) * w];
        q.clear();
        let mut head = 0usize;
        let mut next = 0usize; // next index to push
        for (x, ox) in o.iter_mut().enumerate() {
            let hi = (x + r).min(w - 1);
            while next <= hi {
                let v = row[next];
                while q.len() > head && {
                    let l = row[q[q.len() - 1]];
                    if max { l <= v } else { l >= v }
                } {
                    q.pop();
                }
                q.push(next);
                next += 1;
            }
            let lo = x.saturating_sub(r);
            while q[head] < lo {
                head += 1;
            }
            *ox = row[q[head]];
        }
    }
    out
}

/// Paint what lies outside the paper with `paper`. A page photographed on a bag or a
/// tablecloth keeps, once levelled and cut, wedges of that background in its corners; layout
/// analysis and OCR take them for print and the content box grows over them. The paper is the
/// smooth, bright, colourless surface (text removed by a closing; the lace of a bag has
/// texture and colour); its largest blob, cleaned of thin leaks by an erosion, gives a convex
/// outline which is grown back by the same amount. Returns false (image untouched) when no
/// page-sized paper blob is found.
pub fn trim_to_paper(img: &mut RgbImage, dpi: u32, paper: Rgb<u8>) -> bool {
    if img.width() < 64 || img.height() < 64 {
        return false;
    }
    let f = (dpi / 75).max(1);
    let small = crate::transform::downscale_rgb(img, f);
    let (w, h) = (small.width() as usize, small.height() as usize);
    if w < 40 || h < 40 {
        return false;
    }
    let raw = small.as_raw();
    let plane = |c: usize| -> Vec<u8> { (0..w * h).map(|i| raw[i * 3 + c]).collect() };
    let gray: Vec<u8> = (0..w * h)
        .map(|i| ((raw[i * 3] as u32 * 30 + raw[i * 3 + 1] as u32 * 59 + raw[i * 3 + 2] as u32 * 11) / 100) as u8)
        .collect();
    let close = |p: &[u8], r: usize| box_filter(&box_filter(p, w, h, r, true), w, h, r, false);
    let c = close(&gray, 4);
    let (cr, cg, cb) = (close(&plane(0), 4), close(&plane(1), 4), close(&plane(2), 4));
    let (dmax, dmin) = (box_filter(&c, w, h, 4, true), box_filter(&c, w, h, 4, false));
    let mut sorted = c.clone();
    sorted.sort_unstable();
    let p90 = sorted[sorted.len() * 9 / 10] as f32;
    let mut mask: Vec<u8> = (0..w * h)
        .map(|i| {
            let tex = dmax[i] as i32 - dmin[i] as i32;
            let hi = cr[i].max(cg[i]).max(cb[i]) as i32;
            let lo = cr[i].min(cg[i]).min(cb[i]) as i32;
            (c[i] as f32 > 0.8 * p90 && tex < 22 && hi - lo < 45) as u8
        })
        .collect();
    let open = |m: &[u8], r: usize| box_filter(&box_filter(m, w, h, r, false), w, h, r, true);
    let closeb = |m: &[u8], r: usize| box_filter(&box_filter(m, w, h, r, true), w, h, r, false);
    mask = open(&mask, 3);
    mask = closeb(&mask, 7);
    mask = open(&mask, 15);
    let grow = 21usize;
    let eroded = box_filter(&mask, w, h, grow, false);
    let (lab, comps) = label(&eroded, w, h);
    let Some((bi, best)) = comps.iter().enumerate().max_by_key(|(_, c)| c.area) else {
        return false;
    };
    if (best.area as usize) * 100 < w * h * 20 {
        return false;
    }
    let _ = bi;
    // Every big blob counts as paper: a dark gutter or a banner can split the sheet in two,
    // and painting the smaller piece white would wipe a page.
    let keep: Vec<bool> = std::iter::once(false)
        .chain(
            comps
                .iter()
                .map(|c| c.area * 4 >= best.area && (c.area as usize) * 100 >= w * h * 3),
        )
        .collect();
    // convex hull of the blob grown by `grow` (Minkowski sum with a square = hull of the
    // corners of squares around its row ends)
    let mut pts: Vec<(i64, i64)> = Vec::new();
    for y in 0..h {
        let (mut a, mut b) = (usize::MAX, 0usize);
        for x in 0..w {
            if keep.get(lab[y * w + x] as usize).copied().unwrap_or(false) {
                a = a.min(x);
                b = b.max(x);
            }
        }
        if a != usize::MAX {
            for x in [a, b] {
                for (dx, dy) in [(-1i64, -1i64), (1, -1), (1, 1), (-1, 1)] {
                    pts.push((x as i64 + dx * grow as i64, y as i64 + dy * grow as i64));
                }
            }
        }
    }
    pts.sort_unstable();
    pts.dedup();
    let cross = |o: (i64, i64), a: (i64, i64), b: (i64, i64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut hull: Vec<(i64, i64)> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let it: Box<dyn Iterator<Item = &(i64, i64)>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in it {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return false;
    }
    // x-range of the hull on every small row
    let mut lo = vec![f64::INFINITY; h];
    let mut hi = vec![f64::NEG_INFINITY; h];
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let (y0, y1) = (a.1.min(b.1).max(0), a.1.max(b.1).min(h as i64 - 1));
        for y in y0..=y1 {
            let x = if a.1 == b.1 {
                a.0.min(b.0) as f64
            } else {
                a.0 as f64 + (b.0 - a.0) as f64 * (y - a.1) as f64 / (b.1 - a.1) as f64
            };
            lo[y as usize] = lo[y as usize].min(x);
            hi[y as usize] = hi[y as usize].max(x);
            if a.1 == b.1 {
                hi[y as usize] = hi[y as usize].max(a.0.max(b.0) as f64);
            }
        }
    }
    let (iw, ih) = (img.width() as usize, img.height() as usize);
    for y in 0..ih {
        let sy = (y / f as usize).min(h - 1);
        let (a, b) = (lo[sy], hi[sy]);
        for x in 0..iw {
            let sx = x as f64 / f as f64;
            if !(sx >= a && sx <= b) {
                img.put_pixel(x as u32, y as u32, paper);
            }
        }
    }
    true
}

/// Remove fingers holding the page: skin-coloured regions (judged relative to the
/// paper colour, so tinted paper does not fool it) that reach the image border and
/// are at least 8 mm long. They are painted with `paper`.
pub fn remove_fingers(img: &mut RgbImage, dpi: u32, paper: Rgb<u8>) -> usize {
    if img.width() < 16 || img.height() < 16 {
        return 0;
    }
    let f = (dpi / 100).max(1);
    let small = crate::transform::downscale_rgb(img, f);
    let (w, h) = (small.width() as usize, small.height() as usize);
    let (py, _, pcr) = ycbcr(paper.0);
    let mask: Vec<u8> = small
        .as_raw()
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let (y, cb, cr) = ycbcr([p[0], p[1], p[2]]);
            ((77.0..=127.0).contains(&cb)
                && cr >= pcr + 9.0
                && cr <= 180.0
                && y >= 70.0
                && y <= 0.97 * py) as u8
        })
        .collect();
    let sdpi = dpi as f32 / f as f32;
    // A finger is a solid blob; warm shadowed paper around printed text is full of holes (the
    // ink is not skin-coloured). Opening with a ~2.5 mm square keeps the first and drops the
    // second, so a thumb in the corner no longer takes the text lines next to it along.
    let mask = open_mask(&mask, w, h, ((2.5 / 25.4 * sdpi).round() as usize).max(1));
    let (lab, comps) = label(&mask, w, h);
    let min_len = 8.0 / 25.4 * sdpi;
    let min_area = (40.0 / 645.16) * sdpi * sdpi; // 40 mm²
    let mut hit = vec![false; comps.len() + 1];
    let mut n = 0;
    for (i, c) in comps.iter().enumerate() {
        let touches = c.x0 == 0 || c.y0 == 0 || c.x1 as usize == w - 1 || c.y1 as usize == h - 1;
        let len = ((c.x1 - c.x0 + 1) as f32).max((c.y1 - c.y0 + 1) as f32);
        // a hand is a small part of the picture; a "finger" the size of a quarter page is
        // warm shadow or a cover, and wiping it would take the page with it
        let small_enough = (c.area as usize) * 8 <= w * h;
        if touches && len >= min_len && c.area as f32 >= min_area && small_enough {
            hit[i + 1] = true;
            n += 1;
        }
    }
    if n == 0 {
        return 0;
    }
    // A finger holds no print. Between the first and last skin pixel of every row, a real
    // finger is solid skin; text lines in that span (dark pixels that are not skin) mean the
    // region is page, not hand: leave it alone.
    {
        let mut spans: std::collections::HashMap<u32, Vec<(u32, u32)>> = Default::default();
        for (id, on) in hit.iter().enumerate() {
            if *on {
                spans.insert(id as u32, vec![(u32::MAX, 0); h]);
            }
        }
        for y in 0..h {
            for x in 0..w {
                let id = lab[y * w + x];
                if id != 0 {
                    if let Some(sp) = spans.get_mut(&id) {
                        let e = &mut sp[y];
                        e.0 = e.0.min(x as u32);
                        e.1 = e.1.max(x as u32);
                    }
                }
            }
        }
        let raw = small.as_raw();
        for (id, sp) in &spans {
            let (mut total, mut ink) = (0u64, 0u64);
            for (y, &(a, b)) in sp.iter().enumerate() {
                if a == u32::MAX {
                    continue;
                }
                for x in a as usize..=b as usize {
                    total += 1;
                    if lab[y * w + x] == *id {
                        continue;
                    }
                    let q = (y * w + x) * 3;
                    let lum = ycbcr([raw[q], raw[q + 1], raw[q + 2]]).0;
                    if lum < 0.6 * py {
                        ink += 1;
                    }
                }
            }
            if total > 0 && ink as f64 > 0.02 * total as f64 {
                hit[*id as usize] = false;
                n -= 1;
            }
        }
        if n == 0 {
            return 0;
        }
    }
    // grow by one small pixel so the soft edge of the finger goes too
    let grown = |sx: i64, sy: i64| -> bool {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y) = (sx + dx, sy + dy);
                if x >= 0
                    && y >= 0
                    && (x as usize) < w
                    && (y as usize) < h
                    && hit[lab[y as usize * w + x as usize] as usize]
                {
                    return true;
                }
            }
        }
        false
    };
    let (iw, ih) = (img.width() as usize, img.height() as usize);
    for y in 0..ih {
        let sy = (y / f as usize).min(h - 1) as i64;
        for x in 0..iw {
            if grown((x / f as usize).min(w - 1) as i64, sy) {
                img.put_pixel(x as u32, y as u32, paper);
            }
        }
    }
    n
}

/// Erase ruled lines (form rules, table borders, scanner streaks) from a 1-bit page.
/// A line is a horizontal or vertical ink run of at least 25 mm that is at most
/// 1.2 mm thick; pixels that are also part of a thicker structure (a glyph stem
/// crossing the rule) stay. Returns the number of pixels erased.
pub fn remove_lines(bw: &mut BitImage, dpi: u32) -> u64 {
    let (w, h) = (bw.w as usize, bw.h as usize);
    if w == 0 || h == 0 {
        return 0;
    }
    let min_run = ((25.0 / 25.4 * dpi as f32).round() as usize).max(8);
    let max_thick = ((1.2 / 25.4 * dpi as f32).round() as usize).max(2);
    let data = &bw.data;
    let thick_v = |x: usize, y: usize| -> usize {
        let (mut a, mut b) = (y, y);
        while a > 0 && data[(a - 1) * w + x] != 0 && y - a <= max_thick {
            a -= 1;
        }
        while b + 1 < h && data[(b + 1) * w + x] != 0 && b - y <= max_thick {
            b += 1;
        }
        b - a + 1
    };
    let thick_h = |x: usize, y: usize| -> usize {
        let (mut a, mut b) = (x, x);
        while a > 0 && data[y * w + a - 1] != 0 && x - a <= max_thick {
            a -= 1;
        }
        while b + 1 < w && data[y * w + b + 1] != 0 && b - x <= max_thick {
            b += 1;
        }
        b - a + 1
    };
    let mut kill: Vec<usize> = Vec::new();
    // horizontal runs
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if data[y * w + x] == 0 {
                x += 1;
                continue;
            }
            let s = x;
            while x < w && data[y * w + x] != 0 {
                x += 1;
            }
            if x - s >= min_run {
                for xx in s..x {
                    if thick_v(xx, y) <= max_thick {
                        kill.push(y * w + xx);
                    }
                }
            }
        }
    }
    // vertical runs
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if data[y * w + x] == 0 {
                y += 1;
                continue;
            }
            let s = y;
            while y < h && data[y * w + x] != 0 {
                y += 1;
            }
            if y - s >= min_run {
                for yy in s..y {
                    if thick_h(x, yy) <= max_thick {
                        kill.push(yy * w + x);
                    }
                }
            }
        }
    }
    kill.sort_unstable();
    kill.dedup();
    for &i in &kill {
        bw.data[i] = 0;
    }
    kill.len() as u64
}

#[cfg(test)]
mod tests {
    /// Blur every channel with two box passes per axis (≈ Gaussian), radius `r` px.
    fn blur_reference(img: &RgbImage, r: usize) -> RgbImage {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let mut cur: Vec<f32> = img.as_raw().iter().map(|&v| v as f32).collect();
        let mut tmp = vec![0f32; cur.len()];
        for _ in 0..2 {
            // horizontal
            for y in 0..h {
                for c in 0..3 {
                    let at = |x: usize| (y * w + x) * 3 + c;
                    let mut sum = 0f32;
                    let mut lo = 0usize;
                    let mut hi = 0usize; // window [lo, hi)
                    for x in 0..w {
                        let (nlo, nhi) = (x.saturating_sub(r), (x + r + 1).min(w));
                        while hi < nhi {
                            sum += cur[at(hi)];
                            hi += 1;
                        }
                        while lo < nlo {
                            sum -= cur[at(lo)];
                            lo += 1;
                        }
                        tmp[at(x)] = sum / (hi - lo) as f32;
                    }
                }
            }
            // vertical
            for x in 0..w {
                for c in 0..3 {
                    let at = |y: usize| (y * w + x) * 3 + c;
                    let mut sum = 0f32;
                    let mut lo = 0usize;
                    let mut hi = 0usize;
                    for y in 0..h {
                        let (nlo, nhi) = (y.saturating_sub(r), (y + r + 1).min(h));
                        while hi < nhi {
                            sum += tmp[at(hi)];
                            hi += 1;
                        }
                        while lo < nlo {
                            sum -= tmp[at(lo)];
                            lo += 1;
                        }
                        cur[at(y)] = sum / (hi - lo) as f32;
                    }
                }
            }
        }
        let mut out = RgbImage::new(w as u32, h as u32);
        for (o, &v) in out.as_mut().iter_mut().zip(&cur) {
            *o = v.round().clamp(0.0, 255.0) as u8;
        }
        out
    }

    #[test]
    fn banded_blur_equals_whole_image_blur() {
        let (w, h) = (97u32, 613u32); // several bands, odd sizes
        let img = RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x * 7 + y * 3) as u8, (x ^ y) as u8, ((x * y) % 251) as u8])
        });
        for r in [1usize, 3, 9, 40] {
            let (a, b) = (super::blur(&img, r), blur_reference(&img, r));
            // identical up to f32 rounding: the running sums start at the band, not at row 0
            let worst = a
                .as_raw()
                .iter()
                .zip(b.as_raw())
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap();
            let off = a
                .as_raw()
                .iter()
                .zip(b.as_raw())
                .filter(|(x, y)| x != y)
                .count();
            assert!(worst <= 1, "radius {r}: differs by {worst}");
            assert!(
                off * 1000 < a.as_raw().len(),
                "radius {r}: {off} samples differ"
            );
        }
        let thin = RgbImage::from_fn(5, 3, |x, y| image::Rgb([(x * 50) as u8, (y * 80) as u8, 7]));
        assert_eq!(super::blur(&thin, 2), blur_reference(&thin, 2));
    }

    use super::*;

    #[test]
    fn erase_zones_paint_only_their_rectangle() {
        let mut img = RgbImage::from_pixel(254, 254, Rgb([10, 10, 10]));
        erase_zones(
            &mut img,
            254,
            &[
                [2.0, 3.0, 4.0, 5.0],
                [-10.0, -10.0, 5.0, 5.0],
                [500.0, 0.0, 9.0, 9.0],
            ],
            Rgb([250, 250, 250]),
        );
        // 254 dpi → 10 px per mm: x 20..60, y 30..80
        assert_eq!(img.get_pixel(20, 30).0, [250; 3]);
        assert_eq!(img.get_pixel(59, 79).0, [250; 3]);
        assert_eq!(img.get_pixel(60, 79).0, [10; 3]);
        assert_eq!(img.get_pixel(19, 30).0, [10; 3]);
        assert_eq!(img.get_pixel(0, 0).0, [10; 3]);
    }

    #[test]
    fn zone_text_roundtrip_and_rejection() {
        let z = parse_zones("1,2,3,4; 5.5,6,7,8").unwrap();
        assert_eq!(z, vec![[1.0, 2.0, 3.0, 4.0], [5.5, 6.0, 7.0, 8.0]]);
        assert_eq!(parse_zones(&zones_to_string(&z)).unwrap(), z);
        assert_eq!(parse_zones("").unwrap().len(), 0);
        for bad in ["1,2,3", "1,2,0,4", "a,b,c,d", "1,2,3,nan", "1,2,3,4,5"] {
            assert!(parse_zones(bad).is_none(), "{bad}");
        }
    }

    fn page(w: u32, h: u32, c: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(c))
    }

    #[test]
    fn negative_is_inverted_and_normal_page_is_not() {
        let mut neg = page(200, 200, [20, 20, 20]);
        for y in 80..90 {
            for x in 20..180 {
                neg.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
        for y in 120..180 {
            for x in 20..180 {
                neg.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
        assert!(invert_if_negative(&mut neg));
        assert!(neg.get_pixel(5, 5).0[0] > 200);
        let mut pos = page(200, 200, [240, 240, 240]);
        assert!(!invert_if_negative(&mut pos));
        assert_eq!(pos.get_pixel(5, 5).0, [240, 240, 240]);
        let mut dark = page(200, 200, [15, 15, 15]);
        assert!(
            !invert_if_negative(&mut dark),
            "uniform dark page has no bright tail"
        );
    }

    #[test]
    fn levels_stretch_a_washed_out_page() {
        let mut p = page(100, 100, [200, 200, 200]);
        for y in 0..30 {
            for x in 0..100 {
                p.put_pixel(x, y, Rgb([120, 120, 120]));
            }
        }
        auto_levels(&mut p);
        assert!(p.get_pixel(5, 5).0[0] <= 5);
        assert!(p.get_pixel(5, 80).0[0] >= 250);
        let mut flat = page(50, 50, [100, 100, 100]);
        auto_levels(&mut flat);
        assert_eq!(flat.get_pixel(1, 1).0, [100, 100, 100]);
    }

    #[test]
    fn sharpen_raises_edge_contrast_and_keeps_flat_areas() {
        let mut p = page(40, 40, [128, 128, 128]);
        for y in 0..40 {
            for x in 20..40 {
                p.put_pixel(x, y, Rgb([200, 200, 200]));
            }
        }
        let s = sharpen(&p, 300, 1.0);
        assert!(s.get_pixel(21, 20).0[0] > 200, "bright side overshoots");
        assert!(s.get_pixel(18, 20).0[0] < 128, "dark side undershoots");
        assert_eq!(s.get_pixel(2, 2).0, [128, 128, 128]);
        assert_eq!(sharpen(&p, 300, 0.0).as_raw(), p.as_raw());
        assert_eq!(sharpen(&p, 300, f32::NAN).as_raw(), p.as_raw());
    }

    #[test]
    fn dropout_removes_only_the_chosen_colour() {
        let mut p = page(10, 1, [250, 250, 250]);
        p.put_pixel(1, 0, Rgb([220, 40, 40]));
        p.put_pixel(2, 0, Rgb([20, 20, 20]));
        p.put_pixel(3, 0, Rgb([40, 40, 220]));
        dropout(&mut p, Dropout::Red, Rgb([255, 255, 255]));
        assert_eq!(p.get_pixel(1, 0).0, [255, 255, 255]);
        assert_eq!(p.get_pixel(2, 0).0, [20, 20, 20]);
        assert_eq!(p.get_pixel(3, 0).0, [40, 40, 220]);
    }

    #[test]
    fn punch_hole_near_the_edge_is_filled_but_a_dot_in_text_is_not() {
        let dpi = 200;
        let mut p = page(800, 1000, [240, 235, 225]);
        let (cx, cy, r) = (60i32, 400i32, 28i32); // ≈ 7 mm hole, 8 mm from the edge
        for y in 0..1000 {
            for x in 0..800 {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    p.put_pixel(x as u32, y as u32, Rgb([10, 10, 10]));
                }
            }
        }
        // same disc deep inside the page
        for y in 0..1000 {
            for x in 0..800 {
                if (x - 400i32).pow(2) + (y - 500i32).pow(2) <= r * r {
                    p.put_pixel(x as u32, y as u32, Rgb([10, 10, 10]));
                }
            }
        }
        assert_eq!(remove_punch_holes(&mut p, dpi), 1);
        assert_eq!(p.get_pixel(60, 400).0, [240, 235, 225]);
        assert_eq!(p.get_pixel(400, 500).0, [10, 10, 10]);
    }

    #[test]
    fn warm_shadowed_paper_with_text_lines_is_not_taken_for_a_finger() {
        // regression: a phone photo's warm shadow at the page foot touched the border and was
        // wiped together with the text lines in it (text is not skin-coloured: holes in the mask)
        let dpi = 150;
        let paper = Rgb([235, 235, 230]);
        let mut p = RgbImage::from_pixel(600, 800, paper);
        let warm = Rgb([205, 150, 125]);
        for y in 600..800 {
            for x in 0..600 {
                p.put_pixel(x, y, warm);
            }
        }
        for y in (610..790).step_by(14) {
            for yy in y..y + 5 {
                for x in 20..580 {
                    p.put_pixel(x, yy, Rgb([20, 20, 20]));
                }
            }
        }
        assert_eq!(remove_fingers(&mut p, dpi, paper), 0);
        assert_eq!(p.get_pixel(300, 612).0, [20, 20, 20]);
    }

    #[test]
    fn textured_background_beside_the_paper_is_painted_over_but_the_page_stays() {
        let paper = Rgb([240, 240, 236]);
        let (w, h) = (1200u32, 1600u32);
        let mut p = RgbImage::from_pixel(w, h, paper);
        // a bag: a bluish checkerboard strip along the left edge and the top
        for y in 0..h {
            for x in 0..w {
                if (x < 220 || y < 140) && ((x / 40 + y / 40) % 2 == 0) {
                    p.put_pixel(x, y, Rgb([90, 80, 140]));
                } else if x < 220 || y < 140 {
                    p.put_pixel(x, y, Rgb([210, 205, 235]));
                }
            }
        }
        // text lines on the page
        for y in (300..1500).step_by(40) {
            for yy in y..y + 8 {
                for x in 320..1100 {
                    p.put_pixel(x, yy, Rgb([30, 30, 30]));
                }
            }
        }
        assert!(trim_to_paper(&mut p, 300, paper));
        assert_eq!(p.get_pixel(60, 800).0, paper.0, "bag at the left is gone");
        assert_eq!(p.get_pixel(600, 40).0, paper.0, "bag at the top is gone");
        assert_eq!(p.get_pixel(500, 303).0, [30, 30, 30], "text stays");
        assert_eq!(p.get_pixel(700, 1000).0, paper.0);
    }

    #[test]
    fn a_page_with_no_background_is_left_alone() {
        let paper = Rgb([240, 240, 236]);
        let mut p = RgbImage::from_pixel(1200, 1600, paper);
        for y in (200..1400).step_by(40) {
            for yy in y..y + 8 {
                for x in 100..1100 {
                    p.put_pixel(x, yy, Rgb([30, 30, 30]));
                }
            }
        }
        let before = p.clone();
        trim_to_paper(&mut p, 300, paper);
        assert_eq!(p, before);
    }

    #[test]
    fn finger_at_the_border_is_wiped_but_an_interior_patch_stays() {
        let dpi = 150;
        let paper = Rgb([235, 235, 230]);
        let mut p = RgbImage::from_pixel(600, 800, paper);
        let skin = Rgb([205, 150, 125]);
        for y in 300..420 {
            for x in 0..110 {
                p.put_pixel(x, y, skin);
            }
        }
        for y in 500..620 {
            for x in 300..410 {
                p.put_pixel(x, y, skin);
            }
        }
        assert_eq!(remove_fingers(&mut p, dpi, paper), 1);
        assert_eq!(p.get_pixel(40, 350).0, paper.0);
        assert_eq!(p.get_pixel(350, 560).0, skin.0);
    }

    #[test]
    fn ruled_lines_go_but_text_strokes_stay() {
        let dpi = 200;
        let mut b = BitImage::new(900, 300);
        for x in 20..880 {
            b.set(x, 100, true); // 1-px rule
        }
        for y in 20..280 {
            b.set(50, y, true); // vertical rule
        }
        // a thick block (picture/heading) must stay
        for y in 150..190 {
            for x in 200..600 {
                b.set(x, y, true);
            }
        }
        // a glyph stem crossing the rule
        for y in 90..112 {
            b.set(400, y, true);
            b.set(401, y, true);
            b.set(402, y, true);
        }
        let removed = remove_lines(&mut b, dpi);
        assert!(removed > 800);
        assert!(!b.get(100, 100) && !b.get(50, 30));
        assert!(b.get(300, 170), "thick block stays");
        assert!(b.get(401, 95) && b.get(401, 105), "stem stays");
    }

    #[test]
    fn show_through_fades_ghosts_but_keeps_ink() {
        let mut p = page(200, 200, [240, 240, 235]);
        for y in 50..60 {
            for x in 20..180 {
                p.put_pixel(x, y, Rgb([205, 205, 200])); // ghost from the other side
            }
        }
        for y in 100..110 {
            for x in 20..180 {
                p.put_pixel(x, y, Rgb([30, 30, 30])); // real ink
            }
        }
        let out = show_through(&p, 200, Rgb([240, 240, 235]));
        assert!(
            out.get_pixel(100, 55).0[0] >= 232,
            "ghost gone: {:?}",
            out.get_pixel(100, 55)
        );
        assert!(out.get_pixel(100, 105).0[0] <= 60, "ink stays");
    }

    #[test]
    fn descreen_smooths_a_halftone_patch_only() {
        let mut p = page(300, 300, [250, 250, 250]);
        for y in 100..200 {
            for x in 100..200 {
                let v = if (x + y) % 2 == 0 { 90 } else { 190 };
                p.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
        for y in 20..30 {
            for x in 20..200 {
                p.put_pixel(x, y, Rgb([0, 0, 0])); // text-like bar
            }
        }
        let n = descreen(&mut p, 150);
        assert!(n > 10);
        let v = p.get_pixel(150, 150).0[0];
        let w = p.get_pixel(151, 150).0[0];
        assert!(
            (v as i32 - w as i32).abs() < 40,
            "checkerboard smoothed: {v} {w}"
        );
        assert_eq!(p.get_pixel(50, 25).0, [0, 0, 0]);
    }

    #[test]
    fn labelling_counts_components() {
        let mut m = vec![0u8; 100];
        for i in [11, 12, 21, 55, 56, 66] {
            m[i] = 1;
        }
        let (_, c) = label(&m, 10, 10);
        assert_eq!(c.len(), 2);
        assert_eq!(c.iter().map(|c| c.area).sum::<u32>(), 6);
    }

    #[test]
    fn degenerate_images_do_not_panic() {
        for (w, h) in [(0, 0), (1, 1), (3, 200), (200, 3)] {
            let mut p = page(w, h, [100, 100, 100]);
            invert_if_negative(&mut p);
            auto_levels(&mut p);
            remove_punch_holes(&mut p, 300);
            remove_fingers(&mut p, 300, Rgb([255, 255, 255]));
            dropout(&mut p, Dropout::Blue, Rgb([255, 255, 255]));
            let _ = sharpen(&p, 300, 1.0);
            let mut b = BitImage::new(w, h);
            remove_lines(&mut b, 300);
        }
    }

    #[test]
    fn a_pale_sheet_on_a_dark_scanner_lid_is_not_taken_for_a_negative() {
        // 60 % black lid, 40 % light paper (with a little text)
        let mut img = RgbImage::from_pixel(100, 100, image::Rgb([15, 15, 15]));
        for y in 0..100 {
            for x in 60..100 {
                let v = if (x + y) % 17 == 0 { 40 } else { 225 };
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        assert!(!invert_if_negative(&mut img));
        // light text on a dark page still is
        let mut neg = RgbImage::from_pixel(100, 100, image::Rgb([20, 20, 20]));
        for y in (10..90).step_by(8) {
            for x in 10..90 {
                if (x / 3) % 2 == 0 {
                    neg.put_pixel(x, y, image::Rgb([230, 230, 230]));
                }
            }
        }
        assert!(invert_if_negative(&mut neg));
    }
}

#[cfg(test)]
mod degenerate_tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn fill(w: u32, h: u32, c: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(c))
    }

    fn noise(w: u32, h: u32) -> RgbImage {
        let mut s = 12345u32;
        RgbImage::from_fn(w, h, |_, _| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (s >> 24) as u8;
            Rgb([v, v ^ 0x55, v.wrapping_add(90)])
        })
    }

    #[test]
    fn trim_and_fingers_survive_degenerate_images() {
        let paper = Rgb([240, 240, 235]);
        let mut cases: Vec<RgbImage> = vec![
            fill(1, 1, [255, 255, 255]),
            fill(1, 500, [255, 255, 255]),
            fill(500, 1, [0, 0, 0]),
            fill(2, 2, [10, 10, 10]),
            fill(300, 400, [255, 255, 255]),
            fill(300, 400, [0, 0, 0]),
            fill(300, 400, [200, 120, 90]),
            noise(300, 400),
            noise(37, 11),
        ];
        let mut half = fill(300, 400, [255, 255, 255]);
        for y in 0..400 {
            for x in 0..150 {
                half.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        cases.push(half);
        for (i, c) in cases.iter().enumerate() {
            for dpi in [30u32, 75, 150, 300, 600] {
                let mut a = c.clone();
                let _ = trim_to_paper(&mut a, dpi, paper);
                assert_eq!((a.width(), a.height()), (c.width(), c.height()), "case {i}");
                let mut b = c.clone();
                let _ = remove_fingers(&mut b, dpi, paper);
                assert_eq!((b.width(), b.height()), (c.width(), c.height()), "case {i}");
            }
        }
    }

    #[test]
    fn an_all_black_or_all_white_image_is_never_painted_over() {
        let paper = Rgb([240, 240, 235]);
        let mut w = fill(300, 400, [255, 255, 255]);
        let before = w.clone();
        let _ = trim_to_paper(&mut w, 300, paper);
        assert_eq!(w, before, "a plain white page must stay as it is");
    }
}

#[cfg(test)]
mod big_image_tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn a_6000_by_8000_page_is_trimmed_in_reasonable_time() {
        let mut img = RgbImage::from_pixel(6000, 8000, Rgb([240, 240, 235]));
        for y in (500..7500).step_by(120) {
            for x in 800..5200 {
                if (x / 9) % 2 == 0 {
                    for dy in 0..30 {
                        img.put_pixel(x, y + dy, Rgb([20, 20, 20]));
                    }
                }
            }
        }
        let t = std::time::Instant::now();
        let _ = trim_to_paper(&mut img, 600, Rgb([240, 240, 235]));
        let _ = remove_fingers(&mut img, 600, Rgb([240, 240, 235]));
        let dt = t.elapsed().as_secs_f32();
        eprintln!("big image: {dt:.2}s");
        assert!(dt < 30.0, "too slow: {dt}s");
    }
}

#[cfg(test)]
mod split_sheet_tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn a_dark_gutter_does_not_make_one_page_vanish() {
        let paper = Rgb([235, 235, 230]);
        let mut img = RgbImage::from_pixel(1200, 800, paper);
        for x in 570..630 {
            for y in 0..800 {
                img.put_pixel(x, y, Rgb([175, 175, 172]));
            }
        }
        for (x0, x1) in [(100u32, 500u32), (700, 1100)] {
            for y in (100..700).step_by(40) {
                for x in x0..x1 {
                    for dy in 0..8 {
                        img.put_pixel(x, y + dy, Rgb([30, 30, 30]));
                    }
                }
            }
        }
        let _ = trim_to_paper(&mut img, 150, paper);
        assert_eq!(img.get_pixel(300, 100), &Rgb([30, 30, 30]), "left page text wiped");
        assert_eq!(img.get_pixel(900, 100), &Rgb([30, 30, 30]), "right page text wiped");
    }
}

#[cfg(test)]
mod box_filter_tests {
    use super::*;

    fn naive(src: &[u8], w: usize, h: usize, r: usize, max: bool) -> Vec<u8> {
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (x0, x1) = (x.saturating_sub(r), (x + r).min(w - 1));
                let (y0, y1) = (y.saturating_sub(r), (y + r).min(h - 1));
                let mut m = if max { 0u8 } else { 255u8 };
                for yy in y0..=y1 {
                    for xx in x0..=x1 {
                        let v = src[yy * w + xx];
                        m = if max { m.max(v) } else { m.min(v) };
                    }
                }
                out[y * w + x] = m;
            }
        }
        out
    }

    #[test]
    fn box_filter_equals_the_naive_window_on_random_images() {
        let mut s = 99u32;
        for (w, h) in [(1usize, 1usize), (1, 9), (9, 1), (5, 7), (31, 17), (64, 40)] {
            for r in [0usize, 1, 2, 3, 8, 21, 100] {
                let src: Vec<u8> = (0..w * h)
                    .map(|_| {
                        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                        (s >> 24) as u8
                    })
                    .collect();
                for max in [true, false] {
                    assert_eq!(
                        box_filter(&src, w, h, r, max),
                        naive(&src, w, h, r, max),
                        "{w}x{h} r={r} max={max}"
                    );
                }
            }
        }
    }
}
