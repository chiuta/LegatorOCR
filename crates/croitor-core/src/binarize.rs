use crate::BitImage;
use image::GrayImage;

/// Global Otsu threshold (pixels `<= t` are ink).
pub fn otsu_threshold(g: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in g.as_raw() {
        hist[*p as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
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

/// Sauvola adaptive threshold using integral images.
/// `window` is forced odd; `k` ≈ 0.2–0.5; dynamic range R = 128.
pub fn sauvola(g: &GrayImage, window: u32, k: f32) -> BitImage {
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
