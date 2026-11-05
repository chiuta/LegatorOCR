//! Find the fill-in places of a scanned paper form (roadmap 6.8): ruled boxes and underlines.
//!
//! Boxes: a closed rectangle drawn with horizontal and vertical lines, empty inside.
//! Underlines: a long horizontal line with clear space above it (the field is the strip above).
//! Both are found from long straight runs of ink, so handwriting, text and photographs are not
//! mistaken for fields. Checkboxes (small squares) are reported as `Check`. Scans of forms whose
//! lines are broken, very faint or heavily skewed will lose fields: deskew first.

use crate::bitimage::BitImage;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub kind: Kind,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

fn runs(bw: &BitImage, min_len: usize, horizontal: bool) -> Vec<(usize, usize, usize)> {
    // (fixed coordinate, start, end exclusive)
    let (w, h) = (bw.w as usize, bw.h as usize);
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let mut out = Vec::new();
    for o in 0..outer {
        let mut i = 0;
        while i < inner {
            let at = |i: usize| {
                if horizontal {
                    bw.data[o * w + i]
                } else {
                    bw.data[i * w + o]
                }
            };
            if at(i) == 0 {
                i += 1;
                continue;
            }
            let s = i;
            let mut gap = 0;
            let mut e = i;
            // tolerate 1-2 px breaks in a drawn line
            while i < inner && (at(i) != 0 || gap < 2 && i + 1 < inner && at(i + 1) != 0) {
                if at(i) != 0 {
                    e = i + 1;
                    gap = 0;
                } else {
                    gap += 1;
                }
                i += 1;
            }
            if e - s >= min_len {
                out.push((o, s, e));
            }
        }
    }
    out
}

/// Merge runs on adjacent rows/columns (a thick line) into one line: (fixed centre, start, end).
fn lines(rs: Vec<(usize, usize, usize)>) -> Vec<(f32, usize, usize, usize)> {
    // (centre, start, end, thickness)
    let mut out: Vec<(f32, usize, usize, usize, usize)> = Vec::new(); // (sum, n, s, e, last)
    for (o, s, e) in rs {
        match out
            .iter_mut()
            .find(|l| l.4 + 1 >= o && l.4 <= o && s.abs_diff(l.2) <= 3 && e.abs_diff(l.3) <= 3)
        {
            Some(l) => {
                l.0 += o as f32;
                l.1 += 1;
                l.2 = l.2.min(s);
                l.3 = l.3.max(e);
                l.4 = o;
            }
            None => out.push((o as f32, 1, s, e, o)),
        }
    }
    out.into_iter()
        .map(|(sum, n, s, e, _)| (sum / n as f32, s, e, n))
        .collect()
}

fn ink_in(bw: &BitImage, x0: usize, y0: usize, x1: usize, y1: usize) -> f32 {
    let (w, h) = (bw.w as usize, bw.h as usize);
    let (x1, y1) = (x1.min(w), y1.min(h));
    if x1 <= x0 || y1 <= y0 {
        return 1.0;
    }
    let mut n = 0usize;
    for y in y0..y1 {
        n += bw.data[y * w + x0..y * w + x1]
            .iter()
            .map(|&b| b as usize)
            .sum::<usize>();
    }
    n as f32 / ((x1 - x0) * (y1 - y0)) as f32
}

