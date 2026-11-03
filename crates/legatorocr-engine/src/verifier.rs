//! Document verifier (roadmap 7.5): checks that can be done on a picture of a document without
//! knowing what is genuine. Each result is a **hint for a person to look at**, never a verdict
//! of forgery or authenticity.
//!
//! - QR codes: decoded with the `rqrr` crate (payload text; a verifier-URL still has to be
//!   checked by the person with the issuer).
//! - Copy-move: blocks of the picture that appear again elsewhere with the same offset (a patch
//!   copied inside one picture). Repeated letters of a text page are common and can give false
//!   alarms; the measured rate is in the roadmap.
//!
//! Error-level analysis (ELA) was tried and removed: on a page spliced into a JPEG it did not
//! point at the splice and it flagged blocks of an untouched original (see ROADMAP 7.5).

use image::GrayImage;
use std::collections::HashMap;

/// Decoded QR payloads.
pub fn qr_codes(g: &GrayImage) -> Vec<String> {
    qr_read(g).into_iter().map(|(t, _)| t).collect()
}

/// Decoded QR payloads with their bounding boxes (x0, y0, x1, y1).
pub fn qr_read(g: &GrayImage) -> Vec<(String, (i32, i32, i32, i32))> {
    let mut p = rqrr::PreparedImage::prepare_from_greyscale(g.width() as usize, g.height() as usize, |x, y| g.get_pixel(x as u32, y as u32)[0]);
    p.detect_grids()
        .into_iter()
        .filter_map(|grid| {
            let (_, text) = grid.decode().ok()?;
            let xs = grid.bounds.iter().map(|b| b.x);
            let ys = grid.bounds.iter().map(|b| b.y);
            Some((text, (xs.clone().min()?, ys.clone().min()?, xs.max()?, ys.max()?)))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyMove {
    /// Offset between the two copies, in pixels.
    pub dx: i32,
    pub dy: i32,
    /// Number of matching blocks that agree on this offset.
    pub blocks: usize,
    /// Bounding box (x0, y0, x1, y1) of the source copy.
    pub bbox: (u32, u32, u32, u32),
}

const BLOCK: u32 = 16;
const STRIDE: u32 = 4;

/// Hash keys of a block: 4×4 cell means quantised in steps of 32 grey levels, once as is and once
/// shifted by half a step, so a cell near a step boundary still agrees in one of the two keys
/// after the grey levels moved by re-compression. Low-contrast blocks carry no evidence.
fn block_keys(g: &GrayImage, x: u32, y: u32) -> Option<[u64; 2]> {
    let mut cells = [0u32; 16];
    let (mut s, mut s2) = (0u64, 0u64);
    for j in 0..BLOCK {
        for i in 0..BLOCK {
            let v = g.get_pixel(x + i, y + j)[0] as u32;
            cells[((j / 4) * 4 + i / 4) as usize] += v;
            s += v as u64;
            s2 += (v * v) as u64;
        }
    }
    let n = (BLOCK * BLOCK) as f64;
    let var = s2 as f64 / n - (s as f64 / n).powi(2);
    if var < 100.0 {
        return None;
    }
    let mut k = [0u64; 2];
    for c in cells {
        let m = c / 16;
        k[0] = k[0].wrapping_mul(31).wrapping_add((m / 32) as u64);
        k[1] = k[1].wrapping_mul(31).wrapping_add(((m + 16) / 32) as u64);
    }
    Some(k)
}

/// Copy-move candidates with at least `min_blocks` agreeing blocks and a distance of at least `min_dist` px.
pub fn copy_move(g: &GrayImage, min_blocks: usize, min_dist: i32) -> Vec<CopyMove> {
    let (w, h) = g.dimensions();
    if w < BLOCK * 2 || h < BLOCK * 2 {
        return Vec::new();
    }
    let mut buckets: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    let mut y = 0;
    while y + BLOCK <= h {
        let mut x = 0;
        while x + BLOCK <= w {
            if let Some(keys) = block_keys(g, x, y) {
                for (variant, k) in keys.into_iter().enumerate() {
                    let v = buckets.entry(k ^ ((variant as u64) << 63)).or_default();
                    if v.len() < 12 {
                        v.push((x, y));
                    }
                }
            }
            x += STRIDE;
        }
        y += STRIDE;
    }
    let mut votes: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
    for v in buckets.values() {
        for (i, a) in v.iter().enumerate() {
            for b in &v[i + 1..] {
                let (mut dx, mut dy) = (b.0 as i32 - a.0 as i32, b.1 as i32 - a.1 as i32);
                if dx * dx + dy * dy < min_dist * min_dist {
                    continue;
                }
                let mut src = *a;
                // canonical direction so both orders vote for the same offset
                if dy < 0 || (dy == 0 && dx < 0) {
                    dx = -dx;
                    dy = -dy;
                    src = *b;
                }
                votes.entry((dx, dy)).or_default().push(src);
            }
        }
    }
    // a periodic pattern (rules, a QR code, columns of text) repeats at multiples of its period
    let counts: HashMap<(i32, i32), usize> = votes.iter().map(|(k, v)| (*k, v.len())).collect();
    let mut out: Vec<CopyMove> = votes
        .into_iter()
        .filter(|(_, v)| v.len() >= min_blocks)
        .filter(|((dx, dy), v)| {
            let twice = counts.get(&(2 * dx, 2 * dy)).copied().unwrap_or(0);
            twice * 3 < v.len()
        })
        .filter(|(_, v)| {
            // a pasted patch fills its bounding box; scattered coincidences do not
            let x0 = v.iter().map(|p| p.0).min().unwrap();
            let y0 = v.iter().map(|p| p.1).min().unwrap();
            let x1 = v.iter().map(|p| p.0).max().unwrap() + BLOCK;
            let y1 = v.iter().map(|p| p.1).max().unwrap() + BLOCK;
            let area = ((x1 - x0) * (y1 - y0)) as f64;
            let covered = (v.len() as f64) * (STRIDE * STRIDE) as f64;
            covered / area > 0.5
        })
        .map(|((dx, dy), v)| {
            let x0 = v.iter().map(|p| p.0).min().unwrap();
            let y0 = v.iter().map(|p| p.1).min().unwrap();
            let x1 = v.iter().map(|p| p.0).max().unwrap() + BLOCK;
            let y1 = v.iter().map(|p| p.1).max().unwrap() + BLOCK;
            CopyMove { dx, dy, blocks: v.len(), bbox: (x0, y0, x1, y1) }
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.blocks));
    out.truncate(5);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(w: u32, h: u32, seed: u64) -> GrayImage {
        let mut s = seed;
        let mut g = GrayImage::new(w, h);
        for p in g.pixels_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            p[0] = (s >> 56) as u8;
        }
        g
    }

    #[test]
    fn copied_patch_is_found_and_plain_noise_is_not() {
        let mut g = texture(300, 200, 1);
        assert!(copy_move(&g, 15, 24).is_empty(), "independent noise has no copy");
        // copy a 64×48 patch from (20,20) to (180,100)
        for j in 0..48 {
            for i in 0..64 {
                let v = *g.get_pixel(20 + i, 20 + j);
                g.put_pixel(180 + i, 100 + j, v);
            }
        }
        let found = copy_move(&g, 15, 24);
        assert!(!found.is_empty());
        assert_eq!((found[0].dx, found[0].dy), (160, 80));
    }

    #[test]
    fn qr_code_is_read_and_noise_is_not() {
        // fixture made with python-qrcode: https://example.org/verifica/ABC-123 on a grey page
        let img = image::load_from_memory(include_bytes!("../tests/data/qr.png")).unwrap().to_luma8();
        assert_eq!(qr_codes(&img), vec!["https://example.org/verifica/ABC-123".to_string()]);
        assert!(qr_codes(&texture(120, 120, 3)).is_empty());
    }
}

/// Result of the JPEG-ghost test (Farid 2009): the picture is re-saved at many
/// qualities; a picture that was saved at a lower quality earlier shows a dip in the
/// difference curve at that earlier quality.
#[derive(Debug, Clone)]
pub struct Ghost {
    /// (quality, mean squared difference of the luma) for each quality tried.
    pub curve: Vec<(u8, f64)>,
    /// Earlier quality the dip points to, if a clear dip exists.
    pub earlier_quality: Option<u8>,
}

/// Look for traces of an earlier, lower-quality JPEG save. `None` if `bytes` is not a JPEG.
/// A hint only: resizing, cropping off the 8x8 grid or heavy noise hide the dip.
pub fn jpeg_ghost(bytes: &[u8]) -> Option<Ghost> {
    use image::codecs::jpeg::JpegEncoder;
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let g = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).ok()?.to_luma8();
    let (w, h) = (g.width() / 8 * 8, g.height() / 8 * 8);
    if w < 64 || h < 64 {
        return None;
    }
    let g = image::imageops::crop_imm(&g, 0, 0, w, h).to_image();
    let mut curve = Vec::new();
    let mut q = 30u8;
    while q <= 96 {
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, q)
            .encode(g.as_raw(), w, h, image::ExtendedColorType::L8)
            .ok()?;
        let r = image::load_from_memory_with_format(&buf, image::ImageFormat::Jpeg).ok()?.to_luma8();
        let d: f64 = g
            .as_raw()
            .iter()
            .zip(r.as_raw())
            .map(|(a, b)| {
                let e = *a as f64 - *b as f64;
                e * e
            })
            .sum::<f64>()
            / g.as_raw().len() as f64;
        curve.push((q, d));
        q += 3;
    }
    // a dip: lower than both neighbours two steps away by a clear relative margin,
    // followed later by a rise, while a single save falls steadily with quality
    // the last save always makes the deepest dip (difference near zero); look below it
    let last = curve.iter().enumerate().min_by(|a, b| a.1 .1.total_cmp(&b.1 .1)).map(|(i, _)| i)?;
    let mut earlier = None;
    for i in 2..curve.len().saturating_sub(2).min(last.saturating_sub(1)) {
        let (q, d) = curve[i];
        let (l, r) = (curve[i - 2].1, curve[i + 2].1);
        if d < l * 0.97 && d < r * 0.97 && d > 0.0 {
            earlier = Some(q);
            break;
        }
    }
    if std::env::var_os("LEGATOROCR_GHOST_DEBUG").is_some() {
        eprintln!("GHOSTCURVE {:?}", curve);
    }
    Some(Ghost { curve, earlier_quality: earlier })
}

