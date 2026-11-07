//! Page-quality heuristics (ROADMAP 1.5, 1.13 and the pure part of 1.14).
//!
//! Everything here is a pure function on [`GrayImage`] or on plain numbers, so
//! the CLI/GUI can call it per page and decide what to show. All thresholds are
//! heuristics chosen from first principles (see each constant); they have not
//! been tuned on a corpus of real scans.

use crate::binarize::otsu_threshold;
use image::{imageops, GrayImage};

// ---------------------------------------------------------------- 1.13 dpi

/// Below this effective resolution OCR starts to misread glyphs.
pub const DPI_LOW: f32 = 200.0;
/// Above this, plain-text pages gain nothing but file size and time.
pub const DPI_HIGH: f32 = 400.0;

/// Verdict of the effective-dpi advisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpiAdvice {
    /// Under [`DPI_LOW`]: OCR is likely to misread.
    TooLow,
    /// Between the two limits (inclusive).
    Good,
    /// Over [`DPI_HIGH`]: pointless for text-only pages.
    ExcessForText,
}

impl DpiAdvice {
    /// Stable message key, for UIs that translate.
    pub fn key(self) -> &'static str {
        match self {
            DpiAdvice::TooLow => "quality.dpi.too_low",
            DpiAdvice::Good => "quality.dpi.good",
            DpiAdvice::ExcessForText => "quality.dpi.excess_for_text",
        }
    }

    /// English message for the verdict.
    pub fn advice_text(self) -> &'static str {
        match self {
            DpiAdvice::TooLow => {
                "Resolution is below 200 dpi: OCR is likely to misread characters. Rescan at 300 dpi or more."
            }
            DpiAdvice::Good => "Resolution is suitable for OCR.",
            DpiAdvice::ExcessForText => {
                "Resolution is above 400 dpi: plain text gains nothing from it, only larger files and slower processing."
            }
        }
    }
}

/// Effective dpi from pixel size and physical page size in millimetres
/// (mean of the two axes). Returns 0.0 for non-positive or non-finite input.
pub fn effective_dpi(width_px: u32, height_px: u32, page_mm: (f32, f32)) -> f32 {
    let (w_mm, h_mm) = page_mm;
    if !(w_mm.is_finite() && h_mm.is_finite()) || w_mm <= 0.0 || h_mm <= 0.0 {
        return 0.0;
    }
    let dx = width_px as f32 / (w_mm / 25.4);
    let dy = height_px as f32 / (h_mm / 25.4);
    (dx + dy) / 2.0
}

/// Classify a dpi value. Non-finite or non-positive values count as too low.
pub fn dpi_advice(dpi: f32) -> DpiAdvice {
    if !dpi.is_finite() || dpi < DPI_LOW {
        DpiAdvice::TooLow
    } else if dpi > DPI_HIGH {
        DpiAdvice::ExcessForText
    } else {
        DpiAdvice::Good
    }
}

// ------------------------------------------------------------ 1.5 sharpness

/// Longest side used when measuring sharpness, so values are comparable
/// across scan resolutions.
const SHARP_SIDE: u32 = 640;
/// A page is "blurred" only below this fraction of the batch median...
pub const BLUR_RELATIVE: f32 = 0.40;
/// ...and below this absolute Laplacian variance (0-255 scale at 640 px).
/// Chosen so a page that is clearly crisp in absolute terms is never flagged
/// just because the rest of the batch is extremely crisp.
pub const BLUR_ABSOLUTE: f32 = 200.0;

fn downscaled(img: &GrayImage, max_side: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let m = w.max(h);
    if m <= max_side || m == 0 {
        return img.clone();
    }
    let nw = ((w as u64 * max_side as u64) / m as u64).max(1) as u32;
    let nh = ((h as u64 * max_side as u64) / m as u64).max(1) as u32;
    imageops::resize(img, nw, nh, imageops::FilterType::Triangle)
}

