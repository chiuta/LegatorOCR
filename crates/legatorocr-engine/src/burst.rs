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
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || y < 1990
        || hh > 23
        || mi > 59
        || ss > 60
    {
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
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
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
    /// Frames that cannot be pages (hand over the lens, dark, flat), with the reason. They are
    /// left out of `paths` only when asked (`drop_unusable`); always listed.
    pub unusable: Vec<(PathBuf, q::Unusable)>,
    /// Shooting rhythm of the session, when every file has a capture time.
    pub pace: Option<Pace>,
}

/// How fast the pages were photographed, from the EXIF times of consecutive frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Pace {
    /// Frames in the session (unusable ones included).
    pub shots: usize,
    /// Unusable frames followed by a good frame: the "hand over the lens, then retake" pattern.
    pub retakes: usize,
    /// Frames that are missing from a regular timer rhythm: with a 10 s timer, a 20 s gap between
    /// two kept frames means one shot was taken and deleted in between (usually a retake).
    pub implied_retakes: usize,
    /// The timer interval when the session shows a regular rhythm (most gaps equal, in seconds).
    pub interval_s: Option<f64>,
    /// Median seconds between a good frame and the next good one (pauses over 120 s ignored).
    pub median_s: f64,
    /// 90th percentile of the same gaps.
    pub p90_s: f64,
    /// Interval to set on the camera timer: just above what most pages needed, a few seconds
    /// more when many frames had to be retaken.
    pub suggested_s: u32,
}

/// Rhythm of a session. `times` are in shooting order, `bad[i]` marks unusable frames. `None`
/// with fewer than 4 usable gaps.
pub fn pace(times: &[i64], bad: &[bool]) -> Option<Pace> {
    // (gap, no frame at all between the two kept frames)
    let mut gaps: Vec<(f64, bool)> = Vec::new();
    let mut last_good: Option<usize> = None;
    let mut retakes = 0;
    for i in 0..times.len() {
        if bad.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some(j) = last_good {
            if i > j + 1 {
                retakes += 1;
            }
            let g = (times[i] - times[j]) as f64;
            if (1.0..=120.0).contains(&g) {
                gaps.push((g, i == j + 1));
            }
        } else if i > 0 {
            retakes += 1;
        }
        last_good = Some(i);
    }
    if gaps.len() < 4 {
        return None;
    }
    let mut sorted: Vec<f64> = gaps.iter().map(|g| g.0).collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |f: f64| sorted[((sorted.len() - 1) as f64 * f).round() as usize];
    let (median, p90, p75) = (at(0.5), at(0.9), at(0.75));
    // a timer rhythm: most gaps sit on one value (the lower quartile), a few on its multiples
    let base = at(0.25);
    let tol = (0.15 * base).max(1.0);
    let regular =
        sorted.iter().filter(|g| (**g - base).abs() <= tol).count() * 10 >= sorted.len() * 6;
    let mut implied = 0;
    if regular {
        for (g, consecutive) in &gaps {
            let m = (g / base).round();
            if *consecutive && m >= 2.0 && (g - m * base).abs() <= tol * 1.5 {
                implied += m as usize - 1;
            }
        }
    }
    let wasted = retakes + implied;
    let extra = if wasted * 10 > times.len() + implied {
        3.0
    } else {
        0.0
    };
    Some(Pace {
        shots: times.len(),
        retakes,
        implied_retakes: implied,
        interval_s: regular.then_some(base),
        median_s: median,
        p90_s: p90,
        suggested_s: ((p75 + extra).ceil() as u32).clamp(3, 60),
    })
}

/// Order `paths` by capture time (only when every file has one) and keep the sharpest frame of
/// each burst: consecutive frames within `max_gap_s` seconds, or, without times, whose
/// perceptual hashes differ in at most 6 bits.
pub fn ingest(paths: &[PathBuf], max_gap_s: i64) -> Ingest {
    ingest_opts(paths, max_gap_s, false)
}

