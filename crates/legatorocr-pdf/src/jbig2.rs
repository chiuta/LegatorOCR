//! JBIG2 (ITU-T T.88) decoder for the *generic-region* subset used by many scanners and
//! "lossless" PDF producers: embedded streams made of a page-information segment and
//! immediate generic regions, arithmetic-coded (templates 0–3, adaptive pixels,
//! typical prediction) or MMR-coded. Symbol dictionaries, text, halftone and refinement
//! regions are not supported and are reported as such.
//!
//! Written from the standard's description of the MQ arithmetic coder and templates.

use crate::jbig2huff::{Bits, Table};
use std::fmt;
use std::rc::Rc;

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

pub(crate) struct Mq<'a> {
    d: &'a [u8],
    bp: usize,
    c: u32,
    a: u32,
    ct: i32,
}

impl<'a> Mq<'a> {
    pub(crate) fn new(d: &'a [u8]) -> Self {
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
    pub(crate) fn decode(&mut self, cx: &mut (u8, u8)) -> u8 {
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
    generic_full(mq, cx, w, h, t, tpgdon, at, None)
}

/// Generic region decoding with an optional skip mask (halftone regions).
#[allow(clippy::too_many_arguments)]
fn generic_full(
    mq: &mut Mq,
    cx: &mut [(u8, u8)],
    w: usize,
    h: usize,
    t: u8,
    tpgdon: bool,
    at: [(i32, i32); 4],
    skip: Option<&[u8]>,
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
            if skip.is_some_and(|k| k[y * w + x] != 0) {
                continue;
            }
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

/// Generic refinement context (6.3.5.3); the bit order is arbitrary but fixed, which is all
/// the adaptive coder needs. Returns the context and the position of the reference centre
/// pixel's bit (the typical-prediction context has only that bit set).
#[allow(clippy::too_many_arguments)]
pub(crate) fn refine_ctx(
    t: u8,
    at: &[(i32, i32); 2],
    cur: &[u8],
    w: usize,
    h: usize,
    rf: &Bitmap,
    dx: i32,
    dy: i32,
    x: usize,
    y: usize,
) -> usize {
    let c = |ox: i32, oy: i32| -> usize {
        let (px, py) = (x as i64 + ox as i64, y as i64 + oy as i64);
        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
            cur[py as usize * w + px as usize] as usize
        } else {
            0
        }
    };
    let r = |ox: i32, oy: i32| -> usize {
        let (px, py) = (
            x as i64 - dx as i64 + ox as i64,
            y as i64 - dy as i64 + oy as i64,
        );
        if px >= 0 && py >= 0 && (px as usize) < rf.w && (py as usize) < rf.h {
            rf.d[py as usize * rf.w + px as usize] as usize
        } else {
            0
        }
    };
    if t == 0 {
        c(at[0].0, at[0].1)
            | c(0, -1) << 1
            | c(1, -1) << 2
            | c(-1, 0) << 3
            | r(at[1].0, at[1].1) << 4
            | r(0, -1) << 5
            | r(1, -1) << 6
            | r(-1, 0) << 7
            | r(0, 0) << 8
            | r(1, 0) << 9
            | r(-1, 1) << 10
            | r(0, 1) << 11
            | r(1, 1) << 12
    } else {
        c(-1, -1)
            | c(0, -1) << 1
            | c(1, -1) << 2
            | c(-1, 0) << 3
            | r(0, -1) << 4
            | r(-1, 0) << 5
            | r(0, 0) << 6
            | r(1, 0) << 7
            | r(0, 1) << 8
            | r(1, 1) << 9
    }
}

pub(crate) fn refine_sltp(t: u8) -> usize {
    if t == 0 {
        1 << 8
    } else {
        1 << 6
    }
}

/// The reference value when its 3×3 neighbourhood around (x-dx, y-dy) is uniform.
#[allow(clippy::too_many_arguments)]
pub(crate) fn refine_uniform(rf: &Bitmap, dx: i32, dy: i32, x: usize, y: usize) -> Option<u8> {
    let mut v = None;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let (px, py) = (
                x as i64 - dx as i64 + ox as i64,
                y as i64 - dy as i64 + oy as i64,
            );
            let b = if px >= 0 && py >= 0 && (px as usize) < rf.w && (py as usize) < rf.h {
                rf.d[py as usize * rf.w + px as usize]
            } else {
                0
            };
            match v {
                None => v = Some(b),
                Some(o) if o != b => return None,
                _ => {}
            }
        }
    }
    v
}

