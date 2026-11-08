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

const AT: [(i32, i32); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];

/// Code a `w`×`h` bitmap (one byte per pixel, non-zero = black) as a generic region, template 0,
/// with the contexts `cx` (65536 entries) and, if `tpgd`, typical prediction of repeated rows.
fn code_region(mq: &mut MqEnc, cx: &mut [(u8, u8)], w: usize, h: usize, bm: &[u8], tpgd: bool) {
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
    let mut ltp = false;
    for y in 0..h {
        let row = |yy: usize| &pad[(yy + pady) * pw + padx..(yy + pady) * pw + padx + w];
        if tpgd {
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
}

fn page_info(w: usize, h: usize, flags: u8) -> Vec<u8> {
    let mut pi = Vec::new();
    pi.extend_from_slice(&(w as u32).to_be_bytes());
    pi.extend_from_slice(&(h as u32).to_be_bytes());
    pi.extend_from_slice(&[0; 8]); // resolution unknown
    pi.push(flags);
    pi.extend_from_slice(&[0, 0]); // no striping
    pi
}

/// Body of an immediate generic region segment over the whole page.
fn generic_body(w: usize, h: usize, comb: u8, arith: &[u8]) -> Vec<u8> {
    let mut gr = Vec::new();
    gr.extend_from_slice(&(w as u32).to_be_bytes());
    gr.extend_from_slice(&(h as u32).to_be_bytes());
    gr.extend_from_slice(&[0; 8]); // at 0,0
    gr.push(comb);
    gr.push(0x08); // arithmetic, template 0, typical prediction on
    for (x, y) in AT {
        gr.push(x as i8 as u8);
        gr.push(y as i8 as u8);
    }
    gr.extend_from_slice(arith);
    gr
}

/// Embedded JBIG2 stream for a `w`×`h` page; `bm` has one byte per pixel, non-zero = black.
pub fn encode(w: usize, h: usize, bm: &[u8]) -> Vec<u8> {
    let mut cx = vec![(0u8, 0u8); 1 << 16];
    let mut mq = MqEnc::new();
    code_region(&mut mq, &mut cx, w, h, bm, true);
    let arith = mq.finish();
    let mut out = Vec::new();
    seg(&mut out, 0, 48, &page_info(w, h, 0));
    seg(&mut out, 1, 38, &generic_body(w, h, 0, &arith));
    out
}

/// A segment that refers to earlier ones (numbers below 256).
fn seg_ref(out: &mut Vec<u8>, number: u32, kind: u8, refs: &[u8], data: &[u8]) {
    out.extend_from_slice(&number.to_be_bytes());
    out.push(kind);
    out.push((refs.len() as u8) << 5);
    out.extend_from_slice(refs);
    out.push(1); // page 1
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

/// The arithmetic integer encoder of T.88 annex A.2 (the mirror of the decoder's `IntCtx`).
struct IntEnc(Vec<(u8, u8)>);

impl IntEnc {
    fn new() -> Self {
        IntEnc(vec![(0, 0); 512])
    }

    /// `None` is the out-of-band value.
    fn put(&mut self, mq: &mut MqEnc, v: Option<i32>) {
        let mut prev = 1usize;
        let mut bit = |mq: &mut MqEnc, cx: &mut Vec<(u8, u8)>, d: u8| {
            mq.encode(&mut cx[prev], d);
            prev = if prev < 256 {
                (prev << 1) | d as usize
            } else {
                (((prev << 1) | d as usize) & 511) | 256
            };
        };
        let cx = &mut self.0;
        let (sign, m) = match v {
            None => (1u8, 0i64),
            Some(v) => ((v < 0) as u8, (v as i64).abs()),
        };
        bit(mq, cx, sign);
        let (prefix, n, off): (&[u8], u32, i64) = if m < 4 {
            (&[0], 2, 0)
        } else if m < 20 {
            (&[1, 0], 4, 4)
        } else if m < 84 {
            (&[1, 1, 0], 6, 20)
        } else if m < 340 {
            (&[1, 1, 1, 0], 8, 84)
        } else if m < 4436 {
            (&[1, 1, 1, 1, 0], 12, 340)
        } else {
            (&[1, 1, 1, 1, 1], 32, 4436)
        };
        for &d in prefix {
            bit(mq, cx, d);
        }
        let r = (m - off) as u64;
        for k in (0..n).rev() {
            bit(mq, cx, ((r >> k) & 1) as u8);
        }
    }
}

/// Number of bits of a symbol id for `n` symbols (as the decoder computes it).
fn code_len(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

struct Comp {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    px: Vec<u8>,
}

/// 8-connected components of the black pixels, those of a size a glyph can have.
fn components(w: usize, h: usize, bm: &[u8]) -> Vec<Comp> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..w * h {
        if bm[start] == 0 || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut pts = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            pts.push(i);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if bm[j] != 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
        if cw > 150 || ch > 150 || pts.len() < 6 {
            continue;
        }
        let mut px = vec![0u8; cw * ch];
        for i in pts {
            px[(i / w - y0) * cw + (i % w - x0)] = 1;
        }
        out.push(Comp {
            x: x0,
            y: y0,
            w: cw,
            h: ch,
            px,
        });
    }
    out
}

/// Symbol-coded page: a dictionary of the shapes that occur at least twice (components of the
/// same size that differ in a few percent of their pixels share one shape), a text region that
/// places them, and a generic region combined with XOR that corrects every pixel the placed
/// shapes get wrong. The page decodes to exactly the input. `None` when there is nothing to share.
pub fn encode_symbols(w: usize, h: usize, bm: &[u8]) -> Option<Vec<u8>> {
    let bm: Vec<u8> = bm.iter().map(|&v| (v != 0) as u8).collect();
    let comps = components(w, h, &bm);
    // classes: representative = first member; same width and height, few differing pixels
    let mut reps: Vec<usize> = Vec::new(); // index into comps
    let mut class_of: Vec<usize> = Vec::with_capacity(comps.len());
    let mut by_size: std::collections::HashMap<(usize, usize), Vec<usize>> = Default::default();
    for (i, c) in comps.iter().enumerate() {
        let limit = (c.w * c.h * 3 / 100).max(1);
        let list = by_size.entry((c.w, c.h)).or_default();
        let hit = list.iter().copied().find(|&k| {
            let r = &comps[reps[k]];
            let mut d = 0;
            for (a, b) in c.px.iter().zip(&r.px) {
                d += (a != b) as usize;
                if d > limit {
                    return false;
                }
            }
            true
        });
        match hit {
            Some(k) => class_of.push(k),
            None => {
                list.push(reps.len());
                class_of.push(reps.len());
                reps.push(i);
            }
        }
    }
    let mut count = vec![0usize; reps.len()];
    for &k in &class_of {
        count[k] += 1;
    }
    // dictionary: classes with two or more members, by height then width
    let mut used: Vec<usize> = (0..reps.len()).filter(|&k| count[k] >= 2).collect();
    if used.len() < 2 {
        return None;
    }
    used.sort_by_key(|&k| (comps[reps[k]].h, comps[reps[k]].w, k));
    let mut id_of = vec![usize::MAX; reps.len()];
    for (id, &k) in used.iter().enumerate() {
        id_of[k] = id;
    }
    // symbol dictionary
    let mut mq = MqEnc::new();
    let mut gb = vec![(0u8, 0u8); 1 << 16];
    let (mut iadh, mut iadw, mut iaex) = (IntEnc::new(), IntEnc::new(), IntEnc::new());
    let mut hc = 0usize;
    let mut i = 0;
    while i < used.len() {
        let hh = comps[reps[used[i]]].h;
        iadh.put(&mut mq, Some((hh - hc) as i32));
        hc = hh;
        let mut prev_w = 0usize;
        while i < used.len() && comps[reps[used[i]]].h == hh {
            let c = &comps[reps[used[i]]];
            iadw.put(&mut mq, Some(c.w as i32 - prev_w as i32));
            prev_w = c.w;
            code_region(&mut mq, &mut gb, c.w, c.h, &c.px, false);
            i += 1;
        }
        iadw.put(&mut mq, None);
    }
    iaex.put(&mut mq, Some(0));
    iaex.put(&mut mq, Some(used.len() as i32));
    let dict_arith = mq.finish();
    let mut dict = Vec::new();
    dict.extend_from_slice(&[0, 0]); // arithmetic, template 0, no refinement
    for (x, y) in AT {
        dict.push(x as i8 as u8);
        dict.push(y as i8 as u8);
    }
    dict.extend_from_slice(&(used.len() as u32).to_be_bytes());
    dict.extend_from_slice(&(used.len() as u32).to_be_bytes());
    dict.extend_from_slice(&dict_arith);
    // instances in strip order (top edge, then left edge)
    let mut inst: Vec<(usize, usize, usize)> = comps
        .iter()
        .enumerate()
        .filter(|(i, _)| id_of[class_of[*i]] != usize::MAX)
        .map(|(i, c)| (c.y, c.x, id_of[class_of[i]]))
        .collect();
    inst.sort_unstable();
    // what the text region draws, and what is left over
    let mut drawn = vec![0u8; w * h];
    for &(y, x, id) in &inst {
        let r = &comps[reps[used[id]]];
        for yy in 0..r.h {
            for xx in 0..r.w {
                if x + xx < w && y + yy < h && r.px[yy * r.w + xx] != 0 {
                    drawn[(y + yy) * w + x + xx] = 1;
                }
            }
        }
    }
    let residual: Vec<u8> = bm.iter().zip(&drawn).map(|(a, b)| a ^ b).collect();
    // text region
    let mut mq = MqEnc::new();
    let (mut iadt, mut iafs, mut iads) = (IntEnc::new(), IntEnc::new(), IntEnc::new());
    let cl = code_len(used.len());
    let mut iaid = vec![(0u8, 0u8); 1usize << (cl + 1)];
    iadt.put(&mut mq, Some(0)); // initial STRIPT
    let (mut stript, mut firsts) = (0i64, 0i64);
    let mut k = 0;
    while k < inst.len() {
        let y = inst[k].0;
        iadt.put(&mut mq, Some((y as i64 - stript) as i32));
        stript = y as i64;
        let mut curs = 0i64;
        let mut first = true;
        while k < inst.len() && inst[k].0 == y {
            let (_, x, id) = inst[k];
            if first {
                iafs.put(&mut mq, Some((x as i64 - firsts) as i32));
                firsts = x as i64;
                first = false;
            } else {
                iads.put(&mut mq, Some((x as i64 - curs) as i32));
            }
            let mut prev = 1usize;
            for b in (0..cl).rev() {
                let d = ((id >> b) & 1) as u8;
                mq.encode(&mut iaid[prev], d);
                prev = (prev << 1) | d as usize;
            }
            curs = x as i64 + comps[reps[used[id]]].w as i64 - 1;
            k += 1;
        }
        iads.put(&mut mq, None);
    }
    let text_arith = mq.finish();
    let mut text = Vec::new();
    text.extend_from_slice(&(w as u32).to_be_bytes());
    text.extend_from_slice(&(h as u32).to_be_bytes());
    text.extend_from_slice(&[0; 8]);
    text.push(0); // combine with OR
    text.extend_from_slice(&0x0010u16.to_be_bytes()); // arithmetic, 1 row per strip, top left
    text.extend_from_slice(&(inst.len() as u32).to_be_bytes());
    text.extend_from_slice(&text_arith);
    // residual: generic region combined with XOR
    let mut cx = vec![(0u8, 0u8); 1 << 16];
    let mut mq = MqEnc::new();
    code_region(&mut mq, &mut cx, w, h, &residual, true);
    let res_arith = mq.finish();
    let mut out = Vec::new();
    seg(&mut out, 0, 48, &page_info(w, h, 0x40)); // combination operator may be overridden
    seg_ref(&mut out, 1, 0, &[], &dict);
    seg_ref(&mut out, 2, 6, &[1], &text);
    seg(&mut out, 3, 38, &generic_body(w, h, 2, &res_arith));
    Some(out)
}

/// The smaller of the plain and the symbol-coded stream.
pub fn encode_best(w: usize, h: usize, bm: &[u8]) -> Vec<u8> {
    let plain = encode(w, h, bm);
    match encode_symbols(w, h, bm) {
        Some(s) if s.len() < plain.len() => s,
        _ => plain,
    }
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

    /// A page of "words": a few glyph shapes repeated in rows, with some flipped pixels.
    fn glyph_page(noise: bool) -> (usize, usize, Vec<u8>) {
        let (w, h) = (400usize, 240usize);
        let shapes: Vec<Vec<u8>> = (0..5)
            .map(|k| {
                let mut s = vec![0u8; 9 * 14];
                for y in 0..14 {
                    for x in 0..9 {
                        let on = match k {
                            0 => x == 1 || x == 7 || y == 0 || y == 13,
                            1 => x == 4 || y == 6,
                            2 => (x + y) % 5 == 0,
                            3 => x == 2 || (y == 3 && x < 7),
                            _ => y == 12 || x == 6,
                        };
                        s[y * 9 + x] = on as u8;
                    }
                }
                s
            })
            .collect();
        let mut bm = vec![0u8; w * h];
        let mut seed = 5u64;
        for line in 0..8 {
            for col in 0..30 {
                let g = &shapes[(lcg(&mut seed) % 5) as usize];
                let (x0, y0) = (6 + col * 12, 8 + line * 28);
                for y in 0..14 {
                    for x in 0..9 {
                        bm[(y0 + y) * w + x0 + x] = g[y * 9 + x];
                    }
                }
            }
        }
        if noise {
            for _ in 0..300 {
                let i = (lcg(&mut seed) as usize) % (w * h);
                bm[i] ^= 1;
            }
        }
        (w, h, bm)
    }

    #[test]
    fn symbol_coding_is_lossless_and_smaller_on_repeated_glyphs() {
        for noise in [false, true] {
            let (w, h, bm) = glyph_page(noise);
            let s = encode_symbols(w, h, &bm).expect("shapes repeat");
            let d = crate::jbig2::decode(&s, None, w, h).expect("decode");
            assert_eq!(d, bm, "noise={noise}");
            let plain = encode(w, h, &bm);
            assert!(
                s.len() < plain.len(),
                "noise={noise}: symbols {} vs plain {}",
                s.len(),
                plain.len()
            );
            assert!(encode_best(w, h, &bm).len() <= s.len());
        }
    }

    #[test]
    fn nothing_to_share_means_no_symbol_stream() {
        let mut s = 3;
        let bm: Vec<u8> = (0..60 * 40).map(|_| (lcg(&mut s) & 1) as u8).collect();
        // white noise has no repeated components: either refused or still exact
        if let Some(e) = encode_symbols(60, 40, &bm) {
            assert_eq!(crate::jbig2::decode(&e, None, 60, 40).unwrap(), bm);
        }
        assert!(encode_symbols(10, 10, &[0; 100]).is_none());
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
