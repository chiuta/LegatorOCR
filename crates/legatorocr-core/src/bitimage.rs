use image::GrayImage;

/// Bilevel image. `1` = ink (foreground), `0` = paper.
#[derive(Clone, Debug)]
pub struct BitImage {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl BitImage {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            data: vec![0; (w as usize) * (h as usize)],
        }
    }
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.data[(y as usize) * (self.w as usize) + x as usize] != 0
    }
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: bool) {
        self.data[(y as usize) * (self.w as usize) + x as usize] = v as u8;
    }
    pub fn count_ink(&self) -> u64 {
        self.data.iter().map(|&b| b as u64).sum()
    }
    /// Paper = 255, ink = 0.
    pub fn to_gray(&self) -> GrayImage {
        let raw: Vec<u8> = self
            .data
            .iter()
            .map(|&b| if b != 0 { 0 } else { 255 })
            .collect();
        GrayImage::from_raw(self.w, self.h, raw).expect("size matches")
    }
    /// Copy the `w`×`h` region of `src` starting at (`sx`,`sy`) to (`dx`,`dy`) in
    /// `self`; either side may extend past its image (clipped, nothing is wrapped).
    pub fn paste_region(&mut self, src: &BitImage, from: (i64, i64, u32, u32), to: (i64, i64)) {
        let ((sx, sy, w, h), (dx, dy)) = (from, to);
        for row in 0..h as i64 {
            let (ys, yd) = (sy + row, dy + row);
            if ys < 0 || ys >= src.h as i64 || yd < 0 || yd >= self.h as i64 {
                continue;
            }
            let x_from = (-sx).max(-dx).max(0);
            let x_to = (w as i64).min(src.w as i64 - sx).min(self.w as i64 - dx);
            if x_to <= x_from {
                continue;
            }
            let n = (x_to - x_from) as usize;
            let s = ys as usize * src.w as usize + (sx + x_from) as usize;
            let d = yd as usize * self.w as usize + (dx + x_from) as usize;
            self.data[d..d + n].copy_from_slice(&src.data[s..s + n]);
        }
    }

    /// Copy of the sub-rectangle `[x0,x1) × [0,h)`.
    pub fn crop_x(&self, x0: u32, x1: u32) -> BitImage {
        let nw = x1 - x0;
        let mut out = BitImage::new(nw, self.h);
        for y in 0..self.h {
            let s = (y as usize) * (self.w as usize) + x0 as usize;
            let d = (y as usize) * (nw as usize);
            out.data[d..d + nw as usize].copy_from_slice(&self.data[s..s + nw as usize]);
        }
        out
    }
    /// Remove 8-connected ink components with fewer than `min_px` pixels.
    /// Returns the number of components removed.
    pub fn despeckle(&mut self, min_px: u32) -> usize {
        if min_px <= 1 {
            return 0;
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let mut seen = vec![false; w * h];
        let mut removed = 0;
        let mut stack: Vec<usize> = Vec::new();
        let mut comp: Vec<usize> = Vec::new();
        for start in 0..w * h {
            if self.data[start] == 0 || seen[start] {
                continue;
            }
            stack.clear();
            comp.clear();
            let mut size = 0usize;
            stack.push(start);
            seen[start] = true;
            while let Some(p) = stack.pop() {
                size += 1;
                // only a component that stays small can be removed: do not hold the pixels
                // of a huge one (a dark scanner bed) in memory
                if comp.len() < min_px as usize {
                    comp.push(p);
                }
                let (px, py) = (p % w, p / w);
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let (nx, ny) = (px as i64 + dx, py as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let q = ny as usize * w + nx as usize;
                        if self.data[q] != 0 && !seen[q] {
                            seen[q] = true;
                            stack.push(q);
                        }
                    }
                }
            }
            if size < min_px as usize {
                for &p in &comp {
                    self.data[p] = 0;
                }
                removed += 1;
            }
        }
        removed
    }
}
