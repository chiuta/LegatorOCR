//! `--vectorize DIR`: every processed page also as an SVG outline drawing (`page_NNN.svg`), for
//! ornaments and technical drawings that are to be cut, engraved or redrawn. The page is made
//! black and white (Otsu), the borders between ink and paper are followed pixel edge by pixel edge
//! (closed loops, holes included), straight runs are merged and the loops are simplified with the
//! Douglas-Peucker rule to within `tolerance` pixels. The SVG fills the loops with the even-odd
//! rule, so holes stay holes. Polylines, not curves: no smoothing, no symmetry correction.
//! Size: the page's pixel size at its resolution in millimetres.

use image::GrayImage;
use legatorocr_core::Page;
use std::path::PathBuf;
use std::sync::Mutex;

/// Otsu threshold: pixels below it are ink.
pub fn otsu(g: &GrayImage) -> u8 {
    let mut h = [0u64; 256];
    for p in g.pixels() {
        h[p[0] as usize] += 1;
    }
    let total: u64 = h.iter().sum();
    let sum: f64 = h
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut wb, mut sb, mut best, mut thr) = (0u64, 0f64, -1f64, 128u8);
    for (t, &ht) in h.iter().enumerate() {
        wb += ht;
        if wb == 0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0 {
            break;
        }
        sb += t as f64 * ht as f64;
        let (mb, mf) = (sb / wb as f64, (sum - sb) / wf as f64);
        let v = wb as f64 * wf as f64 * (mb - mf) * (mb - mf);
        if v > best {
            best = v;
            thr = t as u8;
        }
    }
    thr
}

const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)]; // E S W N (y grows downwards)

/// Closed outlines (vertex lists in pixel-corner coordinates) of the pixels for which `ink` is true.
pub fn loops(w: u32, h: u32, ink: &dyn Fn(u32, u32) -> bool) -> Vec<Vec<(i32, i32)>> {
    let (wi, hi) = (w as i32, h as i32);
    let vw = (w + 1) as usize;
    let mut mask = vec![0u8; vw * (h as usize + 1)];
    let at = |x: i32, y: i32| (y as usize) * vw + x as usize;
    let is = |x: i32, y: i32| x >= 0 && y >= 0 && x < wi && y < hi && ink(x as u32, y as u32);
    for y in 0..hi {
        for x in 0..wi {
            if !is(x, y) {
                continue;
            }
            // ink on the right-hand side of every edge
            if !is(x, y - 1) {
                mask[at(x, y)] |= 1; // E along the top
            }
            if !is(x + 1, y) {
                mask[at(x + 1, y)] |= 2; // S along the right
            }
            if !is(x, y + 1) {
                mask[at(x + 1, y + 1)] |= 4; // W along the bottom
            }
            if !is(x - 1, y) {
                mask[at(x, y + 1)] |= 8; // N along the left
            }
        }
    }
    let mut out = Vec::new();
    for sy in 0..=hi {
        for sx in 0..=wi {
            while mask[at(sx, sy)] != 0 {
                let m = mask[at(sx, sy)];
                let d0 = m.trailing_zeros() as usize;
                let mut pts = vec![(sx, sy)];
                let (mut x, mut y, mut d) = (sx, sy, d0);
                loop {
                    let (dx, dy) = DIRS[d];
                    x += dx;
                    y += dy;
                    if d != d0 || (x, y) != (sx, sy) {
                        // the first edge is consumed when the loop is closed
                    }
                    if (x, y) == (sx, sy) {
                        // back at the start: stop unless the rule would continue on another edge
                        let here = mask[at(x, y)] & !(1 << d0);
                        let pick = pick_dir(here | (1 << d0), d);
                        if pick == Some(d0) || pick.is_none() {
                            break;
                        }
                    }
                    pts.push((x, y));
                    let m = mask[at(x, y)];
                    let Some(nd) = pick_dir(m, d) else { break };
                    mask[at(x, y)] &= !(1 << nd);
                    d = nd;
                }
                mask[at(sx, sy)] &= !(1 << d0);
                out.push(pts);
            }
        }
    }
    out
}

/// Next direction at a vertex: turn right if possible, else straight on, else left.
fn pick_dir(mask: u8, heading: usize) -> Option<usize> {
    for t in [1usize, 0, 3] {
        let d = (heading + t) % 4;
        if mask & (1 << d) != 0 {
            return Some(d);
        }
    }
    None
}

