//! Slanted-edge sharpness (roadmap 5.9, first step): the spatial frequency response of a scan
//! measured on a straight, slightly tilted black/white edge, in the spirit of ISO 12233.
//!
//! Method: find the edge position in every row (centroid of the horizontal derivative), fit a
//! line, project every pixel on the edge normal, average into 4x oversampled bins (the edge
//! spread function), differentiate (line spread function), window with a Hann window, take the
//! discrete Fourier transform and normalise to 1 at zero frequency. `mtf50` is the frequency in
//! cycles per pixel where the response falls to 0.5.
//!
//! Verified on synthetic edges of known blur (see the tests). It is not the full FADGI /
//! Metamorfoze procedure: no target detection, no colour or noise measurement, no calibration
//! against a certified target.

use image::GrayImage;

#[derive(Debug, Clone, PartialEq)]
pub struct Sfr {
    /// Edge angle from vertical, degrees.
    pub angle_deg: f64,
    /// Cycles per pixel at which the MTF crosses 0.5.
    pub mtf50: f64,
    /// Cycles per pixel at which the MTF crosses 0.1.
    pub mtf10: f64,
    /// (frequency in cycles/pixel, response) up to 1 cycle/pixel.
    pub curve: Vec<(f64, f64)>,
}

/// Measure the near-vertical edge inside the box `(x, y, w, h)` (the edge runs mostly top to
/// bottom, dark on one side). Returns `None` when no clear edge is found.
pub fn measure(img: &GrayImage, roi: (u32, u32, u32, u32)) -> Option<Sfr> {
    let (x0, y0, w, h) = roi;
    if w < 16 || h < 16 || x0 + w > img.width() || y0 + h > img.height() {
        return None;
    }
    let px = |x: u32, y: u32| img.get_pixel(x0 + x, y0 + y)[0] as f64;
    // edge centre per row: centroid of |derivative|
    let mut pts: Vec<(f64, f64)> = Vec::new(); // (row, x)
    let mut contrast = 0.0f64;
    for y in 0..h {
        let (mut num, mut den) = (0.0, 0.0);
        for x in 0..w - 1 {
            let d = (px(x + 1, y) - px(x, y)).abs();
            num += d * (x as f64 + 0.5);
            den += d;
        }
        if den > 0.0 {
            pts.push((y as f64, num / den));
        }
        let (a, b) = (px(0, y), px(w - 1, y));
        contrast += (a - b).abs();
    }
    contrast /= h as f64;
    if pts.len() < (h as usize * 3) / 4 || contrast < 40.0 {
        return None;
    }
    // least-squares line x = m*y + c
    let n = pts.len() as f64;
    let (sy, sx) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (syy, sxy) = pts
        .iter()
        .fold((0.0, 0.0), |a, p| (a.0 + p.0 * p.0, a.1 + p.0 * p.1));
    let m = (n * sxy - sy * sx) / (n * syy - sy * sy);
    let c = (sx - m * sy) / n;
    // the fit must explain the rows: otherwise this is not a straight edge
    let rms = (pts
        .iter()
        .map(|p| (p.1 - (m * p.0 + c)).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    if rms > 1.5 {
        return None;
    }
    // ESF: distance along the horizontal normal, 4x oversampled
    const OS: f64 = 4.0;
    let nb = (w as f64 * OS) as usize + 8;
    let (mut sum, mut cnt) = (vec![0.0f64; nb], vec![0u32; nb]);
    let cosa = 1.0 / (1.0 + m * m).sqrt();
    for y in 0..h {
        for x in 0..w {
            let dist = (x as f64 + 0.5 - (m * y as f64 + c)) * cosa; // signed, pixels
            let bin = ((dist + w as f64 / 2.0 + 2.0) * OS).round();
            if bin >= 0.0 && (bin as usize) < nb {
                sum[bin as usize] += px(x, y);
                cnt[bin as usize] += 1;
            }
        }
    }
    let mut esf: Vec<f64> = Vec::new();
    let mut last = f64::NAN;
    for (s, k) in sum.iter().zip(&cnt) {
        if *k > 0 {
            last = s / *k as f64;
        }
        if !last.is_nan() {
            esf.push(last);
        }
    }
    if esf.len() < 32 {
        return None;
    }
    // orient dark->bright so the derivative is positive
    if esf[0] > esf[esf.len() - 1] {
        esf.reverse();
    }
    let mut lsf: Vec<f64> = esf.windows(2).map(|p| p[1] - p[0]).collect();
    // centre a Hann window on the peak
    let peak = lsf
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|p| p.0)?;
    let half = (lsf.len() / 2)
        .min(peak)
        .min(lsf.len() - 1 - peak)
        .min(64 * OS as usize);
    if half < 16 {
        return None;
    }
    lsf = lsf[peak - half..=peak + half].to_vec();
    let len = lsf.len();
    for (i, v) in lsf.iter_mut().enumerate() {
        *v *= 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (len - 1) as f64).cos();
    }
    let dc: f64 = lsf.iter().sum();
    if dc <= 0.0 {
        return None;
    }
    // DFT magnitude at frequencies 0..1 cycle/pixel (sample spacing 1/OS pixel)
    let steps = 200;
    let mut curve = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let f = k as f64 / steps as f64; // cycles per pixel
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in lsf.iter().enumerate() {
            let t = i as f64 / OS;
            let ph = 2.0 * std::f64::consts::PI * f * t;
            re += v * ph.cos();
            im -= v * ph.sin();
        }
        curve.push((f, (re * re + im * im).sqrt() / dc));
    }
    let cross = |level: f64| -> f64 {
        for w in curve.windows(2) {
            if w[0].1 >= level && w[1].1 < level {
                let t = (w[0].1 - level) / (w[0].1 - w[1].1);
                return w[0].0 + t * (w[1].0 - w[0].0);
            }
        }
        1.0
    };
    Some(Sfr {
        angle_deg: m.atan().to_degrees(),
        mtf50: cross(0.5),
        mtf10: cross(0.1),
        curve,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Edge tilted by `slope` (x shift per row), blurred by a Gaussian of `sigma` pixels,
    /// sampled by integrating over each pixel (supersampled).
    fn edge(sigma: f64, slope: f64) -> GrayImage {
        let (w, h) = (96u32, 128u32);
        let mut im = GrayImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for sx in 0..8 {
                    let xs = x as f64 + (sx as f64 + 0.5) / 8.0;
                    let d = (xs - (48.0 + slope * y as f64))
                        / (sigma.max(1e-3) * std::f64::consts::SQRT_2);
                    acc += 0.5 * (1.0 + erf(d));
                }
                im.put_pixel(x, y, image::Luma([(30.0 + 195.0 * acc / 8.0) as u8]));
            }
        }
        im
    }

    fn erf(x: f64) -> f64 {
        // Abramowitz-Stegun 7.1.26
        let t = 1.0 / (1.0 + 0.3275911 * x.abs());
        let y = 1.0
            - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
                + 0.254829592)
                * t
                * (-x * x).exp();
        if x >= 0.0 {
            y
        } else {
            -y
        }
    }

    #[test]
    fn mtf50_follows_the_known_blur() {
        // Gaussian blur sigma s (plus the 1-pixel box of sampling): MTF50 of the Gaussian is
        // 0.1874 / s cycles per pixel; the pixel box lowers it a little, so allow 12 %.
        for &s in &[0.8, 1.2, 2.0] {
            let r = measure(&edge(s, 0.08), (0, 0, 96, 128)).expect("edge found");
            let ideal = 0.1874 / s;
            // combined response of Gaussian and 1-px box at the ideal frequency
            let box_resp = |f: f64| (std::f64::consts::PI * f).sin() / (std::f64::consts::PI * f);
            let mut f = 0.01;
            while (-2.0 * std::f64::consts::PI.powi(2) * s * s * f * f).exp() * box_resp(f) > 0.5 {
                f += 0.001;
            }
            assert!(
                (r.mtf50 - f).abs() / f < 0.12,
                "sigma {s}: measured {} expected {f} (gauss only {ideal})",
                r.mtf50
            );
            assert!(
                (r.angle_deg - (0.08f64).atan().to_degrees()).abs() < 0.5,
                "{}",
                r.angle_deg
            );
        }
        let sharp = measure(&edge(0.8, 0.08), (0, 0, 96, 128)).unwrap().mtf50;
        let soft = measure(&edge(2.0, 0.08), (0, 0, 96, 128)).unwrap().mtf50;
        assert!(sharp > soft * 1.8);
    }

    #[test]
    fn no_edge_no_answer() {
        let flat = GrayImage::from_pixel(64, 64, image::Luma([128]));
        assert!(measure(&flat, (0, 0, 64, 64)).is_none());
        assert!(measure(&flat, (0, 0, 8, 8)).is_none());
    }
}
