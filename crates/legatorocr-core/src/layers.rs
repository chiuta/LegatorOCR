//! Layered page representation (mixed-raster-content): a full-resolution 1-bit
//! *text mask* painted in one ink colour, over an optional low-resolution
//! *background* and optional full-resolution *picture patches*. Stored this way a
//! colour book page costs a few tens of kilobytes instead of megabytes, while the
//! text stays razor sharp — and the same layers drive both the on-screen preview
//! (via [`Layers::compose`]) and the PDF writer.

use crate::BitImage;
use image::{imageops, Rgb, RgbImage};

/// A full-resolution picture region, placed at (`x`,`y`) in page pixels.
#[derive(Clone)]
pub struct Patch {
    pub x: u32,
    pub y: u32,
    pub img: RgbImage,
}

/// Low-resolution paper/background covering the whole page.
#[derive(Clone)]
pub struct Background {
    pub img: RgbImage,
    /// Resolution of `img` (the page's own dpi divided by the downscale factor).
    pub dpi: u32,
}

#[derive(Clone)]
pub struct Layers {
    /// Text ink, page-sized (1 = ink).
    pub mask: BitImage,
    /// Colour the mask is painted with.
    pub fg: [u8; 3],
    pub bg: Option<Background>,
    pub patches: Vec<Patch>,
}

impl Layers {
    /// Flatten to an ordinary RGB page (preview, OCR, image export).
    pub fn compose(&self) -> RgbImage {
        let (w, h) = (self.mask.w, self.mask.h);
        let mut out = match &self.bg {
            Some(b) if b.img.width() > 0 && b.img.height() > 0 => {
                if b.img.dimensions() == (w, h) {
                    b.img.clone()
                } else {
                    imageops::resize(&b.img, w, h, imageops::FilterType::Triangle)
                }
            }
            _ => RgbImage::from_pixel(w, h, Rgb([255, 255, 255])),
        };
        for p in &self.patches {
            imageops::replace(&mut out, &p.img, p.x as i64, p.y as i64);
        }
        let fg = Rgb(self.fg);
        for y in 0..h {
            let row = &self.mask.data[y as usize * w as usize..(y as usize + 1) * w as usize];
            for (x, &m) in row.iter().enumerate() {
                if m != 0 {
                    out.put_pixel(x as u32, y, fg);
                }
            }
        }
        out
    }
}

/// 3×3 dilation of the ink mask (so anti-aliased glyph edges are excluded from the background).
pub fn dilate(m: &BitImage) -> BitImage {
    let (w, h) = (m.w as usize, m.h as usize);
    let mut horiz = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(1), (x + 2).min(w));
            horiz[y * w + x] = m.data[y * w + a..y * w + b].iter().any(|&v| v != 0) as u8;
        }
    }
    let mut out = BitImage::new(m.w, m.h);
    for y in 0..h {
        let (a, b) = (y.saturating_sub(1), (y + 2).min(h));
        for x in 0..w {
            out.data[y * w + x] = (a..b).any(|yy| horiz[yy * w + x] != 0) as u8;
        }
    }
    out
}