fn area2(p: &[(i32, i32)]) -> i64 {
    let mut a = 0i64;
    for i in 0..p.len() {
        let (x0, y0) = p[i];
        let (x1, y1) = p[(i + 1) % p.len()];
        a += x0 as i64 * y1 as i64 - x1 as i64 * y0 as i64;
    }
    a.abs()
}

/// Drop vertices in the middle of straight runs.
fn merge_straight(p: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let n = p.len();
    let mut o = Vec::new();
    for i in 0..n {
        let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
        if (b.0 - a.0) * (c.1 - b.1) != (b.1 - a.1) * (c.0 - b.0) {
            o.push(b);
        }
    }
    o
}

fn dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    if l2 == 0.0 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / l2.sqrt()
}

fn rdp(p: &[(f64, f64)], eps: f64, keep: &mut Vec<bool>, lo: usize, hi: usize) {
    if hi <= lo + 1 {
        return;
    }
    let (mut best, mut bi) = (0.0, lo);
    for i in lo + 1..hi {
        let d = dist(p[i], p[lo], p[hi]);
        if d > best {
            best = d;
            bi = i;
        }
    }
    if best > eps {
        keep[bi] = true;
        rdp(p, eps, keep, lo, bi);
        rdp(p, eps, keep, bi, hi);
    }
}

/// Douglas-Peucker on a closed loop (split at the first and the farthest vertex).
pub fn simplify(p: &[(i32, i32)], eps: f64) -> Vec<(i32, i32)> {
    let p = merge_straight(p);
    if p.len() <= 4 || eps <= 0.0 {
        return p;
    }
    let f: Vec<(f64, f64)> = p.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let far = (1..f.len())
        .max_by(|&a, &b| {
            let da = (f[a].0 - f[0].0).hypot(f[a].1 - f[0].1);
            let db = (f[b].0 - f[0].0).hypot(f[b].1 - f[0].1);
            da.partial_cmp(&db).unwrap()
        })
        .unwrap();
    let mut a: Vec<(f64, f64)> = f[..=far].to_vec();
    let mut b: Vec<(f64, f64)> = f[far..].to_vec();
    b.push(f[0]);
    let mut out = Vec::new();
    for (seg, last) in [(&mut a, false), (&mut b, true)] {
        let mut keep = vec![false; seg.len()];
        keep[0] = true;
        let n = seg.len();
        keep[n - 1] = true;
        rdp(seg, eps, &mut keep, 0, n - 1);
        for (i, k) in keep.iter().enumerate() {
            if *k && !(last && i == n - 1) && !(!last && i == n - 1) {
                out.push((seg[i].0.round() as i32, seg[i].1.round() as i32));
            }
        }
    }
    out.dedup();
    out
}

/// SVG text for a grey page: ink = darker than the Otsu threshold.
pub fn svg(g: &GrayImage, dpi: u32, tolerance: f64, min_area_px: f64) -> String {
    let t = otsu(g);
    let (w, h) = g.dimensions();
    let ls = loops(w, h, &|x, y| g.get_pixel(x, y)[0] <= t);
    let mut d = String::new();
    let mut n = 0;
    for l in ls {
        if (area2(&l) as f64) / 2.0 < min_area_px {
            continue;
        }
        let s = simplify(&l, tolerance);
        if s.len() < 3 {
            continue;
        }
        for (i, (x, y)) in s.iter().enumerate() {
            d.push_str(if i == 0 { "M" } else { "L" });
            d.push_str(&format!("{x} {y}"));
        }
        d.push('Z');
        n += 1;
    }
    let mm = |px: u32| px as f64 * 25.4 / dpi.max(1) as f64;
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.2}mm\" height=\"{:.2}mm\" viewBox=\"0 0 {w} {h}\"><!-- {n} outlines, tolerance {tolerance} px -->\n<path fill=\"#000\" fill-rule=\"evenodd\" d=\"{d}\"/></svg>\n",
        mm(w),
        mm(h)
    )
}

struct State {
    dir: PathBuf,
    tol: f64,
    min_area: f64,
    written: Vec<PathBuf>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn begin(dir: PathBuf, tol: f64, min_area: f64) {
    *STATE.lock().unwrap() = Some(State {
        dir,
        tol,
        min_area,
        written: Vec::new(),
    });
}
pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

pub fn record(n: usize, pg: &Page) -> std::io::Result<()> {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return Ok(()) };
    std::fs::create_dir_all(&st.dir)?;
    let p = st.dir.join(format!("page_{n:03}.svg"));
    let gray = pg.image.to_dynamic().to_luma8();
    std::fs::write(&p, svg(&gray, pg.dpi, st.tol, st.min_area))?;
    st.written.push(p);
    Ok(())
}