/// [`ingest`] that can also leave out the frames that cannot be pages (`drop_unusable`). Such
/// frames are removed before bursts are grouped, so one never stands in for a real page.
pub fn ingest_opts(paths: &[PathBuf], max_gap_s: i64, drop_unusable: bool) -> Ingest {
    let mut items: Vec<(PathBuf, Option<i64>)> =
        paths.iter().map(|p| (p.clone(), exif_time(p))).collect();
    let all_timed = !items.is_empty() && items.iter().all(|i| i.1.is_some());
    let mut reordered = false;
    if all_timed {
        let before: Vec<PathBuf> = items.iter().map(|i| i.0.clone()).collect();
        items.sort_by_key(|i| i.1);
        reordered = before != items.iter().map(|i| i.0.clone()).collect::<Vec<_>>();
    }
    let mut sharp = Vec::new();
    let mut hashes = Vec::new();
    let mut stats = Vec::new();
    for (p, _) in &items {
        match crate::load_image(p) {
            Ok(img) => {
                let small = img.thumbnail(1000, 1000).to_luma8();
                stats.push(q::frame_stats(&small));
                sharp.push(q::sharpness(&small));
                hashes.push(q::perceptual_hash(&small));
            }
            Err(_) => {
                stats.push(q::FrameStats {
                    mean: f32::NAN,
                    std: f32::NAN,
                    sharp: f32::NAN,
                });
                sharp.push(f32::NAN);
                hashes.push(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(sharp.len() as u64 + 1));
                // unlikely to group
            }
        }
    }
    let bad_list = q::unusable_frames(&stats);
    let mut bad = vec![false; items.len()];
    for (i, _) in &bad_list {
        bad[*i] = true;
    }
    let pace = if all_timed {
        let t: Vec<i64> = items.iter().filter_map(|i| i.1).collect();
        pace(&t, &bad)
    } else {
        None
    };
    let unusable: Vec<(PathBuf, q::Unusable)> = bad_list
        .iter()
        .map(|(i, why)| (items[*i].0.clone(), *why))
        .collect();
    if drop_unusable && bad.iter().any(|b| *b) && bad.iter().any(|b| !*b) {
        let mut i = 0;
        items.retain(|_| {
            i += 1;
            !bad[i - 1]
        });
        let mut i = 0;
        sharp.retain(|_| {
            i += 1;
            !bad[i - 1]
        });
        let mut i = 0;
        hashes.retain(|_| {
            i += 1;
            !bad[i - 1]
        });
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
    Ingest {
        paths: out,
        reordered,
        dropped,
        unusable,
        pace,
    }
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

    #[test]
    fn pace_counts_retakes_and_suggests_above_the_typical_gap() {
        // 12 s between pages; frame 3 is a hand over the lens, retaken 6 s later
        let times = [0, 12, 24, 36, 42, 54, 66, 78, 90];
        let mut bad = [false; 9];
        bad[3] = true;
        let p = pace(&times, &bad).unwrap();
        assert_eq!(p.shots, 9);
        assert_eq!(p.retakes, 1);
        assert_eq!(p.median_s, 12.0);
        assert!(p.suggested_s >= 12 && p.suggested_s <= 18, "{p:?}");
        assert!(pace(&[0, 10, 20], &[false; 3]).is_none());
    }
    #[test]
    fn many_retakes_raise_the_suggestion() {
        let times: Vec<i64> = (0..10).map(|i| i * 10).collect();
        let calm = pace(&times, &[false; 10]).unwrap();
        let mut bad = [false; 10];
        bad[2] = true;
        bad[5] = true;
        let rushed = pace(&times, &bad).unwrap();
        assert!(rushed.suggested_s > calm.suggested_s);
    }

    #[test]
    fn deleted_retakes_show_up_as_gaps_that_are_multiples_of_the_timer() {
        // 10 s timer; two shots were deleted before the photos were collected
        let mut t = vec![0i64];
        for g in [10, 10, 20, 10, 10, 10, 20, 10, 10, 10, 10, 10] {
            t.push(t.last().unwrap() + g);
        }
        let p = pace(&t, &vec![false; t.len()]).unwrap();
        assert_eq!(p.interval_s, Some(10.0));
        assert_eq!(p.implied_retakes, 2);
        assert_eq!(p.retakes, 0);
        // 2 of 15 slots wasted: more than 10 %, so a few seconds are added to the 10 s
        assert_eq!(p.suggested_s, 13);
    }
    #[test]
    fn irregular_hand_shooting_has_no_implied_retakes() {
        let t = [0i64, 7, 19, 24, 41, 52, 58, 79, 90];
        let p = pace(&t, &[false; 9]).unwrap();
        assert_eq!(p.interval_s, None);
        assert_eq!(p.implied_retakes, 0);
    }
}
