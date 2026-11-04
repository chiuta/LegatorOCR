//! `--integrity-hints FILE.json` (roadmap 7.5c): a few cheap, explainable indicators, per page,
//! that a person reviewing a document picture may want to look at again. **Neutral by design**:
//! each entry says what was measured, where, and the common innocent causes. It is never a
//! verdict, and the false-alarm risk is high (see the README and ROADMAP 7.5c for measured rates
//! on synthetic edits, which do not carry over to real documents). It must never be the sole
//! basis of a decision about a person or a person's diploma.
//!
//! Signals (names as used in the output and in `--integrity-signals`):
//! - `jpeg_grid`: for JPEG inputs, a square whose 8x8 block-edge pattern is offset from the rest of
//!   the page (a part that was cut from another JPEG and placed at a different offset).
//! - `jpeg_double`: for JPEG inputs, the whole page shows the comb pattern of a second save.
//! - `noise`: a region whose fine-grain noise level differs from the page's by a large factor.
//! - `line_size`: a word whose height or vertical position is out of line with its text line
//!   (needs OCR word boxes).
//! - `copy_move`: image patches that appear again, identical, at another place.
use crate::verifier;
use image::GrayImage;
use legatorocr_core::Word;
use std::collections::HashMap;

pub const ALL_SIGNALS: [&str; 5] = [
    "jpeg_grid",
    "jpeg_double",
    "noise",
    "line_size",
    "copy_move",
];

/// Signals that run when `--integrity-signals` is not given. A signal is left out here when its
/// false-alarm rate on unedited synthetic pages exceeded 10 % (`jpeg_grid` 4 of 12 pages, `noise`
/// 10 of 12, `line_size` 12 of 12) or when it found none of the synthetic edits (`jpeg_double`:
/// no false alarm, but it saw none of 60 edited pages). Numbers in ROADMAP 7.5c.
pub const DEFAULT_SIGNALS: [&str; 1] = ["copy_move"];

/// Notice written at the top of every output file. Tests check that no output text contains a
/// verdict word.
pub const NOTICE: &str = "These are indicators for a human reviewer to check. They are not a verdict. Most come from innocent causes (re-saved scans, stamps, repeated logos, letterheads, scanner noise). Never use them as the only basis for a decision about a person or a person's diploma; ask the issuing institution.";

#[derive(Debug, Clone)]
pub struct Hint {
    pub signal: &'static str,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// 0..1, relative within the signal; not a probability.
    pub strength: f32,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct PageReport {
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub hints: Vec<Hint>,
    /// Signals that were not run on this page, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// `list` is a comma-separated list of signal names, `all` or `default`.
pub fn parse_signals(list: &str) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    for t in list.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match t {
            "all" => out.extend(ALL_SIGNALS),
            "default" => out.extend(DEFAULT_SIGNALS),
            _ => match ALL_SIGNALS.iter().find(|s| **s == t) {
                Some(s) => out.push(*s),
                None => {
                    return Err(format!(
                        "unknown signal '{t}' (known: {}, all, default)",
                        ALL_SIGNALS.join(", ")
                    ))
                }
            },
        }
    }
    out.dedup();
    Ok(out)
}

/// (what it means, common innocent causes) for a signal.
pub fn explain(signal: &str) -> (&'static str, &'static str) {
    match signal {
        "jpeg_grid" => (
            "The 8x8 block pattern of JPEG compression in this square is shifted compared with the rest of the page, as if this part came from another picture or was moved by a number of pixels that is not a multiple of 8.",
            "Cropped or resized pages, a logo or photo placed on the page by the issuer's software, pages assembled from several scans, stitching.",
        ),
        "jpeg_double" => (
            "The picture shows the periodic pattern in its JPEG coefficients that is left when a JPEG is saved a second time at another quality.",
            "Very common: any re-save, e-mail or messaging apps, scanner software that re-compresses, a viewer's 'save as'. A page that was only saved once shows nothing, but this says nothing about what was in the page.",
        ),
        "noise" => (
            "The fine-grain noise in this region differs from the rest of the page by a large factor (a different noise level can mean a different source for that part).",
            "Photos or stamps on the page, a coloured or shaded background, a laminated or glossy part, a fold or shadow, scanner noise that varies across the glass, smoothed areas from the scanner's own clean-up.",
        ),
        "line_size" => (
            "A word is taller, shorter or higher/lower than the other words of its text line.",
            "Ascenders and descenders, capital letters and digits, superscripts, different fonts in one line (names, headings, stamps), curved or skewed lines, OCR boxes that are simply wrong.",
        ),
        "copy_move" => (
            "A patch of the picture appears again, identical within compression noise, at another place on the same page.",
            "Repeated logos, letterheads, stamps, rules, table grids, form boxes, the same letter or word in the same font, QR code finder squares.",
        ),
        _ => ("", ""),
    }
}

