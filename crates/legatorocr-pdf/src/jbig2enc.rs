//! Lossless JBIG2 encoder for 1-bit pages: one arithmetic-coded generic region (template 0,
//! the standard adaptive pixels, typical prediction of repeated rows) in an embedded stream
//! (page information + immediate generic region), as PDF's /JBIG2Decode expects. Usually 20–40 %
//! smaller than CCITT G4 on text scans. Written from the standard's MQ coder description
//! (the same coder as JPEG 2000) and checked by decoding with this crate's decoder.

use crate::jbig2::{template, QE};

/// The MQ arithmetic encoder (T.88 annex E).
struct MqEnc {
    a: u32,
    c: u32,
    ct: u32,
    /// `out[0]` is the dummy byte before the first one.
    out: Vec<u8>,
}

impl MqEnc {
    fn new() -> MqEnc {
        MqEnc {
            a: 0x8000,
            c: 0,
            ct: 12,
            out: vec![0],
        }
    }

    fn byte_out(&mut self) {
        let last = *self.out.last().unwrap();
        if last == 0xFF {
            self.out.push((self.c >> 20) as u8);
            self.c &= 0xF_FFFF;
            self.ct = 7;
        } else if self.c < 0x800_0000 {
            self.out.push((self.c >> 19) as u8);
            self.c &= 0x7_FFFF;
            self.ct = 8;
        } else {
            *self.out.last_mut().unwrap() += 1;
            if *self.out.last().unwrap() == 0xFF {
                self.c &= 0x7FF_FFFF;
                self.out.push((self.c >> 20) as u8);
                self.c &= 0xF_FFFF;
                self.ct = 7;
            } else {
                self.out.push((self.c >> 19) as u8);
                self.c &= 0x7_FFFF;
                self.ct = 8;
            }
        }
    }