/// Variance of the 4-neighbour Laplacian on a copy downscaled to at most
/// 640 px. Higher is sharper; 0.0 for images smaller than 3x3 or flat ones.
pub fn sharpness(img: &GrayImage) -> f32 {
    let s = downscaled(img, SHARP_SIDE);
    let (w, h) = s.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }
    let raw = s.as_raw();
    let w = w as usize;
    let h = h as usize;
    let (mut sum, mut sum2, mut n) = (0f64, 0f64, 0u64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = raw[y * w + x] as f64;
            let l = raw[y * w + x - 1] as f64
                + raw[y * w + x + 1] as f64
                + raw[(y - 1) * w + x] as f64
                + raw[(y + 1) * w + x] as f64
                - 4.0 * c;
            sum += l;
            sum2 += l * l;
            n += 1;
        }
    }
    let mean = sum / n as f64;
    ((sum2 / n as f64) - mean * mean).max(0.0) as f32
}

/// Is `sharp` blurred relative to a reference (batch median) sharpness?
/// Uses only the relative test; see [`blurred_pages`] for the combined rule.
pub fn is_blurred(sharp: f32, reference: f32) -> bool {
    sharp.is_finite()
        && reference.is_finite()
        && reference > 0.0
        && sharp < BLUR_RELATIVE * reference
}

fn median(values: &[f32]) -> Option<f32> {
    let mut v: Vec<f32> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// Indices of pages below 40 % of the batch median sharpness *and* below the
/// absolute floor [`BLUR_ABSOLUTE`]. Non-finite values are ignored.
pub fn blurred_pages(values: &[f32]) -> Vec<usize> {
    let Some(med) = median(values) else {
        return Vec::new();
    };
    values
        .iter()
        .enumerate()
        .filter(|(_, &v)| is_blurred(v, med) && v < BLUR_ABSOLUTE)
        .map(|(i, _)| i)
        .collect()
}

// --------------------------------------------------------- 1.5 cut-off text

/// Longest side of the copy analysed for cut-off text.
const EDGE_SIDE: u32 = 800;
/// Border band depth as a fraction of the shorter side.
const EDGE_BAND: f32 = 0.015;
/// Fraction of each side's length ignored at both ends (corner shadows).
const EDGE_CORNER: f32 = 0.05;
/// A side whose band is more than this fraction ink is a frame/shadow, not text.
const EDGE_FRAME_FRACTION: f32 = 0.5;
/// Longest stroke-like run, as a fraction of the side length (a glyph or a
/// short word block; longer runs are rules, frames or shadows).
const EDGE_MAX_RUN: f32 = 0.12;
/// Share of the way from the darkest pixel to the Otsu threshold that still counts as ink.
const EDGE_STRICT: f32 = 0.6;

/// Does ink touch the outermost border on two or more sides (text cut off)?
///
/// Works on a binarised (Otsu) copy. A side counts only if its outer band is
/// mostly paper (so a black scanner frame or full-length shadow is ignored),
/// corners are skipped (runs reaching a corner zone are dropped), and it holds at least one stroke-like ink run (not
/// longer than 12 % of the side) that continues inward past the band (so
/// isolated dust specks do not count).
pub fn edge_text_cut(img: &GrayImage) -> bool {
    let g = downscaled(img, EDGE_SIDE);
    let (w, h) = g.dimensions();
    if w < 16 || h < 16 {
        return false;
    }
    let (lo, hi) = g
        .as_raw()
        .iter()
        .fold((255u8, 0u8), |(l, h), &p| (l.min(p), h.max(p)));
    if hi - lo < 40 {
        return false; // blank or flat: no ink to speak of
    }
    // Otsu on aged paper puts the browned edges and the shadow of the spine on the ink side;
    // only the darker part of the ink side counts (text is near black, those shadows are not).
    let t = otsu_threshold(&g);
    let t = lo + ((t.saturating_sub(lo)) as f32 * EDGE_STRICT) as u8;
    let ink = |x: u32, y: u32| g.get_pixel(x, y).0[0] <= t;
    let band = ((w.min(h) as f32 * EDGE_BAND).round() as u32).max(2);
    if band * 4 >= w.min(h) {
        return false;
    }
    let mut sides = 0;
    for side in 0..4 {
        // `at(i, d)`: pixel at position i along the side, depth d from the edge.
        let len = if side < 2 { w } else { h };
        let at = |i: u32, d: u32| -> bool {
            match side {
                0 => ink(i, d),
                1 => ink(i, h - 1 - d),
                2 => ink(d, i),
                _ => ink(w - 1 - d, i),
            }
        };
        let skip = (len as f32 * EDGE_CORNER) as u32;
        let (a, b) = (skip, len - skip);
        let outer: Vec<bool> = (a..b).map(|i| (0..band).any(|d| at(i, d))).collect();
        let covered = outer.iter().filter(|&&x| x).count();
        if covered == 0 || covered as f32 / outer.len() as f32 > EDGE_FRAME_FRACTION {
            continue;
        }
        let max_run = ((len as f32 * EDGE_MAX_RUN) as usize).max(1);
        let mut found = false;
        let mut i = 0;
        while i < outer.len() {
            if !outer[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < outer.len() && outer[i] {
                i += 1;
            }
            let run = i - start;
            // Runs reaching the skipped corner zone continue into a corner shadow.
            if run > max_run || start == 0 || i == outer.len() {
                continue;
            }
            let inward = (start..i)
                .filter(|&k| (band..band * 2).any(|d| at(a + k as u32, d)))
                .count();
            if inward * 2 >= run {
                found = true;
                break;
            }
        }
        if found {
            sides += 1;
        }
    }
    sides >= 2
}

// ------------------------------------------------------ 1.5 / 1.14 duplicates

/// 64-bit difference hash: 9x8 grayscale thumbnail, one bit per horizontally
/// adjacent pair (left brighter than right). Empty images hash to 0.
pub fn perceptual_hash(img: &GrayImage) -> u64 {
    if img.width() == 0 || img.height() == 0 {
        return 0;
    }
    let t = imageops::resize(img, 9, 8, imageops::FilterType::Triangle);
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            hash <<= 1;
            if t.get_pixel(x, y).0[0] > t.get_pixel(x + 1, y).0[0] {
                hash |= 1;
            }
        }
    }
    hash
}