fn clampf(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Run the chosen signals on one page picture. `jpeg` is the file's bytes when the input is a
/// JPEG (the two JPEG signals are skipped otherwise), `words` are OCR boxes in the same pixel
/// grid (the `line_size` signal is skipped when empty).
pub fn analyse(
    g: &GrayImage,
    jpeg: Option<&[u8]>,
    words: &[Word],
    signals: &[&str],
    source: &str,
) -> PageReport {
    let mut rep = PageReport {
        source: source.to_string(),
        width: g.width(),
        height: g.height(),
        ..Default::default()
    };
    for s in signals {
        match *s {
            "jpeg_grid" | "jpeg_double" => {
                let Some(b) = jpeg.filter(|b| b.starts_with(&[0xFF, 0xD8])) else {
                    rep.skipped
                        .push((s.to_string(), "input is not a JPEG file".into()));
                    continue;
                };
                if *s == "jpeg_grid" {
                    rep.hints.extend(jpeg_grid(g));
                } else if let Some(d) = verifier::jpeg_double_quant(b) {
                    if d.likely_double {
                        rep.hints.push(Hint {
                            signal: "jpeg_double",
                            x: 0,
                            y: 0,
                            w: g.width(),
                            h: g.height(),
                            strength: clampf((d.score as f32 - 5.0) / 20.0 + 0.2),
                            detail: format!("coefficient comb score {:.1}", d.score),
                        });
                    }
                } else {
                    rep.skipped.push((
                        s.to_string(),
                        "picture too small or table unreadable".into(),
                    ));
                }
            }
            "noise" => rep.hints.extend(noise_regions(g)),
            "line_size" => {
                if words.is_empty() {
                    rep.skipped
                        .push((s.to_string(), "needs OCR word boxes (--ocr)".into()));
                } else {
                    rep.hints.extend(line_outliers(words));
                }
            }
            "copy_move" => rep.hints.extend(copy_move(g)),
            _ => {}
        }
    }
    rep
}

// ---------------------------------------------------------------- copy-move

const CB: u32 = 16; // block size
const CSTRIDE: u32 = 4; // blocks that are put in the index sit on this grid; queries use every pixel
const CMAX_BUCKET: usize = 6; // a block that occurs more often than this is a repeated glyph or rule: ignored

/// Summed-area tables of the grey values and their squares, `(w+1) x (h+1)`.
fn sat(g: &GrayImage) -> (Vec<u64>, Vec<u64>) {
    let (w, h) = (g.width() as usize, g.height() as usize);
    let raw = g.as_raw();
    let mut s = vec![0u64; (w + 1) * (h + 1)];
    let mut q = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h {
        let (mut rs, mut rq) = (0u64, 0u64);
        for x in 0..w {
            let v = raw[y * w + x] as u64;
            rs += v;
            rq += v * v;
            s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + rs;
            q[(y + 1) * (w + 1) + x + 1] = q[y * (w + 1) + x + 1] + rq;
        }
    }
    (s, q)
}

/// Two hash keys of the 16x16 block at (x, y): its 4x4 cell means in steps of 32 grey levels, once
/// as they are and once shifted by half a step, so a cell near a step edge still agrees in one of
/// the keys after re-compression. Blocks with little contrast (blank paper) give `None`.
fn cm_keys(s: &[u64], q: &[u64], w: usize, x: usize, y: usize) -> Option<[u64; 2]> {
    let st = w + 1;
    let rect = |t: &[u64], x0: usize, y0: usize, n: usize| {
        t[(y0 + n) * st + x0 + n] + t[y0 * st + x0] - t[y0 * st + x0 + n] - t[(y0 + n) * st + x0]
    };
    let n = (CB * CB) as f64;
    let (bs, bq) = (rect(s, x, y, 16) as f64, rect(q, x, y, 16) as f64);
    if bq / n - (bs / n).powi(2) < 100.0 {
        return None;
    }
    let mut k = [0u64; 2];
    for j in 0..4 {
        for i in 0..4 {
            let m = (rect(s, x + i * 4, y + j * 4, 4) / 16) as u32;
            k[0] = k[0].wrapping_mul(31).wrapping_add((m / 32) as u64);
            k[1] = k[1].wrapping_mul(31).wrapping_add(((m + 16) / 32) as u64);
        }
    }
    Some(k)
}

/// Patches that occur twice. The index holds the blocks on a 4 px grid; every pixel position is
/// looked up, so a patch pasted at any offset is found (the earlier `--verify-document` search
/// compares grid with grid and only finds offsets that are multiples of 4). Blank areas and
/// blocks that occur more than a handful of times (letters, rules) are not used. Agreeing blocks
/// vote for one offset; a pasted patch fills its box, scattered coincidences do not.
pub const CM_MIN_BLOCKS: usize = 40;
pub const CM_MIN_DIST: i32 = 48;
pub const CM_MIN_FILL: f64 = 0.3;
/// Both sides of the matching area must be at least this long (px at any dpi): in the first test
/// run every false alarm on re-saved pages was one repeated letter (26-52 px). Chosen after seeing
/// those, on the same pages: see ROADMAP 7.5c.
pub const CM_MIN_SIDE: u32 = 64;

fn copy_move(g: &GrayImage) -> Vec<Hint> {
    let (w, h) = (g.width(), g.height());
    if w < CB * 3 || h < CB * 3 {
        return Vec::new();
    }
    let (s, q) = sat(g);
    let mut index: HashMap<u64, Vec<(u32, u32)>> = HashMap::new();
    let mut y = 0;
    while y + CB <= h {
        let mut x = 0;
        while x + CB <= w {
            if let Some(k) = cm_keys(&s, &q, w as usize, x as usize, y as usize) {
                for (v, key) in k.into_iter().enumerate() {
                    let e = index.entry(key ^ ((v as u64) << 63)).or_default();
                    if e.len() <= CMAX_BUCKET {
                        e.push((x, y));
                    }
                }
            }
            x += CSTRIDE;
        }
        y += CSTRIDE;
    }
    index.retain(|_, v| v.len() <= CMAX_BUCKET);
    // (offset, position of the later copy)
    let mut votes: Vec<((i32, i32), (u32, u32))> = Vec::new();
    for y in 0..=(h - CB) {
        for x in 0..=(w - CB) {
            let Some(k) = cm_keys(&s, &q, w as usize, x as usize, y as usize) else {
                continue;
            };
            for (v, key) in k.into_iter().enumerate() {
                if let Some(e) = index.get(&(key ^ ((v as u64) << 63))) {
                    for &(sx, sy) in e {
                        let (dx, dy) = (x as i32 - sx as i32, y as i32 - sy as i32);
                        if dx * dx + dy * dy < CM_MIN_DIST * CM_MIN_DIST {
                            continue;
                        }
                        // canonical direction: the "second" copy lies below (or right of) the first
                        if dy > 0 || (dy == 0 && dx > 0) {
                            votes.push(((dx, dy), (x, y)));
                        } else {
                            votes.push(((-dx, -dy), (sx, sy)));
                        }
                    }
                }
            }
        }
    }
    votes.sort_unstable();
    votes.dedup();
    type Cand = ((i32, i32), usize, u32, u32, u32, u32); // offset, votes, box
    let mut cands: Vec<Cand> = Vec::new();
    let mut i = 0;
    while i < votes.len() {
        let mut j = i;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        while j < votes.len() && votes[j].0 == votes[i].0 {
            let (px, py) = votes[j].1;
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px + CB);
            y1 = y1.max(py + CB);
            j += 1;
        }
        let n = j - i;
        if n >= CM_MIN_BLOCKS && x1 - x0 >= CM_MIN_SIDE && y1 - y0 >= CM_MIN_SIDE {
            let covered = n as f64 * (CSTRIDE * CSTRIDE) as f64;
            if covered / (((x1 - x0) * (y1 - y0)) as f64) > CM_MIN_FILL {
                cands.push((votes[i].0, n, x0, y0, x1, y1));
            }
        }
        i = j;
    }
    // a periodic pattern repeats at multiples of its period: drop offsets whose double is as strong
    let counts: HashMap<(i32, i32), usize> = cands.iter().map(|c| (c.0, c.1)).collect();
    cands.retain(|c| counts.get(&(2 * c.0 .0, 2 * c.0 .1)).copied().unwrap_or(0) * 3 < c.1);
    cands.sort_by_key(|c| std::cmp::Reverse(c.1));
    cands.truncate(5);
    let qr = verifier::qr_read(g);
    let mut out = Vec::new();
    for ((dx, dy), n, x0, y0, x1, y1) in cands {
        let (cx, cy) = ((x0 + x1) as i32 / 2, (y0 + y1) as i32 / 2);
        if qr
            .iter()
            .any(|(_, b)| cx >= b.0 - 8 && cx <= b.2 + 8 && cy >= b.1 - 8 && cy <= b.3 + 8)
        {
            continue; // a QR code repeats its finder squares by design
        }
        let strength = clampf(n as f32 / 600.0);
        let (bw, bh) = (x1 - x0, y1 - y0);
        out.push(Hint {
            signal: "copy_move",
            x: x0,
            y: y0,
            w: bw,
            h: bh,
            strength,
            detail: format!(
                "{n} matching blocks; the same patch is also at offset ({}, {})",
                -dx, -dy
            ),
        });
        let (bx, by) = (x0 as i32 - dx, y0 as i32 - dy);
        if bx >= 0 && by >= 0 {
            out.push(Hint {
                signal: "copy_move",
                x: bx as u32,
                y: by as u32,
                w: bw,
                h: bh,
                strength,
                detail: format!("the same patch as the one at ({x0}, {y0})"),
            });
        }
    }
    out
}

