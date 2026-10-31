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
    if percentile(&h, 0.5) < 90 && percentile(&h, 0.97) > 150 {
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
fn blur(img: &RgbImage, r: usize) -> RgbImage {
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
struct Comp {
    x0: u32,
    y0: u32,
    x1: u32, // inclusive
    y1: u32,
    area: u32,
}

/// 8-connected labelling of a `w×h` mask. Labels start at 1 (0 = background).
fn label(mask: &[u8], w: usize, h: usize) -> (Vec<u32>, Vec<Comp>) {
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
        .chunks_exact(3)
        .map(|p| {
            let (y, cb, cr) = ycbcr([p[0], p[1], p[2]]);
            ((77.0..=127.0).contains(&cb)
                && cr >= pcr + 9.0
                && cr <= 180.0
                && y >= 70.0
                && y <= 0.97 * py) as u8
        })
        .collect();
    let (lab, comps) = label(&mask, w, h);
    let sdpi = dpi as f32 / f as f32;
    let min_len = 8.0 / 25.4 * sdpi;
    let min_area = (40.0 / 645.16) * sdpi * sdpi; // 40 mm²
    let mut hit = vec![false; comps.len() + 1];
    let mut n = 0;
    for (i, c) in comps.iter().enumerate() {
        let touches = c.x0 == 0 || c.y0 == 0 || c.x1 as usize == w - 1 || c.y1 as usize == h - 1;
        let len = ((c.x1 - c.x0 + 1) as f32).max((c.y1 - c.y0 + 1) as f32);
        if touches && len >= min_len && c.area as f32 >= min_area {
            hit[i + 1] = true;
            n += 1;
        }
    }
    if n == 0 {
        return 0;
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
}