/// 32x32 grayscale thumbnail, row-major, for [`thumb_correlation`].
pub fn thumbnail(img: &GrayImage) -> Vec<u8> {
    if img.width() == 0 || img.height() == 0 {
        return Vec::new();
    }
    imageops::resize(img, 32, 32, imageops::FilterType::Triangle).into_raw()
}

/// Pearson correlation (-1..1) of two thumbnails; 0 when either is empty, of another size or
/// flat. The difference hash alone cannot tell two text pages apart (margins and columns
/// dominate its 64 bits), so duplicates are confirmed with this.
pub fn thumb_correlation(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let n = a.len() as f64;
    let ma = a.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mb = b.iter().map(|&v| v as f64).sum::<f64>() / n;
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (x as f64 - ma, y as f64 - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa <= 0.0 || bb <= 0.0 {
        return 0.0;
    }
    (ab / (aa.sqrt() * bb.sqrt())) as f32
}

/// All index pairs `(i, j)`, `i < j`, whose hashes differ in at most
/// `max_hamming` bits. O(n^2); fine for book-sized batches.
pub fn near_duplicates(hashes: &[u64], max_hamming: u32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..hashes.len() {
        for j in i + 1..hashes.len() {
            if (hashes[i] ^ hashes[j]).count_ones() <= max_hamming {
                out.push((i, j));
            }
        }
    }
    out
}

/// Keep-the-sharpest-of-a-burst, using EXIF times (seconds). Consecutive
/// frames belong to one group while the time gap is `<= max_gap_s`. A frame
/// with unknown time (or a missing entry in `times`) starts its own group, so
/// nothing is dropped without evidence; use [`best_of_burst_hashed`] when no
/// times exist. Returns one index per group, in order; ties keep the first.
pub fn best_of_burst(sharp: &[f32], times: &[Option<i64>], max_gap_s: i64) -> Vec<usize> {
    let time = |i: usize| times.get(i).copied().flatten();
    let same = |i: usize| match (time(i - 1), time(i)) {
        (Some(a), Some(b)) => (b - a).abs() <= max_gap_s,
        _ => false,
    };
    best_per_group(sharp, same)
}

/// Like [`best_of_burst`], but consecutive frames are grouped when their
/// perceptual hashes are within `max_hamming` bits (for files without EXIF time).
pub fn best_of_burst_hashed(sharp: &[f32], hashes: &[u64], max_hamming: u32) -> Vec<usize> {
    let same = |i: usize| match (hashes.get(i - 1), hashes.get(i)) {
        (Some(a), Some(b)) => (a ^ b).count_ones() <= max_hamming,
        _ => false,
    };
    best_per_group(sharp, same)
}

fn best_per_group(sharp: &[f32], same_as_prev: impl Fn(usize) -> bool) -> Vec<usize> {
    let key = |i: usize| {
        if sharp[i].is_nan() {
            f32::NEG_INFINITY
        } else {
            sharp[i]
        }
    };
    let mut out = Vec::new();
    let mut best: Option<usize> = None;
    for i in 0..sharp.len() {
        if i > 0 && same_as_prev(i) {
            if let Some(b) = best {
                if key(i) > key(b) {
                    best = Some(i);
                }
            }
        } else {
            if let Some(b) = best {
                out.push(b);
            }
            best = Some(i);
        }
    }
    out.extend(best);
    out
}

// ------------------------------------------------------- 1.5 page numbers

/// Gaps and repeats in detected page numbers (`None` entries are skipped).
/// For each pair of consecutive detected numbers `prev`, `next` where
/// `next != prev + 1`, reports `(index_of_next, prev + 1, next)`: the index in
/// `nums`, the number expected and the number found. A single misread number
/// therefore yields two entries (into and out of the outlier).
pub fn page_number_gaps(nums: &[Option<u32>]) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let mut prev: Option<u32> = None;
    for (i, n) in nums.iter().enumerate() {
        let Some(n) = *n else { continue };
        if let Some(p) = prev {
            let expected = p.saturating_add(1);
            if n != expected {
                out.push((i, expected, n));
            }
        }
        prev = Some(n);
    }
    out
}