/// Generic refinement region decoding (6.3).
#[allow(clippy::too_many_arguments)]
fn refine(
    mq: &mut Mq,
    cx: &mut [(u8, u8)],
    w: usize,
    h: usize,
    rf: &Bitmap,
    dx: i32,
    dy: i32,
    t: u8,
    tpgron: bool,
    at: [(i32, i32); 2],
) -> Vec<u8> {
    let mut bm = vec![0u8; w * h];
    let mut ltp = false;
    for y in 0..h {
        if tpgron {
            ltp ^= mq.decode(&mut cx[refine_sltp(t)]) == 1;
        }
        for x in 0..w {
            if ltp {
                if let Some(v) = refine_uniform(rf, dx, dy, x, y) {
                    bm[y * w + x] = v;
                    continue;
                }
            }
            let c = refine_ctx(t, &at, &bm, w, h, rf, dx, dy, x, y);
            bm[y * w + x] = mq.decode(&mut cx[c]);
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
pub(crate) struct Bitmap {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) d: Vec<u8>,
}

fn be16(b: &[u8], at: usize) -> R<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or_else(|| Jbig2Error("truncated JBIG2 data".into()))
}

struct RefCtx {
    gr: Vec<(u8, u8)>,
    iari: IntCtx,
    iardw: IntCtx,
    iardh: IntCtx,
    iardx: IntCtx,
    iardy: IntCtx,
}
impl RefCtx {
    fn new() -> Self {
        RefCtx {
            gr: vec![(0, 0); 1 << 13],
            iari: IntCtx::new(),
            iardw: IntCtx::new(),
            iardh: IntCtx::new(),
            iardx: IntCtx::new(),
            iardy: IntCtx::new(),
        }
    }
}

struct TextCtx {
    iadt: IntCtx,
    iafs: IntCtx,
    iads: IntCtx,
    iait: IntCtx,
    iaid: Vec<(u8, u8)>,
    rc: RefCtx,
}
impl TextCtx {
    fn new(code_len: u32) -> Self {
        TextCtx {
            iadt: IntCtx::new(),
            iafs: IntCtx::new(),
            iads: IntCtx::new(),
            iait: IntCtx::new(),
            iaid: vec![(0u8, 0u8); 1usize << (code_len + 1)],
            rc: RefCtx::new(),
        }
    }
}

struct TextP {
    rw: usize,
    rh: usize,
    strips: i64,
    corner: u32,
    transposed: bool,
    comb: u8,
    def_pixel: u8,
    dso: i32,
    n_inst: usize,
    refine: bool,
    rt: u8,
    rat: [(i32, i32); 2],
    code_len: u32,
}

fn code_len_for(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

fn int_or_fail(v: Option<i32>) -> R<i64> {
    v.map(|v| v as i64)
        .ok_or_else(|| Jbig2Error("bad JBIG2 coded integer".into()))
}

/// Symbol dictionary (arithmetic coding, with refinement/aggregation) → exported symbols.
fn symbol_dict(data: &[u8], inputs: &[Bitmap]) -> R<Vec<Bitmap>> {
    let flags = be16(data, 0)?;
    if flags & 1 != 0 {
        return fail("JBIG2 Huffman-coded symbol dictionaries are not supported");
    }
    let refagg = flags & 2 != 0;
    if flags & 0x100 != 0 {
        return fail("JBIG2 symbol dictionaries with retained contexts are not supported");
    }
    let t = ((flags >> 10) & 3) as u8;
    let rt = ((flags >> 12) & 1) as u8;
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
    let mut rat = [(-1i32, -1i32); 2];
    if refagg && rt == 0 {
        let raw = data
            .get(p..p + 4)
            .ok_or_else(|| Jbig2Error("truncated JBIG2 symbol dictionary".into()))?;
        rat = [
            (raw[0] as i8 as i32, raw[1] as i8 as i32),
            (raw[2] as i8 as i32, raw[3] as i8 as i32),
        ];
        p += 4;
    }
    let n_ex = be32(data, p)? as usize;
    let n_new = be32(data, p + 4)? as usize;
    p += 8;
    if n_new > 65_536 || n_ex > 131_072 + inputs.len() {
        return fail("JBIG2 symbol dictionary too large");
    }
    let mut mq = Mq::new(&data[p.min(data.len())..]);
    let mut gb = vec![(0u8, 0u8); 1 << template(t, &at).len()];
    let (mut iadh, mut iadw, mut iaex, mut iaai) =
        (IntCtx::new(), IntCtx::new(), IntCtx::new(), IntCtx::new());
    let code_len = code_len_for(inputs.len() + n_new);
    let mut tc = TextCtx::new(if refagg { code_len } else { 0 });
    // inputs followed by the symbols decoded so far
    let mut pool: Vec<Bitmap> = inputs.to_vec();
    let mut budget = 0u64;
    let mut hc_h: i64 = 0;
    let mut idle_rounds = 0u32;
    while pool.len() - inputs.len() < n_new {
        // a height class that adds no symbol, many times over, means the data ran out
        let before = pool.len();
        idle_rounds += 1;
        if idle_rounds > 100_000 {
            return fail("JBIG2 symbol dictionary makes no progress");
        }
        let dh = int_or_fail(iadh.decode(&mut mq))?;
        hc_h += dh;
        if !(0..=65_535).contains(&hc_h) {
            return fail("bad JBIG2 symbol height");
        }
        let mut sym_w: i64 = 0;
        while let Some(dw) = iadw.decode(&mut mq) {
            if pool.len() - inputs.len() >= n_new {
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
            } else if !refagg {
                generic_with(&mut mq, &mut gb, w, h, t, false, at)
            } else {
                let ninst = int_or_fail(iaai.decode(&mut mq))?;
                if ninst == 1 {
                    let id = decode_iaid(&mut mq, &mut tc.iaid, code_len);
                    let rdx = int_or_fail(tc.rc.iardx.decode(&mut mq))? as i32;
                    let rdy = int_or_fail(tc.rc.iardy.decode(&mut mq))? as i32;
                    let rf = pool
                        .get(id)
                        .ok_or_else(|| Jbig2Error("JBIG2 symbol id out of range".into()))?
                        .clone();
                    refine(&mut mq, &mut tc.rc.gr, w, h, &rf, rdx, rdy, rt, false, rat)
                } else if ninst > 1 && ninst < 100_000 {
                    let tp = TextP {
                        rw: w,
                        rh: h,
                        strips: 1,
                        corner: 1,
                        transposed: false,
                        comb: 0,
                        def_pixel: 0,
                        dso: 0,
                        n_inst: ninst as usize,
                        refine: true,
                        rt,
                        rat,
                        code_len,
                    };
                    text_decode(&mut mq, &mut tc, &tp, &pool)?
                } else {
                    return fail("bad JBIG2 aggregate instance count");
                }
            };
            pool.push(Bitmap { w, h, d });
        }
        if pool.len() > before {
            idle_rounds = 0;
        }
    }
    let total = pool.len();
    let mut out = Vec::new();
    let (mut idx, mut flag) = (0usize, false);
    let mut guard = 0;
    while idx < total && guard < total + 8 {
        guard += 1;
        let run = int_or_fail(iaex.decode(&mut mq))?;
        if run < 0 || idx + run as usize > total {
            return fail("bad JBIG2 export run");
        }
        if flag {
            out.extend(pool[idx..idx + run as usize].iter().cloned());
        }
        idx += run as usize;
        flag = !flag;
    }
    if out.len() != n_ex {
        return fail("JBIG2 symbol dictionary exports a different number of symbols");
    }
    Ok(out)
}

fn herr(e: crate::jbig2huff::HuffError) -> Jbig2Error {
    Jbig2Error(e.to_string())
}

/// Pick a Huffman table by its selection value: the standard ones listed, or the next
/// user-supplied table (selection 3 / 1 for the single-bit fields).
fn pick(
    sel: u16,
    std: &[(u16, u8)],
    custom: u16,
    ct: &[Rc<Table>],
    used: &mut usize,
) -> R<Rc<Table>> {
    if let Some(&(_, n)) = std.iter().find(|(v, _)| *v == sel) {
        return Ok(Rc::new(Table::standard(n)));
    }
    if sel != custom {
        return fail("JBIG2 uses a reserved Huffman table selection");
    }
    let t = ct
        .get(*used)
        .ok_or_else(|| Jbig2Error("JBIG2 refers to a Huffman table that is missing".into()))?;
    *used += 1;
    Ok(Rc::clone(t))
}

/// Symbol dictionary, Huffman coding without refinement or aggregation (T.88 6.5, 7.4.2).
fn symbol_dict_huff(data: &[u8], inputs: &[Bitmap], ct: &[Rc<Table>]) -> R<Vec<Bitmap>> {
    let flags = be16(data, 0)?;
    if flags & 2 != 0 {
        return fail(
            "JBIG2 Huffman symbol dictionaries with refinement or aggregation are not supported",
        );
    }
    if flags & 0x100 != 0 {
        return fail("JBIG2 symbol dictionaries with retained contexts are not supported");
    }
    let mut used = 0usize;
    let dh_t = pick((flags >> 2) & 3, &[(0, 4), (1, 5)], 3, ct, &mut used)?;
    let dw_t = pick((flags >> 4) & 3, &[(0, 2), (1, 3)], 3, ct, &mut used)?;
    let bm_t = pick((flags >> 6) & 1, &[(0, 1)], 1, ct, &mut used)?;
    let _agg = pick((flags >> 7) & 1, &[(0, 1)], 1, ct, &mut used)?;
    // no adaptive-template bytes with Huffman coding
    let n_ex = be32(data, 2)? as usize;
    let n_new = be32(data, 6)? as usize;
    if n_new > 65_536 || n_ex > 131_072 + inputs.len() {
        return fail("JBIG2 symbol dictionary too large");
    }
    let mut r = Bits::new(&data[10.min(data.len())..]);
    let mut pool: Vec<Bitmap> = inputs.to_vec();
    let mut hc_h: i64 = 0;
    let mut budget = 0u64;
    let mut idle_rounds = 0u32;
    while pool.len() - inputs.len() < n_new {
        idle_rounds += 1;
        if idle_rounds > 100_000 {
            return fail("JBIG2 symbol dictionary makes no progress");
        }
        hc_h += dh_t.int(&mut r).map_err(herr)?;
        if !(0..=65_535).contains(&hc_h) {
            return fail("bad JBIG2 symbol height");
        }
        let (mut sym_w, mut tot_w) = (0i64, 0i64);
        let mut widths: Vec<usize> = Vec::new();
        while let Some(dw) = dw_t.decode(&mut r).map_err(herr)? {
            if pool.len() - inputs.len() + widths.len() >= n_new {
                return fail("JBIG2 symbol dictionary overrun");
            }
            sym_w += dw;
            if !(0..=65_535).contains(&sym_w) {
                return fail("bad JBIG2 symbol width");
            }
            tot_w += sym_w;
            widths.push(sym_w as usize);
        }
        let bmsize = bm_t.int(&mut r).map_err(herr)?;
        r.align();
        let (tw, h) = (tot_w as usize, hc_h as usize);
        budget += tw as u64 * h as u64;
        if tw as u64 * h as u64 > 16_000_000 || budget > 200_000_000 {
            return fail("JBIG2 symbols are too large");
        }
        let rows: Vec<Vec<bool>> = if tw == 0 || h == 0 {
            // empty symbols: nothing to build, but any bitmap bytes are still skipped
            if bmsize > 0 {
                r.skip_bytes(usize::try_from(bmsize).unwrap_or(usize::MAX))
                    .map_err(herr)?;
            }
            Vec::new()
        } else if bmsize == 0 {
            // uncompressed: rows padded to whole bytes
            let stride = tw.div_ceil(8);
            let mut rows = Vec::with_capacity(h);
            for _ in 0..h {
                let mut row = Vec::with_capacity(tw);
                for _ in 0..tw {
                    row.push(r.bit().map_err(herr)? != 0);
                }
                for _ in tw..stride * 8 {
                    r.bit().map_err(herr)?;
                }
                rows.push(row);
            }
            rows
        } else {
            let n =
                usize::try_from(bmsize).map_err(|_| Jbig2Error("bad JBIG2 bitmap size".into()))?;
            let body = r
                .rest()
                .get(..n)
                .ok_or_else(|| Jbig2Error("truncated JBIG2 bitmap".into()))?;
            let (rows, _) = crate::g4::decode(body, tw, h, false);
            r.skip_bytes(n).map_err(herr)?;
            rows
        };
        let mut x0 = 0usize;
        for &wd in &widths {
            let mut d = Vec::with_capacity(wd * h);
            if wd > 0 {
                for row in rows.iter().take(h) {
                    for x in x0..x0 + wd {
                        d.push(u8::from(row.get(x).copied().unwrap_or(false)));
                    }
                }
            }
            x0 += wd;
            pool.push(Bitmap {
                w: wd,
                h,
                d: if wd == 0 { Vec::new() } else { d },
            });
        }
    }
    // export flags: alternating run lengths coded with table B.1
    let run_t = Table::standard(1);
    let total = pool.len();
    let mut out = Vec::new();
    let (mut idx, mut flag, mut guard) = (0usize, false, 0usize);
    while idx < total && guard < total + 8 {
        guard += 1;
        let run = run_t.int(&mut r).map_err(herr)?;
        if run < 0 || idx + run as usize > total {
            return fail("bad JBIG2 export run");
        }
        if flag {
            out.extend(pool[idx..idx + run as usize].iter().cloned());
        }
        idx += run as usize;
        flag = !flag;
    }
    Ok(out)
}

/// Text region, Huffman coding without refinement (T.88 6.4, 7.4.3).
fn text_region_huff(
    data: &[u8],
    rw: usize,
    rh: usize,
    syms: &[Bitmap],
    ct: &[Rc<Table>],
) -> R<Vec<u8>> {
    let flags = be16(data, 0)?;
    if flags & 2 != 0 {
        return fail("JBIG2 Huffman text regions with refinement are not supported");
    }
    let hf = be16(data, 2)?;
    let mut dso = ((flags >> 10) & 31) as i32;
    if dso > 15 {
        dso -= 32;
    }
    let n_inst = be32(data, 4)? as usize;
    if n_inst > 4_000_000 {
        return fail("too many JBIG2 symbol instances");
    }
    let mut used = 0usize;
    let fs_t = pick(hf & 3, &[(0, 6), (1, 7)], 3, ct, &mut used)?;
    let ds_t = pick((hf >> 2) & 3, &[(0, 8), (1, 9), (2, 10)], 3, ct, &mut used)?;
    let dt_t = pick(
        (hf >> 4) & 3,
        &[(0, 11), (1, 12), (2, 13)],
        3,
        ct,
        &mut used,
    )?;
    let tp = TextP {
        rw,
        rh,
        strips: 1i64 << ((flags >> 2) & 3),
        corner: ((flags >> 4) & 3) as u32,
        transposed: flags & 0x40 != 0,
        comb: ((flags >> 7) & 3) as u8,
        def_pixel: ((flags >> 9) & 1) as u8,
        dso,
        n_inst,
        refine: false,
        rt: 0,
        rat: [(-1, -1); 2],
        code_len: 0,
    };
    let log_strips = (flags >> 2) & 3;
    let mut r = Bits::new(&data[8.min(data.len())..]);
    // symbol-ID codes: 35 run-code lengths, then one code length per symbol (7.4.3.1.7)
    let mut runlens = [0u8; 35];
    for l in runlens.iter_mut() {
        *l = r.bits(4).map_err(herr)? as u8;
    }
    let run_t = Table::from_lengths(&runlens).map_err(herr)?;
    let mut lens: Vec<u8> = Vec::with_capacity(syms.len());
    while lens.len() < syms.len() {
        let c = run_t.int(&mut r).map_err(herr)?;
        let (val, n) = match c {
            0..=31 => (c as u8, 1usize),
            32 => (
                *lens
                    .last()
                    .ok_or_else(|| Jbig2Error("JBIG2 symbol code repeat without a code".into()))?,
                3 + r.bits(2).map_err(herr)? as usize,
            ),
            33 => (0, 3 + r.bits(3).map_err(herr)? as usize),
            34 => (0, 11 + r.bits(7).map_err(herr)? as usize),
            _ => return fail("bad JBIG2 symbol code length"),
        };
        if lens.len() + n > syms.len() {
            return fail("JBIG2 symbol code lengths overrun");
        }
        lens.extend(std::iter::repeat_n(val, n));
    }
    r.align();
    let id_t = Table::from_lengths(&lens).map_err(herr)?;

    let strips = tp.strips;
    let mut bm = vec![tp.def_pixel; rw * rh];
    let mut stript: i64 = -dt_t.int(&mut r).map_err(herr)? * strips;
    let (mut firsts, mut done, mut drawn) = (0i64, 0usize, 0u64);
    while done < n_inst {
        stript += dt_t.int(&mut r).map_err(herr)? * strips;
        let mut curs = 0i64;
        let mut first = true;
        loop {
            if first {
                firsts += fs_t.int(&mut r).map_err(herr)?;
                curs = firsts;
                first = false;
            } else {
                match ds_t.decode(&mut r).map_err(herr)? {
                    None => break,
                    Some(v) => curs += v + dso as i64,
                }
            }
            if done >= n_inst {
                return fail("JBIG2 text region overrun");
            }
            let curt = if strips == 1 {
                0
            } else {
                r.bits(log_strips as u32).map_err(herr)? as i64
            };
            let id = id_t.int(&mut r).map_err(herr)? as usize;
            let sym = syms
                .get(id)
                .ok_or_else(|| Jbig2Error("JBIG2 symbol id out of range".into()))?;
            place_instance(
                &mut bm,
                (rw, rh),
                &tp,
                sym,
                &mut curs,
                stript + curt,
                &mut drawn,
            )?;
            done += 1;
        }
    }
    Ok(bm)
}

/// Text region (arithmetic coding) → region bitmap.
fn text_region(data: &[u8], rw: usize, rh: usize, syms: &[Bitmap]) -> R<Vec<u8>> {
    let flags = be16(data, 0)?;
    if flags & 1 != 0 {
        return fail("JBIG2 Huffman-coded text regions are not supported");
    }
    let refine = flags & 2 != 0;
    let rt = ((flags >> 15) & 1) as u8;
    let mut p = 2;
    let mut rat = [(-1i32, -1i32); 2];
    if refine && rt == 0 {
        let raw = data
            .get(p..p + 4)
            .ok_or_else(|| Jbig2Error("truncated JBIG2 text region".into()))?;
        rat = [
            (raw[0] as i8 as i32, raw[1] as i8 as i32),
            (raw[2] as i8 as i32, raw[3] as i8 as i32),
        ];
        p += 4;
    }
    let mut dso = ((flags >> 10) & 31) as i32;
    if dso > 15 {
        dso -= 32;
    }
    let n_inst = be32(data, p)? as usize;
    p += 4;
    if n_inst > 4_000_000 {
        return fail("too many JBIG2 symbol instances");
    }
    let code_len = code_len_for(syms.len());
    let tp = TextP {
        rw,
        rh,
        strips: 1i64 << ((flags >> 2) & 3),
        corner: ((flags >> 4) & 3) as u32,
        transposed: flags & 0x40 != 0,
        comb: ((flags >> 7) & 3) as u8,
        def_pixel: ((flags >> 9) & 1) as u8,
        dso,
        n_inst,
        refine,
        rt,
        rat,
        code_len,
    };
    let mut mq = Mq::new(&data[p.min(data.len())..]);
    let mut tc = TextCtx::new(code_len);
    text_decode(&mut mq, &mut tc, &tp, syms)
}

/// Draw one symbol instance into the region bitmap, advancing the strip position `curs` the way
/// the reference corner and orientation require (T.88 6.4.5, steps 3c vi to x).
fn place_instance(
    bm: &mut [u8],
    (rw, rh): (usize, usize),
    tp: &TextP,
    sym: &Bitmap,
    curs: &mut i64,
    ti: i64,
    drawn: &mut u64,
) -> R<()> {
    let (transposed, corner) = (tp.transposed, tp.corner);
    let (wi, hi) = (sym.w as i64, sym.h as i64);
    if !transposed && corner >= 2 {
        *curs += wi - 1;
    } else if transposed && (corner == 0 || corner == 2) {
        *curs += hi - 1;
    }
    let si = *curs;
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
    *drawn += (sym.w * sym.h) as u64 + 1;
    if *drawn > 1_000_000_000 {
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
            *dst = match tp.comb {
                0 => *dst | src,
                1 => *dst & src,
                2 => *dst ^ src,
                _ => 1 - (*dst ^ src),
            };
        }
    }
    if !transposed && corner < 2 {
        *curs += wi - 1;
    } else if transposed && (corner == 1 || corner == 3) {
        *curs += hi - 1;
    }
    Ok(())
}

fn text_decode(mq: &mut Mq, tc: &mut TextCtx, tp: &TextP, syms: &[Bitmap]) -> R<Vec<u8>> {
    let (rw, rh, strips) = (tp.rw, tp.rh, tp.strips);
    let mut bm = vec![tp.def_pixel; rw * rh];
    let mut stript: i64 = -int_or_fail(tc.iadt.decode(mq))? * strips;
    let mut firsts: i64 = 0;
    let mut done = 0usize;
    let mut drawn = 0u64;
    while done < tp.n_inst {
        let dt = int_or_fail(tc.iadt.decode(mq))?;
        stript += dt * strips;
        let mut curs: i64 = 0;
        let mut first = true;
        loop {
            if first {
                firsts += int_or_fail(tc.iafs.decode(mq))?;
                curs = firsts;
                first = false;
            } else {
                match tc.iads.decode(mq) {
                    None => break,
                    Some(ids) => curs += ids as i64 + tp.dso as i64,
                }
            }
            if done >= tp.n_inst {
                return fail("JBIG2 text region overrun");
            }
            let curt = if strips == 1 {
                0
            } else {
                int_or_fail(tc.iait.decode(mq))?
            };
            let ti = stript + curt;
            let id = decode_iaid(mq, &mut tc.iaid, tp.code_len);
            let base = syms
                .get(id)
                .ok_or_else(|| Jbig2Error("JBIG2 symbol id out of range".into()))?;
            let mut refined: Option<Bitmap> = None;
            if tp.refine && int_or_fail(tc.rc.iari.decode(mq))? != 0 {
                let rdw = int_or_fail(tc.rc.iardw.decode(mq))?;
                let rdh = int_or_fail(tc.rc.iardh.decode(mq))?;
                let rdx = int_or_fail(tc.rc.iardx.decode(mq))?;
                let rdy = int_or_fail(tc.rc.iardy.decode(mq))?;
                let (nw, nh) = (base.w as i64 + rdw, base.h as i64 + rdh);
                if !(0..=65_535).contains(&nw)
                    || !(0..=65_535).contains(&nh)
                    || nw * nh > 16_000_000
                {
                    return fail("bad JBIG2 refined symbol size");
                }
                let (nw, nh) = (nw as usize, nh as usize);
                let d = if nw == 0 || nh == 0 {
                    Vec::new()
                } else {
                    refine(
                        mq,
                        &mut tc.rc.gr,
                        nw,
                        nh,
                        base,
                        (rdw.div_euclid(2) + rdx) as i32,
                        (rdh.div_euclid(2) + rdy) as i32,
                        tp.rt,
                        false,
                        tp.rat,
                    )
                };
                refined = Some(Bitmap { w: nw, h: nh, d });
            }
            let sym = refined.as_ref().unwrap_or(base);
            place_instance(&mut bm, (rw, rh), tp, sym, &mut curs, ti, &mut drawn)?;
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

/// Immediate regions are drawn on the page; intermediate ones are kept for later reference.
#[allow(clippy::too_many_arguments)]
fn place(
    page: &mut [u8],
    inter: &mut std::collections::HashMap<u32, Bitmap>,
    s: &Seg,
    w: usize,
    h: usize,
    bm: Vec<u8>,
    rw: usize,
    rh: usize,
    rx: usize,
    ry: usize,
    op: u8,
) {
    if matches!(s.kind, 4 | 20 | 36 | 40) {
        inter.insert(
            s.num,
            Bitmap {
                w: rw,
                h: rh,
                d: bm,
            },
        );
    } else {
        blit(page, w, h, &bm, rw, rh, rx, ry, op);
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
    let mut inter: std::collections::HashMap<u32, Bitmap> = std::collections::HashMap::new();
    // Total area of all regions of this stream: a few hundred bytes of arithmetic-coded data can
    // describe a region of 400 million pixels, and 10 000 segments are allowed.
    let mut region_px: u64 = 0;
    let mut dict_px: u64 = 0;
    // user-supplied Huffman tables (segment type 53)
    let mut tables: std::collections::HashMap<u32, Rc<Table>> = std::collections::HashMap::new();
    for s in &segs {
        let data: &[u8] = &s.data;
        if matches!(
            s.kind,
            4 | 6 | 7 | 20 | 22 | 23 | 36 | 38 | 39 | 40 | 42 | 43
        ) && data.len() >= 8
        {
            region_px = region_px.saturating_add(
                be32(data, 0).unwrap_or(0) as u64 * be32(data, 4).unwrap_or(0) as u64,
            );
            if region_px > 1_500_000_000 {
                return fail("JBIG2 stream describes too much image data");
            }
        }
        // custom tables matter to dictionaries and text regions only, each once, a few at most
        let mut ct: Vec<Rc<Table>> = Vec::new();
        if matches!(s.kind, 0 | 4 | 6 | 7) {
            for r in &s.refs {
                if let Some(t) = tables.get(r) {
                    if ct.len() < 16 {
                        ct.push(Rc::clone(t));
                    }
                }
            }
        }
        match s.kind {
            53 => {
                let t = Table::parse_custom(data).map_err(|e| Jbig2Error(e.to_string()))?;
                if tables.len() < 256 {
                    tables.insert(s.num, Rc::new(t));
                }
            }
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
                place(&mut page, &mut inter, s, w, h, bm, rw, rh, rx, ry, op);
                regions += 1;
            }
            0 => {
                let mut inputs: Vec<Bitmap> = Vec::new();
                for r in &s.refs {
                    if let Some(d) = dicts.get(r) {
                        if inputs.len() + d.len() > 131_072 {
                            return fail("JBIG2 symbol dictionary too large");
                        }
                        inputs.extend(d.iter().cloned());
                    }
                }
                if inputs.len() > 131_072 {
                    return fail("JBIG2 symbol dictionary too large");
                }
                // dictionaries that re-export their inputs copy every symbol: bound the total
                // pixel count of everything exported by this stream
                let exported = if be16(data, 0)? & 1 != 0 {
                    symbol_dict_huff(data, &inputs, &ct)?
                } else {
                    symbol_dict(data, &inputs)?
                };
                dict_px = exported
                    .iter()
                    .fold(dict_px, |a, b| a.saturating_add(b.w as u64 * b.h as u64));
                if dict_px > 600_000_000 {
                    return fail("JBIG2 symbol dictionaries hold too much data");
                }
                dicts.insert(s.num, exported);
            }
            4 | 6 | 7 => {
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
                        if syms.len() + d.len() > 131_072 {
                            return fail("JBIG2 too many symbols");
                        }
                        syms.extend(d.iter().cloned());
                    }
                }
                let bm = if be16(&data[17..], 0)? & 1 != 0 {
                    text_region_huff(&data[17..], rw, rh, &syms, &ct)?
                } else {
                    text_region(&data[17..], rw, rh, &syms)?
                };
                place(&mut page, &mut inter, s, w, h, bm, rw, rh, rx, ry, op);
                regions += 1;
            }
            16 => {
                let flags = *data
                    .first()
                    .ok_or_else(|| Jbig2Error("truncated JBIG2 pattern dictionary".into()))?;
                let t = (flags >> 1) & 3;
                let pw = *data
                    .get(1)
                    .ok_or_else(|| Jbig2Error("truncated JBIG2 pattern dictionary".into()))?
                    as usize;
                let ph = *data
                    .get(2)
                    .ok_or_else(|| Jbig2Error("truncated JBIG2 pattern dictionary".into()))?
                    as usize;
                let n = be32(data, 3)? as usize + 1;
                if pw == 0 || ph == 0 || n > 65_536 || n * pw * ph > 64_000_000 {
                    return fail("bad JBIG2 pattern dictionary");
                }
                let cw = n * pw;
                let body = data.get(7..).unwrap_or(&[]);
                let coll: Vec<u8> = if flags & 1 != 0 {
                    let (rows, _) = crate::g4::decode(body, cw, ph, false);
                    let mut v = vec![0u8; cw * ph];
                    for (y, r) in rows.iter().enumerate().take(ph) {
                        for (x, &b) in r.iter().enumerate().take(cw) {
                            v[y * cw + x] = b as u8;
                        }
                    }
                    v
                } else {
                    let at = [(-(pw as i32), 0), (-3, -1), (2, -2), (-2, -2)];
                    generic_arith(body, cw, ph, t, false, at)
                };
                let mut pats = Vec::with_capacity(n);
                for k in 0..n {
                    let mut d = Vec::with_capacity(pw * ph);
                    for y in 0..ph {
                        d.extend_from_slice(&coll[y * cw + k * pw..y * cw + (k + 1) * pw]);
                    }
                    pats.push(Bitmap { w: pw, h: ph, d });
                }
                dicts.insert(s.num, pats);
            }
            20 | 22 | 23 => {
                if data.len() < 17 + 1 + 16 + 4 {
                    return fail("truncated JBIG2 halftone region");
                }
                let (rw, rh) = (be32(data, 0)? as usize, be32(data, 4)? as usize);
                let (rx, ry) = (be32(data, 8)? as usize, be32(data, 12)? as usize);
                let op = data[16] & 7;
                if rw == 0 || rh == 0 || rw > w || rh > h {
                    return fail("invalid JBIG2 region size");
                }
                let f = data[17];
                let (mmr, t, enable_skip, comb, defpix) = (
                    f & 1 != 0,
                    (f >> 1) & 3,
                    f & 8 != 0,
                    (f >> 4) & 7,
                    (f >> 7) & 1,
                );
                let (gw, gh) = (be32(data, 18)? as usize, be32(data, 22)? as usize);
                let gx = be32(data, 26)? as i32 as i64;
                let gy = be32(data, 30)? as i32 as i64;
                let rxs = be16(data, 34)? as i64;
                let rys = be16(data, 36)? as i64;
                if gw == 0 || gh == 0 || gw.checked_mul(gh).is_none_or(|n| n > 64_000_000) {
                    return fail("bad JBIG2 halftone grid");
                }
                let pats = s.refs.iter().find_map(|r| dicts.get(r)).ok_or_else(|| {
                    Jbig2Error("JBIG2 halftone without a pattern dictionary".into())
                })?;
                let np = pats.len();
                if np == 0 || pats[0].w == 0 || pats[0].h == 0 {
                    return fail("JBIG2 halftone with an empty pattern dictionary");
                }
                let mut bpp = 0;
                while (1usize << bpp) < np {
                    bpp += 1;
                }
                let (pw, ph) = (pats[0].w as i64, pats[0].h as i64);
                if (gw as u64)
                    .saturating_mul(gh as u64)
                    .saturating_mul(pw as u64 * ph as u64)
                    > 4_000_000_000
                {
                    return fail("JBIG2 halftone is too expensive to decode");
                }
                let pos = |m: usize, n: usize| -> (i64, i64) {
                    (
                        (gx + m as i64 * rys + n as i64 * rxs) >> 8,
                        (gy + m as i64 * rxs - n as i64 * rys) >> 8,
                    )
                };
                let skip: Option<Vec<u8>> = enable_skip.then(|| {
                    let mut k = vec![0u8; gw * gh];
                    for m in 0..gh {
                        for n in 0..gw {
                            let (x, y) = pos(m, n);
                            if x + pw <= 0 || x >= rw as i64 || y + ph <= 0 || y >= rh as i64 {
                                k[m * gw + n] = 1;
                            }
                        }
                    }
                    k
                });
                let body = &data[38..];
                let mut gray = vec![0u32; gw * gh];
                if bpp > 0 {
                    let mut planes: Vec<Vec<u8>> = Vec::new();
                    if mmr {
                        if bpp > 1 {
                            return fail("multi-plane MMR JBIG2 halftones are not supported");
                        }
                        let (rows, _) = crate::g4::decode(body, gw, gh, false);
                        let mut v = vec![0u8; gw * gh];
                        for (y, r) in rows.iter().enumerate().take(gh) {
                            for (x, &b) in r.iter().enumerate().take(gw) {
                                v[y * gw + x] = b as u8;
                            }
                        }
                        planes.push(v);
                    } else {
                        let at = [
                            (if t <= 1 { 3 } else { 2 }, -1),
                            (-3, -1),
                            (2, -2),
                            (-2, -2),
                        ];
                        let mut mq = Mq::new(body);
                        let mut cx = vec![(0u8, 0u8); 1 << template(t, &at).len()];
                        for _ in 0..bpp {
                            planes.push(generic_full(
                                &mut mq,
                                &mut cx,
                                gw,
                                gh,
                                t,
                                false,
                                at,
                                skip.as_deref(),
                            ));
                        }
                    }
                    // planes[0] is the most significant bit-plane; Gray-code decoding
                    for j in 1..planes.len() {
                        let (a, b) = planes.split_at_mut(j);
                        for (x, y) in b[0].iter_mut().zip(a[j - 1].iter()) {
                            *x ^= *y;
                        }
                    }
                    for pl in &planes {
                        for (g, &b) in gray.iter_mut().zip(pl) {
                            *g = (*g << 1) | b as u32;
                        }
                    }
                }
                let mut bm = vec![defpix; rw * rh];
                for m in 0..gh {
                    for n in 0..gw {
                        let (x, y) = pos(m, n);
                        if x + pw <= 0 || y + ph <= 0 || x >= rw as i64 || y >= rh as i64 {
                            continue; // the pattern lands entirely outside the region
                        }
                        let pat = &pats[(gray[m * gw + n] as usize).min(np - 1)];
                        for yy in 0..pat.h {
                            for xx in 0..pat.w {
                                let (px, py) = (x + xx as i64, y + yy as i64);
                                if px < 0 || py < 0 || px >= rw as i64 || py >= rh as i64 {
                                    continue;
                                }
                                let dst = &mut bm[py as usize * rw + px as usize];
                                let src = pat.d[yy * pat.w + xx];
                                *dst = match comb {
                                    0 => *dst | src,
                                    1 => *dst & src,
                                    2 => *dst ^ src,
                                    3 => 1 - (*dst ^ src),
                                    _ => src,
                                };
                            }
                        }
                    }
                }
                place(&mut page, &mut inter, s, w, h, bm, rw, rh, rx, ry, op);
                regions += 1;
            }
            40 | 42 | 43 => {
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
                let (rt, tpgron) = (f & 1, f & 2 != 0);
                let mut p = 18;
                let mut at = [(-1i32, -1i32); 2];
                if rt == 0 {
                    let raw = data
                        .get(p..p + 4)
                        .ok_or_else(|| Jbig2Error("truncated JBIG2 region".into()))?;
                    at = [
                        (raw[0] as i8 as i32, raw[1] as i8 as i32),
                        (raw[2] as i8 as i32, raw[3] as i8 as i32),
                    ];
                    p += 4;
                }
                let rf = match s.refs.first().and_then(|r| inter.get(r)) {
                    Some(b) if b.w == rw && b.h == rh => b.clone(),
                    Some(_) => return fail("JBIG2 refinement reference has a different size"),
                    None => {
                        let mut d = vec![0u8; rw * rh];
                        for yy in 0..rh {
                            for xx in 0..rw {
                                if rx + xx < w && ry + yy < h {
                                    d[yy * rw + xx] = page[(ry + yy) * w + rx + xx];
                                }
                            }
                        }
                        Bitmap { w: rw, h: rh, d }
                    }
                };
                let mut mq = Mq::new(&data[p..]);
                let mut cx = vec![(0u8, 0u8); 1 << 13];
                let bm = refine(&mut mq, &mut cx, rw, rh, &rf, 0, 0, rt, tpgron, at);
                place(&mut page, &mut inter, s, w, h, bm, rw, rh, rx, ry, op);
                regions += 1;
            }
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

    // ---- refinement, aggregate symbols and halftones (test encoders)

    pub fn bm(w: usize, h: usize, d: Vec<u8>) -> Bitmap {
        Bitmap { w, h, d }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enc_refine(
        e: &mut Mqe,
        base: usize,
        target: &[u8],
        w: usize,
        h: usize,
        rf: &Bitmap,
        dx: i32,
        dy: i32,
        t: u8,
        tpgron: bool,
        at: [(i32, i32); 2],
    ) {
        let mut ltp = false;
        for y in 0..h {
            if tpgron {
                let typical = (0..w).all(|x| {
                    refine_uniform(rf, dx, dy, x, y).is_none_or(|v| v == target[y * w + x])
                });
                e.encode((typical != ltp) as u8, base + refine_sltp(t));
                ltp = typical;
            }
            for x in 0..w {
                if ltp && refine_uniform(rf, dx, dy, x, y).is_some() {
                    continue;
                }
                let c = refine_ctx(t, &at, target, w, h, rf, dx, dy, x, y);
                e.encode(target[y * w + x], base + c);
            }
        }
    }

    fn page_info(w: usize, h: usize) -> Vec<u8> {
        let mut pi = (w as u32).to_be_bytes().to_vec();
        pi.extend((h as u32).to_be_bytes());
        pi.extend([0; 8]);
        pi.extend([0, 0, 0]);
        seg(0, 48, 1, &pi)
    }

    fn region_info(w: usize, h: usize, op: u8) -> Vec<u8> {
        let mut r = (w as u32).to_be_bytes().to_vec();
        r.extend((h as u32).to_be_bytes());
        r.extend([0; 8]);
        r.push(op);
        r
    }

    /// A generic region carrying `base`, then a refinement region turning it into `target`.
    pub fn refinement_stream(
        base: &[u8],
        target: &[u8],
        w: usize,
        h: usize,
        t: u8,
        tpgron: bool,
        at: [(i32, i32); 2],
    ) -> Vec<u8> {
        let at0 = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        let mut g = region_info(w, h, 0);
        g.push(0);
        for a in at0 {
            g.push(a.0 as i8 as u8);
            g.push(a.1 as i8 as u8);
        }
        g.extend(encode_generic(base, w, h, 0, false, at0));
        let mut r = region_info(w, h, 4);
        r.push(t | ((tpgron as u8) << 1));
        if t == 0 {
            for a in at {
                r.push(a.0 as i8 as u8);
                r.push(a.1 as i8 as u8);
            }
        }
        let mut e = Mqe::new(1 << 13);
        let rf = bm(w, h, base.to_vec());
        enc_refine(&mut e, 0, target, w, h, &rf, 0, 0, t, tpgron, at);
        r.extend(e.finish());
        let mut out = page_info(w, h);
        out.extend(seg(1, 38, 1, &g));
        out.extend(seg(2, 42, 1, &r));
        out
    }

    /// A refinement of one symbol inside a text region / aggregate dictionary.
    pub struct Refined {
        pub w: usize,
        pub h: usize,
        pub d: Vec<u8>,
        pub rdx: i32,
        pub rdy: i32,
    }

    // contexts of the text coder
    struct TextBases {
        ri: usize,
        rdw: usize,
        rdh: usize,
        rdx: usize,
        rdy: usize,
        gr: usize,
        aai: usize,
        total: usize,
    }
    fn text_bases(code_len: u32) -> TextBases {
        let ri = IAID + (1usize << (code_len + 1));
        TextBases {
            ri,
            rdw: ri + 512,
            rdh: ri + 1024,
            rdx: ri + 1536,
            rdy: ri + 2048,
            aai: ri + 2560,
            gr: ri + 3072,
            total: ri + 3072 + 8192,
        }
    }

    /// Text region, one instance per strip, top-left reference corner, optional refinement.
    /// `placed` = (symbol id, x, y, refinement), sorted by y.
    pub fn refined_text_region(
        w: usize,
        h: usize,
        syms: &[Bitmap],
        placed: &[(usize, i64, i64, Option<Refined>)],
        refine: bool,
        rt: u8,
        rat: [(i32, i32); 2],
    ) -> Vec<u8> {
        let code_len = if syms.len() <= 1 {
            0
        } else {
            usize::BITS - (syms.len() - 1).leading_zeros()
        };
        let b = text_bases(code_len);
        let mut e = Mqe::new(b.total);
        enc_int(&mut e, IADT, Some(0));
        let (mut cur_t, mut firsts) = (0i64, 0i64);
        for (id, x, y, rf) in placed {
            enc_int(&mut e, IADT, Some((*y - cur_t) as i32));
            cur_t = *y;
            enc_int(&mut e, IAFS, Some((*x - firsts) as i32));
            firsts = *x;
            enc_iaid(&mut e, *id, code_len);
            if refine {
                match rf {
                    None => enc_int(&mut e, b.ri, Some(0)),
                    Some(r) => {
                        enc_int(&mut e, b.ri, Some(1));
                        let base = &syms[*id];
                        let rdw = r.w as i32 - base.w as i32;
                        let rdh = r.h as i32 - base.h as i32;
                        enc_int(&mut e, b.rdw, Some(rdw));
                        enc_int(&mut e, b.rdh, Some(rdh));
                        enc_int(&mut e, b.rdx, Some(r.rdx));
                        enc_int(&mut e, b.rdy, Some(r.rdy));
                        enc_refine(
                            &mut e,
                            b.gr,
                            &r.d,
                            r.w,
                            r.h,
                            base,
                            rdw.div_euclid(2) + r.rdx,
                            rdh.div_euclid(2) + r.rdy,
                            rt,
                            false,
                            rat,
                        );
                    }
                }
            }
            enc_int(&mut e, IADS, None);
        }
        let data = e.finish();
        let mut region = region_info(w, h, 0);
        let flags: u16 = ((refine as u16) << 1) | (1 << 4) | ((rt as u16) << 15);
        region.extend(flags.to_be_bytes());
        if refine && rt == 0 {
            for a in rat {
                region.push(a.0 as i8 as u8);
                region.push(a.1 as i8 as u8);
            }
        }
        region.extend((placed.len() as u32).to_be_bytes());
        region.extend(data);
        region
    }

    fn plain_dict(syms: &[Bitmap]) -> Vec<u8> {
        let at = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        let n = syms.len();
        let code_len = if n <= 1 {
            0
        } else {
            usize::BITS - (n - 1).leading_zeros()
        };
        let mut e = Mqe::new(IAID + (1usize << (code_len + 1)));
        let mut prev_h = 0i32;
        let mut i = 0;
        while i < n {
            let hh = syms[i].h as i32;
            enc_int(&mut e, IADH, Some(hh - prev_h));
            prev_h = hh;
            let mut prev_w = 0i32;
            while i < n && syms[i].h as i32 == hh {
                let ww = syms[i].w as i32;
                enc_int(&mut e, IADW, Some(ww - prev_w));
                prev_w = ww;
                encode_generic_with(&mut e, &syms[i].d, syms[i].w, syms[i].h, 0, false, at);
                i += 1;
            }
            enc_int(&mut e, IADW, None);
        }
        enc_int(&mut e, IAEX, Some(0));
        enc_int(&mut e, IAEX, Some(n as i32));
        let mut sd = 0u16.to_be_bytes().to_vec();
        for a in at {
            sd.push(a.0 as i8 as u8);
            sd.push(a.1 as i8 as u8);
        }
        sd.extend((n as u32).to_be_bytes());
        sd.extend((n as u32).to_be_bytes());
        sd.extend(e.finish());
        sd
    }

    /// Dictionary of `syms`, then a text region that refines some of them while drawing.
    pub fn refined_text_stream(
        w: usize,
        h: usize,
        syms: &[Bitmap],
        placed: &[(usize, i64, i64, Option<Refined>)],
        rt: u8,
        rat: [(i32, i32); 2],
    ) -> Vec<u8> {
        let mut out = page_info(w, h);
        out.extend(seg(1, 0, 1, &plain_dict(syms)));
        let region = refined_text_region(w, h, syms, placed, true, rt, rat);
        out.extend(seg_refs(2, 6, 1, &[1], &region));
        out
    }

    /// A plain dictionary, an aggregate (refinement) dictionary built from it, and a text
    /// region drawing the refined symbols. `news` = (height-sorted) (source id, refined).
    pub fn aggregate_stream(
        w: usize,
        h: usize,
        syms: &[Bitmap],
        news: &[(usize, Refined)],
        placed: &[(usize, i64, i64)],
        rt: u8,
        rat: [(i32, i32); 2],
    ) -> Vec<u8> {
        let nin = syms.len();
        let nnew = news.len();
        let code_len = if nin + nnew <= 1 {
            0
        } else {
            usize::BITS - (nin + nnew - 1).leading_zeros()
        };
        let b = text_bases(code_len);
        let mut e = Mqe::new(b.total);
        let mut prev_h = 0i32;
        let mut i = 0;
        while i < nnew {
            let hh = news[i].1.h as i32;
            enc_int(&mut e, IADH, Some(hh - prev_h));
            prev_h = hh;
            let mut prev_w = 0i32;
            while i < nnew && news[i].1.h as i32 == hh {
                let (src, r) = &news[i];
                enc_int(&mut e, IADW, Some(r.w as i32 - prev_w));
                prev_w = r.w as i32;
                enc_int(&mut e, b.aai, Some(1));
                enc_iaid(&mut e, *src, code_len);
                enc_int(&mut e, b.rdx, Some(r.rdx));
                enc_int(&mut e, b.rdy, Some(r.rdy));
                enc_refine(
                    &mut e,
                    b.gr,
                    &r.d,
                    r.w,
                    r.h,
                    &syms[*src],
                    r.rdx,
                    r.rdy,
                    rt,
                    false,
                    rat,
                );
                i += 1;
            }
            enc_int(&mut e, IADW, None);
        }
        enc_int(&mut e, IAEX, Some(nin as i32));
        enc_int(&mut e, IAEX, Some(nnew as i32));
        let at = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        let mut sd = (2u16 | ((rt as u16) << 12)).to_be_bytes().to_vec();
        for a in at {
            sd.push(a.0 as i8 as u8);
            sd.push(a.1 as i8 as u8);
        }
        if rt == 0 {
            for a in rat {
                sd.push(a.0 as i8 as u8);
                sd.push(a.1 as i8 as u8);
            }
        }
        sd.extend((nnew as u32).to_be_bytes());
        sd.extend((nnew as u32).to_be_bytes());
        sd.extend(e.finish());
        let new_syms: Vec<Bitmap> = news
            .iter()
            .map(|(_, r)| bm(r.w, r.h, r.d.clone()))
            .collect();
        let pl: Vec<(usize, i64, i64, Option<Refined>)> =
            placed.iter().map(|&(id, x, y)| (id, x, y, None)).collect();
        let region = refined_text_region(w, h, &new_syms, &pl, false, 0, rat);
        let mut out = page_info(w, h);
        out.extend(seg(1, 0, 1, &plain_dict(syms)));
        out.extend(seg_refs(2, 0, 1, &[1], &sd));
        out.extend(seg_refs(3, 6, 1, &[2], &region));
        out
    }

    /// Halftone: pattern dictionary + region. Returns the stream.
    #[allow(clippy::too_many_arguments)]
    pub fn halftone_stream(
        w: usize,
        h: usize,
        pats: &[Bitmap],
        gray: &[u32],
        gw: usize,
        gh: usize,
        (gx, gy, hrx, hry): (i32, i32, u16, u16),
        t: u8,
        skip: bool,
    ) -> Vec<u8> {
        let (pw, ph) = (pats[0].w, pats[0].h);
        let n = pats.len();
        let cw = n * pw;
        let mut coll = vec![0u8; cw * ph];
        for (k, p) in pats.iter().enumerate() {
            for y in 0..ph {
                coll[y * cw + k * pw..y * cw + (k + 1) * pw]
                    .copy_from_slice(&p.d[y * pw..(y + 1) * pw]);
            }
        }
        let pat_at = [(-(pw as i32), 0), (-3, -1), (2, -2), (-2, -2)];
        let mut pd = vec![t << 1, pw as u8, ph as u8];
        pd.extend(((n - 1) as u32).to_be_bytes());
        pd.extend(encode_generic(&coll, cw, ph, t, false, pat_at));
        let mut bpp = 0;
        while (1usize << bpp) < n {
            bpp += 1;
        }
        let at = [
            (if t <= 1 { 3 } else { 2 }, -1),
            (-3, -1),
            (2, -2),
            (-2, -2),
        ];
        let skipm: Option<Vec<u8>> = skip.then(|| {
            let mut k = vec![0u8; gw * gh];
            for m in 0..gh {
                for nn in 0..gw {
                    let x = (gx as i64 + m as i64 * hry as i64 + nn as i64 * hrx as i64) >> 8;
                    let y = (gy as i64 + m as i64 * hrx as i64 - nn as i64 * hry as i64) >> 8;
                    if x + pw as i64 <= 0 || x >= w as i64 || y + ph as i64 <= 0 || y >= h as i64 {
                        k[m * gw + nn] = 1;
                    }
                }
            }
            k
        });
        let mut e = Mqe::new(1 << template(t, &at).len());
        let offs = template(t, &at);
        for j in (0..bpp).rev() {
            let plane: Vec<u8> = gray
                .iter()
                .map(|&g| {
                    let b = (g >> j) & 1;
                    let hi = if j + 1 < bpp { (g >> (j + 1)) & 1 } else { 0 };
                    (b ^ hi) as u8
                })
                .collect();
            for y in 0..gh {
                for x in 0..gw {
                    if skipm.as_ref().is_some_and(|k| k[y * gw + x] != 0) {
                        continue;
                    }
                    let mut ctx = 0usize;
                    for (bit, &(dx, dy)) in offs.iter().enumerate() {
                        let (px, py) = (x as i32 + dx, y as i32 + dy);
                        if px >= 0 && py >= 0 && (px as usize) < gw && (py as usize) < gh {
                            let skipped = skipm
                                .as_ref()
                                .is_some_and(|k| k[py as usize * gw + px as usize] != 0);
                            if !skipped {
                                ctx |= (plane[py as usize * gw + px as usize] as usize) << bit;
                            }
                        }
                    }
                    e.encode(plane[y * gw + x], ctx);
                }
            }
        }
        let mut r = region_info(w, h, 0);
        r.push((t << 1) | ((skip as u8) << 3));
        r.extend((gw as u32).to_be_bytes());
        r.extend((gh as u32).to_be_bytes());
        r.extend(gx.to_be_bytes());
        r.extend(gy.to_be_bytes());
        r.extend(hrx.to_be_bytes());
        r.extend(hry.to_be_bytes());
        r.extend(e.finish());
        let mut out = page_info(w, h);
        out.extend(seg(1, 16, 1, &pd));
        out.extend(seg_refs(2, 22, 1, &[1], &r));
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

    /// Huffman-coded JBIG2 streams made by an independent encoder (a Python script written from
    /// the T.88 text): (stream, expected page rows). Vectors 0, 1 and 3 also decode identically in
    /// pdfium; vector 2 (collective bitmap coded with MMR) could not be cross-checked there
    /// (pdfium renders it blank), so it rests on this decoder and the G4 decoder alone.
    const HUFFMAN_VECTORS: [(&str, &[&str]); 4] = [
        (
            "000000003000010000001300000040000000280000000000000000000000000000010000010000002800010000000500000005e5c83f00279fef80546118008fa11f00fc6118008c6118008f9fef800140000000020620010100000038000000400000002800000000000000000000110000000000080011000000000000000000000000000000030074051a75b5f53e600a078190",
            &[
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "............#...####...####.####..#####.........................",
                "...........#.#..#...#.#.....#...#.#.............................",
                "..........#...#.####..#.....#...#.####..........................",
                "..........#####.#...#.#.....#...#.#.............................",
                "..........#...#.#...#.#.....#...#.#.............................",
                "..........#...#.####...####.####..#####.........................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "..........#####...####.....####.................................",
                "..........#.......#...#...#.....................................",
                "..........####....#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#####...####.....####.................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
            ],
        ),
        (
            "000000003000010000001300000040000000280000000000000000000000000000010000010000002800010000000500000005e5c83f00279fef80546118008fa11f00fc6118008c6118008f9fef8001400000000206200101000000390000004000000028000000000000000000000900000000000800110000000000000000000000000000000300505069756d7f44e000a03e0590",
            &[
                "................................................................",
                "................................................................",
                "................................................................",
                "............#.....................#####.........................",
                "...........#.#..####..............#.............................",
                "..........#...#.#...#..####.......####..........................",
                "..........#####.####..#.....####..#.............................",
                "..........#...#.#...#.#.....#...#.#.............................",
                "..........#...#.#...#.#.....#...#.#####.........................",
                "................####..#.....#...#...............................",
                ".......................####.#...#...............................",
                "............................####................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "..........#####.................................................",
                "..........#.......####..........................................",
                "..........####....#...#....####.................................",
                "..........#.......#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#####...#...#...#.....................................",
                "..................####....#.....................................",
                "...........................####.................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
            ],
        ),
        (
            "000000003000010000001300000040000000280000000000000000000000000000010000010000003100010000000500000005e5c83f84002e8bb2e291cd208a1c1047b048f69306475c1e209782ca8ffe0cb8a4730010010140000000020620010100000038000000400000002800000000000000000000110000000000080011000000000000000000000000000000030074051a75b5f53e600a078190",
            &[
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "............#...####...####.####..#####.........................",
                "...........#.#..#...#.#.....#...#.#.............................",
                "..........#...#.####..#.....#...#.####..........................",
                "..........#####.#...#.#.....#...#.#.............................",
                "..........#...#.#...#.#.....#...#.#.............................",
                "..........#...#.####...####.####..#####.........................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "..........#####...####.....####.................................",
                "..........#.......#...#...#.....................................",
                "..........####....#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#####...####.....####.................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
            ],
        ),
        (
            "000000003000010000001300000040000000280000000000000000000000000000010000010000002800010000000500000005e5c83f00279fef80546118008fa11f00fc6118008c6118008f9fef8001400000000206200101000000380000004000000028000000000000000000001500000000000800010000000000000000000000000000100568141a9d16af48f20048170090",
            &[
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "............#..........####.......#####.........................",
                "...........#.#..####..#.....####..#.............................",
                "..........#...#.#...#.#.....#...#.####..........................",
                "..........#####.####..#.....#...#.#.............................",
                "..........#...#.#...#.#.....#...#.#.............................",
                "..........#...#.#...#..####.#...#.#####.........................",
                "................####........####................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "..........#####............####.................................",
                "..........#.......####....#.....................................",
                "..........####....#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#.......#...#...#.....................................",
                "..........#####...#...#....####.................................",
                "..................####..........................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
                "................................................................",
            ],
        ),
    ];

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn reserved_table_selections_are_errors_not_garbage() {
        let t = vec![Rc::new(Table::standard(1))];
        let mut used = 0;
        assert!(pick(2, &[(0, 4), (1, 5)], 3, &t, &mut used).is_err());
        assert!(pick(3, &[(0, 4), (1, 5)], 3, &[], &mut used).is_err());
        assert!(pick(3, &[(0, 4), (1, 5)], 3, &t, &mut used).is_ok());
        assert_eq!(used, 1);
        assert!(pick(1, &[(0, 1)], 1, &t, &mut 0).is_ok());
    }

    #[test]
    fn huffman_streams_decode_like_the_reference_decoder() {
        for (i, (stream, rows)) in HUFFMAN_VECTORS.iter().enumerate() {
            let (w, h) = (rows[0].len(), rows.len());
            let expected: Vec<u8> = rows
                .iter()
                .flat_map(|r| r.bytes().map(|c| u8::from(c == b'#')))
                .collect();
            match decode(&hex(stream), None, w, h) {
                Ok(got) => assert_eq!(got, expected, "vector {i}"),
                Err(e) => panic!("vector {i}: {e}"),
            }
        }
    }

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
        // the budget is for optimised builds; debug builds are an order of magnitude slower
        let budget = if cfg!(debug_assertions) { 300 } else { 30 };
        assert!(
            t0.elapsed().as_secs() < budget,
            "too slow: {:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn damaged_refinement_aggregate_and_halftone_streams_never_panic() {
        let (w, h) = (40usize, 30usize);
        let base = sample(w, h, 3);
        let target = sample(w, h, 4);
        let syms: Vec<Bitmap> = (0..4).map(|i| bm(6, 7, sample(6, 7, 10 + i))).collect();
        let rf = |id: usize| Refined {
            w: 7,
            h: 8,
            d: sample(7, 8, 20 + id as u64),
            rdx: 0,
            rdy: 1,
        };
        let placed = vec![(0usize, 2i64, 1i64, Some(rf(0))), (1, 12, 9, None)];
        let news = vec![(0usize, rf(1)), (2, rf(2))];
        let pats: Vec<Bitmap> = (0..4).map(|i| bm(3, 3, sample(3, 3, 30 + i))).collect();
        let gray: Vec<u32> = (0..20).map(|i| i % 4).collect();
        let streams = [
            refinement_stream(&base, &target, w, h, 0, true, [(-1, -1), (-1, -1)]),
            refinement_stream(&base, &target, w, h, 1, false, [(-1, -1), (-1, -1)]),
            refined_text_stream(w, h, &syms, &placed, 0, [(-1, -1), (-1, -1)]),
            aggregate_stream(
                w,
                h,
                &syms,
                &news,
                &[(0, 1, 1), (1, 20, 12)],
                1,
                [(-1, -1); 2],
            ),
            halftone_stream(w, h, &pats, &gray, 5, 4, (0, 0, 3 << 8, 0), 0, false),
            halftone_stream(w, h, &pats, &gray, 5, 4, (-256, 0, 3 << 8, 0), 2, true),
        ];
        let iters: u64 = std::env::var("LEGATOROCR_FUZZ_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150);
        let mut s = 0xC0FFEEu64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for st in &streams {
            assert!(decode(st, None, w, h).is_ok());
            for _ in 0..iters {
                let mut d = st.clone();
                for _ in 0..1 + rnd() % 4 {
                    let i = (rnd() as usize) % d.len();
                    match rnd() % 3 {
                        0 => d[i] = rnd() as u8,
                        1 => d[i] ^= 1 << (rnd() % 8),
                        _ => d.truncate(i.max(12)),
                    }
                }
                let _ = decode(&d, None, w, h);
            }
        }
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
