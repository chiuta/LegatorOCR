//! JBIG2 (ITU-T T.88) decoder for the *generic-region* subset used by many scanners and
//! "lossless" PDF producers: embedded streams made of a page-information segment and
//! immediate generic regions, arithmetic-coded (templates 0–3, adaptive pixels,
//! typical prediction) or MMR-coded. Symbol dictionaries, text, halftone and refinement
//! regions are not supported and are reported as such.
//!
//! Written from the standard's description of the MQ arithmetic coder and templates.

use std::fmt;

#[derive(Debug)]
pub struct Jbig2Error(pub String);
impl fmt::Display for Jbig2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
type R<T> = Result<T, Jbig2Error>;
fn fail<T>(m: impl Into<String>) -> R<T> {
    Err(Jbig2Error(m.into()))
}

/// (Qe, NMPS, NLPS, SWITCH)
const QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

struct Mq<'a> {
    d: &'a [u8],
    bp: usize,
    c: u32,
    a: u32,
    ct: i32,
}

impl<'a> Mq<'a> {
    fn new(d: &'a [u8]) -> Self {
        let mut m = Mq {
            d,
            bp: 0,
            c: 0,
            a: 0x8000,
            ct: 0,
        };
        m.c = (m.byte(0) as u32) << 16;
        m.bytein();
        m.c <<= 7;
        m.ct -= 7;
        m
    }
    fn byte(&self, i: usize) -> u8 {
        self.d.get(i).copied().unwrap_or(0xFF)
    }
    fn bytein(&mut self) {
        let b = self.byte(self.bp);
        if b == 0xFF {
            let b1 = self.byte(self.bp + 1);
            if b1 > 0x8F {
                self.c = self.c.wrapping_add(0xFF00);
                self.ct = 8;
            } else {
                self.bp += 1;
                self.c = self.c.wrapping_add((b1 as u32) << 9);
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c = self.c.wrapping_add((self.byte(self.bp) as u32) << 8);
            self.ct = 8;
        }
    }
    fn renorm(&mut self) {
        loop {
            if self.ct == 0 {
                self.bytein();
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }
    fn decode(&mut self, cx: &mut (u8, u8)) -> u8 {
        let (i, mps) = (cx.0 as usize, cx.1);
        let (qe, nmps, nlps, sw) = QE[i];
        self.a -= qe;
        if (self.c >> 16) < qe {
            // LPS path (conditional exchange)
            let d;
            if self.a < qe {
                d = mps;
                cx.0 = nmps;
            } else {
                d = 1 - mps;
                if sw {
                    cx.1 = 1 - mps;
                }
                cx.0 = nlps;
            }
            self.a = qe;
            self.renorm();
            d
        } else {
            self.c -= qe << 16;
            if self.a & 0x8000 == 0 {
                let d;
                if self.a < qe {
                    d = 1 - mps;
                    if sw {
                        cx.1 = 1 - mps;
                    }
                    cx.0 = nlps;
                } else {
                    d = mps;
                    cx.0 = nmps;
                }
                self.renorm();
                d
            } else {
                mps
            }
        }
    }
}

/// Template pixel offsets in context-bit order; `at` fills the adaptive positions.
fn template(t: u8, at: &[(i32, i32); 4]) -> Vec<(i32, i32)> {
    match t {
        0 => vec![
            (-1, 0),
            (-2, 0),
            (-3, 0),
            (-4, 0),
            at[0],
            (2, -1),
            (1, -1),
            (0, -1),
            (-1, -1),
            (-2, -1),
            at[1],
            at[2],
            (1, -2),
            (0, -2),
            (-1, -2),
            at[3],
        ],
        1 => vec![
            (-1, 0),
            (-2, 0),
            (-3, 0),
            at[0],
            (2, -1),
            (1, -1),
            (0, -1),
            (-1, -1),
            (-2, -1),
            (2, -2),
            (1, -2),
            (0, -2),
            (-1, -2),
        ],
        2 => vec![
            (-1, 0),
            (-2, 0),
            at[0],
            (1, -1),
            (0, -1),
            (-1, -1),
            (-2, -1),
            (1, -2),
            (0, -2),
            (-1, -2),
        ],
        _ => vec![
            (-1, 0),
            (-2, 0),
            (-3, 0),
            (-4, 0),
            at[0],
            (1, -1),
            (0, -1),
            (-1, -1),
            (-2, -1),
            (-3, -1),
        ],
    }
}

/// Arithmetic-coded generic region → `w*h` bytes (1 = black).
fn generic_arith(
    data: &[u8],
    w: usize,
    h: usize,
    t: u8,
    tpgdon: bool,
    at: [(i32, i32); 4],
) -> Vec<u8> {
    let mut cx = vec![(0u8, 0u8); 1 << template(t, &at).len()];
    let mut mq = Mq::new(data);
    generic_with(&mut mq, &mut cx, w, h, t, tpgdon, at)
}

/// The same with a caller-owned decoder and context set (symbol dictionaries share them).
fn generic_with(
    mq: &mut Mq,
    cx: &mut [(u8, u8)],
    w: usize,
    h: usize,
    t: u8,
    tpgdon: bool,
    at: [(i32, i32); 4],
) -> Vec<u8> {
    let offs = template(t, &at);
    let sltp: usize = [0x9B25, 0x0795, 0x00E5, 0x0195][t as usize & 3];
    let mut bm = vec![0u8; w * h];
    let mut ltp = false;
    for y in 0..h {
        if tpgdon {
            ltp ^= mq.decode(&mut cx[sltp]) == 1;
            if ltp {
                if y > 0 {
                    bm.copy_within((y - 1) * w..y * w, y * w);
                }
                continue;
            }
        }
        for x in 0..w {
            let mut ctx = 0usize;
            for (bit, &(dx, dy)) in offs.iter().enumerate() {
                let (px, py) = (x as i32 + dx, y as i32 + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    ctx |= (bm[py as usize * w + px as usize] as usize) << bit;
                }
            }
            bm[y * w + x] = mq.decode(&mut cx[ctx]);
        }
    }
    bm
}

/// Arithmetic integer decoder (Annex A.2); `None` is the out-of-band value.
struct IntCtx(Vec<(u8, u8)>);
impl IntCtx {
    fn new() -> Self {
        IntCtx(vec![(0, 0); 512])
    }
    fn decode(&mut self, mq: &mut Mq) -> Option<i32> {
        let mut prev = 1usize;
        let mut bit = |mq: &mut Mq, cx: &mut Vec<(u8, u8)>| {
            let d = mq.decode(&mut cx[prev]) as usize;
            prev = if prev < 256 {
                (prev << 1) | d
            } else {
                (((prev << 1) | d) & 511) | 256
            };
            d
        };
        let cx = &mut self.0;
        let sign = bit(mq, cx);
        let (n, off) = if bit(mq, cx) == 0 {
            (2, 0)
        } else if bit(mq, cx) == 0 {
            (4, 4)
        } else if bit(mq, cx) == 0 {
            (6, 20)
        } else if bit(mq, cx) == 0 {
            (8, 84)
        } else if bit(mq, cx) == 0 {
            (12, 340)
        } else {
            (32, 4436)
        };
        let mut v: i64 = 0;
        for _ in 0..n {
            v = (v << 1) | bit(mq, cx) as i64;
        }
        v += off;
        match (sign, v) {
            (0, v) => Some(v.min(i32::MAX as i64) as i32),
            (_, v) if v > 0 => Some((-v).max(i32::MIN as i64) as i32),
            _ => None,
        }
    }
}

fn decode_iaid(mq: &mut Mq, cx: &mut [(u8, u8)], len: u32) -> usize {
    let mut prev = 1usize;
    for _ in 0..len {
        let d = mq.decode(&mut cx[prev]) as usize;
        prev = (prev << 1) | d;
    }
    prev - (1usize << len)
}

#[derive(Clone)]
struct Bitmap {
    w: usize,
    h: usize,
    d: Vec<u8>,
}

fn be16(b: &[u8], at: usize) -> R<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or_else(|| Jbig2Error("truncated JBIG2 data".into()))
}

/// Symbol dictionary (arithmetic coding, no refinement/aggregation) → exported symbols.
fn symbol_dict(data: &[u8], inputs: &[Bitmap]) -> R<Vec<Bitmap>> {
    let flags = be16(data, 0)?;
    if flags & 1 != 0 {
        return fail("JBIG2 Huffman-coded symbol dictionaries are not supported");
    }
    if flags & 2 != 0 {
        return fail("JBIG2 refinement/aggregate symbol dictionaries are not supported");
    }
    if flags & 0x100 != 0 {
        return fail("JBIG2 symbol dictionaries with retained contexts are not supported");
    }
    let t = ((flags >> 10) & 3) as u8;
    let mut p = 2;
    let n_at = if t == 0 { 4 } else { 1 };
    let raw = data
        .get(p..p + 2 * n_at)
        .ok_or_else(|| Jbig2Error("truncated JBIG2 symbol dictionary".into()))?;
    let mut at = [(0i32, 0i32); 4];
    for (i, a) in at.iter_mut().enumerate().take(n_at) {
        *a = (raw[2 * i] as i8 as i32, raw[2 * i + 1] as i8 as i32);
    }
    p += 2 * n_at;
    let n_ex = be32(data, p)? as usize;
    let n_new = be32(data, p + 4)? as usize;
    p += 8;
    if n_new > 65_536 || n_ex > 131_072 + inputs.len() {
        return fail("JBIG2 symbol dictionary too large");
    }
    let mut mq = Mq::new(&data[p.min(data.len())..]);
    let mut gb = vec![(0u8, 0u8); 1 << template(t, &at).len()];
    let (mut iadh, mut iadw, mut iaex) = (IntCtx::new(), IntCtx::new(), IntCtx::new());
    let mut news: Vec<Bitmap> = Vec::with_capacity(n_new.min(4096));
    let mut budget = 0u64;
    let mut hc_h: i64 = 0;
    while news.len() < n_new {
        let dh = iadh
            .decode(&mut mq)
            .ok_or_else(|| Jbig2Error("bad JBIG2 symbol height".into()))?;
        hc_h += dh as i64;
        if !(0..=65_535).contains(&hc_h) {
            return fail("bad JBIG2 symbol height");
        }
        let mut sym_w: i64 = 0;
        while let Some(dw) = iadw.decode(&mut mq) {
            if news.len() >= n_new {
                return fail("JBIG2 symbol dictionary overrun");
            }
            sym_w += dw as i64;
            if !(0..=65_535).contains(&sym_w) {
                return fail("bad JBIG2 symbol width");
            }
            let (w, h) = (sym_w as usize, hc_h as usize);
            budget += w as u64 * h as u64;
            if w as u64 * h as u64 > 16_000_000 || budget > 200_000_000 {
                return fail("JBIG2 symbols are too large");
            }
            let d = if w == 0 || h == 0 {
                Vec::new()
            } else {
                generic_with(&mut mq, &mut gb, w, h, t, false, at)
            };
            news.push(Bitmap { w, h, d });
        }
    }
    let total = inputs.len() + news.len();
    let mut out = Vec::new();
    let (mut idx, mut flag) = (0usize, false);
    let mut guard = 0;
    while idx < total && guard < total + 8 {
        guard += 1;
        let run = iaex
            .decode(&mut mq)
            .ok_or_else(|| Jbig2Error("bad JBIG2 export run".into()))?;
        if run < 0 || idx + run as usize > total {
            return fail("bad JBIG2 export run");
        }
        if flag {
            for k in idx..idx + run as usize {
                out.push(if k < inputs.len() {
                    inputs[k].clone()
                } else {
                    news[k - inputs.len()].clone()
                });
            }
        }
        idx += run as usize;
        flag = !flag;
    }
    if out.len() != n_ex && n_ex != 0 {
        // tolerate producers that count differently, but never exceed the declared total
        if out.len() > total {
            return fail("bad JBIG2 export table");
        }
    }
    Ok(out)
}

/// Text region (arithmetic coding, no refinement) → region bitmap.
fn text_region(data: &[u8], rw: usize, rh: usize, syms: &[Bitmap]) -> R<Vec<u8>> {
    let flags = be16(data, 0)?;
    if flags & 1 != 0 {
        return fail("JBIG2 Huffman-coded text regions are not supported");
    }
    if flags & 2 != 0 {
        return fail("JBIG2 text regions with refinement are not supported");
    }
    let log_strips = (flags >> 2) & 3;
    let strips = 1i64 << log_strips;
    let corner = (flags >> 4) & 3; // 0 BL, 1 TL, 2 BR, 3 TR
    let transposed = flags & 0x40 != 0;
    let comb = ((flags >> 7) & 3) as u8;
    let def_pixel = ((flags >> 9) & 1) as u8;
    let mut dso = ((flags >> 10) & 31) as i32;
    if dso > 15 {
        dso -= 32;
    }
    let n_inst = be32(data, 2)? as usize;
    if n_inst > 4_000_000 {
        return fail("too many JBIG2 symbol instances");
    }
    let mut mq = Mq::new(&data[6.min(data.len())..]);
    let n_sym = syms.len();
    let code_len = if n_sym <= 1 {
        0
    } else {
        usize::BITS - (n_sym - 1).leading_zeros()
    };
    let (mut iadt, mut iafs, mut iads, mut iait) =
        (IntCtx::new(), IntCtx::new(), IntCtx::new(), IntCtx::new());
    let mut iaid = vec![(0u8, 0u8); 1usize << (code_len + 1)];
    let mut bm = vec![def_pixel; rw * rh];
    let mut stript: i64 = -(iadt
        .decode(&mut mq)
        .ok_or_else(|| Jbig2Error("bad JBIG2 text region".into()))?
        as i64)
        * strips;
    let mut firsts: i64 = 0;
    let mut done = 0usize;
    let mut drawn = 0u64;
    while done < n_inst {
        let dt = iadt
            .decode(&mut mq)
            .ok_or_else(|| Jbig2Error("bad JBIG2 text region".into()))? as i64;
        stript += dt * strips;
        let mut curs: i64 = 0;
        let mut first = true;
        loop {
            if first {
                let dfs = iafs
                    .decode(&mut mq)
                    .ok_or_else(|| Jbig2Error("bad JBIG2 text region".into()))?
                    as i64;
                firsts += dfs;
                curs = firsts;
                first = false;
            } else {
                match iads.decode(&mut mq) {
                    None => break,
                    Some(ids) => curs += ids as i64 + dso as i64,
                }
            }
            if done >= n_inst {
                return fail("JBIG2 text region overrun");
            }
            let curt = if strips == 1 {
                0
            } else {
                iait.decode(&mut mq)
                    .ok_or_else(|| Jbig2Error("bad JBIG2 text region".into()))?
                    as i64
            };
            let ti = stript + curt;
            let id = decode_iaid(&mut mq, &mut iaid, code_len);
            let sym = syms
                .get(id)
                .ok_or_else(|| Jbig2Error("JBIG2 symbol id out of range".into()))?;
            let (wi, hi) = (sym.w as i64, sym.h as i64);
            if !transposed && corner >= 2 {
                curs += wi - 1;
            } else if transposed && (corner == 0 || corner == 2) {
                curs += hi - 1;
            }
            let si = curs;
            let (x, y) = if !transposed {
                match corner {
                    1 => (si, ti),
                    3 => (si - wi + 1, ti),
                    0 => (si, ti - hi + 1),
                    _ => (si - wi + 1, ti - hi + 1),
                }
            } else {
                match corner {
                    1 => (ti, si),
                    3 => (ti - wi + 1, si),
                    0 => (ti, si - hi + 1),
                    _ => (ti - wi + 1, si - hi + 1),
                }
            };
            drawn += (sym.w * sym.h) as u64 + 1;
            if drawn > 1_000_000_000 {
                return fail("JBIG2 text region is too expensive");
            }
            for yy in 0..sym.h {
                for xx in 0..sym.w {
                    let (px, py) = (x + xx as i64, y + yy as i64);
                    if px < 0 || py < 0 || px >= rw as i64 || py >= rh as i64 {
                        continue;
                    }
                    let (dst, src) = (
                        &mut bm[py as usize * rw + px as usize],
                        sym.d[yy * sym.w + xx],
                    );
                    *dst = match comb {
                        0 => *dst | src,
                        1 => *dst & src,
                        2 => *dst ^ src,
                        _ => 1 - (*dst ^ src),
                    };
                }
            }
            if !transposed && corner < 2 {
                curs += wi - 1;
            } else if transposed && (corner == 1 || corner == 3) {
                curs += hi - 1;
            }
            done += 1;
        }
    }
    Ok(bm)
}

struct Seg {
    num: u32,
    kind: u8,
    refs: Vec<u32>,
    data: Vec<u8>,
}

fn be32(b: &[u8], at: usize) -> R<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| Jbig2Error("truncated JBIG2 data".into()))
}