#[cfg(test)]
mod ghost_tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;

    fn texture() -> image::GrayImage {
        // pseudo-random smooth-ish texture with edges, as a scanned page would have
        let mut s = 12345u32;
        image::GrayImage::from_fn(320, 320, |x, y| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (s >> 24) as i32 % 40;
            let base = if (x / 16 + y / 16) % 2 == 0 { 80 } else { 170 };
            let wave = ((x as f32 * 0.21).sin() * 30.0 + (y as f32 * 0.13).cos() * 25.0) as i32;
            image::Luma([(base + wave + n).clamp(0, 255) as u8])
        })
    }
    fn save(g: &image::GrayImage, q: u8) -> Vec<u8> {
        let mut b = Vec::new();
        JpegEncoder::new_with_quality(&mut b, q).encode(g.as_raw(), g.width(), g.height(), image::ExtendedColorType::L8).unwrap();
        b
    }
    fn report(label: &str, bytes: &[u8]) -> Option<u8> {
        let gh = jpeg_ghost(bytes).unwrap();
        eprintln!("{label}: {:?} -> {:?}", gh.curve.iter().map(|(q, d)| (*q, (d * 100.0).round() / 100.0)).collect::<Vec<_>>(), gh.earlier_quality);
        gh.earlier_quality
    }

    #[test]
    fn a_jpeg_saved_twice_shows_the_earlier_quality_and_a_single_save_does_not() {
        let t = texture();
        let single = save(&t, 90);
        let first = save(&t, 60);
        let dec = image::load_from_memory(&first).unwrap().to_luma8();
        let double = save(&dec, 90);
        let s = report("single q90", &single);
        let d = report("60 then 90", &double);
        assert!(s.is_none(), "false alarm on a single save: {s:?}");
        let q = d.expect("double save not found");
        assert!((54..=66).contains(&q), "dip at {q}");
        assert!(jpeg_ghost(b"not a jpeg").is_none());
    }
}