/// Background with the text removed: each `f`×`f` cell is the mean of its non-ink
/// pixels; cells that were all ink (heavy headings, solid rules) borrow the average
/// of their neighbours, repeated until filled.
pub fn build_background(img: &RgbImage, ink: &BitImage, f: u32) -> RgbImage {
    let f = f.max(1);
    let (w, h) = img.dimensions();
    let (nw, nh) = (w.div_ceil(f) as usize, h.div_ceil(f) as usize);
    let mut sum = vec![[0u32; 3]; nw * nh];
    let mut cnt = vec![0u32; nw * nh];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if ink.data[y * w as usize + x] == 0 {
                let c = (y / f as usize) * nw + x / f as usize;
                let p = img.get_pixel(x as u32, y as u32);
                for k in 0..3 {
                    sum[c][k] += p[k] as u32;
                }
                cnt[c] += 1;
            }
        }
    }
    let mut val: Vec<Option<[f32; 3]>> = (0..nw * nh)
        .map(|i| {
            (cnt[i] > 0).then(|| {
                [
                    sum[i][0] as f32 / cnt[i] as f32,
                    sum[i][1] as f32 / cnt[i] as f32,
                    sum[i][2] as f32 / cnt[i] as f32,
                ]
            })
        })
        .collect();
    // Fill empty cells from filled neighbours (bounded number of sweeps).
    for _ in 0..64 {
        if val.iter().all(|v| v.is_some()) {
            break;
        }
        let snapshot = val.clone();
        for y in 0..nh {
            for x in 0..nw {
                if snapshot[y * nw + x].is_some() {
                    continue;
                }
                let (mut acc, mut n) = ([0f32; 3], 0f32);
                for yy in y.saturating_sub(1)..(y + 2).min(nh) {
                    for xx in x.saturating_sub(1)..(x + 2).min(nw) {
                        if let Some(v) = snapshot[yy * nw + xx] {
                            for k in 0..3 {
                                acc[k] += v[k];
                            }
                            n += 1.0;
                        }
                    }
                }
                if n > 0.0 {
                    val[y * nw + x] = Some([acc[0] / n, acc[1] / n, acc[2] / n]);
                }
            }
        }
    }
    let mut out = RgbImage::new(nw as u32, nh as u32);
    for (i, p) in out.pixels_mut().enumerate() {
        let v = val[i].unwrap_or([255.0; 3]);
        *p = Rgb([v[0].round() as u8, v[1].round() as u8, v[2].round() as u8]);
    }
    out
}

/// Mean colour of the dark core of the text (so coloured print stays coloured, black stays black).
pub fn ink_colour(img: &RgbImage, mask: &BitImage) -> [u8; 3] {
    let (mut acc, mut n) = ([0u64; 3], 0u64);
    for (i, &m) in mask.data.iter().enumerate() {
        if m == 0 {
            continue;
        }
        let p = img.as_raw()[i * 3..i * 3 + 3]
            .iter()
            .map(|&v| v as u32)
            .collect::<Vec<_>>();
        if (p[0] * 299 + p[1] * 587 + p[2] * 114) / 1000 <= 110 {
            for k in 0..3 {
                acc[k] += p[k] as u64;
            }
            n += 1;
        }
    }
    if n < 40 {
        return [0, 0, 0];
    }
    [(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_paints_layers_in_order() {
        let mut mask = BitImage::new(8, 8);
        mask.set(1, 1, true);
        mask.set(6, 6, true);
        let l = Layers {
            mask,
            fg: [10, 20, 30],
            bg: Some(Background {
                img: RgbImage::from_pixel(4, 4, Rgb([200, 210, 220])),
                dpi: 50,
            }),
            patches: vec![Patch {
                x: 5,
                y: 5,
                img: RgbImage::from_pixel(3, 3, Rgb([1, 2, 3])),
            }],
        };
        let c = l.compose();
        assert_eq!(c.dimensions(), (8, 8));
        assert_eq!(c.get_pixel(0, 7).0, [200, 210, 220]);
        assert_eq!(c.get_pixel(5, 7).0, [1, 2, 3]);
        assert_eq!(c.get_pixel(1, 1).0, [10, 20, 30]);
        assert_eq!(c.get_pixel(6, 6).0, [10, 20, 30], "mask wins over patch");
    }

    #[test]
    fn background_ignores_ink_and_fills_solid_cells() {
        let mut img = RgbImage::from_pixel(32, 32, Rgb([220, 200, 180]));
        let mut ink = BitImage::new(32, 32);
        for y in 0..8 {
            for x in 0..8 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
                ink.set(x, y, true);
            }
        }
        let bg = build_background(&img, &ink, 8);
        assert_eq!(bg.dimensions(), (4, 4));
        assert_eq!(bg.get_pixel(1, 1).0, [220, 200, 180]);
        // the fully inked cell (0,0) is filled from its neighbours, not black
        assert_eq!(bg.get_pixel(0, 0).0, [220, 200, 180]);
        assert_eq!(dilate(&ink).count_ink(), 81);
        assert_eq!(ink_colour(&img, &ink), [0, 0, 0]);
    }
}