fn parse_segments(mut d: &[u8], out: &mut Vec<Seg>) -> R<()> {
    while d.len() >= 11 {
        let num = be32(d, 0)?;
        let flags = d[4];
        let kind = flags & 0x3F;
        let big_page = flags & 0x40 != 0;
        let mut p = 5;
        let first = *d
            .get(p)
            .ok_or_else(|| Jbig2Error("truncated segment header".into()))?;
        let count = if first >> 5 == 7 {
            let n = (be32(d, p)? & 0x1FFF_FFFF) as usize;
            p += 4 + (n + 1).div_ceil(8);
            n
        } else {
            p += 1;
            (first >> 5) as usize
        };
        let ref_size = if num <= 256 {
            1
        } else if num <= 65536 {
            2
        } else {
            4
        };
        if count > 100_000 {
            return fail("bad JBIG2 segment");
        }
        let mut refs = Vec::with_capacity(count);
        for k in 0..count {
            let at = p + k * ref_size;
            let v = match ref_size {
                1 => *d
                    .get(at)
                    .ok_or_else(|| Jbig2Error("truncated segment header".into()))?
                    as u32,
                2 => be16(d, at)? as u32,
                _ => be32(d, at)?,
            };
            refs.push(v);
        }
        p += count * ref_size;
        p += if big_page { 4 } else { 1 };
        let len = be32(d, p)? as usize;
        p += 4;
        if len == 0xFFFF_FFFF {
            return fail("JBIG2 segment of unknown length is not supported");
        }
        let body = d
            .get(p..p.saturating_add(len))
            .ok_or_else(|| Jbig2Error("truncated JBIG2 segment".into()))?;
        out.push(Seg {
            num,
            kind,
            refs,
            data: body.to_vec(),
        });
        d = &d[p + len..];
        if out.len() > 10_000 {
            return fail("too many JBIG2 segments");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blit(
    page: &mut [u8],
    w: usize,
    h: usize,
    bm: &[u8],
    rw: usize,
    rh: usize,
    rx: usize,
    ry: usize,
    op: u8,
) {
    for yy in 0..rh {
        for xx in 0..rw {
            let (px, py) = (rx + xx, ry + yy);
            if px >= w || py >= h {
                continue;
            }
            let (dst, src) = (&mut page[py * w + px], bm[yy * rw + xx]);
            *dst = match op {
                0 => *dst | src,
                1 => *dst & src,
                2 => *dst ^ src,
                3 => 1 - (*dst ^ src),
                _ => src,
            };
        }
    }
}

/// Decode an embedded JBIG2 stream (plus optional globals) to a `w`×`h` bitmap, one byte
/// per pixel, 1 = black.
pub fn decode(data: &[u8], globals: Option<&[u8]>, w: usize, h: usize) -> R<Vec<u8>> {
    if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > 400_000_000) {
        return fail("invalid JBIG2 page size");
    }
    let mut segs = Vec::new();
    if let Some(g) = globals {
        parse_segments(g, &mut segs)?;
    }
    parse_segments(data, &mut segs)?;
    let mut page = vec![0u8; w * h];
    let mut regions = 0;
    let mut dicts: std::collections::HashMap<u32, Vec<Bitmap>> = std::collections::HashMap::new();
    for s in &segs {
        let data: &[u8] = &s.data;
        match s.kind {
            48 => {
                // page information: pages w/h may be 0xFFFFFFFF (striped); the PDF dictionary wins
                if data.len() >= 17 && data[16] & 4 != 0 {
                    page.iter_mut().for_each(|p| *p = 1);
                }
            }
            36 | 38 | 39 => {
                if data.len() < 18 {
                    return fail("truncated JBIG2 region");
                }
                let (rw, rh) = (be32(data, 0)? as usize, be32(data, 4)? as usize);
                let (rx, ry) = (be32(data, 8)? as usize, be32(data, 12)? as usize);
                let op = data[16] & 7;
                if rw == 0 || rh == 0 || rw > w || rh > h {
                    return fail("invalid JBIG2 region size");
                }
                let f = data[17];
                let mmr = f & 1 != 0;
                let t = (f >> 1) & 3;
                let tpgdon = f & 8 != 0;
                let mut p = 18;
                let mut at = [(0i32, 0i32); 4];
                let defaults: [[(i32, i32); 4]; 4] = [
                    [(3, -1), (-3, -1), (2, -2), (-2, -2)],
                    [(3, -1), (0, 0), (0, 0), (0, 0)],
                    [(2, -1), (0, 0), (0, 0), (0, 0)],
                    [(2, -1), (0, 0), (0, 0), (0, 0)],
                ];
                let bm = if mmr {
                    let (rows, _) = crate::g4::decode(&data[p..], rw, rh, false);
                    rows.iter()
                        .flat_map(|r| r.iter().map(|&b| b as u8))
                        .collect()
                } else {
                    let n = if t == 0 { 4 } else { 1 };
                    if f & 0x10 != 0 {
                        return fail("JBIG2 extended templates are not supported");
                    }
                    let raw = data
                        .get(p..p + 2 * n)
                        .ok_or_else(|| Jbig2Error("truncated JBIG2 region".into()))?;
                    for (i, a) in at.iter_mut().enumerate().take(n) {
                        *a = (raw[2 * i] as i8 as i32, raw[2 * i + 1] as i8 as i32);
                    }
                    for (i, a) in at.iter_mut().enumerate().skip(n) {
                        *a = defaults[t as usize][i];
                    }
                    p += 2 * n;
                    // adaptive pixels must reference decoded pixels only
                    if at
                        .iter()
                        .take(n)
                        .any(|&(x, y)| y > 0 || (y == 0 && x >= 0) || x.abs() > 127 || y < -128)
                    {
                        return fail("invalid JBIG2 adaptive template pixel");
                    }
                    generic_arith(&data[p..], rw, rh, t, tpgdon, at)
                };
                blit(&mut page, w, h, &bm, rw, rh, rx, ry, op);
                regions += 1;
            }
            0 => {
                let mut inputs: Vec<Bitmap> = Vec::new();
                for r in &s.refs {
                    if let Some(d) = dicts.get(r) {
                        inputs.extend(d.iter().cloned());
                    }
                }
                if inputs.len() > 131_072 {
                    return fail("JBIG2 symbol dictionary too large");
                }
                dicts.insert(s.num, symbol_dict(data, &inputs)?);
            }
            6 | 7 => {
                if data.len() < 17 {
                    return fail("truncated JBIG2 region");
                }
                let (rw, rh) = (be32(data, 0)? as usize, be32(data, 4)? as usize);
                let (rx, ry) = (be32(data, 8)? as usize, be32(data, 12)? as usize);
                let op = data[16] & 7;
                if rw == 0 || rh == 0 || rw > w || rh > h {
                    return fail("invalid JBIG2 region size");
                }
                let mut syms: Vec<Bitmap> = Vec::new();
                for r in &s.refs {
                    if let Some(d) = dicts.get(r) {
                        syms.extend(d.iter().cloned());
                    }
                }
                let bm = text_region(&data[17..], rw, rh, &syms)?;
                blit(&mut page, w, h, &bm, rw, rh, rx, ry, op);
                regions += 1;
            }
            16 | 22 | 23 => return fail("JBIG2 halftone regions are not supported"),
            40 | 42 | 43 => return fail("JBIG2 refinement regions are not supported"),
            _ => {}
        }
    }
    if regions == 0 {
        return fail("no JBIG2 image data found");
    }
    Ok(page)
}

#[cfg(test)]
pub(crate) mod enc {
    //! Test-only MQ encoder + generic-region writer (symmetric to the decoder, and
    //! cross-checked against poppler in the reader tests).
    use super::*;

    pub struct Mqe {
        out: Vec<u8>,
        c: u32,
        a: u32,
        ct: i32,
        cx: Vec<(u8, u8)>,
    }
    impl Mqe {
        pub fn new(n: usize) -> Self {
            Mqe {
                out: vec![0],
                c: 0,
                a: 0x8000,
                ct: 12,
                cx: vec![(0, 0); n],
            }
        }
        fn byteout(&mut self) {
            let b = *self.out.last().unwrap();
            if b == 0xFF {
                self.out.push((self.c >> 20) as u8);
                self.c &= 0xFFFFF;
                self.ct = 7;
            } else if self.c < 0x800_0000 {
                self.out.push((self.c >> 19) as u8);
                self.c &= 0x7FFFF;
                self.ct = 8;
            } else {
                *self.out.last_mut().unwrap() += 1;
                if *self.out.last().unwrap() == 0xFF {
                    self.c &= 0x7FF_FFFF;
                    self.out.push((self.c >> 20) as u8);
                    self.c &= 0xFFFFF;
                    self.ct = 7;
                } else {
                    self.out.push((self.c >> 19) as u8);
                    self.c &= 0x7FFFF;
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
                    self.byteout();
                }
                if self.a & 0x8000 != 0 {
                    break;
                }
            }
        }
        pub fn encode(&mut self, d: u8, cx: usize) {
            let (i, mps) = self.cx[cx];
            let (qe, nmps, nlps, sw) = QE[i as usize];
            self.a -= qe;
            if d == mps {
                if self.a & 0x8000 == 0 {
                    if self.a < qe {
                        self.a = qe;
                    } else {
                        self.c += qe;
                    }
                    self.cx[cx].0 = nmps;
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
                    self.cx[cx].1 = 1 - mps;
                }
                self.cx[cx].0 = nlps;
                self.renorm();
            }
        }
        pub fn finish(mut self) -> Vec<u8> {
            let temp = self.c + self.a;
            self.c |= 0xFFFF;
            if self.c >= temp {
                self.c -= 0x8000;
            }
            self.c <<= self.ct;
            self.byteout();
            self.c <<= self.ct;
            self.byteout();
            if *self.out.last().unwrap() != 0xFF {
                self.out.push(0xFF);
            }
            self.out.push(0xAC);
            self.out.remove(0);
            self.out
        }
    }

    pub fn encode_generic(
        bm: &[u8],
        w: usize,
        h: usize,
        t: u8,
        tpgdon: bool,
        at: [(i32, i32); 4],
    ) -> Vec<u8> {
        let mut e = Mqe::new(1 << template(t, &at).len());
        encode_generic_with(&mut e, bm, w, h, t, tpgdon, at);
        e.finish()
    }

    pub fn encode_generic_with(
        e: &mut Mqe,
        bm: &[u8],
        w: usize,
        h: usize,
        t: u8,
        tpgdon: bool,
        at: [(i32, i32); 4],
    ) {
        let offs = template(t, &at);
        let sltp: usize = [0x9B25, 0x0795, 0x00E5, 0x0195][t as usize & 3];
        let mut ltp = false;
        for y in 0..h {
            if tpgdon {
                let same = y > 0 && bm[y * w..(y + 1) * w] == bm[(y - 1) * w..y * w]
                    || y == 0 && bm[..w].iter().all(|&p| p == 0);
                e.encode((same != ltp) as u8, sltp);
                ltp = same;
                if same {
                    continue;
                }
            }
            for x in 0..w {
                let mut ctx = 0usize;
                for (bit, &(dx, dy)) in offs.iter().enumerate() {
                    let (px, py) = (x as i32 + dx, y as i32 + dy);
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        ctx |= (bm[py as usize * w + px as usize] as usize) << bit;
                    }
                }
                e.encode(bm[y * w + x], ctx);
            }
        }
    }

    /// Arithmetic integer (Annex A.2) with contexts at `base..base+512`; `None` = OOB.
    pub fn enc_int(e: &mut Mqe, base: usize, v: Option<i32>) {
        let mut prev = 1usize;
        let mut bit = |e: &mut Mqe, d: usize| {
            e.encode(d as u8, base + prev);
            prev = if prev < 256 {
                (prev << 1) | d
            } else {
                (((prev << 1) | d) & 511) | 256
            };
        };
        let (sign, a) = match v {
            None => (1usize, 0i64),
            Some(v) => ((v < 0) as usize, (v as i64).abs()),
        };
        bit(e, sign);
        let (prefix, n, off): (&[usize], u32, i64) = if a < 4 {
            (&[0], 2, 0)
        } else if a < 20 {
            (&[1, 0], 4, 4)
        } else if a < 84 {
            (&[1, 1, 0], 6, 20)
        } else if a < 340 {
            (&[1, 1, 1, 0], 8, 84)
        } else if a < 4436 {
            (&[1, 1, 1, 1, 0], 12, 340)
        } else {
            (&[1, 1, 1, 1, 1], 32, 4436)
        };
        for &b in prefix {
            bit(e, b);
        }
        let r = a - off;
        for i in (0..n).rev() {
            bit(e, ((r >> i) & 1) as usize);
        }
    }

    // context layout of the combined encoder
    pub const IADH: usize = 1 << 16;
    pub const IADW: usize = IADH + 512;
    pub const IAEX: usize = IADW + 512;
    pub const IADT: usize = IAEX + 512;
    pub const IAFS: usize = IADT + 512;
    pub const IADS: usize = IAFS + 512;
    pub const IAIT: usize = IADS + 512;
    pub const IAID: usize = IAIT + 512;

    pub fn enc_iaid(e: &mut Mqe, id: usize, len: u32) {
        let mut prev = 1usize;
        for i in (0..len).rev() {
            let d = (id >> i) & 1;
            e.encode(d as u8, IAID + prev);
            prev = (prev << 1) | d;
        }
    }

    pub fn seg(num: u32, kind: u8, page: u8, body: &[u8]) -> Vec<u8> {
        seg_refs(num, kind, page, &[], body)
    }

    pub fn seg_refs(num: u32, kind: u8, page: u8, refs: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = num.to_be_bytes().to_vec();
        v.push(kind);
        v.push((refs.len() as u8) << 5);
        v.extend_from_slice(refs);
        v.push(page);
        v.extend((body.len() as u32).to_be_bytes());
        v.extend(body);
        v
    }

    /// A complete embedded stream: page info + one immediate generic region.
    pub fn stream(
        bm: &[u8],
        w: usize,
        h: usize,
        t: u8,
        tpgdon: bool,
        at: [(i32, i32); 4],
    ) -> Vec<u8> {
        let mut pi = (w as u32).to_be_bytes().to_vec();
        pi.extend((h as u32).to_be_bytes());
        pi.extend([0; 8]);
        pi.extend([0, 0, 0]);
        let mut r = (w as u32).to_be_bytes().to_vec();
        r.extend((h as u32).to_be_bytes());
        r.extend([0; 8]);
        r.push(0);
        let n = if t == 0 { 4 } else { 1 };
        r.push((t << 1) | if tpgdon { 8 } else { 0 });
        for a in at.iter().take(n) {
            r.push(a.0 as i8 as u8);
            r.push(a.1 as i8 as u8);
        }
        r.extend(encode_generic(bm, w, h, t, tpgdon, at));
        let mut out = seg(0, 48, 1, &pi);
        out.extend(seg(1, 38, 1, &r));
        out
    }

    pub struct TextCfg {
        pub corner: u8,
        pub transposed: bool,
        pub log_strips: u8,
        pub comb: u8,
        pub dso: i32,
    }

    /// Symbol dictionary (+ text region) stream. `syms` sorted by height; `placed` =
    /// (symbol id, x, y) of each symbol's top-left corner on the page.
    pub fn symbol_stream(
        w: usize,
        h: usize,
        syms: &[(usize, usize, Vec<u8>)],
        placed: &[(usize, i64, i64)],
        t: u8,
        at: [(i32, i32); 4],
        cfg: &TextCfg,
    ) -> Vec<u8> {
        let n = syms.len();
        let code_len = if n <= 1 {
            0
        } else {
            usize::BITS - (n - 1).leading_zeros()
        };
        let mut e = Mqe::new(IAID + (1usize << (code_len + 1)));
        // --- symbol dictionary
        let mut prev_h = 0i32;
        let mut i = 0;
        while i < n {
            let hh = syms[i].1 as i32;
            enc_int(&mut e, IADH, Some(hh - prev_h));
            prev_h = hh;
            let mut prev_w = 0i32;
            while i < n && syms[i].1 as i32 == hh {
                let ww = syms[i].0 as i32;
                enc_int(&mut e, IADW, Some(ww - prev_w));
                prev_w = ww;
                encode_generic_with(&mut e, &syms[i].2, syms[i].0, syms[i].1, t, false, at);
                i += 1;
            }
            enc_int(&mut e, IADW, None);
        }
        enc_int(&mut e, IAEX, Some(0));
        enc_int(&mut e, IAEX, Some(n as i32));
        let sd_data = e.finish();
        let mut sd = ((t as u16) << 10).to_be_bytes().to_vec();
        let nat = if t == 0 { 4 } else { 1 };
        for a in at.iter().take(nat) {
            sd.push(a.0 as i8 as u8);
            sd.push(a.1 as i8 as u8);
        }
        sd.extend((n as u32).to_be_bytes());
        sd.extend((n as u32).to_be_bytes());
        sd.extend(sd_data);

        // --- text region (own coder: the dictionary's was flushed)
        let mut e = Mqe::new(IAID + (1usize << (code_len + 1)));
        let strips = 1i64 << cfg.log_strips;
        let right = cfg.corner >= 2;
        let bottom = cfg.corner == 0 || cfg.corner == 2;
        let top = cfg.corner == 1 || cfg.corner == 3;
        let left = cfg.corner < 2;
        struct Inst {
            strip: i64,
            s: i64,
            curt: i64,
            id: usize,
            pre: i64,
            post: i64,
        }
        let mut inst: Vec<Inst> = placed
            .iter()
            .map(|&(id, x, y)| {
                let (sw, sh) = (syms[id].0 as i64, syms[id].1 as i64);
                let (s_ref, t_ref, pre, post) = if !cfg.transposed {
                    (
                        if right { x + sw - 1 } else { x },
                        if bottom { y + sh - 1 } else { y },
                        if right { sw - 1 } else { 0 },
                        if left { sw - 1 } else { 0 },
                    )
                } else {
                    (
                        if bottom { y + sh - 1 } else { y },
                        if right { x + sw - 1 } else { x },
                        if bottom { sh - 1 } else { 0 },
                        if top { sh - 1 } else { 0 },
                    )
                };
                Inst {
                    strip: t_ref.div_euclid(strips),
                    s: s_ref,
                    curt: t_ref.rem_euclid(strips),
                    id,
                    pre,
                    post,
                }
            })
            .collect();
        inst.sort_by_key(|i| (i.strip, i.s));
        enc_int(&mut e, IADT, Some(0));
        let (mut cur_stript, mut firsts) = (0i64, 0i64);
        let mut k = 0;
        while k < inst.len() {
            let strip = inst[k].strip;
            enc_int(
                &mut e,
                IADT,
                Some(((strip * strips - cur_stript) / strips) as i32),
            );
            cur_stript = strip * strips;
            let mut prev_end = 0i64;
            let mut first = true;
            while k < inst.len() && inst[k].strip == strip {
                let it = &inst[k];
                let curs = it.s - it.pre;
                if first {
                    enc_int(&mut e, IAFS, Some((curs - firsts) as i32));
                    firsts = curs;
                    first = false;
                } else {
                    enc_int(
                        &mut e,
                        IADS,
                        Some((curs - prev_end - cfg.dso as i64) as i32),
                    );
                }
                if strips > 1 {
                    enc_int(&mut e, IAIT, Some(it.curt as i32));
                }
                enc_iaid(&mut e, it.id, code_len);
                prev_end = it.s + it.post;
                k += 1;
            }
            enc_int(&mut e, IADS, None);
        }
        let tr_data = e.finish();
        let mut region = (w as u32).to_be_bytes().to_vec();
        region.extend((h as u32).to_be_bytes());
        region.extend([0; 8]);
        region.push(0);
        let flags: u16 = ((cfg.log_strips as u16) << 2)
            | ((cfg.corner as u16) << 4)
            | ((cfg.transposed as u16) << 6)
            | ((cfg.comb as u16) << 7)
            | (((cfg.dso & 31) as u16) << 10);
        region.extend(flags.to_be_bytes());
        region.extend((placed.len() as u32).to_be_bytes());
        region.extend(tr_data);

        let mut pi = (w as u32).to_be_bytes().to_vec();
        pi.extend((h as u32).to_be_bytes());
        pi.extend([0; 8]);
        pi.extend([0, 0, 0]);
        let mut out = seg(0, 48, 1, &pi);
        out.extend(seg(1, 0, 1, &sd));
        out.extend(seg_refs(2, 6, 1, &[1], &region));
        out
    }

    /// A one-page PDF whose only image is this JBIG2 stream (page = w x h points).
    pub fn pdf_with_stream(data: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut o = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_vec();
        o.extend_from_slice(format!("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n").as_bytes());
        o.extend_from_slice(format!("4 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode /Length {} >>\nstream\n", data.len()).as_bytes());
        o.extend_from_slice(data);
        let c = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q");
        o.extend_from_slice(format!("\nendstream\nendobj\n5 0 obj\n<< /Length {} >>\nstream\n{c}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n", c.len()).as_bytes());
        o
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::enc::*;
    use super::*;

    /// (page w, page h, glyphs, placements, expected page bitmap)
    pub(crate) type SymbolCase = (
        usize,
        usize,
        Vec<(usize, usize, Vec<u8>)>,
        Vec<(usize, i64, i64)>,
        Vec<u8>,
    );

    fn sample(w: usize, h: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        let mut bm = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                // text-like: horizontal bars + noise
                bm[y * w + x] =
                    ((y / 5) % 3 == 0 && x % 11 < 7 && x > 3) as u8 ^ s.is_multiple_of(29) as u8;
            }
        }
        bm
    }

    const DEFAULT_AT: [[(i32, i32); 4]; 4] = [
        [(3, -1), (-3, -1), (2, -2), (-2, -2)],
        [(3, -1), (0, 0), (0, 0), (0, 0)],
        [(2, -1), (0, 0), (0, 0), (0, 0)],
        [(2, -1), (0, 0), (0, 0), (0, 0)],
    ];

    #[test]
    fn arithmetic_roundtrip_all_templates() {
        for t in 0..4u8 {
            for tp in [false, true] {
                for (w, h) in [(1, 1), (7, 3), (64, 40), (130, 17)] {
                    let mut bm = sample(w, h, 77 + t as u64);
                    if tp && h > 6 {
                        let (a, b) = bm.split_at_mut(w * 3);
                        b[..w].copy_from_slice(&a[w * 2..w * 3]); // repeated line
                    }
                    let s = stream(&bm, w, h, t, tp, DEFAULT_AT[t as usize]);
                    assert_eq!(decode(&s, None, w, h).unwrap(), bm, "t{t} tp{tp} {w}x{h}");
                }
            }
        }
    }

    /// Random glyphs sorted by height, a grid of instances and the expected page bitmap.
    pub(crate) fn symbol_case(seed: u64) -> SymbolCase {
        let mut s = seed;
        let mut rnd = |n: u64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s % n
        };
        let mut syms: Vec<(usize, usize, Vec<u8>)> = (0..9)
            .map(|_| {
                let (w, h) = (3 + rnd(9) as usize, 3 + rnd(9) as usize);
                (w, h, (0..w * h).map(|_| (rnd(100) < 55) as u8).collect())
            })
            .collect();
        syms.sort_by_key(|g| g.1);
        let (pw, ph) = (150usize, 75usize);
        let mut placed = Vec::new();
        let mut page = vec![0u8; pw * ph];
        for row in 0..4 {
            for col in 0..8 {
                let id = rnd(syms.len() as u64) as usize;
                let (x, y) = (3 + col * 18 + rnd(4) as i64, 3 + row * 18 + rnd(4) as i64);
                placed.push((id, x, y));
                let g = &syms[id];
                for yy in 0..g.1 {
                    for xx in 0..g.0 {
                        if g.2[yy * g.0 + xx] == 1 {
                            page[(y as usize + yy) * pw + x as usize + xx] = 1;
                        }
                    }
                }
            }
        }
        (pw, ph, syms, placed, page)
    }

    #[test]
    fn symbol_dictionary_and_text_region_all_layouts() {
        let at0 = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        for corner in 0..4u8 {
            for transposed in [false, true] {
                for log_strips in [0u8, 2] {
                    for (t, dso) in [(0u8, 0i32), (2, 3), (1, -2)] {
                        let (w, h, syms, placed, want) =
                            symbol_case(17 + corner as u64 * 7 + log_strips as u64);
                        let mut at = at0;
                        if t == 1 {
                            at[0] = (3, -1);
                        } else if t >= 2 {
                            at[0] = (2, -1);
                        }
                        let cfg = TextCfg {
                            corner,
                            transposed,
                            log_strips,
                            comb: 0,
                            dso,
                        };
                        let data = symbol_stream(w, h, &syms, &placed, t, at, &cfg);
                        let got = decode(&data, None, w, h).unwrap_or_else(|e| {
                            panic!("c{corner} tr{transposed} s{log_strips} t{t}: {e}")
                        });
                        assert_eq!(
                            got, want,
                            "c{corner} tr{transposed} s{log_strips} t{t} dso{dso}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn custom_adaptive_pixels_and_blank_pages() {
        let (w, h) = (50, 30);
        let bm = sample(w, h, 5);
        let at = [(-5, -1), (4, -2), (-1, -3), (6, -1)];
        let s = stream(&bm, w, h, 0, false, at);
        assert_eq!(decode(&s, None, w, h).unwrap(), bm);
        let blank = vec![0u8; w * h];
        assert_eq!(
            decode(&stream(&blank, w, h, 0, true, DEFAULT_AT[0]), None, w, h).unwrap(),
            blank
        );
        let full = vec![1u8; w * h];
        assert_eq!(
            decode(&stream(&full, w, h, 2, false, DEFAULT_AT[2]), None, w, h).unwrap(),
            full
        );
    }

    #[test]
    fn damaged_symbol_streams_never_panic_or_hang() {
        let (w, h, syms, placed, _) = symbol_case(3);
        let at = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        let good = symbol_stream(
            w,
            h,
            &syms,
            &placed,
            0,
            at,
            &TextCfg {
                corner: 1,
                transposed: false,
                log_strips: 2,
                comb: 0,
                dso: 0,
            },
        );
        let mut s = 0xFEED_FACEu64;
        let t0 = std::time::Instant::now();
        for _ in 0..3000 {
            let mut d = good.clone();
            for _ in 0..1 + s % 4 {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let i = (s as usize) % d.len();
                d[i] = (s >> 24) as u8;
            }
            let _ = decode(&d, None, w, h);
        }
        assert!(t0.elapsed().as_secs() < 30, "too slow: {:?}", t0.elapsed());
    }

    #[test]
    fn unsupported_and_damaged_input_is_reported_not_panicked() {
        let e = |d: &[u8]| decode(d, None, 8, 8).err().map(|e| e.0);
        // symbol dictionary segment
        let mut v = 0u32.to_be_bytes().to_vec();
        v.extend([0, 0, 1, 0, 0, 0, 0]);
        assert!(e(&v).is_some());
        assert!(e(&[]).unwrap().contains("no JBIG2"));
        let bm = sample(40, 20, 3);
        let good = stream(&bm, 40, 20, 0, false, DEFAULT_AT[0]);
        let mut s = 0xDEAD_BEEFu64;
        for _ in 0..3000 {
            let mut d = good.clone();
            for _ in 0..1 + s % 4 {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let i = (s as usize) % d.len();
                d[i] = (s >> 20) as u8;
            }
            if s.is_multiple_of(7) {
                d.truncate((s as usize >> 9) % d.len());
            }
            let _ = decode(&d, None, 40, 20);
        }
        assert!(decode(&good, None, 0, 5).is_err());
    }
}