/// The printed page number on a page: a lone 1–4 digit token (optionally wrapped in dashes
/// or dots) on the last line near the foot, else on the first line near the head.
pub fn detect_page_number(words: &[crate::text::Word], page_h: u32) -> Option<u32> {
    if words.is_empty() || page_h == 0 {
        return None;
    }
    let clean = |t: &str| -> Option<u32> {
        let d: String = t
            .trim_matches(|c: char| "-–—.·•|[]() ".contains(c))
            .to_string();
        if (1..=4).contains(&d.len()) && d.chars().all(|c| c.is_ascii_digit()) {
            d.parse().ok().filter(|n| *n > 0)
        } else {
            None
        }
    };
    let line_of =
        |idx: u32| -> Vec<&crate::text::Word> { words.iter().filter(|w| w.line == idx).collect() };
    let (first, last) = (
        words.iter().map(|w| w.line).min()?,
        words.iter().map(|w| w.line).max()?,
    );
    let band = page_h / 8;
    for (idx, foot) in [(last, true), (first, false)] {
        let ws = line_of(idx);
        let y = ws.iter().map(|w| w.y + w.h / 2).sum::<u32>() / ws.len() as u32;
        let near = if foot { y >= page_h - band } else { y <= band };
        if !near {
            continue;
        }
        // Only the number itself (plus decoration such as dashes) may stand on the line.
        let nums: Vec<u32> = ws.iter().filter_map(|w| clean(&w.text)).collect();
        let others = ws
            .iter()
            .filter(|w| {
                clean(&w.text).is_none()
                    && !w
                        .text
                        .trim_matches(|c: char| "-–—.·•|".contains(c))
                        .is_empty()
            })
            .count();
        if nums.len() == 1 && others == 0 {
            return Some(nums[0]);
        }
    }
    None
}