// ---------------------------------------------------------------- noise

const NT: u32 = 32;

/// Connected groups (4-neighbour) of flagged tiles with at least `min_tiles` members, as
/// (x0, y0, x1, y1 in tiles, strongest flag).
fn groups(flag: &[f32], tw: u32, th: u32, min_tiles: usize) -> Vec<(u32, u32, u32, u32, f32)> {
    let mut seen = vec![false; flag.len()];
    let mut out = Vec::new();
    for s in 0..flag.len() {
        if seen[s] || flag[s] <= 0.0 {
            continue;
        }
        let mut stack = vec![s];
        seen[s] = true;
        let (mut x0, mut y0, mut x1, mut y1, mut n, mut best) =
            (u32::MAX, u32::MAX, 0, 0, 0usize, 0f32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i as u32 % tw, i as u32 / tw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            n += 1;
            best = best.max(flag[i]);
            let mut nb = Vec::new();
            if x > 0 {
                nb.push(i - 1);
            }
            if x + 1 < tw {
                nb.push(i + 1);
            }
            if y > 0 {
                nb.push(i - tw as usize);
            }
            if y + 1 < th {
                nb.push(i + tw as usize);
            }
            for j in nb {
                if !seen[j] && flag[j] > 0.0 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if n >= min_tiles {
            out.push((x0, y0, x1, y1, best));
        }
    }
    out
}

/// Typical fine-grain level of each 32 px tile: mean absolute high-pass residual (pixel minus the
/// mean of its four neighbours), trimmed to the lowest 80 % so edges do not count. Tiles with many
/// strong edges (text) are left out (`None`).
fn tile_noise(g: &GrayImage) -> (u32, u32, Vec<Option<f32>>) {
    let (w, h) = g.dimensions();
    let (tw, th) = (w / NT, h / NT);
    let raw = g.as_raw();
    let ws = w as usize;
    let mut out = vec![None; (tw * th) as usize];
    for ty in 0..th {
        for tx in 0..tw {
            let mut r: Vec<f32> = Vec::with_capacity(900);
            let mut strong = 0;
            for y in (ty * NT + 1)..(ty * NT + NT - 1) {
                for x in (tx * NT + 1)..(tx * NT + NT - 1) {
                    let i = y as usize * ws + x as usize;
                    let m = (raw[i - 1] as f32
                        + raw[i + 1] as f32
                        + raw[i - ws] as f32
                        + raw[i + ws] as f32)
                        / 4.0;
                    let d = (raw[i] as f32 - m).abs();
                    if d > 30.0 {
                        strong += 1;
                    }
                    r.push(d);
                }
            }
            if strong * 25 > r.len() {
                continue; // more than 4 % strong edges: text or a drawing, not paper
            }
            r.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let keep = r.len() * 4 / 5;
            out[(ty * tw + tx) as usize] = Some(r[..keep].iter().sum::<f32>() / keep as f32);
        }
    }
    (tw, th, out)
}

/// A tile is reported when its noise level is this many times the page median.
pub const NOISE_RATIO: f32 = 3.0;

fn noise_regions(g: &GrayImage) -> Vec<Hint> {
    let (tw, th, t) = tile_noise(g);
    let mut vals: Vec<f32> = t.iter().flatten().copied().collect();
    if vals.len() < 20 {
        return Vec::new();
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = vals[vals.len() / 2].max(0.3);
    let flag: Vec<f32> = t
        .iter()
        .map(|v| match v {
            Some(v) if *v > med * NOISE_RATIO && *v > 1.5 => {
                clampf((v / med).ln() / 8f32.ln()).max(0.05)
            }
            _ => 0.0,
        })
        .collect();
    groups(&flag, tw, th, 2)
        .into_iter()
        .map(|(x0, y0, x1, y1, s)| Hint {
            signal: "noise",
            x: x0 * NT,
            y: y0 * NT,
            w: (x1 - x0) * NT,
            h: (y1 - y0) * NT,
            strength: s,
            detail: format!(
                "page median noise level {med:.2}, this region up to {:.1} times higher",
                (s * 8f32.ln()).exp() * 1.0
            ),
        })
        .collect()
}

// ---------------------------------------------------------------- jpeg grid

const GW: u32 = 128;
const GS: u32 = 64;

/// Block-edge excess by position modulo 8 along x (`vertical == false`) or y, over a rectangle:
/// at each step between neighbouring pixels in a flat neighbourhood (the steps on both sides are
/// at most 4 and this one at most 6 grey levels), this step minus the mean of its two neighbours.
/// JPEG blocking leaves a small positive excess at the block edges; text edges are left out by the
/// flatness rule.
fn grid_profile(g: &GrayImage, x0: u32, y0: u32, w: u32, h: u32, vertical: bool) -> [f64; 8] {
    let raw = g.as_raw();
    let ws = g.width() as usize;
    let mut acc = [0f64; 8];
    let (len, lines) = if vertical { (h, w) } else { (w, h) };
    let at = |l: u32, i: u32| -> i32 {
        let (x, y) = if vertical {
            (x0 + l, y0 + i)
        } else {
            (x0 + i, y0 + l)
        };
        raw[y as usize * ws + x as usize] as i32
    };
    for l in 0..lines {
        for i in 3..len {
            let d0 = (at(l, i - 2) - at(l, i - 3)).abs();
            let d1 = (at(l, i - 1) - at(l, i - 2)).abs();
            let d2 = (at(l, i) - at(l, i - 1)).abs();
            if d0 <= 4 && d2 <= 4 && d1 <= 6 {
                // the step d1 lies between positions i-2 and i-1
                let (px, py) = if vertical {
                    (0, y0 + i - 1)
                } else {
                    (x0 + i - 1, 0)
                };
                let phase = (if vertical { py } else { px } % 8) as usize;
                acc[phase] += d1 as f64 - (d0 + d2) as f64 / 2.0;
            }
        }
    }
    acc
}

/// (phase, z): the strongest of 8 positions and its distance from the other seven in their spread.
fn phase(p: &[f64; 8]) -> (usize, f64) {
    let (mut bi, mut bv) = (0, f64::MIN);
    for (i, v) in p.iter().enumerate() {
        if *v > bv {
            bv = *v;
            bi = i;
        }
    }
    let rest: Vec<f64> = p
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != bi)
        .map(|(_, v)| *v)
        .collect();
    let m = rest.iter().sum::<f64>() / 7.0;
    let sd = (rest.iter().map(|v| (v - m).powi(2)).sum::<f64>() / 7.0).sqrt();
    (bi, (bv - m) / (sd + 1e-6))
}

/// A window needs this z-score for its own 8x8 phase before it can count as different.
pub const GRID_Z: f64 = 4.0;

fn jpeg_grid(g: &GrayImage) -> Vec<Hint> {
    let (w, h) = g.dimensions();
    if w < GW * 2 || h < GW * 2 {
        return Vec::new();
    }
    let (phx, zx) = phase(&grid_profile(g, 0, 0, w, h, false));
    let (phy, zy) = phase(&grid_profile(g, 0, 0, w, h, true));
    if zx < GRID_Z && zy < GRID_Z {
        return Vec::new(); // the page itself shows no clear 8x8 pattern (high quality)
    }
    let (tw, th) = ((w - GW) / GS + 1, (h - GW) / GS + 1);
    let mut flag = vec![0f32; (tw * th) as usize];
    for ty in 0..th {
        for tx in 0..tw {
            let (ax, az) = phase(&grid_profile(g, tx * GS, ty * GS, GW, GW, false));
            let (by, bz) = phase(&grid_profile(g, tx * GS, ty * GS, GW, GW, true));
            let off_x = zx >= GRID_Z && az >= GRID_Z && ax != phx;
            let off_y = zy >= GRID_Z && bz >= GRID_Z && by != phy;
            if off_x || off_y {
                let z = if off_x && off_y {
                    az.min(bz)
                } else if off_x {
                    az
                } else {
                    bz
                };
                flag[(ty * tw + tx) as usize] = clampf(((z - GRID_Z) / 6.0) as f32).max(0.05);
            }
        }
    }
    groups(&flag, tw, th, 2)
        .into_iter()
        .map(|(x0, y0, x1, y1, s)| Hint {
            signal: "jpeg_grid",
            x: x0 * GS,
            y: y0 * GS,
            w: (x1 - x0 - 1) * GS + GW,
            h: (y1 - y0 - 1) * GS + GW,
            strength: s,
            detail: format!("block-edge pattern differs from the page's (page offset {phx},{phy})"),
        })
        .collect()
}

// ---------------------------------------------------------------- text line outliers

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Height ratios to the line's median that count as out of line, and the vertical shift (in line
/// heights) of both top and bottom edge in the same direction.
pub const LINE_HEIGHT_HI: f32 = 1.5;
pub const LINE_HEIGHT_LO: f32 = 0.55;
pub const LINE_SHIFT: f32 = 0.3;

fn line_outliers(words: &[Word]) -> Vec<Hint> {
    let mut lines: HashMap<u32, Vec<&Word>> = HashMap::new();
    for w in words {
        if w.h >= 8 && w.w >= 3 && !w.text.trim().is_empty() {
            lines.entry(w.line).or_default().push(w);
        }
    }
    let mut out = Vec::new();
    for ws in lines.values() {
        if ws.len() < 5 {
            continue;
        }
        let mh = median(&mut ws.iter().map(|w| w.h as f32).collect::<Vec<_>>());
        let mt = median(&mut ws.iter().map(|w| w.y as f32).collect::<Vec<_>>());
        let mb = median(&mut ws.iter().map(|w| (w.y + w.h) as f32).collect::<Vec<_>>());
        for w in ws {
            let r = w.h as f32 / mh;
            let st = (w.y as f32 - mt) / mh;
            let sb = ((w.y + w.h) as f32 - mb) / mh;
            let tall = r > LINE_HEIGHT_HI;
            let short = r < LINE_HEIGHT_LO;
            let moved =
                st.signum() == sb.signum() && st.abs() > LINE_SHIFT && sb.abs() > LINE_SHIFT;
            if tall || short || moved {
                let (dev, what) = if moved {
                    let s = st.abs().min(sb.abs());
                    (
                        s / 0.6,
                        format!(
                            "sits {:.0} % of a line height {} its neighbours",
                            s * 100.0,
                            if st > 0.0 {
                                "lower than"
                            } else {
                                "higher than"
                            }
                        ),
                    )
                } else {
                    (
                        r.ln().abs(),
                        format!("height is {r:.2} times the line's median"),
                    )
                };
                out.push(Hint {
                    signal: "line_size",
                    x: w.x,
                    y: w.y,
                    w: w.w,
                    h: w.h,
                    strength: clampf(dev),
                    detail: format!("word '{}' {what}", w.text.replace('"', "'")),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- output

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// JSON document: a notice, the signals that ran, and per page the regions to check.
pub fn to_json(pages: &[PageReport], signals: &[&str]) -> String {
    let mut s = String::from("{\n");
    s.push_str(&format!("  \"notice\": \"{}\",\n", esc(NOTICE)));
    s.push_str(&format!(
        "  \"signals_run\": [{}],\n",
        signals
            .iter()
            .map(|x| format!("\"{x}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str("  \"pages\": [\n");
    for (i, p) in pages.iter().enumerate() {
        s.push_str(&format!(
            "    {{\"source\": \"{}\", \"width\": {}, \"height\": {}, \"to_check\": [",
            esc(&p.source),
            p.width,
            p.height
        ));
        for (j, h) in p.hints.iter().enumerate() {
            let (what, innocent) = explain(h.signal);
            s.push_str(&format!(
                "{}\n      {{\"signal\": \"{}\", \"box\": [{}, {}, {}, {}], \"strength\": {:.2}, \"measured\": \"{}\", \"what_it_means\": \"{}\", \"common_innocent_causes\": \"{}\"}}",
                if j > 0 { "," } else { "" },
                h.signal, h.x, h.y, h.w, h.h, h.strength, esc(&h.detail), esc(what), esc(innocent)
            ));
        }
        s.push_str(&format!(
            "{}], \"skipped\": [",
            if p.hints.is_empty() { "" } else { "\n    " }
        ));
        s.push_str(
            &p.skipped
                .iter()
                .map(|(a, b)| format!("{{\"signal\": \"{}\", \"reason\": \"{}\"}}", a, esc(b)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str(&format!(
            "]}}{}\n",
            if i + 1 < pages.len() { "," } else { "" }
        ));
    }
    s.push_str("  ]\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn rng(seed: &mut u64) -> u32 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 33) as u32
    }

    /// Short dark strokes on light paper with a little noise: enough structure for blocks.
    fn page(w: u32, h: u32, seed: u64) -> GrayImage {
        let mut s = seed;
        let mut g = GrayImage::from_pixel(w, h, Luma([235]));
        for _ in 0..(w * h / 220) {
            let (x, y) = (rng(&mut s) % (w - 8), rng(&mut s) % (h - 3));
            let l = 2 + rng(&mut s) % 6;
            for i in 0..l {
                g.put_pixel(x + i, y, Luma([40]));
                g.put_pixel(x + i, y + 1, Luma([60]));
            }
        }
        for p in g.pixels_mut() {
            p[0] = (p[0] as i32 + (rng(&mut s) % 5) as i32 - 2).clamp(0, 255) as u8;
        }
        g
    }

    /// Busy texture (every 16 px block has contrast), for the block-matching signal.
    fn dense(w: u32, h: u32, seed: u64) -> GrayImage {
        let mut s = seed;
        let mut g = GrayImage::new(w, h);
        for p in g.pixels_mut() {
            p[0] = (rng(&mut s) % 256) as u8;
        }
        image::imageops::blur(&g, 1.2)
    }

    fn jpeg(g: &GrayImage, q: u8) -> Vec<u8> {
        let mut b = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut b, q)
            .encode_image(g)
            .unwrap();
        b
    }
    fn dec(b: &[u8]) -> GrayImage {
        image::load_from_memory(b).unwrap().to_luma8()
    }

    #[test]
    fn signals_parse_and_reject_unknown() {
        assert_eq!(
            parse_signals("noise,copy_move").unwrap(),
            vec!["noise", "copy_move"]
        );
        assert_eq!(parse_signals("all").unwrap().len(), 5);
        assert_eq!(parse_signals("default").unwrap(), vec!["copy_move"]);
        assert!(parse_signals("verdict").is_err());
    }

    #[test]
    fn copied_patch_is_reported_at_both_places_and_a_plain_page_is_not() {
        let mut g = dense(640, 480, 5);
        assert!(copy_move(&g).is_empty());
        let patch = image::imageops::crop_imm(&g, 40, 40, 160, 120).to_image();
        image::imageops::overlay(&mut g, &patch, 383, 301); // offset (343, 261): not a multiple of 4
        let h = copy_move(&g);
        assert!(h.len() >= 2, "{h:?}");
        assert!(h
            .iter()
            .any(|x| (x.x as i32 - 383).abs() < 24 && (x.y as i32 - 301).abs() < 24));
    }

    #[test]
    fn a_region_with_added_noise_is_reported_and_a_plain_page_is_not() {
        let mut g = page(640, 480, 9);
        assert!(noise_regions(&g).is_empty());
        let mut s = 77u64;
        for y in 200..320 {
            for x in 200..360 {
                let n = (rng(&mut s) % 41) as i32 - 20;
                let p = g.get_pixel_mut(x, y);
                p[0] = (p[0] as i32 + n).clamp(0, 255) as u8;
            }
        }
        let h = noise_regions(&g);
        assert!(!h.is_empty());
        assert!(
            h.iter()
                .any(|x| x.x < 360 && x.x + x.w > 200 && x.y < 320 && x.y + x.h > 200),
            "{h:?}"
        );
    }

    #[test]
    fn a_part_pasted_at_another_block_offset_changes_the_jpeg_grid_signal() {
        let a = dec(&jpeg(&page(640, 512, 1), 30));
        let b = dec(&jpeg(&page(640, 512, 2), 30));
        let mut edited = a.clone();
        assert!(jpeg_grid(&edited).is_empty(), "plain page flagged");
        let patch = image::imageops::crop_imm(&b, 0, 0, 256, 192).to_image();
        image::imageops::overlay(&mut edited, &patch, 131, 99); // offsets 3 and 3 modulo 8
        let h = jpeg_grid(&edited);
        assert!(
            h.iter()
                .any(|x| x.x < 387 && x.x + x.w > 131 && x.y < 291 && x.y + x.h > 99),
            "{h:?}"
        );
    }

    fn word(t: &str, x: u32, y: u32, w: u32, h: u32) -> Word {
        Word {
            text: t.into(),
            x,
            y,
            w,
            h,
            conf: 90.0,
            line: 0,
        }
    }

    #[test]
    fn a_word_out_of_line_is_reported_and_an_even_line_is_not() {
        let mut ws: Vec<Word> = (0..8)
            .map(|i| word("abcd", 20 + i * 60, 100, 50, 30))
            .collect();
        assert!(line_outliers(&ws).is_empty());
        ws[4] = word("7", 260, 112, 18, 30); // same height, 12 px lower
        let h = line_outliers(&ws);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].x, h[0].y), (260, 112));
        ws[4] = word("7", 260, 100, 18, 52); // much taller
        assert_eq!(line_outliers(&ws).len(), 1);
    }

    #[test]
    fn json_is_neutral_and_carries_explanations() {
        let g = page(640, 480, 5);
        let rep = analyse(&g, None, &[], &ALL_SIGNALS, "a.png");
        assert_eq!(rep.skipped.len(), 3); // two JPEG signals and line_size
        let mut p = rep.clone();
        for s in ALL_SIGNALS {
            p.hints.push(Hint {
                signal: s,
                x: 1,
                y: 2,
                w: 3,
                h: 4,
                strength: 0.5,
                detail: "x".into(),
            });
        }
        let j = to_json(&[p], &ALL_SIGNALS).to_lowercase();
        for bad in ["forg", "fake", "tamper", "fraud", "counterfeit"] {
            assert!(!j.contains(bad), "{bad}");
        }
        assert!(
            j.contains("common_innocent_causes")
                && j.contains("not a verdict")
                && j.contains("diploma")
        );
    }
}