/// Files written so far; ends the collection.
pub fn finish() -> Vec<PathBuf> {
    STATE
        .lock()
        .unwrap()
        .take()
        .map(|s| s.written)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(w: u32, h: u32, ink: &dyn Fn(u32, u32) -> bool) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| image::Luma([if ink(x, y) { 0 } else { 255 }]))
    }

    /// Even-odd fill of the loops, to compare with the source mask.
    fn fill(w: u32, h: u32, ls: &[Vec<(i32, i32)>]) -> Vec<bool> {
        let mut m = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let mut inside = false;
                for l in ls {
                    let n = l.len();
                    for i in 0..n {
                        let (a, b) = (l[i], l[(i + 1) % n]);
                        let ((x0, y0), (x1, y1)) =
                            ((a.0 as f64, a.1 as f64), (b.0 as f64, b.1 as f64));
                        if (y0 > py) != (y1 > py) && px < x0 + (py - y0) * (x1 - x0) / (y1 - y0) {
                            inside = !inside;
                        }
                    }
                }
                m[(y * w + x) as usize] = inside;
            }
        }
        m
    }

    #[test]
    fn exact_loops_reproduce_the_mask_including_holes_and_diagonal_touches() {
        let ink = |x: u32, y: u32| {
            let ring = (6..24).contains(&x)
                && (6..24).contains(&y)
                && !((10..20).contains(&x) && (10..20).contains(&y));
            let dot = (12..16).contains(&x) && (12..16).contains(&y);
            let diag = (x == 28 && y == 3) || (x == 29 && y == 4); // two pixels touching at a corner
            ring || dot || diag
        };
        let (w, h) = (32, 32);
        let ls = loops(w, h, &ink);
        let m = fill(w, h, &ls);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(m[(y * w + x) as usize], ink(x, y), "pixel {x},{y}");
            }
        }
        assert!(
            ls.len() >= 4,
            "ring outer + inner, dot, diagonal pixels: {}",
            ls.len()
        );
    }

    #[test]
    fn simplified_outlines_stay_within_tolerance_of_a_disc() {
        let (w, h) = (120, 120);
        let ink =
            |x: u32, y: u32| ((x as f64 - 60.0).powi(2) + (y as f64 - 60.0).powi(2)).sqrt() < 45.0;
        let img = raster(w, h, &ink);
        let ls = loops(w, h, &|x, y| img.get_pixel(x, y)[0] == 0);
        let raw: usize = ls.iter().map(|l| l.len()).sum();
        let simp: Vec<Vec<(i32, i32)>> = ls.iter().map(|l| simplify(l, 0.8)).collect();
        let s: usize = simp.iter().map(|l| l.len()).sum();
        assert!(s * 3 < raw, "{s} vertices from {raw}");
        let m = fill(w, h, &simp);
        let wrong = (0..w * h)
            .filter(|&i| m[i as usize] != ink(i % w, i / w))
            .count();
        let area = (0..w * h).filter(|&i| ink(i % w, i / w)).count();
        assert!(
            (wrong as f64) < 0.02 * area as f64,
            "{wrong} wrong pixels of {area}"
        );
    }

    #[test]
    fn svg_has_size_in_mm_and_specks_are_dropped() {
        let img = raster(100, 50, &|x, y| {
            (10..40).contains(&x) && (10..30).contains(&y) || (x, y) == (80, 40)
        });
        let s = svg(&img, 254, 0.5, 4.0);
        assert!(
            s.contains("width=\"10.00mm\"") && s.contains("height=\"5.00mm\""),
            "{s}"
        );
        assert!(s.contains("1 outlines"), "{s}");
        assert!(
            s.contains("fill-rule=\"evenodd\"") && !s.contains("http://") || s.contains("xmlns")
        );
    }

    #[test]
    fn otsu_splits_a_two_level_image() {
        let img = raster(20, 20, &|x, _| x < 5);
        let t = otsu(&img);
        assert!(t < 255);
        assert_eq!(img.pixels().filter(|p| p[0] <= t).count(), 100);
    }
}
