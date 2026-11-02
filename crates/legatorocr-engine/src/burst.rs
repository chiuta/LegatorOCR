//! Phone-capture ingest: put photos in the order they were taken and keep only the sharpest
//! frame of each burst. Thresholds are untuned heuristics; what is dropped is always printed.

use legatorocr_core::quality as q;
use std::path::{Path, PathBuf};

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026:11:01 14:03:09" → seconds since 1970 (the camera's clock, read as UTC).
fn parse_datetime(s: &[u8]) -> Option<i64> {
    let t = std::str::from_utf8(s).ok()?.trim_matches('\0').trim();
    let (d, h) = t.split_once(' ')?;
    let n = |x: &str| x.parse::<i64>().ok();
    let mut dp = d.split(':');
    let (y, mo, da) = (n(dp.next()?)?, n(dp.next()?)?, n(dp.next()?)?);
    let mut hp = h.split(':');
    let (hh, mi, ss) = (n(hp.next()?)?, n(hp.next()?)?, n(hp.next()?)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || y < 1990 || hh > 23 || mi > 59 || ss > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, da) * 86_400 + hh * 3600 + mi * 60 + ss)
}

/// Capture time from the EXIF block of a JPEG (`DateTimeOriginal`, else `DateTime`).
pub fn exif_time_of(data: &[u8]) -> Option<i64> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() && data[i] == 0xFF {
        let marker = data[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let seg = data.get(i + 4..i + 2 + len)?;
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return tiff_time(&seg[6..]);
        }
        i += 2 + len;
    }
    None
}

fn tiff_time(t: &[u8]) -> Option<i64> {
    let le = match t.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    // (tag → value offset or inline value, count) of one IFD
    let entries = |ifd: usize| -> Vec<(u16, u16, u32, usize)> {
        let mut v = Vec::new();
        let Some(n) = u16_at(ifd) else { return v };
        for k in 0..(n as usize).min(200) {
            let e = ifd + 2 + k * 12;
            if let (Some(tag), Some(ty), Some(cnt)) = (u16_at(e), u16_at(e + 2), u32_at(e + 4)) {
                v.push((tag, ty, cnt, e + 8));
            }
        }
        v
    };
    let ascii = |ty: u16, cnt: u32, at: usize| -> Option<i64> {
        if ty != 2 || !(19..=40).contains(&cnt) {
            return None;
        }
        let off = u32_at(at)? as usize;
        parse_datetime(t.get(off..off + cnt as usize)?)
    };
    let ifd0 = u32_at(4)? as usize;
    let e0 = entries(ifd0);
    if let Some(&(_, _, _, at)) = e0.iter().find(|e| e.0 == 0x8769) {
        let exif_ifd = u32_at(at)? as usize;
        for tag in [0x9003u16, 0x9004] {
            if let Some(&(_, ty, cnt, at)) = entries(exif_ifd).iter().find(|e| e.0 == tag) {
                if let Some(t) = ascii(ty, cnt, at) {
                    return Some(t);
                }
            }
        }
    }
    e0.iter()
        .find(|e| e.0 == 0x0132)
        .and_then(|&(_, ty, cnt, at)| ascii(ty, cnt, at))
}

pub fn exif_time(path: &Path) -> Option<i64> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 256 * 1024];
    let n = f.read(&mut head).ok()?;
    exif_time_of(&head[..n])
}

/// Result of [`ingest`].
pub struct Ingest {
    pub paths: Vec<PathBuf>,
    /// Whether the order was changed by capture time.
    pub reordered: bool,
    /// Frames left out as weaker copies of a burst: `(dropped, kept instead)`.
    pub dropped: Vec<(PathBuf, PathBuf)>,
}

/// Order `paths` by capture time (only when every file has one) and keep the sharpest frame of
/// each burst: consecutive frames within `max_gap_s` seconds, or, without times, whose
/// perceptual hashes differ in at most 6 bits.
pub fn ingest(paths: &[PathBuf], max_gap_s: i64) -> Ingest {
    let mut items: Vec<(PathBuf, Option<i64>)> = paths.iter().map(|p| (p.clone(), exif_time(p))).collect();
    let all_timed = !items.is_empty() && items.iter().all(|i| i.1.is_some());
    let mut reordered = false;
    if all_timed {
        let before: Vec<PathBuf> = items.iter().map(|i| i.0.clone()).collect();
        items.sort_by_key(|i| i.1);
        reordered = before != items.iter().map(|i| i.0.clone()).collect::<Vec<_>>();
    }
    let mut sharp = Vec::new();
    let mut hashes = Vec::new();
    for (p, _) in &items {
        match crate::load_image(p) {
            Ok(img) => {
                let small = img.thumbnail(1000, 1000).to_luma8();
                sharp.push(q::sharpness(&small));
                hashes.push(q::perceptual_hash(&small));
            }
            Err(_) => {
                sharp.push(f32::NAN);
                hashes.push(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(sharp.len() as u64 + 1)); // unlikely to group
            }
        }
    }
    let keep: Vec<usize> = if all_timed {
        let times: Vec<Option<i64>> = items.iter().map(|i| i.1).collect();
        q::best_of_burst(&sharp, &times, max_gap_s)
    } else {
        q::best_of_burst_hashed(&sharp, &hashes, 6)
    };
    let mut out = Vec::new();
    let mut dropped = Vec::new();
    for (i, (p, _)) in items.iter().enumerate() {
        if keep.contains(&i) {
            out.push(p.clone());
        } else if let Some(k) = keep
            .iter()
            .copied()
            .min_by_key(|k| ((hashes[*k] ^ hashes[i]).count_ones(), k.abs_diff(i)))
        {
            dropped.push((p.clone(), items[k].0.clone()));
        }
    }
    Ingest { paths: out, reordered, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JPEG with an APP1/Exif block carrying DateTimeOriginal (big-endian TIFF).
    fn jpeg_with_time(dt: &str) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\0\x2A\0\0\0\x08");
        // IFD0: one entry, Exif IFD pointer → offset 26
        tiff.extend_from_slice(&[0, 1, 0x87, 0x69, 0, 4, 0, 0, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0]);
        // Exif IFD at 26: one entry, 0x9003 ASCII count 20 → offset 44
        tiff.extend_from_slice(&[0, 1, 0x90, 0x03, 0, 2, 0, 0, 0, 20, 0, 0, 0, 44, 0, 0, 0, 0]);
        tiff.extend_from_slice(dt.as_bytes());
        tiff.push(0);
        let mut app = b"Exif\0\0".to_vec();
        app.extend_from_slice(&tiff);
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE1];
        j.extend_from_slice(&((app.len() + 2) as u16).to_be_bytes());
        j.extend_from_slice(&app);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    #[test]
    fn exif_capture_time_is_read() {
        let a = exif_time_of(&jpeg_with_time("2026:11:01 14:03:09")).unwrap();
        let b = exif_time_of(&jpeg_with_time("2026:11:01 14:03:12")).unwrap();
        assert_eq!(b - a, 3);
        assert_eq!(a, 1_793_541_789); // 2026-11-01T14:03:09Z
        assert!(exif_time_of(b"not a jpeg").is_none());
        assert!(exif_time_of(&jpeg_with_time("0000:00:00 00:00:00")).is_none());
    }
}