pub fn detect(bw: &BitImage, dpi: u32) -> Vec<Field> {
    let px = |mm: f32| (mm / 25.4 * dpi as f32).round() as usize;
    let hl = lines(runs(bw, px(3.0).max(8), true));
    let vl = lines(runs(bw, px(3.0).max(6), false));
    let tol = px(1.2).max(3);
    let mut out: Vec<Field> = Vec::new();
    let mut used_h = vec![false; hl.len()];
    // boxes: two horizontal lines joined by two vertical lines at their ends
    for (i, top) in hl.iter().enumerate() {
        for (j, bot) in hl.iter().enumerate() {
            let dy = bot.0 - top.0;
            if j == i || dy < px(3.0) as f32 || dy > px(30.0) as f32 {
                continue;
            }
            if top.1.abs_diff(bot.1) > tol || top.2.abs_diff(bot.2) > tol {
                continue;
            }
            let (x0, x1) = (top.1.min(bot.1), top.2.max(bot.2));
            let has_v = |x: usize| {
                vl.iter().any(|v| {
                    (v.0 - x as f32).abs() <= tol as f32
                        && v.1 <= top.0 as usize + tol
                        && v.2 + tol >= bot.0 as usize
                })
            };
            if !(has_v(x0 + tol / 2) && has_v(x1.saturating_sub(tol / 2 + 1))) {
                continue;
            }
            let (ix0, iy0) = (x0 + tol, top.0 as usize + top.3 + 1);
            let (ix1, iy1) = (
                x1.saturating_sub(tol),
                (bot.0 as usize).saturating_sub(bot.3),
            );
            if ix1 <= ix0 + 2 || iy1 <= iy0 + 2 || ink_in(bw, ix0, iy0, ix1, iy1) > 0.04 {
                continue;
            }
            let (w, h) = ((ix1 - ix0) as u32, (iy1 - iy0) as u32);
            let kind =
                if w <= px(9.0) as u32 && h <= px(9.0) as u32 && w.abs_diff(h) <= px(2.0) as u32 {
                    Kind::Check
                } else {
                    Kind::Text
                };
            let f = Field {
                kind,
                x: ix0 as u32,
                y: iy0 as u32,
                w,
                h,
            };
            // a box inside an already found one is a duplicate of the same cell
            if !out.iter().any(|o| {
                o.x <= f.x + 2
                    && o.y <= f.y + 2
                    && o.x + o.w + 2 >= f.x + f.w
                    && o.y + o.h + 2 >= f.y + f.h
            }) {
                out.push(f);
                used_h[i] = true;
                used_h[j] = true;
            }
        }
    }
    // underlines: long, not part of a box, clear strip above
    for (i, l) in hl.iter().enumerate() {
        let len = l.2 - l.1;
        if used_h[i] || len < px(25.0) || l.3 > px(1.8).max(3) {
            continue;
        }
        let y = l.0 as usize;
        // the edge of a frame (a vertical line leaves it downward at an end) is not an underline
        if vl.iter().any(|v| {
            ((v.0 - l.1 as f32).abs() <= tol as f32 || (v.0 - l.2 as f32).abs() <= tol as f32)
                && v.1 <= y + tol
                && v.2 > y + px(2.0)
        }) {
            continue;
        }
        let strip = px(7.0);
        if y < strip + 2 {
            continue;
        }
        let (x0, x1) = (l.1 + 2, l.2.saturating_sub(2));
        let (y0, y1) = (y - strip, y.saturating_sub(l.3 + 1));
        if ink_in(bw, x0, y0, x1, y1) > 0.02 {
            continue;
        }
        let f = Field {
            kind: Kind::Text,
            x: x0 as u32,
            y: y0 as u32,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        };
        if out
            .iter()
            .any(|o| f.x < o.x + o.w && o.x < f.x + f.w && f.y < o.y + o.h && o.y < f.y + f.h)
        {
            continue;
        }
        out.push(f);
    }
    out.sort_by_key(|f| (f.y / px(4.0).max(1) as u32, f.x));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(b: &mut BitImage, x0: u32, y0: u32, x1: u32, y1: u32, t: u32) {
        for x in x0..x1 {
            for k in 0..t {
                b.set(x, y0 + k, true);
                b.set(x, y1 - 1 - k, true);
            }
        }
        for y in y0..y1 {
            for k in 0..t {
                b.set(x0 + k, y, true);
                b.set(x1 - 1 - k, y, true);
            }
        }
    }

    #[test]
    fn finds_boxes_checkboxes_and_underlines_but_not_text() {
        let dpi = 150;
        let mut b = BitImage::new(1240, 1000);
        rect(&mut b, 100, 100, 700, 160, 2); // text box
        rect(&mut b, 100, 300, 130, 330, 2); // checkbox
        for x in 150..800 {
            b.set(x, 500, true); // underline
            b.set(x, 501, true);
        }
        // a line of "text" (short blobs) must not become a field
        for i in 0..20u32 {
            for y in 620..640 {
                for x in 100 + i * 14..108 + i * 14 {
                    b.set(x, y, true);
                }
            }
        }
        // a filled box (not empty inside) is not a field
        rect(&mut b, 100, 700, 600, 760, 2);
        for y in 705..755 {
            for x in 110..590 {
                b.set(x, y, true);
            }
        }
        let f = detect(&b, dpi);
        let text: Vec<_> = f.iter().filter(|f| f.kind == Kind::Text).collect();
        assert_eq!(
            f.iter().filter(|f| f.kind == Kind::Check).count(),
            1,
            "{f:?}"
        );
        assert_eq!(text.len(), 2, "{f:?}");
        let boxed = text.iter().find(|t| t.y < 200).expect("box");
        assert!(boxed.x >= 100 && boxed.x + boxed.w <= 700 && boxed.w > 500 && boxed.h > 40);
        let under = text.iter().find(|t| t.y > 300).expect("underline");
        assert!(under.y + under.h <= 500 && under.w > 600);
        assert!(!f.iter().any(|f| f.y >= 600), "{f:?}");
    }
}
