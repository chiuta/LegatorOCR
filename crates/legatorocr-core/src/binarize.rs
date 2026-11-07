use crate::BitImage;
use image::GrayImage;

/// Global Otsu threshold (pixels `<= t` are ink).
pub fn otsu_threshold(g: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in g.as_raw() {
        hist[*p as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w_b, mut sum_b) = (0u64, 0f64);
    let (mut best, mut best_t) = (-1.0f64, 128u8);
    for t in 0..256usize {
        w_b += hist[t];
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * hist[t] as f64;
        let m_b = sum_b / w_b as f64;
        let m_f = (sum_all - sum_b) / w_f as f64;
        let between = w_b as f64 * w_f as f64 * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

pub fn global(g: &GrayImage, t: u8) -> BitImage {
    let mut out = BitImage::new(g.width(), g.height());
    for (d, s) in out.data.iter_mut().zip(g.as_raw()) {
        *d = (*s <= t) as u8;
    }
    out
}

/// Local-threshold method. Parameter `k` (0–1, "sensitivity") is mapped onto each
/// method's own constant, so one slider means "more ink" for every choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Threshold {
    /// Sauvola: `T = m·(1 + k·(s/128 − 1))`; robust default for text.
    #[default]
    Sauvola,
    /// Wolf–Jolion: Sauvola variant normalised by the page's global minimum and maximum local deviation.
    Wolf,
    /// Niblack: `T = m + k'·s`; keeps thin strokes, noisier on empty paper.
    Niblack,
    /// NICK: Niblack corrected for light backgrounds.
    Nick,
    /// Bradley–Roth: `T = m·(1 − t)`, fast and tolerant of uneven light.
    Bradley,
    /// Bernsen: the midpoint of the local darkest and lightest pixel, used only where the local
    /// contrast is at least 40 levels (flat neighbourhoods are paper). Keeps strokes on uneven light.
    Bernsen,
    /// Phansalkar: Sauvola with a term that raises the threshold on dark, low-contrast
    /// neighbourhoods; made for faint, stained or very uneven pages.
    Phansalkar,
    /// Sauvola kept only where the neighbourhood has real contrast (or the pixel is clearly dark):
    /// stains, shadows and show-through that Sauvola would pick up in flat grey areas stay paper.
    EdgePlus,
    /// One global Otsu threshold (clean, evenly lit scans).
    Otsu,
    /// Per page: try the local methods on a small copy and keep the one that leaves
    /// the fewest isolated specks while giving a plausible amount of ink.
    Auto,
}

impl Threshold {
    pub const ALL: [Threshold; 10] = [
        Threshold::Sauvola,
        Threshold::Wolf,
        Threshold::Niblack,
        Threshold::Nick,
        Threshold::Bradley,
        Threshold::Bernsen,
        Threshold::Phansalkar,
        Threshold::EdgePlus,
        Threshold::Otsu,
        Threshold::Auto,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Threshold::Sauvola => "sauvola",
            Threshold::Wolf => "wolf",
            Threshold::Niblack => "niblack",
            Threshold::Nick => "nick",
            Threshold::Bradley => "bradley",
            Threshold::Bernsen => "bernsen",
            Threshold::Phansalkar => "phansalkar",
            Threshold::EdgePlus => "edgeplus",
            Threshold::Otsu => "otsu",
            Threshold::Auto => "auto",
        }
    }
    pub fn from_name(s: &str) -> Option<Threshold> {
        Threshold::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

/// Local deviation below which a neighbourhood is treated as flat paper (Niblack/NICK).
const FLAT_SD: f64 = 7.0;

/// Mean and standard deviation of every pixel's `(2r+1)²` neighbourhood for the
/// rows `y0..y1`, via sliding column sums: O(1) per pixel and O(width) memory (an
/// integral image of a 35-megapixel scan would need over half a gigabyte).
fn band_stats(
    g: &GrayImage,
    r: usize,
    y0: usize,
    y1: usize,
    mut f: impl FnMut(usize, &[f32], &[f32]),
) {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let raw = g.as_raw();
    let mut cs = vec![0u32; w];
    let mut cq = vec![0u32; w];
    let add = |cs: &mut [u32], cq: &mut [u32], y: usize, sign: i64| {
        let row = &raw[y * w..(y + 1) * w];
        for (x, &p) in row.iter().enumerate() {
            let p = p as u32;
            if sign > 0 {
                cs[x] += p;
                cq[x] += p * p;
            } else {
                cs[x] -= p;
                cq[x] -= p * p;
            }
        }
    };
    let (mut lo, mut hi) = (y0.saturating_sub(r), (y0 + r + 1).min(h)); // rows currently in the sums: lo..hi
    for y in lo..hi {
        add(&mut cs, &mut cq, y, 1);
    }
    let mut mean = vec![0f32; w];
    let mut sd = vec![0f32; w];
    for y in y0..y1 {
        let (nlo, nhi) = (y.saturating_sub(r), (y + r + 1).min(h));
        while hi < nhi {
            add(&mut cs, &mut cq, hi, 1);
            hi += 1;
        }
        while lo < nlo {
            add(&mut cs, &mut cq, lo, -1);
            lo += 1;
        }
        let rows = (nhi - nlo) as f64;
        // Horizontal window by running sums.
        let (mut s, mut q) = (0u64, 0u64);
        let mut a = 0usize; // window [a, b)
        let mut b = 0usize;
        for x in 0..w {
            let (nx0, nx1) = (x.saturating_sub(r), (x + r + 1).min(w));
            while b < nx1 {
                s += cs[b] as u64;
                q += cq[b] as u64;
                b += 1;
            }
            while a < nx0 {
                s -= cs[a] as u64;
                q -= cq[a] as u64;
                a += 1;
            }
            let area = rows * (nx1 - nx0) as f64;
            let m = s as f64 / area;
            mean[x] = m as f32;
            sd[x] = (q as f64 / area - m * m).max(0.0).sqrt() as f32;
        }
        f(y, &mean, &sd);
    }
}

/// Pick the local method for [`Threshold::Auto`]. Candidates are run on a copy of at
/// most ≈150 dpi (`window` ≈ dpi/3). Score = share of ink that is isolated specks
/// (noise) plus a penalty when the ink share leaves the 1–30 % band typical of print.
fn choose_auto(g: &GrayImage, window: u32, k: f32) -> Threshold {
    let factor = (window.min(30_000) * 3 / 150).max(1);
    let small = crate::transform::downscale_gray(g, factor);
    let win = ((window / factor) | 1).max(9);
    let mut best = (f64::MAX, Threshold::Sauvola);
    for m in [
        Threshold::Sauvola,
        Threshold::Wolf,
        Threshold::Nick,
        Threshold::Bradley,
    ] {
        let mut bw = threshold(&small, m, win, k);
        let total = bw.count_ink() as f64;
        let area = (bw.w as f64 * bw.h as f64).max(1.0);
        if total == 0.0 {
            continue;
        }
        let removed = bw.despeckle(3) as f64;
        let _ = removed;
        let kept = bw.count_ink() as f64;
        let noise = (total - kept) / total;
        let frac = total / area;
        let penalty = if frac < 0.01 {
            (0.01 - frac) * 20.0
        } else if frac > 0.30 {
            (frac - 0.30) * 4.0
        } else {
            0.0
        };
        let score = noise + penalty;
        // ties (and near ties) keep Sauvola, the safest default
        if score < best.0 - 0.01 {
            best = (score, m);
        }
    }
    best.1
}

/// No tonal range worth separating: the 0.01 % darkest and lightest pixels lie within 40 grey
/// levels (noise spreads that far; even one short word of text is farther than that).
fn is_flat(g: &GrayImage) -> bool {
    let mut hist = [0u64; 256];
    for &p in g.as_raw() {
        hist[p as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let cut = (total / 10_000).max(1);
    let (mut acc, mut lo) = (0u64, 0usize);
    for (v, &n) in hist.iter().enumerate() {
        acc += n;
        if acc > cut {
            lo = v;
            break;
        }
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    for (v, &n) in hist.iter().enumerate().rev() {
        acc += n;
        if acc > cut {
            hi = v;
            break;
        }
    }
    hi.saturating_sub(lo) < 40
}

/// Darkest and lightest value of every pixel's `(2r+1)²` window (separable, monotonic deque:
/// O(1) per pixel whatever the radius). Returns `(min, max)` planes.
fn local_extrema(g: &GrayImage, r: usize) -> (Vec<u8>, Vec<u8>) {
    use std::collections::VecDeque;
    fn pass(
        srcs: (&[u8], &[u8]),
        dims: (usize, usize, usize),
        r: usize,
        mn: &mut [u8],
        mx: &mut [u8],
    ) {
        let (len, stride, count) = dims;
        let (lo_src, hi_src) = srcs;
        for line in 0..count {
            let at = |i: usize| line * if stride == 1 { len } else { 1 } + i * stride;
            let mut dmin: VecDeque<usize> = VecDeque::new();
            let mut dmax: VecDeque<usize> = VecDeque::new();
            let mut next = 0usize;
            for i in 0..len {
                let hi = (i + r).min(len - 1);
                while next <= hi {
                    let (vl, vh) = (lo_src[at(next)], hi_src[at(next)]);
                    while dmin.back().is_some_and(|&b| lo_src[at(b)] >= vl) {
                        dmin.pop_back();
                    }
                    dmin.push_back(next);
                    while dmax.back().is_some_and(|&b| hi_src[at(b)] <= vh) {
                        dmax.pop_back();
                    }
                    dmax.push_back(next);
                    next += 1;
                }
                let lo = i.saturating_sub(r);
                while dmin.front().is_some_and(|&f| f < lo) {
                    dmin.pop_front();
                }
                while dmax.front().is_some_and(|&f| f < lo) {
                    dmax.pop_front();
                }
                mn[at(i)] = lo_src[at(*dmin.front().unwrap())];
                mx[at(i)] = hi_src[at(*dmax.front().unwrap())];
            }
        }
    }
    let (w, h) = (g.width() as usize, g.height() as usize);
    let raw = g.as_raw();
    let mut mn1 = vec![0u8; w * h];
    let mut mx1 = vec![0u8; w * h];
    pass((raw, raw), (w, 1, h), r, &mut mn1, &mut mx1);
    let mut mn2 = vec![0u8; w * h];
    let mut mx2 = vec![0u8; w * h];
    pass((&mn1, &mx1), (h, w, w), r, &mut mn2, &mut mx2);
    (mn2, mx2)
}

/// Binarise with the chosen local method. `window` ≈ one third of an inch is a good size.
/// Ink = 1. Rows are processed in parallel bands.
pub fn threshold(g: &GrayImage, method: Threshold, window: u32, k: f32) -> BitImage {
    use rayon::prelude::*;
    let (w, h) = (g.width() as usize, g.height() as usize);
    let mut out = BitImage::new(w as u32, h as u32);
    if w == 0 || h == 0 {
        return out;
    }
    // A page with no tonal range at all (blank paper with sensor noise, a uniform fill) has
    // nothing to separate: every method would invent ink from the noise (Otsu splits it at its
    // mean, Wolf's threshold equals the flat mean). Leave it all paper.
    if is_flat(g) {
        return out;
    }
    if method == Threshold::Otsu {
        return global(g, otsu_threshold(g));
    }
    if method == Threshold::Auto {
        return threshold(g, choose_auto(g, window, k), window, k);
    }
    let r = ((window | 1) / 2) as usize;
    let area_px = ((2 * r + 1) * (2 * r + 1)) as f64;
    let k = k.clamp(0.0, 1.0) as f64;
    let raw = g.as_raw();
    let band = (h / (rayon::current_num_threads() * 2)).clamp(64, 512);
    let extrema = (method == Threshold::Bernsen).then(|| local_extrema(g, r));
    // Wolf needs the page-wide extremes first.
    let (gmin, smax) = if method == Threshold::Wolf {
        (0..h.div_ceil(band))
            .into_par_iter()
            .map(|b| {
                let (y0, y1) = (b * band, ((b + 1) * band).min(h));
                let mut smax = 0f32;
                band_stats(g, r, y0, y1, |_, _, sd| {
                    smax = sd.iter().fold(smax, |a, &v| a.max(v))
                });
                let gmin = raw[y0 * w..y1 * w].iter().copied().min().unwrap_or(255) as f32;
                (gmin, smax)
            })
            .reduce(|| (255.0, 0.0), |a, b| (a.0.min(b.0), a.1.max(b.1)))
    } else {
        (0.0, 1.0)
    };
    out.data
        .par_chunks_mut(band * w)
        .enumerate()
        .for_each(|(b, chunk)| {
            let y0 = b * band;
            let y1 = (y0 + chunk.len() / w).min(h);
            band_stats(g, r, y0, y1, |y, mean, sd| {
                let row = &raw[y * w..(y + 1) * w];
                let o = &mut chunk[(y - y0) * w..(y - y0 + 1) * w];
                for x in 0..w {
                    let (m, s) = (mean[x] as f64, sd[x] as f64);
                    let t = match method {
                        Threshold::Sauvola => m * (1.0 + k * (s / 128.0 - 1.0)),
                        Threshold::Wolf => {
                            let a = k.max(0.05);
                            (1.0 - a) * m
                                + a * gmin as f64
                                + a * (s / (smax as f64).max(1.0)) * (m - gmin as f64)
                        }
                        // Niblack and NICK have no notion of "empty paper": with their plain
                        // formulas sensor noise turns into specks. Flat neighbourhoods are paper.
                        Threshold::Niblack if s < FLAT_SD => 0.0,
                        Threshold::Nick if s < FLAT_SD => 0.0,
                        Threshold::Niblack => m - 0.67 * k * s,
                        Threshold::Nick => m - 0.67 * k * (s * s + m * m / area_px).sqrt(),
                        Threshold::Bradley => m * (1.0 - 0.5 * k),
                        Threshold::Bernsen => {
                            let (mn, mx) = extrema.as_ref().expect("extrema computed");
                            let i = y * w + x;
                            let (lo, hi) = (mn[i] as f64, mx[i] as f64);
                            if hi - lo < 40.0 {
                                0.0
                            } else {
                                (lo + hi) / 2.0
                            }
                        }
                        Threshold::Phansalkar => {
                            let (mn, sn) = (m / 255.0, s / 255.0);
                            let kk = 0.15 + 0.25 * k;
                            255.0 * mn * (1.0 + 2.0 * (-10.0 * mn).exp() + kk * (sn / 0.5 - 1.0))
                        }
                        Threshold::EdgePlus => {
                            if s < 12.0 && row[x] > 100 {
                                0.0
                            } else {
                                m * (1.0 + k * (s / 128.0 - 1.0))
                            }
                        }
                        Threshold::Otsu | Threshold::Auto => unreachable!(),
                    };
                    o[x] = (row[x] as f64 <= t) as u8;
                }
            });
        });
    out
}

/// Sauvola adaptive threshold (`window` forced odd; `k` ≈ 0.2–0.5).
pub fn sauvola(g: &GrayImage, window: u32, k: f32) -> BitImage {
    threshold(g, Threshold::Sauvola, window, k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    #[allow(dead_code)]
    /// Straightforward integral-image Sauvola, kept as the correctness reference.
    /// `window` is forced odd; `k` ≈ 0.2–0.5; dynamic range R = 128.
    pub fn sauvola_reference(g: &GrayImage, window: u32, k: f32) -> BitImage {
        let (w, h) = (g.width() as usize, g.height() as usize);
        let mut out = BitImage::new(w as u32, h as u32);
        if w == 0 || h == 0 {
            return out;
        }
        let stride = w + 1;
        let mut sum = vec![0u64; stride * (h + 1)];
        let mut sq = vec![0u64; stride * (h + 1)];
        let raw = g.as_raw();
        for y in 0..h {
            let (mut rs, mut rq) = (0u64, 0u64);
            for x in 0..w {
                let v = raw[y * w + x] as u64;
                rs += v;
                rq += v * v;
                sum[(y + 1) * stride + x + 1] = sum[y * stride + x + 1] + rs;
                sq[(y + 1) * stride + x + 1] = sq[y * stride + x + 1] + rq;
            }
        }
        let r = ((window | 1) / 2) as usize;
        for y in 0..h {
            let y0 = y.saturating_sub(r);
            let y1 = (y + r + 1).min(h);
            for x in 0..w {
                let x0 = x.saturating_sub(r);
                let x1 = (x + r + 1).min(w);
                let area = ((y1 - y0) * (x1 - x0)) as f64;
                let s = sum[y1 * stride + x1] + sum[y0 * stride + x0]
                    - sum[y0 * stride + x1]
                    - sum[y1 * stride + x0];
                let q = sq[y1 * stride + x1] + sq[y0 * stride + x0]
                    - sq[y0 * stride + x1]
                    - sq[y1 * stride + x0];
                let mean = s as f64 / area;
                let var = (q as f64 / area - mean * mean).max(0.0);
                let t = mean * (1.0 + k as f64 * (var.sqrt() / 128.0 - 1.0));
                out.data[y * w + x] = (raw[y * w + x] as f64 <= t) as u8;
            }
        }
        out
    }

    fn page() -> GrayImage {
        let (w, h) = (300u32, 220u32);
        let mut g = GrayImage::new(w, h);
        let mut seed = 7u32;
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 24) as f32 / 255.0 * 10.0;
                let light =
                    120.0 + 110.0 * (x as f32 / w as f32) * (0.6 + 0.4 * y as f32 / h as f32);
                let ink = (y / 9) % 3 == 0 && (x / 4) % 3 != 0 && (20..h - 20).contains(&y);
                g.put_pixel(
                    x,
                    y,
                    Luma([(if ink { light * 0.3 } else { light } + noise).min(255.0) as u8]),
                );
            }
        }
        g
    }

    #[test]
    fn sliding_sauvola_matches_integral_reference() {
        let g = page();
        for window in [15u32, 31, 61] {
            let a = sauvola(&g, window, 0.3);
            let b = sauvola_reference(&g, window, 0.3);
            let diff = a.data.iter().zip(&b.data).filter(|(x, y)| x != y).count();
            // floating-point summation order may flip a handful of borderline pixels
            assert!(
                diff * 5000 < a.data.len(),
                "window {window}: {diff} pixels differ"
            );
        }
    }

    #[test]
    fn every_method_finds_the_text_and_keeps_paper_clean() {
        let g = page();
        for m in Threshold::ALL {
            let b = threshold(&g, m, 41, 0.3);
            let ink = b.count_ink() as f64 / b.data.len() as f64;
            assert!((0.04..0.45).contains(&ink), "{m:?}: ink fraction {ink}");
            // a paper-only row band (top margin) must stay almost empty
            let top: u64 = (0..8)
                .flat_map(|y| (0..300).map(move |x| (x, y)))
                .map(|(x, y)| b.get(x, y) as u64)
                .sum();
            assert!(top < 8 * 300 / 8, "{m:?}: {top} specks in blank rows");
        }
    }

    #[test]
    fn local_extrema_match_a_brute_force_window() {
        let g = page();
        let g = image::imageops::crop_imm(&g, 0, 0, 60, 45).to_image();
        for r in [1usize, 4, 9] {
            let (mn, mx) = local_extrema(&g, r);
            let (w, h) = (g.width() as i64, g.height() as i64);
            for y in 0..h {
                for x in 0..w {
                    let (mut lo, mut hi) = (255u8, 0u8);
                    for yy in (y - r as i64).max(0)..=(y + r as i64).min(h - 1) {
                        for xx in (x - r as i64).max(0)..=(x + r as i64).min(w - 1) {
                            let v = g.get_pixel(xx as u32, yy as u32).0[0];
                            lo = lo.min(v);
                            hi = hi.max(v);
                        }
                    }
                    let i = (y * w + x) as usize;
                    assert_eq!((mn[i], mx[i]), (lo, hi), "r={r} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn edgeplus_drops_a_flat_grey_stain_that_sauvola_keeps() {
        use image::Luma;
        let mut g = GrayImage::from_pixel(300, 200, Luma([235]));
        for y in 20..180 {
            for x in 20..120 {
                g.put_pixel(x, y, Luma([150])); // a stain, no strokes
            }
        }
        for y in 100..108 {
            for x in 180..260 {
                g.put_pixel(x, y, Luma([20])); // one real stroke
            }
        }
        let stain = |b: &BitImage| (60..140).map(|y| b.get(70, y) as u32).sum::<u32>();
        let e = threshold(&g, Threshold::EdgePlus, 41, 0.3);
        assert_eq!(stain(&e), 0, "stain stays paper");
        assert!(e.count_ink() > 400, "stroke kept");
    }

    #[test]
    fn names_round_trip() {
        for m in Threshold::ALL {
            assert_eq!(Threshold::from_name(m.name()), Some(m));
        }
        assert_eq!(Threshold::from_name("SAUVOLA"), Some(Threshold::Sauvola));
        assert_eq!(Threshold::from_name("x"), None);
    }

    #[test]
    fn blank_and_uniform_pages_stay_paper_for_every_method() {
        use image::Luma;
        // noisy blank paper, a flat dark fill, a flat white page
        let mut noisy = GrayImage::from_pixel(300, 300, Luma([235]));
        let mut seed = 12345u32;
        for p in noisy.pixels_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            p.0[0] = (235 + ((seed >> 24) % 9) as i32 - 4) as u8;
        }
        let dark = GrayImage::from_pixel(300, 300, Luma([60]));
        let white = GrayImage::from_pixel(300, 300, Luma([255]));
        for m in [
            Threshold::Otsu,
            Threshold::Wolf,
            Threshold::Sauvola,
            Threshold::Niblack,
            Threshold::Nick,
            Threshold::Bernsen,
            Threshold::Phansalkar,
            Threshold::EdgePlus,
            Threshold::Auto,
        ] {
            for (name, g) in [("noisy", &noisy), ("dark", &dark), ("white", &white)] {
                assert_eq!(threshold(g, m, 31, 0.3).count_ink(), 0, "{m:?} {name}");
            }
        }
        // one short word on paper is still found
        let mut word = GrayImage::from_pixel(300, 300, Luma([235]));
        for y in 140..150 {
            for x in 100..140 {
                word.put_pixel(x, y, Luma([30]));
            }
        }
        assert!(threshold(&word, Threshold::Sauvola, 31, 0.3).count_ink() > 100);
        assert!(threshold(&word, Threshold::Otsu, 31, 0.3).count_ink() > 100);
    }
}