    fn renorm(&mut self) {
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// Code `bit` in the context `cx` = (state index, more probable symbol).
    fn encode(&mut self, cx: &mut (u8, u8), bit: u8) {
        let (qe, nmps, nlps, sw) = QE[cx.0 as usize];
        self.a -= qe;
        if bit == cx.1 {
            if self.a & 0x8000 == 0 {
                if self.a < qe {
                    self.a = qe;
                } else {
                    self.c += qe;
                }
                cx.0 = nmps;
                self.renorm();
            } else {
                self.c += qe;
            }
        } else {
            if self.a < qe {
                self.c += qe;
            } else {
                self.a = qe;
            }
            if sw {
                cx.1 = 1 - cx.1;
            }
            cx.0 = nlps;
            self.renorm();
        }
    }

    fn finish(mut self) -> Vec<u8> {
        // SETBITS
        let t = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= t {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if *self.out.last().unwrap() != 0xFF {
            self.out.push(0xFF);
        }
        self.out.push(0xAC);
        self.out.remove(0);
        self.out
    }
}

fn seg(out: &mut Vec<u8>, number: u32, kind: u8, data: &[u8]) {
    out.extend_from_slice(&number.to_be_bytes());
    out.push(kind); // type, page association in one byte
    out.push(0); // no referred-to segments
    out.push(1); // page 1
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

/// Embedded JBIG2 stream for a `w`×`h` page; `bm` has one byte per pixel, non-zero = black.
pub fn encode(w: usize, h: usize, bm: &[u8]) -> Vec<u8> {
    const AT: [(i32, i32); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
    let offs = template(0, &AT);
    // padded copy so that every context pixel is a constant distance away
    let (padx, pady) = (8usize, 2usize);
    let pw = w + 2 * padx;
    let mut pad = vec![0u8; pw * (h + pady)];
    for y in 0..h {
        for x in 0..w {
            pad[(y + pady) * pw + x + padx] = (bm[y * w + x] != 0) as u8;
        }
    }
    let deltas: Vec<isize> = offs
        .iter()
        .map(|&(dx, dy)| dy as isize * pw as isize + dx as isize)
        .collect();
    let mut cx = vec![(0u8, 0u8); 1 << 16];
    let mut mq = MqEnc::new();
    let mut ltp = false;
    for y in 0..h {
        let row = |yy: usize| &pad[(yy + pady) * pw + padx..(yy + pady) * pw + padx + w];
        let typical = if y == 0 {
            row(0).iter().all(|&v| v == 0)
        } else {
            row(y) == row(y - 1)
        };
        // typical prediction: one bit per row, coded as a change of the running flag
        mq.encode(&mut cx[0x9B25], (typical ^ ltp) as u8);
        ltp = typical;
        if typical {
            continue;
        }
        for x in 0..w {
            let at = ((y + pady) * pw + x + padx) as isize;
            let mut ctx = 0usize;
            for (bit, d) in deltas.iter().enumerate() {
                ctx |= (pad[(at + d) as usize] as usize) << bit;
            }
            mq.encode(&mut cx[ctx], pad[at as usize]);
        }
    }
    let arith = mq.finish();
    let mut out = Vec::new();
    // page information
    let mut pi = Vec::new();
    pi.extend_from_slice(&(w as u32).to_be_bytes());
    pi.extend_from_slice(&(h as u32).to_be_bytes());
    pi.extend_from_slice(&[0; 8]); // resolution unknown
    pi.push(0); // flags
    pi.extend_from_slice(&[0, 0]); // no striping
    seg(&mut out, 0, 48, &pi);
    // immediate generic region
    let mut gr = Vec::new();
    gr.extend_from_slice(&(w as u32).to_be_bytes());
    gr.extend_from_slice(&(h as u32).to_be_bytes());
    gr.extend_from_slice(&[0; 8]); // at 0,0
    gr.push(0); // combine with OR
    gr.push(0x08); // arithmetic, template 0, typical prediction on
    for (x, y) in AT {
        gr.push(x as i8 as u8);
        gr.push(y as i8 as u8);
    }
    gr.extend_from_slice(&arith);
    seg(&mut out, 1, 38, &gr);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    }

    fn roundtrip(w: usize, h: usize, bm: &[u8]) -> usize {
        let e = encode(w, h, bm);
        let d = crate::jbig2::decode(&e, None, w, h).expect("decode");
        let want: Vec<u8> = bm.iter().map(|&v| (v != 0) as u8).collect();
        assert_eq!(d, want, "{w}x{h}");
        e.len()
    }

    #[test]
    fn blank_and_full_pages() {
        roundtrip(37, 11, &vec![0; 37 * 11]);
        roundtrip(37, 11, &vec![1; 37 * 11]);
        roundtrip(1, 1, &[1]);
        roundtrip(8, 1, &[1, 0, 1, 0, 1, 1, 0, 0]);
    }

    #[test]
    fn noise_and_text_like_pages_decode_exactly() {
        let mut s = 7;
        // white noise: every context sees something different
        let (w, h) = (101, 53);
        let noise: Vec<u8> = (0..w * h).map(|_| (lcg(&mut s) & 1) as u8).collect();
        roundtrip(w, h, &noise);
        // text-like: strokes in rows with repeats and gaps
        let (w, h) = (300, 200);
        let mut bm = vec![0u8; w * h];
        for y in 0..h {
            if (y / 10) % 3 == 0 {
                for x in 0..w {
                    if (x / 4) % 5 != 0 && !lcg(&mut s).is_multiple_of(40) {
                        bm[y * w + x] = 1;
                    }
                }
            }
        }
        let n = roundtrip(w, h, &bm);
        assert!(n < w * h / 8, "no better than raw: {n} bytes");
    }

    #[test]
    fn many_sizes_decode_exactly() {
        let mut s = 99;
        for w in [2usize, 7, 8, 9, 31, 64, 65] {
            for h in [1usize, 2, 3, 17] {
                let bm: Vec<u8> = (0..w * h)
                    .map(|_| lcg(&mut s).is_multiple_of(3) as u8)
                    .collect();
                roundtrip(w, h, &bm);
            }
        }
    }
}