/// Where the printed numbering starts a new run: `(page_index, number)` pairs. A run needs at
/// least 3 detected pages that agree on the offset `number - index`; undetected pages inside or
/// next to a run take the run's offset. Stray misreads therefore never start a run.
pub fn label_runs(nums: &[Option<u32>]) -> Vec<(usize, u32)> {
    let offs: Vec<Option<i64>> = nums
        .iter()
        .enumerate()
        .map(|(i, n)| n.map(|n| n as i64 - i as i64))
        .collect();
    let det: Vec<(usize, i64)> = offs
        .iter()
        .enumerate()
        .filter_map(|(i, o)| o.map(|o| (i, o)))
        .collect();
    let mut runs: Vec<(usize, usize, i64)> = Vec::new(); // (first detected idx, last detected idx, offset)
    let mut k = 0;
    while k < det.len() {
        let mut j = k;
        while j + 1 < det.len() && det[j + 1].1 == det[k].1 {
            j += 1;
        }
        // tolerate single stray misreads between two agreeing stretches
        if j - k + 1 >= 3 {
            match runs.last_mut() {
                Some(r) if r.2 == det[k].1 => r.1 = det[j].0,
                _ => runs.push((det[k].0, det[j].0, det[k].1)),
            }
        }
        k = j + 1;
    }
    let mut out = Vec::new();
    for (n, r) in runs.iter().enumerate() {
        let start = if n == 0 {
            r.0.max((1 - r.2).max(0) as usize)
        } else {
            runs[n - 1].1 + 1
        };
        let number = r.2 + start as i64;
        if number >= 1 {
            out.push((start, number as u32));
        }
    }
    // numbering that already is 1.. from page 0 needs no label
    if out.len() == 1 && out[0] == (0, 1) {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn w(text: &str, y: u32, line: u32) -> crate::text::Word {
        crate::text::Word {
            text: text.into(),
            x: 100,
            y,
            w: 30,
            h: 20,
            conf: 90.0,
            line,
        }
    }

    #[test]
    fn page_number_found_at_foot_or_head_only_when_alone() {
        let body = vec![w("Hello", 100, 0), w("world", 100, 0), w("17", 1180, 1)];
        assert_eq!(detect_page_number(&body, 1200), Some(17));
        let wrapped = vec![
            w("text", 100, 0),
            w("-", 1180, 1),
            w("42", 1180, 1),
            w("-", 1180, 1),
        ];
        assert_eq!(detect_page_number(&wrapped, 1200), Some(42));
        let head = vec![
            w("5", 20, 0),
            w("text", 600, 1),
            w("more", 700, 2),
            w("end", 800, 3),
        ];
        assert_eq!(detect_page_number(&head, 1200), Some(5));
        let year = vec![w("in", 100, 0), w("1999", 1180, 1), w("war", 1180, 1)];
        assert_eq!(detect_page_number(&year, 1200), None);
        let mid = vec![w("a", 100, 0), w("12", 600, 1), w("b", 1100, 2)];
        assert_eq!(detect_page_number(&mid, 1200), None);
    }

    #[test]
    fn label_runs_follow_consistent_numbering() {
        // pages 0..3 unnumbered cover, then 1,2,3,4 from index 3 on
        let n = [None, None, None, Some(1), Some(2), None, Some(4), Some(5)];
        assert_eq!(label_runs(&n), vec![(3, 1)]);
        // a lone misread does not start a run
        assert!(label_runs(&[Some(1), Some(2), Some(3), Some(40), Some(5)]).is_empty());
        // numbering 1.. from the first page needs no labels
        assert!(label_runs(&[Some(1), Some(2), Some(3), Some(4)]).is_empty());
        // numbers offset by 8 across the whole book
        assert_eq!(
            label_runs(&[Some(9), Some(10), Some(11), Some(12)]),
            vec![(0, 9)]
        );
        // unnumbered cover keeps the default label, numbering starts at the first printed page
        assert_eq!(
            label_runs(&[None, Some(17), Some(18), Some(19)]),
            vec![(1, 17)]
        );
    }

    /// Text-like page: dark horizontal strokes on paper, 3 px thick, irregular.
    fn stripes(w: u32, h: u32) -> GrayImage {
        let mut g = GrayImage::from_pixel(w, h, Luma([235]));
        let mut seed: u32 = 777;
        let mut rnd = move |m: u32| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) % m
        };
        let mut y = h / 10;
        while y + 20 < h * 9 / 10 {
            let mut x = w / 10;
            while x + 40 < w * 9 / 10 {
                let bw = 6 + rnd(30);
                for yy in y..y + 3 {
                    for xx in x..x + bw {
                        g.put_pixel(xx, yy, Luma([30]));
                    }
                }
                for yy in y + 5..y + 12 {
                    for xx in x..(x + 2) {
                        g.put_pixel(xx, yy, Luma([30]));
                    }
                }
                x += bw + 5 + rnd(8);
            }
            y += 26;
        }
        g
    }

    fn fill(g: &mut GrayImage, x0: u32, y0: u32, x1: u32, y1: u32, v: u8) {
        for y in y0..y1 {
            for x in x0..x1 {
                g.put_pixel(x, y, Luma([v]));
            }
        }
    }

    #[test]
    fn dpi_math_and_advice() {
        // A4 at 300 dpi = 2480 x 3508 px.
        let d = effective_dpi(2480, 3508, (210.0, 297.0));
        assert!((d - 300.0).abs() < 1.0, "{d}");
        assert_eq!(dpi_advice(d), DpiAdvice::Good);
        assert_eq!(dpi_advice(150.0), DpiAdvice::TooLow);
        assert_eq!(dpi_advice(199.9), DpiAdvice::TooLow);
        assert_eq!(dpi_advice(200.0), DpiAdvice::Good);
        assert_eq!(dpi_advice(400.0), DpiAdvice::Good);
        assert_eq!(dpi_advice(600.0), DpiAdvice::ExcessForText);
        assert_eq!(dpi_advice(f32::NAN), DpiAdvice::TooLow);
        assert_eq!(effective_dpi(100, 100, (0.0, 297.0)), 0.0);
        assert_eq!(effective_dpi(100, 100, (f32::NAN, 1.0)), 0.0);
        assert_eq!(effective_dpi(0, 0, (210.0, 297.0)), 0.0);
        for a in [DpiAdvice::TooLow, DpiAdvice::Good, DpiAdvice::ExcessForText] {
            assert!(!a.advice_text().is_empty());
            assert!(a.key().starts_with("quality.dpi."));
        }
    }

    #[test]
    fn sharp_vs_blurred() {
        let sharp = stripes(800, 1100);
        let blur = imageops::blur(&sharp, 3.0);
        let (s, b) = (sharpness(&sharp), sharpness(&blur));
        assert!(s > 3.0 * b, "sharp {s} blur {b}");
        assert!(b < BLUR_ABSOLUTE, "blur {b}");
        assert_eq!(blurred_pages(&[s, s * 0.9, b, s * 1.1]), vec![2]);
        assert!(is_blurred(b, s));
        assert!(!is_blurred(s, s));
    }

    #[test]
    fn blurred_pages_edge_cases() {
        assert!(blurred_pages(&[]).is_empty());
        assert!(blurred_pages(&[f32::NAN]).is_empty());
        assert!(blurred_pages(&[5.0]).is_empty());
        // Crisp in absolute terms: never flagged even if far below median.
        assert!(blurred_pages(&[5000.0, 5000.0, 5000.0, 1000.0]).is_empty());
        // Low but close to the median: not flagged.
        assert!(blurred_pages(&[50.0, 60.0, 55.0]).is_empty());
        assert!(!is_blurred(1.0, 0.0));
        assert!(!is_blurred(f32::NAN, 10.0));
    }

    #[test]
    fn sharpness_degenerate() {
        assert_eq!(sharpness(&GrayImage::new(0, 0)), 0.0);
        assert_eq!(sharpness(&GrayImage::new(2, 2)), 0.0);
        assert_eq!(
            sharpness(&GrayImage::from_pixel(100, 100, Luma([255]))),
            0.0
        );
        assert_eq!(sharpness(&GrayImage::from_pixel(100, 100, Luma([0]))), 0.0);
    }

    #[test]
    fn cut_text_detected_on_two_sides() {
        let mut g = stripes(800, 1100);
        assert!(!edge_text_cut(&g));
        // Glyph-like bars touching the top and the left edges, running inward.
        fill(&mut g, 300, 0, 320, 40, 30);
        fill(&mut g, 0, 500, 40, 520, 30);
        assert!(edge_text_cut(&g));
    }

    #[test]
    fn cut_text_one_side_only_is_not_enough() {
        let mut g = stripes(800, 1100);
        fill(&mut g, 300, 0, 320, 40, 30);
        assert!(!edge_text_cut(&g));
    }

    #[test]
    fn scanner_frame_and_shadow_are_ignored() {
        // Full black frame.
        let mut g = stripes(800, 1100);
        fill(&mut g, 0, 0, 800, 12, 0);
        fill(&mut g, 0, 1088, 800, 1100, 0);
        fill(&mut g, 0, 0, 12, 1100, 0);
        fill(&mut g, 788, 0, 800, 1100, 0);
        assert!(!edge_text_cut(&g));
        // Dark shadow strip down the left (binding) plus a corner blob.
        let mut g = stripes(800, 1100);
        fill(&mut g, 0, 0, 60, 1100, 20);
        fill(&mut g, 0, 0, 90, 90, 0);
        assert!(!edge_text_cut(&g));
    }

    #[test]
    fn dust_speck_is_not_cut_text() {
        let mut g = stripes(800, 1100);
        fill(&mut g, 300, 0, 303, 2, 0);
        fill(&mut g, 0, 500, 2, 503, 0);
        assert!(!edge_text_cut(&g));
    }

    #[test]
    fn browned_page_edges_are_not_cut_text() {
        // paper 215, edges and spine darkening to 110 (aged paper), text rows 25 inside the margins
        let page = |cut: bool| {
            GrayImage::from_fn(600, 800, |x, y| {
                let d = x.min(599 - x).min(y).min(799 - y) as f32;
                let paper = 215.0 - 105.0 * (1.0 - (d / 60.0).min(1.0));
                let in_text = if cut {
                    (0..600).contains(&x) && (y / 14) % 2 == 0 && (0..800).contains(&y)
                } else {
                    (80..520).contains(&x) && (80..720).contains(&y) && (y / 14) % 2 == 0
                } && (x / 7) % 3 != 0;
                Luma([if in_text { 25 } else { paper as u8 }])
            })
        };
        assert!(
            !edge_text_cut(&page(false)),
            "shadowed margins, text inside: not cut"
        );
        assert!(
            edge_text_cut(&page(true)),
            "text running off every side: cut"
        );
    }

    #[test]
    fn edge_cut_degenerate_images() {
        assert!(!edge_text_cut(&GrayImage::new(0, 0)));
        assert!(!edge_text_cut(&GrayImage::new(5, 5)));
        assert!(!edge_text_cut(&GrayImage::from_pixel(
            300,
            300,
            Luma([255])
        )));
        assert!(!edge_text_cut(&GrayImage::from_pixel(300, 300, Luma([0]))));
    }

    #[test]
    fn hash_and_duplicates() {
        let a = stripes(800, 1100);
        // Same page shifted by a few pixels (re-shot) and a different page.
        let mut b = GrayImage::from_pixel(800, 1100, Luma([235]));
        imageops::replace(&mut b, &a, 4, 3);
        let mut c = GrayImage::from_pixel(800, 1100, Luma([235]));
        for y in 0..1100 {
            for x in 0..800 {
                c.put_pixel(x, y, Luma([(x * 255 / 799) as u8]));
            }
        }
        let (ha, hb, hc) = (
            perceptual_hash(&a),
            perceptual_hash(&b),
            perceptual_hash(&c),
        );
        assert!((ha ^ hb).count_ones() <= 6, "{}", (ha ^ hb).count_ones());
        assert!((ha ^ hc).count_ones() > 6);
        assert_eq!(near_duplicates(&[ha, hc, hb], 6), vec![(0, 2)]);
        assert_eq!(perceptual_hash(&a), perceptual_hash(&a.clone()));
    }

    #[test]
    fn correlation_separates_pages_the_hash_cannot() {
        let a = stripes(800, 1100);
        let mut b = GrayImage::from_pixel(800, 1100, Luma([235]));
        imageops::replace(&mut b, &a, 4, 3);
        // same layout, other text: stripes of another phase inside the same margins
        let c = GrayImage::from_fn(800, 1100, |x, y| {
            if (60..1040).contains(&y) && (80..720).contains(&x) && ((y + 7) / 11 + x / 23) % 2 == 0
            {
                Luma([20])
            } else {
                Luma([235])
            }
        });
        let (ta, tb, tc) = (thumbnail(&a), thumbnail(&b), thumbnail(&c));
        assert!(
            thumb_correlation(&ta, &tb) > 0.9,
            "{}",
            thumb_correlation(&ta, &tb)
        );
        assert!(
            thumb_correlation(&ta, &tc) < 0.9,
            "{}",
            thumb_correlation(&ta, &tc)
        );
        assert_eq!(thumb_correlation(&[], &[]), 0.0);
        assert_eq!(thumb_correlation(&[5; 4], &[5; 4]), 0.0, "flat images");
        assert_eq!(thumb_correlation(&ta, &ta[1..]), 0.0);
    }

    #[test]
    fn hash_degenerate() {
        assert_eq!(perceptual_hash(&GrayImage::new(0, 0)), 0);
        let _ = perceptual_hash(&GrayImage::new(1, 1));
        assert_eq!(
            perceptual_hash(&GrayImage::from_pixel(50, 50, Luma([0]))),
            perceptual_hash(&GrayImage::from_pixel(50, 50, Luma([255])))
        );
        assert!(near_duplicates(&[], 3).is_empty());
        assert!(near_duplicates(&[1], 3).is_empty());
        assert_eq!(near_duplicates(&[0, 0], 0), vec![(0, 1)]);
    }

    #[test]
    fn burst_by_time() {
        let sharp = [1.0, 5.0, 3.0, 2.0, 9.0, 4.0];
        let times = [Some(100), Some(101), Some(102), Some(200), Some(201), None];
        // Groups: {0,1,2}, {3,4}, {5} (unknown time keeps itself).
        assert_eq!(best_of_burst(&sharp, &times, 2), vec![1, 4, 5]);
        // Gap too small to join anything.
        assert_eq!(best_of_burst(&sharp, &times, 0), vec![0, 1, 2, 3, 4, 5]);
        assert!(best_of_burst(&[], &[], 2).is_empty());
        // Missing times: nothing grouped.
        assert_eq!(best_of_burst(&[1.0, 2.0], &[], 2), vec![0, 1]);
        // Ties keep the first; NaN loses.
        assert_eq!(
            best_of_burst(&[2.0, 2.0, f32::NAN], &[Some(0), Some(1), Some(2)], 5),
            vec![0]
        );
    }

    #[test]
    fn burst_by_hash() {
        let sharp = [1.0, 7.0, 3.0, 2.0];
        let hashes = [0u64, 1, u64::MAX, u64::MAX - 1];
        assert_eq!(best_of_burst_hashed(&sharp, &hashes, 2), vec![1, 2]);
        assert!(best_of_burst_hashed(&[], &[], 2).is_empty());
    }

    #[test]
    fn page_number_sequences() {
        assert!(page_number_gaps(&[]).is_empty());
        assert!(page_number_gaps(&[None, None]).is_empty());
        assert!(page_number_gaps(&[Some(1), Some(2), Some(3)]).is_empty());
        assert_eq!(
            page_number_gaps(&[Some(1), None, Some(3), Some(4), Some(4), Some(9)]),
            vec![(2, 2, 3), (4, 5, 4), (5, 5, 9)]
        );
        // None in between is skipped, consecutive across it is fine.
        assert!(page_number_gaps(&[Some(7), None, Some(8)]).is_empty());
        assert_eq!(page_number_gaps(&[Some(u32::MAX), Some(1)]).len(), 1);
    }
}
