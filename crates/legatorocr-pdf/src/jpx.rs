//! JPEG 2000 (ITU-T T.800) decoder, written from the standard's description:
//! JP2 boxes or raw codestreams, tiles and tile-parts, all five progression orders,
//! quality layers, precincts, SOP/EPH markers, the reversible 5/3 and irreversible 9/7
//! wavelets, RCT/ICT colour transforms, component subsampling (nearest-neighbour upsampling),
//! and the EBCOT tier-1 coder (MQ arithmetic decoder shared with the JBIG2 module) with the
//! reset, vertically-causal, segmentation-symbol, predictable-termination, selective
//! arithmetic-bypass ("lazy", raw sig-prop/mag-ref passes with bit-stuffing) and
//! termination-on-every-pass styles (several codeword segments per code-block), progression
//! order changes (POC) and packed packet headers (PPM and PPT).
//!
//! Not supported (reported as errors): palettes, the high-throughput (Part 15) and other
//! extended code-block style bits (0x40, 0x80).

use crate::jbig2::Mq;
use rayon::prelude::*;
use std::fmt;

#[derive(Debug)]
pub struct JpxError(pub String);
impl fmt::Display for JpxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
type R<T> = Result<T, JpxError>;
fn fail<T>(m: impl Into<String>) -> R<T> {
    Err(JpxError(m.into()))
}

const MAX_SAMPLES: u64 = 120_000_000;
const MAX_COMPONENTS: usize = 64;
const MAX_PRECINCTS: u64 = 500_000;
const MAX_T1_WORK: u64 = 8_000_000_000;
const MAX_PACKETS: u64 = 20_000_000;

/// 8-bit interleaved result.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub nc: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct CompInfo {
    depth: u32,
    signed: bool,
    dx: u64,
    dy: u64,
}

struct Siz {
    x1: u64,
    y1: u64,
    x0: u64,
    y0: u64,
    tw: u64,
    th: u64,
    tx0: u64,
    ty0: u64,
    comps: Vec<CompInfo>,
}

#[derive(Clone, Copy)]
struct CodG {
    prog: u8,
    layers: u32,
    mct: u8,
    sop: bool,
    eph: bool,
}

#[derive(Clone)]
struct CompCod {
    nl: u32,
    xcb: u32,
    ycb: u32,
    style: u8,
    rev: bool,
    prec: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Quant {
    style: u8,
    guard: u32,
    steps: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Params {
    g: CodG,
    cc: Vec<CompCod>,
    q: Vec<Quant>,
    poc: Vec<Poc>,
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}
impl Rd<'_> {
    fn u8(&mut self) -> R<u32> {
        let v = *self
            .d
            .get(self.p)
            .ok_or_else(|| JpxError("truncated header".into()))?;
        self.p += 1;
        Ok(v as u32)
    }
    fn u16(&mut self) -> R<u32> {
        Ok((self.u8()? << 8) | self.u8()?)
    }
    fn u32(&mut self) -> R<u64> {
        Ok(((self.u16()? as u64) << 16) | self.u16()? as u64)
    }
}

fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}
fn ceil_shr(a: u64, s: u32) -> u64 {
    if s >= 63 {
        u64::from(a > 0)
    } else {
        (a + (1u64 << s) - 1) >> s
    }
}

// ---------------------------------------------------------------- container

fn unwrap_jp2(d: &[u8]) -> R<(&[u8], Option<u32>)> {
    if d.len() >= 4 && d[0] == 0xFF && d[1] == 0x4F && d[2] == 0xFF && d[3] == 0x51 {
        return Ok((d, None));
    }
    let mut cs = None;
    let mut code = None;
    fn boxes(
        d: &[u8],
        depth: u32,
        cs: &mut Option<u32>,
        code: &mut Option<(usize, usize)>,
        base: usize,
    ) -> R<()> {
        let mut p = 0usize;
        while p + 8 <= d.len() {
            let l = u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]) as usize;
            let t = &d[p + 4..p + 8];
            let (hdr, len) = if l == 1 {
                if p + 16 > d.len() {
                    return fail("truncated JP2 box");
                }
                let x = u64::from_be_bytes(d[p + 8..p + 16].try_into().unwrap());
                (16, x as usize)
            } else if l == 0 {
                (8, d.len() - p)
            } else {
                (8, l)
            };
            if len < hdr || p.checked_add(len).is_none_or(|end| end > d.len()) {
                return fail("damaged JP2 box");
            }
            let body = &d[p + hdr..p + len];
            match t {
                b"jp2h" if depth < 2 => boxes(body, depth + 1, cs, code, base + p + hdr)?,
                b"colr" if cs.is_none() && body.len() >= 7 && body[0] == 1 => {
                    *cs = Some(u32::from_be_bytes([body[3], body[4], body[5], body[6]]));
                }
                b"pclr" => return fail("JPEG 2000 palettes are not supported"),
                b"jp2c" if code.is_none() => *code = Some((base + p + hdr, base + p + len)),
                _ => {}
            }
            p += len;
        }
        Ok(())
    }
    boxes(d, 0, &mut cs, &mut code, 0)?;
    match code {
        Some((a, b)) => Ok((&d[a..b], cs)),
        None => fail("no JPEG 2000 codestream found"),
    }
}

// ---------------------------------------------------------------- bit reader, tag trees

struct Bits<'a> {
    d: &'a [u8],
    p: usize,
    cur: u32,
    nb: u32,
}
impl<'a> Bits<'a> {
    fn new(d: &'a [u8], p: usize) -> Self {
        Bits {
            d,
            p,
            cur: 0,
            nb: 0,
        }
    }
    fn bit(&mut self) -> R<u32> {
        if self.nb == 0 {
            let prev_ff = self.cur == 0xFF;
            self.cur = *self
                .d
                .get(self.p)
                .ok_or_else(|| JpxError("truncated packet header".into()))?
                as u32;
            self.p += 1;
            self.nb = if prev_ff { 7 } else { 8 };
        }
        self.nb -= 1;
        Ok((self.cur >> self.nb) & 1)
    }
    fn bits(&mut self, n: u32) -> R<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }
    fn align(&mut self) {
        self.nb = 0;
        if self.cur == 0xFF {
            self.p += 1;
        }
        self.cur = 0;
    }
}

type TagLevel = (usize, usize, Vec<(u32, u32)>);
struct Tag {
    lv: Vec<TagLevel>,
}
impl Tag {
    fn new(w: usize, h: usize) -> Self {
        let mut lv = Vec::new();
        let (mut cw, mut ch) = (w, h);
        loop {
            lv.push((cw, ch, vec![(u32::MAX, 0u32); cw * ch]));
            if cw <= 1 && ch <= 1 {
                break;
            }
            cw = cw.div_ceil(2);
            ch = ch.div_ceil(2);
        }
        Tag { lv }
    }
    /// True when the leaf's value is below `thr` (reading as many bits as needed).
    fn decode(&mut self, b: &mut Bits, x: usize, y: usize, thr: u32) -> R<bool> {
        let mut low = 0u32;
        for l in (0..self.lv.len()).rev() {
            let (w, _, nodes) = &mut self.lv[l];
            let n = &mut nodes[(y >> l) * *w + (x >> l)];
            if low > n.1 {
                n.1 = low;
            } else {
                low = n.1;
            }
            while low < thr && low < n.0 {
                if b.bit()? == 1 {
                    n.0 = low;
                } else {
                    low += 1;
                }
            }
            n.1 = low;
        }
        let n = &self.lv[0].2[y * self.lv[0].0 + x];
        Ok(n.0 < thr)
    }
}

// ---------------------------------------------------------------- tile structures

struct Cb {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    included: bool,
    zbp: u32,
    lblock: u32,
    passes: u32,
    segs: Vec<Seg>,
}
/// One codeword segment of a code-block: the passes it holds and the bytes that terminate it.
struct Seg {
    passes: u32,
    max: u32,
    data: Vec<u8>,
}
struct PrecBand {
    cbs: Vec<Cb>,
    nw: usize,
    incl: Tag,
    zbp: Tag,
}
struct Band {
    orient: u8,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    mb: u32,
    delta: f32,
    data: Vec<f32>,
}
struct Res {
    x0: u64,
    y0: u64,
    x1: u64,
    y1: u64,
    ppx: u32,
    ppy: u32,
    npw: usize,
    nph: usize,
    bands: Vec<Band>,
    precs: Vec<Vec<PrecBand>>,
}
struct TileComp {
    x0: u64,
    y0: u64,
    x1: u64,
    y1: u64,
    cc: CompCod,
    res: Vec<Res>,
}

fn gain(orient: u8) -> u32 {
    match orient {
        0 => 0,
        3 => 2,
        _ => 1,
    }
}

fn build_tile_comp(
    cc: &CompCod,
    q: &Quant,
    ci: &CompInfo,
    t: (u64, u64, u64, u64),
    total_prec: &mut u64,
) -> R<TileComp> {
    let (x0, y0, x1, y1) = (
        ceil_div(t.0, ci.dx),
        ceil_div(t.1, ci.dy),
        ceil_div(t.2, ci.dx),
        ceil_div(t.3, ci.dy),
    );
    let nl = cc.nl;
    let mut res = Vec::new();
    for r in 0..=nl {
        let sh = nl - r;
        let (rx0, ry0, rx1, ry1) = (
            ceil_shr(x0, sh),
            ceil_shr(y0, sh),
            ceil_shr(x1, sh),
            ceil_shr(y1, sh),
        );
        let (ppx, ppy) = cc.prec.get(r as usize).copied().unwrap_or((15, 15));
        if r > 0 && (ppx == 0 || ppy == 0) {
            return fail("invalid precinct size");
        }
        let npw = if rx1 > rx0 {
            ((ceil_shr(rx1, ppx)) - (rx0 >> ppx)) as usize
        } else {
            0
        };
        let nph = if ry1 > ry0 {
            ((ceil_shr(ry1, ppy)) - (ry0 >> ppy)) as usize
        } else {
            0
        };
        *total_prec += (npw as u64) * (nph as u64);
        if *total_prec > MAX_PRECINCTS {
            return fail("too many precincts");
        }
        let orients: &[u8] = if r == 0 { &[0] } else { &[1, 2, 3] };
        let mut bands = Vec::new();
        for &o in orients {
            let nb = if r == 0 { nl } else { nl - r + 1 };
            let (xo, yo) = (u64::from(o & 1 == 1), u64::from(o >= 2));
            let f = |v: u64, off: u64| -> u64 {
                if nb == 0 {
                    v
                } else if off == 0 {
                    ceil_shr(v, nb)
                } else {
                    let sub = off << (nb - 1);
                    ceil_shr(v.saturating_sub(sub), nb)
                }
            };
            let (bx0, by0, bx1, by1) = (f(x0, xo), f(y0, yo), f(x1, xo), f(y1, yo));
            // quantisation
            let bi = if r == 0 {
                0
            } else {
                3 * (r as usize - 1) + o as usize
            };
            let (eps, mant) = match q.style {
                1 => {
                    let (e0, m0) = *q.steps.first().ok_or_else(|| JpxError("bad QCD".into()))?;
                    let e = e0 as i64 - nl as i64 + nb as i64;
                    (e.max(0) as u32, m0)
                }
                _ => *q
                    .steps
                    .get(bi)
                    .ok_or_else(|| JpxError("QCD has too few entries".into()))?,
            };
            let mb = (q.guard + eps).saturating_sub(1);
            if mb > 37 {
                return fail("too many bit-planes");
            }
            let delta = if cc.rev {
                1.0
            } else {
                let rb = ci.depth as i32 + gain(o) as i32;
                2f32.powi(rb - eps as i32) * (1.0 + mant as f32 / 2048.0)
            };
            let (w, h) = ((bx1 - bx0) as usize, (by1 - by0) as usize);
            bands.push(Band {
                orient: o,
                x0: bx0 as usize,
                y0: by0 as usize,
                x1: bx1 as usize,
                y1: by1 as usize,
                mb,
                delta,
                data: vec![0.0; w * h],
            });
        }
        // precincts and code-blocks
        let mut precs = Vec::with_capacity(npw * nph);
        let (xcb, ycb) = if r == 0 {
            (cc.xcb.min(ppx), cc.ycb.min(ppy))
        } else {
            (cc.xcb.min(ppx - 1), cc.ycb.min(ppy - 1))
        };
        for py in 0..nph {
            for px in 0..npw {
                let mut pbs = Vec::new();
                for b in &bands {
                    let (sx, sy) = if r == 0 {
                        (ppx, ppy)
                    } else {
                        (ppx - 1, ppy - 1)
                    };
                    let gx = (rx0 >> ppx) + px as u64;
                    let gy = (ry0 >> ppy) + py as u64;
                    let (qx0, qy0) = (gx << sx, gy << sy);
                    let (qx1, qy1) = (qx0 + (1u64 << sx), qy0 + (1u64 << sy));
                    let ix0 = qx0.max(b.x0 as u64);
                    let iy0 = qy0.max(b.y0 as u64);
                    let ix1 = qx1.min(b.x1 as u64);
                    let iy1 = qy1.min(b.y1 as u64);
                    let mut cbs = Vec::new();
                    let (mut nw, mut nh) = (0usize, 0usize);
                    if ix1 > ix0 && iy1 > iy0 {
                        let (cx0, cx1) = (ix0 >> xcb, ceil_shr(ix1, xcb));
                        let (cy0, cy1) = (iy0 >> ycb, ceil_shr(iy1, ycb));
                        nw = (cx1 - cx0) as usize;
                        nh = (cy1 - cy0) as usize;
                        if (nw as u64) * (nh as u64) > 4_000_000 {
                            return fail("too many code-blocks");
                        }
                        for cy in cy0..cy1 {
                            for cx in cx0..cx1 {
                                cbs.push(Cb {
                                    x0: (cx << xcb).max(ix0) as usize,
                                    y0: (cy << ycb).max(iy0) as usize,
                                    x1: ((cx + 1) << xcb).min(ix1) as usize,
                                    y1: ((cy + 1) << ycb).min(iy1) as usize,
                                    included: false,
                                    zbp: 0,
                                    lblock: 3,
                                    passes: 0,
                                    segs: Vec::new(),
                                });
                            }
                        }
                    }
                    pbs.push(PrecBand {
                        cbs,
                        nw,
                        incl: Tag::new(nw, nh),
                        zbp: Tag::new(nw, nh),
                    });
                }
                precs.push(pbs);
            }
        }
        res.push(Res {
            x0: rx0,
            y0: ry0,
            x1: rx1,
            y1: ry1,
            ppx,
            ppy,
            npw,
            nph,
            bands,
            precs,
        });
    }
    Ok(TileComp {
        x0,
        y0,
        x1,
        y1,
        cc: cc.clone(),
        res,
    })
}

// ---------------------------------------------------------------- packets

/// One progression-order change (POC) entry; `re`/`ce`/`lye` are exclusive bounds.
#[derive(Clone, Copy)]
struct Poc {
    rs: u32,
    cs: u32,
    lye: u32,
    re: u32,
    ce: u32,
    prog: u8,
}

const MAX_POCS: usize = 256;
const MAX_PROGRESSION_WORK: u64 = 150_000_000;

type Packet = (u32, u32, u32, u32);

/// Collects the packet sequence; with POC every packet is emitted once only.
struct Sink {
    out: Vec<Packet>,
    seen: Option<Vec<u64>>,
    base: Vec<Vec<usize>>,
    layers: u32,
    work: u64,
}
impl Sink {
    fn tick(&mut self, n: u64) -> R<()> {
        self.work += n;
        if self.work > MAX_PROGRESSION_WORK {
            return fail("too many packets");
        }
        Ok(())
    }
    fn push(&mut self, l: u32, r: u32, c: u32, p: u32) -> R<()> {
        self.tick(1)?;
        if let Some(seen) = &mut self.seen {
            let i = (self.base[c as usize][r as usize] + p as usize) * self.layers as usize
                + l as usize;
            let (w, b) = (i / 64, 1u64 << (i % 64));
            if seen[w] & b != 0 {
                return Ok(());
            }
            seen[w] |= b;
        }
        self.out.push((l, r, c, p));
        Ok(())
    }
}

fn progression(
    g: &CodG,
    tcs: &[TileComp],
    comps: &[CompInfo],
    t: (u64, u64, u64, u64),
    poc: &[Poc],
) -> R<Vec<Packet>> {
    let maxr = tcs.iter().map(|c| c.cc.nl).max().unwrap_or(0);
    let layers = g.layers;
    let mut total: u64 = 0;
    let mut base = Vec::with_capacity(tcs.len());
    for c in tcs {
        let mut b = Vec::with_capacity(c.res.len());
        for r in &c.res {
            b.push((total / layers as u64) as usize);
            total += (r.npw * r.nph) as u64 * layers as u64;
        }
        base.push(b);
    }
    if total > MAX_PACKETS {
        return fail("too many packets");
    }
    let mut sink = Sink {
        out: Vec::with_capacity(total as usize),
        seen: (!poc.is_empty()).then(|| vec![0u64; total as usize / 64 + 1]),
        base,
        layers,
        work: 0,
    };
    let default = [Poc {
        rs: 0,
        cs: 0,
        lye: layers,
        re: maxr + 1,
        ce: tcs.len() as u32,
        prog: g.prog,
    }];
    let entries: &[Poc] = if poc.is_empty() { &default } else { poc };
    let nprec = |c: usize, r: u32| -> u32 {
        tcs[c]
            .res
            .get(r as usize)
            .map_or(0, |x| (x.npw * x.nph) as u32)
    };
    // step sizes of the position based orders
    let mut stepx = u64::MAX;
    let mut stepy = u64::MAX;
    if entries.iter().any(|e| (2..=4).contains(&e.prog)) {
        for (c, tc) in tcs.iter().enumerate() {
            for (r, rs) in tc.res.iter().enumerate() {
                let s = tc.cc.nl - r as u32;
                if rs.ppx + s >= 40 || rs.ppy + s >= 40 {
                    return fail("precinct too large");
                }
                stepx = stepx.min(comps[c].dx << (rs.ppx + s));
                stepy = stepy.min(comps[c].dy << (rs.ppy + s));
            }
        }
    }
    let hit = |c: usize, r: u32, x: u64, y: u64| -> Option<u32> {
        let tc = &tcs[c];
        let rs = tc.res.get(r as usize)?;
        if rs.npw == 0 || rs.nph == 0 {
            return None;
        }
        let s = tc.cc.nl - r;
        let (dx, dy) = (comps[c].dx, comps[c].dy);
        let okx = x.is_multiple_of(dx << (rs.ppx + s))
            || (x == t.0 && !(rs.x0 << s).is_multiple_of(1u64 << (rs.ppx + s)));
        let oky = y.is_multiple_of(dy << (rs.ppy + s))
            || (y == t.1 && !(rs.y0 << s).is_multiple_of(1u64 << (rs.ppy + s)));
        if !okx || !oky {
            return None;
        }
        let px = (ceil_div(x, dx << s) >> rs.ppx).checked_sub(rs.x0 >> rs.ppx)?;
        let py = (ceil_div(y, dy << s) >> rs.ppy).checked_sub(rs.y0 >> rs.ppy)?;
        if px as usize >= rs.npw || py as usize >= rs.nph {
            return None;
        }
        Some((py as usize * rs.npw + px as usize) as u32)
    };
    let positions = |sink: &mut Sink, f: &mut dyn FnMut(&mut Sink, u64, u64) -> R<()>| -> R<()> {
        let mut y = t.1;
        while y < t.3 {
            let mut x = t.0;
            while x < t.2 {
                f(sink, x, y)?;
                x = (x / stepx + 1) * stepx;
            }
            y = (y / stepy + 1) * stepy;
        }
        Ok(())
    };
    for e in entries {
        let re = e.re.min(maxr + 1);
        let ce = (e.ce as usize).min(tcs.len());
        let cs = e.cs as usize;
        let lye = e.lye.min(layers);
        let width = (ce.saturating_sub(cs) as u64) * (re.saturating_sub(e.rs) as u64);
        match e.prog {
            0 => {
                for l in 0..lye {
                    for r in e.rs..re {
                        for c in cs..ce {
                            for p in 0..nprec(c, r) {
                                sink.push(l, r, c as u32, p)?;
                            }
                        }
                    }
                }
            }
            1 => {
                for r in e.rs..re {
                    for l in 0..lye {
                        for c in cs..ce {
                            for p in 0..nprec(c, r) {
                                sink.push(l, r, c as u32, p)?;
                            }
                        }
                    }
                }
            }
            2 => {
                for r in e.rs..re {
                    positions(&mut sink, &mut |sink, x, y| {
                        sink.tick(ce.saturating_sub(cs) as u64 + 1)?;
                        for c in cs..ce {
                            if let Some(p) = hit(c, r, x, y) {
                                for l in 0..lye {
                                    sink.push(l, r, c as u32, p)?;
                                }
                            }
                        }
                        Ok(())
                    })?;
                }
            }
            3 => positions(&mut sink, &mut |sink, x, y| {
                sink.tick(width + 1)?;
                for c in cs..ce {
                    for r in e.rs..re {
                        if let Some(p) = hit(c, r, x, y) {
                            for l in 0..lye {
                                sink.push(l, r, c as u32, p)?;
                            }
                        }
                    }
                }
                Ok(())
            })?,
            4 => {
                for c in cs..ce {
                    positions(&mut sink, &mut |sink, x, y| {
                        sink.tick(re.saturating_sub(e.rs) as u64 + 1)?;
                        for r in e.rs..re {
                            if let Some(p) = hit(c, r, x, y) {
                                for l in 0..lye {
                                    sink.push(l, r, c as u32, p)?;
                                }
                            }
                        }
                        Ok(())
                    })?;
                }
            }
            _ => return fail("unknown progression order"),
        }
    }
    Ok(sink.out)
}

/// Maximum number of passes of the next codeword segment of a code-block.
fn next_seg_max(style: u8, prev: Option<u32>) -> u32 {
    if style & 4 != 0 {
        1
    } else if style & 1 != 0 {
        match prev {
            None => 10,
            Some(1) | Some(10) => 2,
            Some(_) => 1,
        }
    } else {
        109
    }
}

const MAX_SEGMENTS: usize = 256;

/// Where each packet lies in the tile data: (start incl. SOP, header start, header end incl.
/// EPH, body end). Only recorded by the tests, which rewrite streams with packed headers.
#[cfg(test)]
type PacketSpan = (usize, usize, usize, usize);
#[cfg(test)]
thread_local! {
    static TRACE: std::cell::RefCell<Vec<Vec<PacketSpan>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn read_packets(
    g: &CodG,
    tcs: &mut [TileComp],
    order: &[Packet],
    d: &[u8],
    packed: Option<&[u8]>,
) -> R<()> {
    let mut pos = 0usize;
    let mut hpos = 0usize;
    #[cfg(test)]
    let mut trace = Vec::new();
    for &(l, r, c, p) in order {
        let exhausted = match packed {
            Some(h) => hpos >= h.len(),
            None => pos >= d.len(),
        };
        if exhausted {
            break; // truncated tile: keep what we have
        }
        #[cfg(test)]
        let pkt_start = pos;
        if g.sop && d.len() >= pos + 6 && d[pos] == 0xFF && d[pos + 1] == 0x91 {
            pos += 6;
        }
        let style = tcs[c as usize].cc.style;
        let rs = &mut tcs[c as usize].res[r as usize];
        let precs = &mut rs.precs[p as usize];
        let (hsrc, hstart) = match packed {
            Some(h) => (h, hpos),
            None => (d, pos),
        };
        let mut b = Bits::new(hsrc, hstart);
        let mut work: Vec<(usize, usize, usize, usize)> = Vec::new(); // band, cb, segment, len
        if b.bit()? == 1 {
            for (bi, pb) in precs.iter_mut().enumerate() {
                let nw = pb.nw;
                for ci in 0..pb.cbs.len() {
                    let (cx, cy) = (ci % nw, ci / nw);
                    let included = if pb.cbs[ci].included {
                        b.bit()? == 1
                    } else {
                        pb.incl.decode(&mut b, cx, cy, l + 1)?
                    };
                    if !included {
                        continue;
                    }
                    if !pb.cbs[ci].included {
                        let mut t = 1;
                        while !pb.zbp.decode(&mut b, cx, cy, t)? {
                            t += 1;
                            if t > 64 {
                                return fail("bad zero-bit-plane count");
                            }
                        }
                        pb.cbs[ci].zbp = t - 1;
                        pb.cbs[ci].included = true;
                    }
                    let np = if b.bit()? == 0 {
                        1
                    } else if b.bit()? == 0 {
                        2
                    } else {
                        let v = b.bits(2)?;
                        if v < 3 {
                            3 + v
                        } else {
                            let v = b.bits(5)?;
                            if v < 31 {
                                6 + v
                            } else {
                                37 + b.bits(7)?
                            }
                        }
                    };
                    let cb = &mut pb.cbs[ci];
                    while b.bit()? == 1 {
                        cb.lblock += 1;
                        if cb.lblock > 32 {
                            return fail("bad code-block length");
                        }
                    }
                    // the new passes may span several codeword segments, each with its own
                    // length (T.800 B.10.7.2)
                    let mut left = np;
                    while left > 0 {
                        if cb.segs.last().is_none_or(|s| s.passes >= s.max) {
                            if cb.segs.len() >= MAX_SEGMENTS {
                                return fail("too many code-block segments");
                            }
                            let max = next_seg_max(style, cb.segs.last().map(|s| s.max));
                            cb.segs.push(Seg {
                                passes: 0,
                                max,
                                data: Vec::new(),
                            });
                        }
                        let si = cb.segs.len() - 1;
                        let seg = &mut cb.segs[si];
                        let k = left.min(seg.max - seg.passes);
                        let nbits = cb.lblock + (31 - k.leading_zeros());
                        if nbits > 31 {
                            return fail("bad code-block length");
                        }
                        let len = b.bits(nbits)? as usize;
                        seg.passes += k;
                        cb.passes = cb.passes.saturating_add(k);
                        left -= k;
                        work.push((bi, ci, si, len));
                    }
                }
            }
        }
        b.align();
        let mut hend = b.p;
        if g.eph && hsrc.len() >= hend + 2 && hsrc[hend] == 0xFF && hsrc[hend + 1] == 0x92 {
            hend += 2;
        }
        #[cfg(test)]
        let hdr_start = hstart;
        if packed.is_some() {
            hpos = hend;
        } else {
            pos = hend;
        }
        for (bi, ci, si, len) in work {
            let end = (pos + len).min(d.len());
            precs[bi].cbs[ci].segs[si]
                .data
                .extend_from_slice(&d[pos.min(end)..end]);
            pos += len;
        }
        #[cfg(test)]
        trace.push((pkt_start, hdr_start, hend, pos));
    }
    #[cfg(test)]
    TRACE.with(|t| t.borrow_mut().push(trace));
    Ok(())
}

// ---------------------------------------------------------------- tier-1

fn zc_ctx(orient: u8, nb: u8) -> usize {
    let (h, v, d) = ((nb & 3) as u32, ((nb >> 2) & 3) as u32, (nb >> 4) as u32);
    if orient == 3 {
        let hv = h + v;
        return match d {
            0 => hv.min(2),
            1 => 3 + hv.min(2),
            2 => {
                if hv == 0 {
                    6
                } else {
                    7
                }
            }
            _ => 8,
        } as usize;
    }
    let (h, v) = if orient == 1 { (v, h) } else { (h, v) };
    (match h {
        2 => 8,
        1 => {
            if v >= 1 {
                7
            } else if d >= 1 {
                6
            } else {
                5
            }
        }
        _ => match v {
            2 => 4,
            1 => 3,
            _ => d.min(2),
        },
    }) as usize
}

fn init_ctx() -> [(u8, u8); 19] {
    let mut c = [(0u8, 0u8); 19];
    c[0].0 = 4;
    c[17].0 = 3;
    c[18].0 = 46;
    c
}

struct T1 {
    sw: usize,
    sig: Vec<u8>,
    neg: Vec<u8>,
    vis: Vec<u8>,
    refd: Vec<u8>,
    nb: Vec<u8>,
    mag: Vec<u32>,
    last: Vec<u8>,
    causal: bool,
}
impl T1 {
    fn set_sig(&mut self, x: usize, y: usize, neg: u8, p: u32) {
        let i = (y + 1) * self.sw + x + 1;
        self.sig[i] = 1;
        self.neg[i] = neg;
        self.mag[i] |= 1 << p;
        self.last[i] = p as u8;
        let sw = self.sw;
        self.nb[i - 1] += 1;
        self.nb[i + 1] += 1;
        if !(self.causal && y.is_multiple_of(4)) {
            self.nb[i - sw] += 4;
            self.nb[i - sw - 1] += 16;
            self.nb[i - sw + 1] += 16;
        }
        self.nb[i + sw] += 4;
        self.nb[i + sw - 1] += 16;
        self.nb[i + sw + 1] += 16;
    }
    fn sign(&self, mq: &mut Mq, cx: &mut [(u8, u8); 19], x: usize, y: usize) -> u8 {
        let i = (y + 1) * self.sw + x + 1;
        let sw = self.sw;
        let c = |j: usize, ok: bool| -> i32 {
            if !ok || self.sig[j] == 0 {
                0
            } else if self.neg[j] == 1 {
                -1
            } else {
                1
            }
        };
        let hc = (c(i - 1, true) + c(i + 1, true)).clamp(-1, 1);
        let vc = (c(i - sw, true) + c(i + sw, !(self.causal && y % 4 == 3))).clamp(-1, 1);
        let (ctx, x) = match (hc, vc) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        };
        mq.decode(&mut cx[ctx]) ^ x
    }
}

/// Raw (bypass) bit reader: bytes after 0xFF carry 7 bits only (bit-stuffing); reading past
/// the segment yields 1 bits like a marker would.
struct RawBits<'a> {
    d: &'a [u8],
    p: usize,
    c: u32,
    ct: u32,
}
impl<'a> RawBits<'a> {
    fn new(d: &'a [u8]) -> Self {
        RawBits {
            d,
            p: 0,
            c: 0,
            ct: 0,
        }
    }
    fn bit(&mut self) -> u8 {
        if self.ct == 0 {
            let b = self.d.get(self.p).copied().unwrap_or(0xFF) as u32;
            if self.c == 0xFF {
                if b > 0x8F {
                    self.c = 0xFF;
                    self.ct = 8;
                } else {
                    self.c = b;
                    self.p += 1;
                    self.ct = 7;
                }
            } else {
                self.c = b;
                if self.p < self.d.len() {
                    self.p += 1;
                }
                self.ct = 8;
            }
        }
        self.ct -= 1;
        ((self.c >> self.ct) & 1) as u8
    }
}

/// Decode one code-block into (magnitude, negative, last-plane) per sample. `segs` are the
/// codeword segments in order; every segment restarts the arithmetic (or raw) decoder, the
/// context states carry over unless the reset style is on.
fn t1_decode(
    w: usize,
    h: usize,
    orient: u8,
    style: u8,
    nbps: u32,
    segs: &[Seg],
) -> (Vec<u32>, Vec<u8>, Vec<u8>) {
    let sw = w + 2;
    let n = sw * (h + 2);
    let mut t = T1 {
        sw,
        sig: vec![0; n],
        neg: vec![0; n],
        vis: vec![0; n],
        refd: vec![0; n],
        nb: vec![0; n],
        mag: vec![0; n],
        last: vec![0; n],
        causal: style & 8 != 0,
    };
    let mut zc = [0usize; 256];
    for (i, z) in zc.iter_mut().enumerate() {
        *z = zc_ctx(orient, i as u8);
    }
    let mut cx = init_ctx();
    let mut p = nbps as i32 - 1;
    let mut kind = 2;
    let mut k = 0u32; // absolute pass index
    'segs: for seg in segs {
        // bypass: from the 11th pass on, sig-prop and mag-ref passes are raw
        let raw = style & 1 != 0 && k >= 10 && kind != 2;
        let mut mq = Mq::new(&seg.data);
        let mut rw = RawBits::new(&seg.data);
        for _ in 0..seg.passes {
            if p < 0 {
                break 'segs;
            }
            let pl = p as u32;
            match kind {
                0 => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            for y in y0..(y0 + 4).min(h) {
                                let i = (y + 1) * sw + x + 1;
                                if t.sig[i] == 0 && t.nb[i] != 0 {
                                    t.vis[i] = 1;
                                    let b = if raw {
                                        rw.bit()
                                    } else {
                                        mq.decode(&mut cx[zc[t.nb[i] as usize]])
                                    };
                                    if b == 1 {
                                        let s = if raw {
                                            rw.bit()
                                        } else {
                                            t.sign(&mut mq, &mut cx, x, y)
                                        };
                                        t.set_sig(x, y, s, pl);
                                    }
                                }
                            }
                        }
                    }
                }
                1 => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            for y in y0..(y0 + 4).min(h) {
                                let i = (y + 1) * sw + x + 1;
                                if t.sig[i] == 1 && t.vis[i] == 0 {
                                    let b = if raw {
                                        rw.bit()
                                    } else {
                                        let c = if t.refd[i] == 1 {
                                            16
                                        } else if t.nb[i] != 0 {
                                            15
                                        } else {
                                            14
                                        };
                                        mq.decode(&mut cx[c])
                                    };
                                    if b == 1 {
                                        t.mag[i] |= 1 << pl;
                                    }
                                    t.last[i] = pl as u8;
                                    t.refd[i] = 1;
                                }
                            }
                        }
                    }
                }
                _ => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            let mut start = y0;
                            if y0 + 4 <= h {
                                let free = (y0..y0 + 4).all(|y| {
                                    let i = (y + 1) * sw + x + 1;
                                    t.sig[i] == 0 && t.vis[i] == 0 && t.nb[i] == 0
                                });
                                if free {
                                    if mq.decode(&mut cx[17]) == 0 {
                                        continue;
                                    }
                                    let r = ((mq.decode(&mut cx[18]) as usize) << 1)
                                        | mq.decode(&mut cx[18]) as usize;
                                    let y = y0 + r;
                                    let s = t.sign(&mut mq, &mut cx, x, y);
                                    t.set_sig(x, y, s, pl);
                                    start = y + 1;
                                }
                            }
                            for y in start..(y0 + 4).min(h) {
                                let i = (y + 1) * sw + x + 1;
                                if t.sig[i] == 0
                                    && t.vis[i] == 0
                                    && mq.decode(&mut cx[zc[t.nb[i] as usize]]) == 1
                                {
                                    let s = t.sign(&mut mq, &mut cx, x, y);
                                    t.set_sig(x, y, s, pl);
                                }
                            }
                        }
                    }
                    t.vis.iter_mut().for_each(|v| *v = 0);
                    if style & 0x20 != 0 {
                        for _ in 0..4 {
                            mq.decode(&mut cx[18]);
                        }
                    }
                }
            }
            if style & 2 != 0 {
                cx = init_ctx();
            }
            k += 1;
            if kind == 2 {
                p -= 1;
                kind = 0;
            } else {
                kind += 1;
            }
        }
    }
    let mut mag = Vec::with_capacity(w * h);
    let mut neg = Vec::with_capacity(w * h);
    let mut last = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let i = (y + 1) * sw + x + 1;
            mag.push(t.mag[i]);
            neg.push(t.neg[i]);
            last.push(t.last[i]);
        }
    }
    (mag, neg, last)
}

// ---------------------------------------------------------------- wavelets

fn mirror(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let period = 2 * (n as isize - 1);
    let mut m = i.rem_euclid(period);
    if m >= n as isize {
        m = period - m;
    }
    m as usize
}

/// Inverse 5/3 lifting on one line; `par` is the parity of the first sample's coordinate.
fn idwt53(a: &mut [f32], par: usize) {
    let n = a.len();
    if n == 1 {
        if par == 1 {
            a[0] = (a[0] as i32 >> 1) as f32; // H/2
        }
        return;
    }
    let mut v: Vec<i32> = a.iter().map(|&x| x as i32).collect();
    // even coordinates are at indices i where (i + par) is even
    let first_even = par & 1;
    let mut i = first_even;
    while i < n {
        let (l, r) = (v[mirror(i as isize - 1, n)], v[mirror(i as isize + 1, n)]);
        v[i] = v[i].wrapping_sub(l.wrapping_add(r).wrapping_add(2) >> 2);
        i += 2;
    }
    let mut i = 1 - first_even;
    while i < n {
        let (l, r) = (v[mirror(i as isize - 1, n)], v[mirror(i as isize + 1, n)]);
        v[i] = v[i].wrapping_add(l.wrapping_add(r) >> 1);
        i += 2;
    }
    for (o, x) in a.iter_mut().zip(v) {
        *o = x as f32;
    }
}

fn idwt97(a: &mut [f32], par: usize) {
    const AL: f32 = -1.586_134_3;
    const BE: f32 = -0.052_980_12;
    const GA: f32 = 0.882_911_1;
    const DE: f32 = 0.443_506_85;
    const K: f32 = 1.230_174_1;
    let n = a.len();
    if n == 1 {
        if par == 1 {
            a[0] *= 0.5;
        }
        return;
    }
    let fe = par & 1;
    for (i, x) in a.iter_mut().enumerate() {
        if (i + par).is_multiple_of(2) {
            *x *= K;
        } else {
            *x /= K;
        }
    }
    let step = |a: &mut [f32], start: usize, c: f32| {
        let mut i = start;
        while i < n {
            let (l, r) = (a[mirror(i as isize - 1, n)], a[mirror(i as isize + 1, n)]);
            a[i] -= c * (l + r);
            i += 2;
        }
    };
    step(a, fe, DE);
    step(a, 1 - fe, GA);
    step(a, fe, BE);
    step(a, 1 - fe, AL);
}

fn idwt_tile_comp(tc: &mut TileComp) -> Vec<f32> {
    let filt: fn(&mut [f32], usize) = if tc.cc.rev { idwt53 } else { idwt97 };
    let mut cur = std::mem::take(&mut tc.res[0].bands[0].data);
    for r in 1..tc.res.len() {
        let prev = &tc.res[r - 1];
        let rs = &tc.res[r];
        let (rx0, ry0) = (rs.x0 as usize, rs.y0 as usize);
        let (w, h) = ((rs.x1 - rs.x0) as usize, (rs.y1 - rs.y0) as usize);
        let (px0, py0) = (prev.x0 as usize, prev.y0 as usize);
        let pw = (prev.x1 - prev.x0) as usize;
        let mut buf = vec![0f32; w * h];
        for y in 0..h {
            let ay = ry0 + y;
            for x in 0..w {
                let ax = rx0 + x;
                let v = match (ax & 1, ay & 1) {
                    (0, 0) => {
                        let (ix, iy) = (ax / 2 - px0, ay / 2 - py0);
                        cur.get(iy * pw + ix).copied().unwrap_or(0.0)
                    }
                    (o, q) => {
                        let bi = if o == 1 && q == 0 {
                            0
                        } else if o == 0 {
                            1
                        } else {
                            2
                        };
                        let b = &rs.bands[bi];
                        let bw = b.x1 - b.x0;
                        let ix = (ax >> 1) - b.x0;
                        let iy = (ay >> 1) - b.y0;
                        b.data.get(iy * bw + ix).copied().unwrap_or(0.0)
                    }
                };
                buf[y * w + x] = v;
            }
        }
        if w > 0 && h > 0 {
            for row in buf.chunks_mut(w) {
                filt(row, rx0 & 1);
            }
            let mut col = vec![0f32; h];
            for x in 0..w {
                for y in 0..h {
                    col[y] = buf[y * w + x];
                }
                filt(&mut col, ry0 & 1);
                for y in 0..h {
                    buf[y * w + x] = col[y];
                }
            }
        }
        cur = buf;
    }
    cur
}

// ---------------------------------------------------------------- codestream

struct Tile {
    params: Option<Params>,
    data: Vec<u8>,
    /// Packed packet headers (PPM/PPT) of the whole tile, in codestream order.
    packed: Vec<u8>,
    has_packed: bool,
    /// A POC in a tile-part header replaces the main header's list for that tile.
    poc_own: bool,
}

/// POC marker segment body (T.800 A.6.6).
fn read_poc(seg: &[u8], ncomp: usize, out: &mut Vec<Poc>) -> R<()> {
    let wide = ncomp >= 257;
    let mut r = Rd { d: seg, p: 0 };
    while r.p < seg.len() {
        let rs = r.u8()?;
        let cs = if wide { r.u16()? } else { r.u8()? };
        let lye = r.u16()?;
        let re = r.u8()?;
        let mut ce = if wide { r.u16()? } else { r.u8()? };
        if ce == 0 {
            ce = if wide { 16384 } else { 256 };
        }
        let prog = r.u8()? as u8;
        if prog > 4 {
            return fail("unknown progression order in POC");
        }
        if out.len() >= MAX_POCS {
            return fail("too many progression order changes");
        }
        out.push(Poc {
            rs,
            cs,
            lye,
            re,
            ce,
            prog,
        });
    }
    Ok(())
}

fn read_cod_body(r: &mut Rd, has_prec: bool) -> R<CompCod> {
    let nl = r.u8()?;
    let xcb = r.u8()? + 2;
    let ycb = r.u8()? + 2;
    let style = r.u8()? as u8;
    let tr = r.u8()?;
    if nl > 32 || xcb > 10 || ycb > 10 || xcb + ycb > 12 {
        return fail("invalid coding style");
    }
    let mut prec = Vec::new();
    for _ in 0..=nl {
        if has_prec {
            let v = r.u8()?;
            prec.push((v & 15, v >> 4));
        } else {
            prec.push((15, 15));
        }
    }
    Ok(CompCod {
        nl,
        xcb,
        ycb,
        style,
        rev: tr == 1,
        prec,
    })
}

fn read_qcd_body(r: &mut Rd, end: usize) -> R<Quant> {
    let s = r.u8()?;
    let style = (s & 31) as u8;
    let guard = s >> 5;
    let mut steps = Vec::new();
    match style {
        0 => {
            while r.p < end {
                steps.push((r.u8()? >> 3, 0));
            }
        }
        1 => {
            let v = r.u16()?;
            steps.push((v >> 11, v & 0x7FF));
        }
        2 => {
            while r.p + 1 < end {
                let v = r.u16()?;
                steps.push((v >> 11, v & 0x7FF));
            }
        }
        _ => return fail("unknown quantisation style"),
    }
    Ok(Quant {
        style,
        guard,
        steps,
    })
}

fn marker_segment<'a>(d: &'a [u8], p: &mut usize) -> R<&'a [u8]> {
    if *p + 2 > d.len() {
        return fail("truncated marker");
    }
    let l = ((d[*p] as usize) << 8) | d[*p + 1] as usize;
    if l < 2 || *p + l > d.len() {
        return fail("bad marker length");
    }
    let s = &d[*p + 2..*p + l];
    *p += l;
    Ok(s)
}

struct Raw {
    siz: Siz,
    comps: Vec<Vec<f32>>, // per component plane (own grid)
    dims: Vec<(usize, usize)>,
}

fn handle_cod_like(mk: u32, seg: &[u8], ncomp: usize, p: &mut Params) -> R<()> {
    let mut r = Rd { d: seg, p: 0 };
    match mk {
        0xFF52 => {
            let scod = r.u8()?;
            let prog = r.u8()? as u8;
            let layers = r.u16()?;
            let mct = r.u8()? as u8;
            let cc = read_cod_body(&mut r, scod & 1 != 0)?;
            if layers == 0 {
                return fail("zero layers");
            }
            p.g = CodG {
                prog,
                layers,
                mct,
                sop: scod & 2 != 0,
                eph: scod & 4 != 0,
            };
            for c in p.cc.iter_mut() {
                *c = cc.clone();
            }
        }
        0xFF53 => {
            let c = if ncomp < 257 { r.u8()? } else { r.u16()? } as usize;
            let scoc = r.u8()?;
            let cc = read_cod_body(&mut r, scoc & 1 != 0)?;
            *p.cc.get_mut(c).ok_or_else(|| JpxError("bad COC".into()))? = cc;
        }
        0xFF5C => {
            let q = read_qcd_body(&mut r, seg.len())?;
            for x in p.q.iter_mut() {
                *x = q.clone();
            }
        }
        0xFF5D => {
            let c = if ncomp < 257 { r.u8()? } else { r.u16()? } as usize;
            let q = read_qcd_body(&mut r, seg.len())?;
            *p.q.get_mut(c).ok_or_else(|| JpxError("bad QCC".into()))? = q;
        }
        _ => {}
    }
    Ok(())
}

fn decode_codestream(d: &[u8]) -> R<Raw> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0x4F {
        return fail("not a JPEG 2000 codestream");
    }
    let mut p = 2usize;
    let mut siz: Option<Siz> = None;
    let mut main: Option<Params> = None;
    let mut tiles: Vec<Tile> = Vec::new();
    let mut have_cod = false;
    let mut have_qcd = false;
    let mut ppm_segs: Vec<(u8, Vec<u8>)> = Vec::new();
    // PPM: one (length, headers) record per tile-part, spread over the marker segments
    let mut ppm_stream: Option<(Vec<u8>, usize)> = None;
    loop {
        if p + 2 > d.len() {
            break;
        }
        let mk = ((d[p] as u32) << 8) | d[p + 1] as u32;
        p += 2;
        match mk {
            0xFFD9 => break,
            0xFF51 => {
                let seg = marker_segment(d, &mut p)?;
                let mut r = Rd { d: seg, p: 0 };
                r.u16()?; // Rsiz
                let (x1, y1, x0, y0) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let (tw, th, tx0, ty0) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let n = r.u16()? as usize;
                if n == 0 || n > MAX_COMPONENTS {
                    return fail("bad component count");
                }
                let mut comps = Vec::new();
                for _ in 0..n {
                    let s = r.u8()?;
                    let dx = r.u8()? as u64;
                    let dy = r.u8()? as u64;
                    if dx == 0 || dy == 0 {
                        return fail("bad subsampling");
                    }
                    comps.push(CompInfo {
                        depth: (s & 0x7F) + 1,
                        signed: s & 0x80 != 0,
                        dx,
                        dy,
                    });
                }
                if x1 <= x0 || y1 <= y0 || tw == 0 || th == 0 || tx0 > x0 || ty0 > y0 {
                    return fail("bad image geometry");
                }
                if tx0 + tw <= x0 || ty0 + th <= y0 {
                    return fail("bad tile geometry");
                }
                if (x1 - x0).saturating_mul(y1 - y0).saturating_mul(n as u64) > MAX_SAMPLES {
                    return fail("JPEG 2000 image too large");
                }
                if comps.iter().any(|c| c.depth > 16) {
                    return fail("JPEG 2000 samples deeper than 16 bits are not supported");
                }
                let ntx = ceil_div(x1 - tx0, tw);
                let nty = ceil_div(y1 - ty0, th);
                if ntx * nty > 65535 {
                    return fail("too many tiles");
                }
                tiles = (0..ntx * nty)
                    .map(|_| Tile {
                        params: None,
                        data: Vec::new(),
                        packed: Vec::new(),
                        has_packed: false,
                        poc_own: false,
                    })
                    .collect();
                let dummy = CompCod {
                    nl: 0,
                    xcb: 6,
                    ycb: 6,
                    style: 0,
                    rev: true,
                    prec: vec![(15, 15)],
                };
                main = Some(Params {
                    g: CodG {
                        prog: 0,
                        layers: 1,
                        mct: 0,
                        sop: false,
                        eph: false,
                    },
                    cc: vec![dummy; n],
                    q: vec![
                        Quant {
                            style: 0,
                            guard: 0,
                            steps: vec![]
                        };
                        n
                    ],
                    poc: Vec::new(),
                });
                siz = Some(Siz {
                    x1,
                    y1,
                    x0,
                    y0,
                    tw,
                    th,
                    tx0,
                    ty0,
                    comps,
                });
            }
            0xFF52 | 0xFF53 | 0xFF5C | 0xFF5D => {
                let seg = marker_segment(d, &mut p)?;
                let (Some(s), Some(m)) = (&siz, &mut main) else {
                    return fail("marker before SIZ");
                };
                if mk == 0xFF52 {
                    have_cod = true;
                }
                if mk == 0xFF5C {
                    have_qcd = true;
                }
                handle_cod_like(mk, seg, s.comps.len(), m)?;
            }
            0xFF5F => {
                let seg = marker_segment(d, &mut p)?;
                let (Some(s), Some(m)) = (&siz, &mut main) else {
                    return fail("marker before SIZ");
                };
                read_poc(seg, s.comps.len(), &mut m.poc)?;
            }
            0xFF60 => {
                let seg = marker_segment(d, &mut p)?;
                let (&z, body) = seg
                    .split_first()
                    .ok_or_else(|| JpxError("bad PPM".into()))?;
                ppm_segs.push((z, body.to_vec()));
            }
            0xFF90 => {
                let sot_start = p - 2;
                let seg = marker_segment(d, &mut p)?;
                let (Some(s), Some(m)) = (&siz, &main) else {
                    return fail("SOT before SIZ");
                };
                if !have_cod || !have_qcd {
                    return fail("missing COD or QCD");
                }
                let mut r = Rd { d: seg, p: 0 };
                let isot = r.u16()? as usize;
                let psot = r.u32()? as usize;
                let tile = tiles
                    .get_mut(isot)
                    .ok_or_else(|| JpxError("tile index out of range".into()))?;
                let end = if psot == 0 {
                    d.len()
                } else {
                    (sot_start + psot).min(d.len())
                };
                if end < p {
                    return fail("bad tile-part length");
                }
                let mut tp = tile.params.take().unwrap_or_else(|| m.clone());
                if !ppm_segs.is_empty() || ppm_stream.is_some() {
                    if ppm_stream.is_none() {
                        ppm_segs.sort_by_key(|x| x.0);
                        let all: Vec<u8> =
                            ppm_segs.iter().flat_map(|x| x.1.iter().copied()).collect();
                        ppm_segs = Vec::new();
                        ppm_stream = Some((all, 0));
                    }
                    let (all, at) = ppm_stream.as_mut().unwrap();
                    if *at + 4 > all.len() {
                        return fail("damaged PPM packed headers");
                    }
                    let n = u32::from_be_bytes([all[*at], all[*at + 1], all[*at + 2], all[*at + 3]])
                        as usize;
                    *at += 4;
                    if n > all.len() - *at {
                        return fail("damaged PPM packed headers");
                    }
                    tile.packed.extend_from_slice(&all[*at..*at + n]);
                    tile.has_packed = true;
                    *at += n;
                }
                let mut ppt: Vec<(u8, Vec<u8>)> = Vec::new();
                // tile-part header
                loop {
                    if p + 2 > end {
                        return fail("tile-part without SOD");
                    }
                    let mk2 = ((d[p] as u32) << 8) | d[p + 1] as u32;
                    p += 2;
                    if mk2 == 0xFF93 {
                        break;
                    }
                    let seg = marker_segment(d, &mut p)?;
                    match mk2 {
                        0xFF5F => {
                            if !tile.poc_own {
                                tile.poc_own = true;
                                tp.poc.clear();
                            }
                            read_poc(seg, s.comps.len(), &mut tp.poc)?;
                        }
                        0xFF61 => {
                            let (&z, body) = seg
                                .split_first()
                                .ok_or_else(|| JpxError("bad PPT".into()))?;
                            ppt.push((z, body.to_vec()));
                        }
                        _ => handle_cod_like(mk2, seg, s.comps.len(), &mut tp)?,
                    }
                }
                if !ppt.is_empty() {
                    if ppm_stream.is_some() {
                        return fail("both PPM and PPT present");
                    }
                    ppt.sort_by_key(|x| x.0);
                    for (_, b) in &ppt {
                        tile.packed.extend_from_slice(b);
                    }
                    tile.has_packed = true;
                }
                tile.params = Some(tp);
                let body_end = end.max(p);
                // a trailing EOC inside the last part is not tile data
                let mut be = body_end;
                if be >= p + 2 && d[be - 2] == 0xFF && d[be - 1] == 0xD9 && psot == 0 {
                    be -= 2;
                }
                tile.data.extend_from_slice(&d[p..be]);
                p = body_end;
            }
            0xFF4F | 0xFF93 => return fail("unexpected marker"),
            _ => {
                if mk >> 8 != 0xFF {
                    return fail("damaged codestream");
                }
                marker_segment(d, &mut p)?;
            }
        }
    }
    let siz = siz.ok_or_else(|| JpxError("missing SIZ".into()))?;
    let main = main.unwrap();
    if tiles
        .iter()
        .all(|t| t.params.is_none() || (t.data.is_empty() && t.packed.is_empty()))
    {
        return fail("JPEG 2000 codestream carries no image data");
    }
    let nc = siz.comps.len();
    // component planes on their own grids
    let mut dims = Vec::new();
    let mut planes = Vec::new();
    for c in &siz.comps {
        let w = (ceil_div(siz.x1, c.dx) - ceil_div(siz.x0, c.dx)) as usize;
        let h = (ceil_div(siz.y1, c.dy) - ceil_div(siz.y0, c.dy)) as usize;
        if w == 0 || h == 0 {
            return fail("empty JPEG 2000 component");
        }
        dims.push((w, h));
        planes.push(vec![0f32; w * h]);
    }
    let ntx = ceil_div(siz.x1 - siz.tx0, siz.tw);
    for (ti, tile) in tiles.iter().enumerate() {
        let Some(tp) = &tile.params else { continue };
        let (pi, qi) = (ti as u64 % ntx, ti as u64 / ntx);
        let t = (
            (siz.tx0 + pi * siz.tw).max(siz.x0),
            (siz.ty0 + qi * siz.th).max(siz.y0),
            (siz.tx0 + (pi + 1) * siz.tw).min(siz.x1),
            (siz.ty0 + (qi + 1) * siz.th).min(siz.y1),
        );
        let _ = &main;
        let packed = tile.has_packed.then_some(&tile.packed[..]);
        decode_tile(&siz, tp, &tile.data, packed, t, &mut planes, &dims)?;
    }
    let _ = nc;
    Ok(Raw {
        siz,
        comps: planes,
        dims,
    })
}

/// (component, resolution, band, x, y, w, h, band width) of a decoded code-block.
type Place = (usize, usize, usize, usize, usize, usize, usize, usize);

fn decode_tile(
    siz: &Siz,
    tp: &Params,
    data: &[u8],
    packed: Option<&[u8]>,
    t: (u64, u64, u64, u64),
    planes: &mut [Vec<f32>],
    dims: &[(usize, usize)],
) -> R<()> {
    let nc = siz.comps.len();
    let mut tcs = Vec::with_capacity(nc);
    let mut total_prec = 0u64;
    for c in 0..nc {
        tcs.push(build_tile_comp(
            &tp.cc[c],
            &tp.q[c],
            &siz.comps[c],
            t,
            &mut total_prec,
        )?);
    }
    let order = progression(&tp.g, &tcs, &siz.comps, t, &tp.poc)?;
    read_packets(&tp.g, &mut tcs, &order, data, packed)?;
    // tier-1 in parallel over all code-blocks
    struct Job<'a> {
        tc: usize,
        r: usize,
        b: usize,
        cb: &'a Cb,
        orient: u8,
        mb: u32,
        delta: f32,
        rev: bool,
        style: u8,
        bx0: usize,
        by0: usize,
        bw: usize,
    }
    let mut jobs = Vec::new();
    for (ci, tc) in tcs.iter().enumerate() {
        if tc.cc.style & 0xC0 != 0 {
            return fail(
                "high-throughput / extended JPEG 2000 code-block styles are not supported",
            );
        }
        for (ri, rs) in tc.res.iter().enumerate() {
            for precs in &rs.precs {
                for (bi, pb) in precs.iter().enumerate() {
                    let b = &rs.bands[bi];
                    for cb in &pb.cbs {
                        if cb.included
                            && cb.passes > 0
                            && cb.segs.iter().any(|s| !s.data.is_empty())
                        {
                            jobs.push(Job {
                                tc: ci,
                                r: ri,
                                b: bi,
                                cb,
                                orient: b.orient,
                                mb: b.mb,
                                delta: b.delta,
                                rev: tc.cc.rev,
                                style: tc.cc.style,
                                bx0: b.x0,
                                by0: b.y0,
                                bw: b.x1 - b.x0,
                            });
                        }
                    }
                }
            }
        }
    }
    let work: u64 = jobs
        .iter()
        .map(|j| {
            let n = ((j.cb.x1 - j.cb.x0) * (j.cb.y1 - j.cb.y0)) as u64;
            n * (j.cb.passes as u64).min(3 * j.mb as u64 + 1)
        })
        .sum();
    if work > MAX_T1_WORK {
        return fail("JPEG 2000 tile is too expensive to decode");
    }
    let results: Vec<Vec<f32>> = jobs
        .par_iter()
        .map(|j| {
            let (w, h) = (j.cb.x1 - j.cb.x0, j.cb.y1 - j.cb.y0);
            let nbps = j.mb.saturating_sub(j.cb.zbp);
            if nbps == 0 || nbps > 31 || w == 0 || h == 0 {
                return vec![0f32; w * h];
            }
            let (mag, neg, last) = t1_decode(w, h, j.orient, j.style, nbps, &j.cb.segs);
            mag.iter()
                .zip(neg.iter())
                .zip(last.iter())
                .map(|((&m, &n), &l)| {
                    if m == 0 {
                        return 0.0;
                    }
                    let v = if j.rev {
                        let add = if l > 0 { (1u32 << l) >> 1 } else { 0 };
                        (m + add) as f32
                    } else {
                        (m as f32 + (1u64 << l) as f32 * 0.5) * j.delta
                    };
                    if n == 1 {
                        -v
                    } else {
                        v
                    }
                })
                .collect()
        })
        .collect();
    let places: Vec<Place> = jobs
        .iter()
        .map(|j| {
            (
                j.tc,
                j.r,
                j.b,
                j.cb.x0 - j.bx0,
                j.cb.y0 - j.by0,
                j.cb.x1 - j.cb.x0,
                j.cb.y1 - j.cb.y0,
                j.bw,
            )
        })
        .collect();
    drop(jobs);
    for (res, (tci, r, b, ox, oy, w, h, bw)) in results.into_iter().zip(places) {
        let band = &mut tcs[tci].res[r].bands[b];
        for y in 0..h {
            let dst = (oy + y) * bw + ox;
            band.data[dst..dst + w].copy_from_slice(&res[y * w..(y + 1) * w]);
        }
    }
    // inverse wavelet
    let mut comp_data: Vec<Vec<f32>> = tcs.par_iter_mut().map(idwt_tile_comp).collect();
    // inverse multiple component transform
    if tp.g.mct == 1 && nc >= 3 {
        let n = comp_data[0].len();
        if comp_data[1].len() != n || comp_data[2].len() != n {
            return fail("colour transform needs equally sized components");
        }
        let (a, rest) = comp_data.split_at_mut(1);
        let (b, c) = rest.split_at_mut(1);
        let (y0, y1, y2) = (&mut a[0], &mut b[0], &mut c[0]);
        if tcs[0].cc.rev {
            for i in 0..n {
                let (a0, a1, a2) = (y0[i] as i32, y1[i] as i32, y2[i] as i32);
                let g = a0.wrapping_sub(a1.wrapping_add(a2) >> 2);
                y0[i] = a2.wrapping_add(g) as f32;
                y1[i] = g as f32;
                y2[i] = a1.wrapping_add(g) as f32;
            }
        } else {
            for i in 0..n {
                let (a0, a1, a2) = (y0[i], y1[i], y2[i]);
                y0[i] = a0 + 1.402 * a2;
                y1[i] = a0 - 0.344_13 * a1 - 0.714_14 * a2;
                y2[i] = a0 + 1.772 * a1;
            }
        }
    }
    // level shift and placement
    for (c, cd) in comp_data.iter().enumerate() {
        let ci = &siz.comps[c];
        let tc = &tcs[c];
        let shift = (1u32 << (ci.depth - 1)) as f32;
        let maxv = ((1u32 << ci.depth) - 1) as f32;
        let (w, h) = ((tc.x1 - tc.x0) as usize, (tc.y1 - tc.y0) as usize);
        let ox = (tc.x0 - ceil_div(siz.x0, ci.dx)) as usize;
        let oy = (tc.y0 - ceil_div(siz.y0, ci.dy)) as usize;
        let (pw, ph) = dims[c];
        let _ = ci.signed;
        for y in 0..h {
            if oy + y >= ph {
                break;
            }
            for x in 0..w {
                if ox + x >= pw {
                    break;
                }
                let v = (cd[y * w + x] + shift).round().clamp(0.0, maxv);
                planes[c][(oy + y) * pw + ox + x] = v;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- public entry

/// Decode a JP2 file or raw codestream into 8-bit gray (1 channel) or RGB (3 channels).
pub fn decode(data: &[u8]) -> R<Image> {
    let (cs, enumcs) = unwrap_jp2(data)?;
    let raw = decode_codestream(cs)?;
    let siz = &raw.siz;
    let (w, h) = ((siz.x1 - siz.x0) as usize, (siz.y1 - siz.y0) as usize);
    let nc = raw.comps.len();
    let colour = match (enumcs, nc) {
        (Some(17), _) | (_, 1) | (_, 2) => 1,
        (Some(12), n) if n >= 4 => 4,
        _ => 3,
    };
    if nc < colour {
        return fail("fewer JPEG 2000 components than the colour space needs");
    }
    let mut out = vec![0u8; w * h * colour];
    for c in 0..colour {
        let ci = &siz.comps[c];
        let (pw, ph) = raw.dims[c];
        let (cx0, cy0) = (ceil_div(siz.x0, ci.dx), ceil_div(siz.y0, ci.dy));
        let depth = ci.depth;
        for y in 0..h {
            let sy = (((siz.y0 + y as u64) / ci.dy).saturating_sub(cy0) as usize).min(ph - 1);
            for x in 0..w {
                let sx = (((siz.x0 + x as u64) / ci.dx).saturating_sub(cx0) as usize).min(pw - 1);
                let v = raw.comps[c][sy * pw + sx] as u32;
                let v8 = if depth > 8 {
                    v >> (depth - 8)
                } else if depth < 8 {
                    v * 255 / ((1 << depth) - 1)
                } else {
                    v
                };
                out[(y * w + x) * colour + c] = v8 as u8;
            }
        }
    }
    if enumcs == Some(18) && colour == 3 {
        for px in out.chunks_mut(3) {
            let (y, cb, cr) = (px[0] as f32, px[1] as f32 - 128.0, px[2] as f32 - 128.0);
            px[0] = (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
            px[1] = (y - 0.344_14 * cb - 0.714_14 * cr)
                .round()
                .clamp(0.0, 255.0) as u8;
            px[2] = (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8;
        }
    }
    if colour == 4 {
        let mut rgb = Vec::with_capacity(w * h * 3);
        for px in out.chunks(4) {
            let k = 255 - px[3] as u32;
            for &c in &px[..3] {
                rgb.push(((255 - c as u32) * k / 255) as u8);
            }
        }
        return Ok(Image {
            w,
            h,
            nc: 3,
            data: rgb,
        });
    }
    Ok(Image {
        w,
        h,
        nc: colour,
        data: out,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn jp2_box_length_overflow_is_an_error_not_a_panic() {
        let mut d = vec![0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a];
        d.extend_from_slice(&[0, 0, 0, 1, b'j', b'p', b'2', b'h']);
        d.extend_from_slice(&0xFFFF_FFFF_FFFF_FFF4u64.to_be_bytes());
        d.extend_from_slice(&[0u8; 32]);
        assert!(super::decode(&d).is_err());
    }

    use super::*;

    macro_rules! fx {
        ($n:literal, $e:literal) => {
            (
                $n,
                &include_bytes!(concat!("../tests/jpx/", $n, ".", $e))[..],
                &include_bytes!(concat!("../tests/jpx/", $n, ".raw"))[..],
            )
        };
    }

    /// Streams written by OpenJPEG (via Pillow) and the pixels OpenJPEG decodes from them.
    fn cases() -> Vec<(&'static str, &'static [u8], &'static [u8], u8)> {
        let c = [
            (fx!("gray_lossless", "j2k"), 0),
            (fx!("rgb_lossless_rct", "j2k"), 0),
            (fx!("rgb_lossless_nomct", "j2k"), 0),
            (fx!("rgb_lossy_ict", "j2k"), 2),
            (fx!("gray_lossy", "j2k"), 2),
            (fx!("tiled", "jp2"), 0),
            (fx!("tiled_lossy", "j2k"), 2),
            (fx!("rpcl", "j2k"), 0),
            (fx!("pcrl", "j2k"), 0),
            (fx!("cprl", "j2k"), 0),
            (fx!("rlcp", "j2k"), 0),
            (fx!("layers_lrcp", "j2k"), 2),
            (fx!("cb16", "j2k"), 0),
            (fx!("res1", "j2k"), 0),
        ];
        c.into_iter().map(|((n, a, b), t)| (n, a, b, t)).collect()
    }

    #[test]
    fn matches_openjpeg_output() {
        for (name, stream, want, tol) in cases() {
            let img = decode(stream).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((img.w, img.h), (67, 45), "{name}");
            assert_eq!(img.data.len(), want.len(), "{name}");
            let worst = img
                .data
                .iter()
                .zip(want)
                .map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs())
                .max()
                .unwrap();
            assert!(worst <= tol as u32, "{name}: worst difference {worst}");
        }
    }

    /// `LEGATOROCR_JPX_FILE=big.jp2 cargo test --release -- --ignored jpx_timing`
    #[test]
    #[ignore]
    fn jpx_timing() {
        let Ok(f) = std::env::var("LEGATOROCR_JPX_FILE") else {
            return;
        };
        let t = std::time::Instant::now();
        let img = decode(&std::fs::read(f).unwrap()).unwrap();
        eprintln!("{}x{}x{} in {:?}", img.w, img.h, img.nc, t.elapsed());
    }

    /// Differential sweep: `LEGATOROCR_JPX_DIR` holds `NAME.j2k` (or .jp2), `NAME.raw`
    /// (OpenJPEG's decoded 8-bit samples) and `NAME.txt` ("w h channels tolerance").
    #[test]
    #[ignore]
    fn jpx_differential() {
        let Ok(dir) = std::env::var("LEGATOROCR_JPX_DIR") else {
            return;
        };
        let mut bad = Vec::new();
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "txt") {
                continue;
            }
            n += 1;
            let meta = std::fs::read_to_string(&p).unwrap();
            let v: Vec<usize> = meta
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            let stem = p.with_extension("");
            let stream = ["j2k", "jp2"]
                .iter()
                .find_map(|x| std::fs::read(stem.with_extension(x)).ok())
                .unwrap();
            let want = std::fs::read(stem.with_extension("raw")).unwrap();
            match decode(&stream) {
                Err(e) => bad.push(format!("{}: error {e}", stem.display())),
                Ok(img) => {
                    if (img.w, img.h, img.nc) != (v[0], v[1], v[2]) || img.data.len() != want.len()
                    {
                        bad.push(format!(
                            "{}: shape {}x{}x{}",
                            stem.display(),
                            img.w,
                            img.h,
                            img.nc
                        ));
                        continue;
                    }
                    let worst = img
                        .data
                        .iter()
                        .zip(&want)
                        .map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs())
                        .max()
                        .unwrap_or(0) as usize;
                    if worst > v[3] {
                        bad.push(format!("{}: worst {worst} > {}", stem.display(), v[3]));
                    }
                }
            }
        }
        eprintln!("{n} streams, {} bad", bad.len());
        for b in bad.iter().take(30) {
            eprintln!("{b}");
        }
        assert!(bad.is_empty());
    }

    /// Hand-made codestream: one component, 8 bit, with the given geometry and coding style.
    fn crafted(
        (w, h, tw, th): (u32, u32, u32, u32),
        nl: u8,
        prec: Option<u8>,
        cb: (u8, u8),
        data: &[u8],
    ) -> Vec<u8> {
        let mut d = vec![0xFF, 0x4F, 0xFF, 0x51];
        d.extend(41u16.to_be_bytes());
        d.extend(0u16.to_be_bytes());
        for v in [w, h, 0, 0, tw, th, 0, 0] {
            d.extend(v.to_be_bytes());
        }
        d.extend(1u16.to_be_bytes());
        d.extend([7, 1, 1]);
        d.extend([0xFF, 0x52]);
        let npr = if prec.is_some() { nl as usize + 1 } else { 0 };
        d.extend((12 + npr as u16).to_be_bytes());
        d.push(prec.is_some() as u8);
        d.extend([0, 0, 1, 0]);
        d.extend([nl, cb.0, cb.1, 0, 1]);
        if let Some(p) = prec {
            d.extend(vec![p; nl as usize + 1]);
        }
        d.extend([0xFF, 0x5C]);
        d.extend((3 + 3 * nl as u16 + 1).to_be_bytes());
        d.push(2 << 5);
        d.extend(vec![8 << 3; 3 * nl as usize + 1]);
        d.extend([0xFF, 0x90, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        d.extend([0xFF, 0x93]);
        d.extend_from_slice(data);
        d.extend([0xFF, 0xD9]);
        d
    }

    #[test]
    fn hostile_headers_fail_fast_and_small() {
        let t = std::time::Instant::now();
        let junk = [0x55u8; 64];
        // absurd canvas, tiny tiles, tiny precincts, many levels, no data at all
        let cases = [
            crafted((100_000, 100_000, 100_000, 100_000), 5, None, (4, 4), &junk),
            crafted((9_000, 9_000, 9_000, 9_000), 5, Some(0x11), (0, 0), &junk),
            crafted((9_000, 9_000, 9_000, 9_000), 32, None, (4, 4), &junk),
            crafted((9_000, 9_000, 9_000, 9_000), 5, None, (4, 4), &[]),
            crafted((3, 3, 1, 1), 32, None, (4, 4), &junk),
            crafted((1, 1, 1, 1), 0, None, (0, 0), &junk),
            crafted((u32::MAX, u32::MAX, 1, 1), 1, None, (4, 4), &junk),
        ];
        for c in &cases {
            let _ = decode(c);
        }
        assert!(
            t.elapsed().as_secs() < 20,
            "hostile headers took {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn damaged_streams_do_not_panic() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let iters: usize = std::env::var("LEGATOROCR_FUZZ_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        for (_, stream, _, _) in cases() {
            for _ in 0..iters {
                let mut d = stream.to_vec();
                for _ in 0..1 + rnd() % 6 {
                    // half of the hits go to the marker segments at the front
                    let span = if rnd() % 2 == 0 {
                        d.len().min(160)
                    } else {
                        d.len()
                    };
                    let i = (rnd() as usize) % span;
                    match rnd() % 3 {
                        0 => d[i] = rnd() as u8,
                        1 => d[i] ^= 1 << (rnd() % 8),
                        _ => d.truncate(i.max(8)),
                    }
                    if d.len() < 8 {
                        break;
                    }
                }
                let _ = decode(&d);
            }
        }
    }

    // ------------------------------------------------------------ code-block styles, POC, PPM/PPT

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// The 32 x 24 grey image behind the vectors (also what OpenJPEG decoded from them).
    fn vector_image() -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..24usize {
            for x in 0..32usize {
                v.push(((x * 7 + y * 13 + ((x * y * 31 + x * 5 + y * 17) % 23) * 3) % 256) as u8);
            }
        }
        v
    }

    /// The code-block style byte of the COD marker.
    fn cod_style(cs: &[u8]) -> u8 {
        let i = cs.windows(2).position(|w| w == [0xFF, 0x52]).unwrap();
        cs[i + 12]
    }

    /// Lossless 5/3 streams written by OpenJPEG 2.5 (libopenjp2 through glymur) with the given
    /// code-block style byte, 16 x 16 code-blocks; the decoded image is `vector_image()`.
    /// (name, style, hex)
    const VECTORS: &[(&str, u8, &str)] = &[
        ("bypass", 1, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000001000102020101ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000003460001ff93df855f45a2c9648066bc0b664d846fb2f1e020fcf7fbc9fa1281631990f1a0dabcacb6fc1d42f8bfff0b1474075975ae5e768fda517622af0918266804ef0d6afd599157225d00eb9aefa9996ef98292b185f7a93c276ddb41663340d49d33c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff275967ad32997d90c17729eeb6ff0f90fdc023f6e24225ad5ff7fbd2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ffd2a15ff7fcfc299d2e164b253f0a7f4b8582c94fcf22c02644c898132203da925a962724c553a4cfecee73d28735c375460439fab982d3187741f3a37fce90b8583ccaff509eee0bc3752c571c0143fae9cb5a2b209ff7c9d50d2be04521f755820dc2b018fc44e3e7fcf68579d16607e4087ccd69ff82b86427a0685bc41148aa0c0a03e3ab2ff7fcabe28be9d54bdce414918bbac66161e3404dbb8d5a2bd546bbcf965ff7f08f12894c952489627ce0ee2a60091442ed19c418232d1a92aff7f401a3d82f581a01c091e62505206ba8d497bce7f5220a550aaff7f46f30fd9f143e729496b15466869f1ab5e6e0f3cc6046849e4435c363d79a1ba5dc6d8f2c06fafdc0d0e754b9802aa829f505c69a82752456d744eb93f26f5f79b11a9ee801e50f0c9579ee690ff7fc7937c342f9ef1bb68d5b5f91d0a1a999018b6c6eb264399f1b2052b32ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f033805507a256869418939256c45e0a811e06e1338520c78ff7f06252950c8a7813a4b39897207944c2b46b6e4d56758b70baaff7f4e5c83a600405d90eb9a86841edef3a2f031fa560e60720167b6ba79298fc0c5d3babcf9fdfb48677c791297f5e45048d60b0fb2a55cff7bb0791bc800a8d8c9be322ea213737d2fff2aa7d3f2ebff72b1fef18ffa1ca90280186080439e0a280e83000615ff7f0d8000000000049b3b048022434640000002400000000058d5ff7f000004040000266dec1200c4869c8480000c8002000000b12aff7f000004aa474d2be1c48b495146a7a965c4d4cd4772a3da68ff7f000007ddbef664bb5c50a5aa75ba4cdefb77b3feff6e3ad52aff7fffd9"),
        ("termall", 4, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000001000102020401ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000003780001ff93df862b4308284a988b5113020e441c0866bc0b664fb8d3f65e3c041f9eff3c9fa1277f0613321e341b579596df83a85f17ff6f993363277da04e5127db2b0dc15f083f249e6804ef0d6afd597fe53f60f242036b4b61a3dc8f1ce6292b185f7a93c276ddb41663340d49d33fff7fe80fc1f9f88a9af33848e0a814c438b7f98c32c1f62160bf6ba5ff7f4920b5df1ec3a15fca21ebc52f3f2c2e1128a537af73ad336a0f911bff7fcb42b536a17dd18fe3a5f4d1842f3dbb69d23b3e191041b84827ff7f7f594920db989ec4726c44dfdf800902939e5d63bd4a337f80ff7fcfc31ce12b469aa0af7895410ae216053f0ca588492a34713e0245442c083914fcf1cc132484d14d36088f10a2215c42a8803da925a962724c53ce4cfecee73f27d0bcb3253fc14f0fcc4af638beb637676b7f445f7f25e554ccb8b593b87f589d99b81a5f3aefab272d55d11adffe80dbd578dfdcb374a2c4a6644f21a7e5013f92b756bac0174a50092c92646761871ea08bd153290ab436f68d3d5e5ebbff7fb2955afdfb992ce6b4bffcdc4253b02a496b3e9b0fcb1480b82be308cab7ff7f919f190af29a806f11b07b337f6a1bd3fafcbf82381509f447ff7fec7f3748f32426e615b386b05f9fb5b70b113a03b59bbda64a7fff7f46f30fd9f143e729496f1f359a1a7c6ad79b83cf3fb5dffc519e73a2028f39fa113f54e62bcc0703c703484a4f702d024d04ee9c7849302b24c2daba72cc340530ef3a5dd3ff7e6b1f599e9579ee691fbc7f40b08411443d0519853899810504479af7a7ad9e0ca83bc93c81f568577ddfff7f87b334de4920127f99d39bf73e56bb0a0fe34f213396ff403cd6e8109fff7f015f9cc078e09d216d5aad5179056cc9953ccfa785bfb3385feaff7fcf7ebb6a3155a5452c056cb71652f782e6fc472b6bccdd09021fff7f4e5c83a600405f32c35cd4341f7f9f60a4305f5f3347b26c4dbbd6e19f22a4a63f41d4b2df4988f9fd08621c7ea855898346c877a6880f5c8ad072bbaf151f2ad60a8d8fedc4451d64dfac97a84f059e9dfc79bc984ccd0b0b71929c3f9d4cc884658b76e8e786cf31ff7f578a80bfff250d8cf20f878a805ccf06afa75fff7f89cbc39cf1351f921a9eaa86fcf815f409fd5fff7fff7f25d98312a5eede7a145beb66726bb7c12c6ce25edb371fff7fff7fd10b5c8c454fbcc8aca41f49d0bd800eb75c73674bff7fffd9"),
        ("bypass_termall", 5, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000001000102020501ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a00000000037d0001ff93df862b4308284a988b510ee20e041c0866bc0b664fb8d3f65e3c041f9eff3c9fa1277f0613321e341b579596df83a85f17ff6f993363277da04e5127db2b0dc15f083f249e6804ef0d6afd597fe53f60f242036b4b61a3dc8f1ce6292b185f7a93c276ddb41663340d49d33fff7fc9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff19d659eb4ca65f64305dca7badbfc3e43f7008fdb890896b5ff7fb5d2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ff6950ff7fcfc31ce12b469aa0af7893310ae215c53f0ca588492a34713de245041c083814fcf1cc132484d14d27089510ec21d843b0803da925a962724c53ce4cfecee73f27d0bcb3253fc14f0fcc4af638beb637676b7f445f7f25e554ccb8b593b87f589d99b81a5f3aefab272d55d11adffe80dbd578dfdcb374a2c4a6644f21a7e5013fcf68579d16607e4087ccd69ff82b951909e81a16f104522a830280f8eacaff7fcabe28be9d54bdce4514918bbac66161e3404dbb8d5a2bd546bbcf96ff7f08f0944a64a9244b13e70771530048a21768ce20c11968d495ff7f401568f60bd606807024798941481aea3525ef39fd48829542ff7f46f30fd9f143e729496f1f359a1a7c6ad79b83cf3fb5dffc519e73a2028f39fa113f54e62bcc0703c703484a4f702d024d04ee9c7849302b24c2daba72cc340530ef3a5dd3ff7e6b1f599e9579ee691fbc7fc7937c342f9ef1bb68d5b5f91d0a2a353320316d8dd64c8733e3640a5665ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f01ce01541e895a1a50624e495b11782a04781b84ce14831e15ff7f061294a86453c09d259cc4b903ca2615a35b726ab3ac5b85d5ff7f4e5c83a600405f32c35cd4341f7f9f60a4305f5f3347b26c4dbbd6e19f22a4a63f41d4b2df4988f9fd08621c7ea855898346c877a6880f5c8ad072bbaf151f2ad60a8d8fedc4451d64df737d2fff2aa7d3f2ebff72b1fef18ffa1ca940a006182010e7828a03a0c00185ff7f0d800015000000126cec1200890d1900000009000000000163ff7f0000040200001336f6090062434e4240000640010000005895ff7f0000029548e9a57c3891692a28d4f52cb89a99a8ee547b4d0aff7f000002fbb7decc976b8a14b54eb7499bdf6ef67fdffb8eb54aff7fffd9"),
        ("all_styles", 63, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000001000102023f01ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000003800001ff93df86338308a8aa984b508ee10e021c0466bc0b664ecbcdfa865f076b07d2080d93687bd94c10e0c3397943e231b2e41a7e6d629f6e50ff3b1790ed8a22fd35c4e54d9157be4eb4b3f227f78de7f2dd90134156662ab1105264cfa6e7f4b473af0a0e67bfe6ccfd4c5e053a8e63b40d49d33635c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220ab67d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5b6f19d659eb4ca65f64305dca7badbfc3e43f7008fdb890896b5b6b5d2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ff6950b6cfc321013386ab1ccf7853308ae115c33f0cb508a92a72823de145021c04380cfcf1cc13a885c94f27049508ec11d823b0403da925a962724ce3261eb1285ed62f9420e8617972785395fecd152980b630025f3c6f65af8dae9b03a720dec8f9810efb274b65bd53c7fac926933450b0a720273b4b8e34464a7b86734a939c55710760ba4a459acf68579d16607e4087ccd69ff82b951909e81a16f104522a830280f8eacab6cabe28be9d54bdce4514918bbac66161e3404dbb8d5a2bd546bbcf96b608f0944a64a9244b13e70771530048a21768ce20c11968d495b6401568f60bd606807024798941481aea3525ef39fd48829542b646f30fd9f143e729496c4625dffaa2cc508c33f134b5dc388b9275fe9fdb5d12564a14713c459ba91e080ca88ea11bded1861ada0bcf0e9ecfd49fed5d21d6ecc76b3b91ea1d88f065300f0c9d1f0b2a9907345ef1e679c0c7937c342f9ef1bb68d5b5f91d0a2a353320316d8dd64c8733e3640a5665b6febe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323ab601ce01541e895a1a50624e495b11782a04781b84ce14831e15b6061294a86453c09d259cc4b903ca2615a35b726ab3ac5b85d5b64e5c83a6004073295694fa776f643945893fae6ab621929a1df4253da840feb198f7741d4fae45da1036151a5241e37415ff1f0568f7091412c5e1a5733b7959c115a039ecfad743808c328d8d737d2fff2aa7d3f2ebff72b1fef18ffa1ca940a006182010e7828a03a0c00185b60d800015000000126cec1200890d1900000009000000000163b60000040200001336f6090062434e4240000640010000005895b60000029548e9a57c3891692a28d4f52cb89a99a8ee547b4d0ab6000002fbb7decc976b8a14b54eb7499bdf6ef67fdffb8eb54ab6ffd9"),
        ("bypass_layers", 1, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000003000102020101ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000003510001ff938080df151866bc0b664d846fb2f1e020fcf7fbc9fa1281631990f1a0dabcacb6fc1d42f8bfff0b1474075975ae5e768fda517622af0918266804ef0d6afd599157225d00eb9aefa9996ef9cf86ae7c14d3e1a23da925a962724c553a4cfecee73d28735c375460439fab982d3187741f3a37fce90b8583ccaff509eee0bc46f30fd9f143e729496b15466869f1ab5e6e0f3cc6046849e4435c363d79a1ba5dc6d8f2c06f4e5c83a600405d90eb9a86841edef3a2f031fa560e60720167b6ba79298fc0c5d3bafd08e8b4592c908292b185f7a93c276ddb41663340d49d33c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff275967ad32997d90c17729eeb6ff0f90fdc023f6e24225ad5ff7fbd2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ffd2a15ff7ffd342e89c1322645f529e970b0592fb28e01322644c099103752c571c0143fae9cb5a2b209ff7c9d50d2be04521f755820dc2b018fc44e3e7fcf68579d16607e4087ccd69ff82b86427a0685bc41148aa0c0a03e3ab2ff7fcabe28be9d54bdce414918bbac66161e3404dbb8d5a2bd546bbcf965ff7f08f12894c952489627ce0ee2a60091442ed19c418232d1a92aff7f401a3d82f581a01c091e62505206ba8d497bce7f5220a550aaff7fafdc0d0e754b9802aa829f505c69a82752456d744eb93f26f5f79b11a9ee801e50f0c9579ee690ff7fc7937c342f9ef1bb68d5b5f91d0a1a999018b6c6eb264399f1b2052b32ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f033805507a256869418939256c45e0a811e06e1338520c78ff7f06252950c8a7813a4b39897207944c2b46b6e4d56758b70baaff7fbcf9fdfb48677c791297f5e45048d60b0fb2a55cff7bb0791bc800a8d8c9be322ea213737d2fff2aa7d3f2ebff72b1fef18ffa1ca90280186080439e0a280e83000615ff7f0d8000000000049b3b048022434640000002400000000058d5ff7f000004040000266dec1200c4869c8480000c8002000000b12aff7f000004aa474d2be1c48b495146a7a965c4d4cd4772a3da68ff7f000007ddbef664bb5c50a5aa75ba4cdefb77b3feff6e3ad52aff7fffd9"),
        ("bypass_sop_eph", 1, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c06000002000102020101ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a00000000036b0001ff93ff9100040000deb2ff9266bc0b664d846fb2f1e020fcf7fbc9fa1281631990f1a0dabcacb6fc1d42f8bfff0b1474075975ae5e768fda517622af0918ff9100040001cea9395e79b0ff923da925a962724c553a4cfecee73d28735c37546046f30fd9f143e729496b15466869f1ab5e6e0f3cc604684e5c83a600405d90eb9a86841edef3a2f031fa560e60720167b6baff9100040002fd69745a2c9648ff92266804ef0d6afd599157225d00eb9aefa9996ef98292b185f7a93c276ddb41663340d49d33c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff275967ad32997d90c17729eeb6ff0f90fdc023f6e24225ad5ff7fbd2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ffd2a15ff7fff9100040003fd971d2e164b25f6b8e970b0592fbb2a804c8991302644ff92439fab982d3187741f3a37fce90b8583ccaff509eee0bc3752c571c0143fae9cb5a2b209ff7c9d50d2be04521f755820dc2b018fc44e3e7fcf68579d16607e4087ccd69ff82b86427a0685bc41148aa0c0a03e3ab2ff7fcabe28be9d54bdce414918bbac66161e3404dbb8d5a2bd546bbcf965ff7f08f12894c952489627ce0ee2a60091442ed19c418232d1a92aff7f401a3d82f581a01c091e62505206ba8d497bce7f5220a550aaff7f49e4435c363d79a1ba5dc6d8f2c06fafdc0d0e754b9802aa829f505c69a82752456d744eb93f26f5f79b11a9ee801e50f0c9579ee690ff7fc7937c342f9ef1bb68d5b5f91d0a1a999018b6c6eb264399f1b2052b32ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f033805507a256869418939256c45e0a811e06e1338520c78ff7f06252950c8a7813a4b39897207944c2b46b6e4d56758b70baaff7f79298fc0c5d3babcf9fdfb48677c791297f5e45048d60b0fb2a55cff7bb0791bc800a8d8c9be322ea213737d2fff2aa7d3f2ebff72b1fef18ffa1ca90280186080439e0a280e83000615ff7f0d8000000000049b3b048022434640000002400000000058d5ff7f000004040000266dec1200c4869c8480000c8002000000b12aff7f000004aa474d2be1c48b495146a7a965c4d4cd4772a3da68ff7f000007ddbef664bb5c50a5aa75ba4cdefb77b3feff6e3ad52aff7fffd9"),
        ("poc", 5, "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000002000202020501ff5c000a4040484850484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a00000000006e0002ff5f00100000000201010001000002030100ff93cd52801183aad155f4e991793fef7d5927effd632352162162162062b3f27fff7f0adadf1a283e493fff7f1115882965be7fff7fda4c6f9f022802ff7f95ad8234a61c0eff7ffb6eb173f531ff7fff90000a0000000003830102ff93a74d15311003ff1f81677f7d07239f99c9e6aae71f103da925a962724c46f30fd9f143e729496f1f359a1a7c6ad79b83cf3f4e5c83a600405f32c35cd4341f7fc7dab0accd89d432197d8c4e69d4b2391cbf1150965e69d4aa591c80280b222b5fca11ee7f7f5e5b572d034b06bf011a3f093a9e128f7fd1d1f19fff7fbb9d1a1e3e4d55d5ff7faf543bdba395ff7f1dee45cf481bff7f54e69b0a197fb87502b0988a22741ea6c7ff7f342a4aa43f98020aff7f8cd562cc54b6e62aff7f4ded7fc90218b5ff7f4a4c914002d6d1ff7f7f849f27568cbf3fa211c46ccc0d3827b64e2857dd8868ce15ff7f60b42a316a62710aff7f21a54981da64c0ff7fb62a7f27a6537555ff7ffdfe13c69a3e8aff7ffdf8e12b469aa0af7893310ae215c5f742124a8d1c4f78914107020e05f8632484d14d27089510ec21d843b08053ce4cfecee73f27d0bcb3253fc14f0fcc4af638beb637676b7f445f7f25e554ccb8b593b87f589d99b81a5f3aefab272d55d11adffe80dbd578dfdcb374a2c4a6644f21a7e5013fcf68579d16607e4087ccd69ff82b951909e81a16f104522a830280f8eacaff7fcabe28be9d54bdce4514918bbac66161e3404dbb8d5a2bd546bbcf96ff7f08f0944a64a9244b13e70771530048a21768ce20c11968d495ff7f401568f60bd606807024798941481aea3525ef39fd48829542ff7fb5dffc519e73a2028f39fa113f54e62bcc0703c703484a4f702d024d04ee9c7849302b24c2daba72cc340530ef3a5dd3ff7e6b1f599e9579ee691fbc7fc7937c342f9ef1bb68d5b5f91d0a2a353320316d8dd64c8733e3640a5665ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f01ce01541e895a1a50624e495b11782a04781b84ce14831e15ff7f061294a86453c09d259cc4b903ca2615a35b726ab3ac5b85d5ff7f9f60a4305f5f3347b26c4dbbd6e19f22a4a63f41d4b2df4988f9fd08621c7ea855898346c877a6880f5c8ad072bbaf151f2ad60a8d8fedc4451d64df737d2fff2aa7d3f2ebff72b1fef18ffa1ca940a006182010e7828a03a0c00185ff7f0d800015000000126cec1200890d1900000009000000000163ff7f0000040200001336f6090062434e4240000640010000005895ff7f0000029548e9a57c3891692a28d4f52cb89a99a8ee547b4d0aff7f000002fbb7decc976b8a14b54eb7499bdf6ef67fdffb8eb54aff7fffd9"),
    ];

    /// 9/7 stream with bypass (style 1), one layer, rate 2, and OpenJPEG's decoded pixels.
    const LOSSY_BYPASS: &str = "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c00000001000102020100ff5c000b42482457d357d35762ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000001100001ff93cf8ab010e0c3397943dc56aaf585c28c5faa548b0f69e4c570aca948c16e5a79f461635127640abeb991b71e3708e5c216db345a18d74d712cdac853048eb1388077eef9b126ab0c01ddfbb8f01cddda21bca470e01cc853173097c7c35d3e45b3e5c5f43da925a962724c45dec6b6dfc653ca8d0dda3d96e4396cbfff845e04c0f798b3dc994105a8b45d9c7f3a80cdcc4046de3d4b2467d0ccc417502271d6eff26b4916395c77efacf43bcfa1c3e3eea5ec6a16e52e6a1721445be8911f4e6fdb40d9b04c8b0dccc854e05dbdd9343e41e61624e54739cc5747472649f5b91250380b407b99e82b27caaef0dfc2405b79ceaa9f99cf9929c382223cd030220168ffd9";
    const LOSSY_BYPASS_RAW: &str = "012a304459213f375d895b5a828e6e9491aebba3929dd8b7bcc2d1f5c0defc15291e413d2d5b3a6e6784678d827594819aa9c2baa5d0b7dfb9fbe1e0e9ec08ff2f423e49313b8587918e8c938495929d969ccee2dadadbced7e0cedfd82b1d294e5769284e78825e6d809c73a6aca49c9cefedb8c2f100eae402e5fa2125fa1c6c666374626d9072a8947dab9cb8b8afd2d1f3d7dff7e32000f6160ff330203e697d607781868f85aa8a9f9dc29bacb1a6ffff02060013240c271e1a1a3c3832659261809d9b94a5b586b1b9babde9b0cfec08e1ff12f205260f0e1553484354809d8b9eb1b3b0bfc4acccc3b9dfd906f6ed2913ee2b2c083537224f423d6d5099a5bbbcbbc3cbd5dad1ddf6fe00020809251b2b353a50496153625457606771938b8f9db1c7d9afb3e8caf300e8f122f518480d1a422139163b6a4e5a7d5d7d849cb8ada2b7d8bcf5ffdec902fbef241c1341202e1d3d313e566141786867949ea9b0b5cfdbe2d9e4f0fd0005171c423b3a3f5c1f443f47554b665d86677e91bbb9b5c1d9e3d0f7cffc12ff163b082d06416440544b5c5e5f7a587b9b709b8be0c8b9ffe4dbc11815f7d9382d12513e383966623b4a5d7580598f827b77abbcdcc4e6f300191c26f3ff00241d24404144656a4d40575e80759298a9bab9c8a8e0e80eff21e91ff01d04195d2a65385a4464756b625785649bb596afb1d1a1d60e00fcf4341f1f110f51533b2c2e6753586485867e8071a5aa9fa6c2c5bcbeaa1214303bff021036344c5e1a495f575f7082907867819e9ab9bdcd8fb5b2cec1f12a00270a4c184624323d643f744875a271ac9ba8b3c695c5c7d5bef4b8f4c7f1473b30233a2e7b5d5b5652718691837777aebf9b969c9cd7d8e4d1d7bc06fb2f59182c3348614a4d5663828f567f8ea6b8bfa7a6b3c5d7dfcac4cdff1adfe747234f35595c665c63687e8d9a799295a0b9c6c3c2c3c5d1cffac5efdc24ec1f163b3d69637265686e6d97999a9c9a979ab2ccdaded6c9d7c9c9091a0315111115345058805f6f808bba84919da29bb2c8cbcbdce5dbcad0b8de0b2bf800141c";

    #[test]
    fn code_block_style_vectors_match_openjpeg() {
        let want = vector_image();
        for (name, style, hex) in VECTORS {
            let cs = unhex(hex);
            assert_eq!(cod_style(&cs), *style, "{name}: style byte");
            let img = decode(&cs).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((img.w, img.h, img.nc), (32, 24, 1), "{name}");
            assert!(img.data == want, "{name}: pixels differ from OpenJPEG's");
        }
        let cs = unhex(LOSSY_BYPASS);
        assert_eq!(cod_style(&cs), 1);
        let img = decode(&cs).unwrap();
        let raw = unhex(LOSSY_BYPASS_RAW);
        assert_eq!(img.data.len(), raw.len());
        let worst = img
            .data
            .iter()
            .zip(&raw)
            .map(|(&a, &b)| (a as i32 - b as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 2, "lossy bypass: worst difference {worst}");
    }

    #[test]
    fn extended_style_bits_are_refused() {
        let mut cs = unhex(VECTORS[0].2);
        let i = cs.windows(2).position(|w| w == [0xFF, 0x52]).unwrap();
        for bit in [0x40u8, 0x80] {
            cs[i + 12] = bit;
            assert!(decode(&cs).is_err());
        }
    }

    /// Rewrites a single-tile-part-per-tile codestream with PPT (or PPM) packed packet headers,
    /// using the packet extents the decoder recorded. OpenJPEG decodes such streams, which is
    /// how the rewriting was checked (see `jpx_repack_export`).
    fn repack(cs: &[u8], ppm: bool) -> Option<Vec<u8>> {
        TRACE.with(|t| t.borrow_mut().clear());
        decode(cs).ok()?;
        let spans = TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let mut p = 2;
        while cs.get(p..p + 2)? != [0xFF, 0x90] {
            p += 2 + (((cs[p + 2] as usize) << 8) | cs[p + 3] as usize);
        }
        let first_sot = p;
        let mut out = cs[..p].to_vec();
        let mut ppm_data = Vec::new();
        let mut tiles = 0;
        let mut last_isot = None;
        while cs.get(p..p + 2) == Some(&[0xFF, 0x90]) {
            let isot = ((cs[p + 4] as usize) << 8) | cs[p + 5] as usize;
            let psot = u32::from_be_bytes(cs[p + 6..p + 10].try_into().ok()?) as usize;
            if psot == 0 || cs[p + 10] != 0 || cs[p + 11] != 1 || last_isot >= Some(isot) {
                return None;
            }
            last_isot = Some(isot);
            let end = p + psot;
            let mut r = p + 12;
            while cs[r..r + 2] != [0xFF, 0x93] {
                r += 2 + (((cs[r + 2] as usize) << 8) | cs[r + 3] as usize);
            }
            let (markers, data) = (&cs[p + 12..r], &cs[r + 2..end]);
            let (mut hdrs, mut bodies, mut cur) = (Vec::new(), Vec::new(), 0);
            for &(ps, hs, he, be) in spans.get(tiles)? {
                let be = be.min(data.len());
                bodies.extend_from_slice(&data[ps..hs]);
                hdrs.extend_from_slice(&data[hs..he]);
                bodies.extend_from_slice(&data[he.min(be)..be]);
                cur = be;
            }
            bodies.extend_from_slice(&data[cur..]);
            tiles += 1;
            let mut ppt = Vec::new();
            if ppm {
                ppm_data.extend((hdrs.len() as u32).to_be_bytes());
                ppm_data.extend(&hdrs);
            } else {
                for (z, ch) in hdrs.chunks(1500).enumerate() {
                    ppt.extend([0xFF, 0x61]);
                    ppt.extend((3 + ch.len() as u16).to_be_bytes());
                    ppt.push(z as u8);
                    ppt.extend(ch);
                }
            }
            let new_psot = 12 + markers.len() + ppt.len() + 2 + bodies.len();
            out.extend(&cs[p..p + 6]);
            out.extend((new_psot as u32).to_be_bytes());
            out.extend(&cs[p + 10..p + 12]);
            out.extend(markers);
            out.extend(&ppt);
            out.extend([0xFF, 0x93]);
            out.extend(&bodies);
            p = end;
        }
        if tiles != spans.len() {
            return None;
        }
        if ppm {
            // records are cut into marker segments at arbitrary points, as the standard allows
            let size = 1500.max(ppm_data.len().div_ceil(200));
            let mut segs = Vec::new();
            for (z, ch) in ppm_data.chunks(size).enumerate() {
                segs.extend([0xFF, 0x60]);
                segs.extend((3 + ch.len() as u16).to_be_bytes());
                segs.push(z as u8);
                segs.extend(ch);
            }
            out.splice(first_sot..first_sot, segs); // just before the first SOT
        }
        out.extend(&cs[p..]);
        Some(out)
    }

    /// Moves the POC marker segment of a one-tile, one-tile-part stream into the main header.
    fn poc_to_main(cs: &[u8]) -> Option<Vec<u8>> {
        let mut p = 2;
        while cs[p..p + 2] != [0xFF, 0x90] {
            p += 2 + (((cs[p + 2] as usize) << 8) | cs[p + 3] as usize);
        }
        let mut r = p + 12;
        while cs[r..r + 2] != [0xFF, 0x93] {
            let l = 2 + (((cs[r + 2] as usize) << 8) | cs[r + 3] as usize);
            if cs[r..r + 2] == [0xFF, 0x5F] {
                let seg = cs[r..r + l].to_vec();
                let psot = u32::from_be_bytes(cs[p + 6..p + 10].try_into().ok()?) as usize;
                let mut out = cs[..p].to_vec();
                out.extend(&seg);
                out.extend(&cs[p..p + 6]);
                out.extend(((psot - l) as u32).to_be_bytes());
                out.extend(&cs[p + 10..r]);
                out.extend(&cs[r + l..]);
                return Some(out);
            }
            r += l;
        }
        None
    }

    #[test]
    fn poc_vector_decodes_from_tile_part_and_main_header() {
        let hex = VECTORS.iter().find(|v| v.0 == "poc").unwrap().2;
        let cs = unhex(hex);
        assert!(cs.windows(2).any(|w| w == [0xFF, 0x5F]));
        let want = vector_image();
        assert!(decode(&cs).unwrap().data == want);
        let main = poc_to_main(&cs).unwrap();
        assert!(decode(&main).unwrap().data == want);
    }

    #[test]
    fn packed_packet_headers_decode_like_the_plain_stream() {
        let mut streams: Vec<(String, Vec<u8>)> = VECTORS
            .iter()
            .map(|v| (v.0.to_string(), unhex(v.2)))
            .collect();
        for (name, stream, _, _) in cases() {
            if let Ok((cs, _)) = unwrap_jp2(stream) {
                streams.push((name.to_string(), cs.to_vec()));
            }
        }
        let mut n = 0;
        for (name, cs) in &streams {
            let want = decode(cs).unwrap();
            for ppm in [false, true] {
                let Some(packed) = repack(cs, ppm) else {
                    continue;
                };
                assert!(packed
                    .windows(2)
                    .any(|w| w == if ppm { [0xFF, 0x60] } else { [0xFF, 0x61] }));
                let got = decode(&packed).unwrap_or_else(|e| panic!("{name} ppm={ppm}: {e}"));
                assert!(got.data == want.data, "{name} ppm={ppm}");
                n += 1;
            }
        }
        assert!(n >= 20, "only {n} streams could be repacked");
    }

    /// PPM and PPT vectors (the "bypass_sop_eph" stream rewritten by `repack`), whose pixels
    /// OpenJPEG reproduced exactly.
    const PACKED: &[(&str, &str)] = &[
        ("ppt", "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c06000002000102020101ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff90000a0000000003700001ff61003100deb2ff92cea9395e79b0ff92fd69745a2c9648ff92fd971d2e164b25f6b8e970b0592fbb2a804c8991302644ff92ff93ff910004000066bc0b664d846fb2f1e020fcf7fbc9fa1281631990f1a0dabcacb6fc1d42f8bfff0b1474075975ae5e768fda517622af0918ff91000400013da925a962724c553a4cfecee73d28735c37546046f30fd9f143e729496b15466869f1ab5e6e0f3cc604684e5c83a600405d90eb9a86841edef3a2f031fa560e60720167b6baff9100040002266804ef0d6afd599157225d00eb9aefa9996ef98292b185f7a93c276ddb41663340d49d33c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff275967ad32997d90c17729eeb6ff0f90fdc023f6e24225ad5ff7fbd2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ffd2a15ff7fff9100040003439fab982d3187741f3a37fce90b8583ccaff509eee0bc3752c571c0143fae9cb5a2b209ff7c9d50d2be04521f755820dc2b018fc44e3e7fcf68579d16607e4087ccd69ff82b86427a0685bc41148aa0c0a03e3ab2ff7fcabe28be9d54bdce414918bbac66161e3404dbb8d5a2bd546bbcf965ff7f08f12894c952489627ce0ee2a60091442ed19c418232d1a92aff7f401a3d82f581a01c091e62505206ba8d497bce7f5220a550aaff7f49e4435c363d79a1ba5dc6d8f2c06fafdc0d0e754b9802aa829f505c69a82752456d744eb93f26f5f79b11a9ee801e50f0c9579ee690ff7fc7937c342f9ef1bb68d5b5f91d0a1a999018b6c6eb264399f1b2052b32ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f033805507a256869418939256c45e0a811e06e1338520c78ff7f06252950c8a7813a4b39897207944c2b46b6e4d56758b70baaff7f79298fc0c5d3babcf9fdfb48677c791297f5e45048d60b0fb2a55cff7bb0791bc800a8d8c9be322ea213737d2fff2aa7d3f2ebff72b1fef18ffa1ca90280186080439e0a280e83000615ff7f0d8000000000049b3b048022434640000002400000000058d5ff7f000004040000266dec1200c4869c8480000c8002000000b12aff7f000004aa474d2be1c48b495146a7a965c4d4cd4772a3da68ff7f000007ddbef664bb5c50a5aa75ba4cdefb77b3feff6e3ad52aff7fffd9"),
        ("ppm", "ff4fff510029000000000020000000180000000000000000000000200000001800000000000000000001070101ff52000c06000002000102020101ff5c00074040484850ff640025000143726561746564206279204f70656e4a5045472076657273696f6e20322e352e30ff600035000000002edeb2ff92cea9395e79b0ff92fd69745a2c9648ff92fd971d2e164b25f6b8e970b0592fbb2a804c8991302644ff92ff90000a00000000033d0001ff93ff910004000066bc0b664d846fb2f1e020fcf7fbc9fa1281631990f1a0dabcacb6fc1d42f8bfff0b1474075975ae5e768fda517622af0918ff91000400013da925a962724c553a4cfecee73d28735c37546046f30fd9f143e729496b15466869f1ab5e6e0f3cc604684e5c83a600405d90eb9a86841edef3a2f031fa560e60720167b6baff9100040002266804ef0d6afd599157225d00eb9aefa9996ef98292b185f7a93c276ddb41663340d49d33c9470dcf9045b633b399e5f2f75a5abfae878790b0c80d56220aff7f7d48efae7fc45dc25911e80623369dcc8352a5ed64831d478fd5ff7ff275967ad32997d90c17729eeb6ff0f90fdc023f6e24225ad5ff7fbd2afecdd16d97758adf6d7fc5b4dd1ad69221b6b80ffd2a15ff7fff9100040003439fab982d3187741f3a37fce90b8583ccaff509eee0bc3752c571c0143fae9cb5a2b209ff7c9d50d2be04521f755820dc2b018fc44e3e7fcf68579d16607e4087ccd69ff82b86427a0685bc41148aa0c0a03e3ab2ff7fcabe28be9d54bdce414918bbac66161e3404dbb8d5a2bd546bbcf965ff7f08f12894c952489627ce0ee2a60091442ed19c418232d1a92aff7f401a3d82f581a01c091e62505206ba8d497bce7f5220a550aaff7f49e4435c363d79a1ba5dc6d8f2c06fafdc0d0e754b9802aa829f505c69a82752456d744eb93f26f5f79b11a9ee801e50f0c9579ee690ff7fc7937c342f9ef1bb68d5b5f91d0a1a999018b6c6eb264399f1b2052b32ff7ffebe2ffcafaedd5404bb9df676ab47205999fbdcec485c572cb0323aff7f033805507a256869418939256c45e0a811e06e1338520c78ff7f06252950c8a7813a4b39897207944c2b46b6e4d56758b70baaff7f79298fc0c5d3babcf9fdfb48677c791297f5e45048d60b0fb2a55cff7bb0791bc800a8d8c9be322ea213737d2fff2aa7d3f2ebff72b1fef18ffa1ca90280186080439e0a280e83000615ff7f0d8000000000049b3b048022434640000002400000000058d5ff7f000004040000266dec1200c4869c8480000c8002000000b12aff7f000004aa474d2be1c48b495146a7a965c4d4cd4772a3da68ff7f000007ddbef664bb5c50a5aa75ba4cdefb77b3feff6e3ad52aff7fffd9"),
    ];

    #[test]
    fn ppm_and_ppt_vectors_match_openjpeg() {
        let want = vector_image();
        for (name, hex) in PACKED {
            let cs = unhex(hex);
            let img = decode(&cs).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(img.data == want, "{name}");
        }
    }

    /// `LEGATOROCR_JPX_DIR=sweep cargo test --release -- --ignored jpx_repack_export` writes
    /// NAME_ppt / NAME_ppm variants of the sweep's streams (checked against OpenJPEG by the
    /// generator script, then decoded by `jpx_differential`).
    #[test]
    #[ignore]
    fn jpx_repack_export() {
        let Ok(dir) = std::env::var("LEGATOROCR_JPX_DIR") else {
            return;
        };
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            if p.extension().is_none_or(|x| x != "j2k") || stem.contains("_pp") {
                continue;
            }
            let cs = std::fs::read(&p).unwrap();
            for (tag, ppm) in [("_ppt", false), ("_ppm", true)] {
                if let Some(v) = repack(&cs, ppm) {
                    let q = format!("{dir}/{stem}{tag}");
                    std::fs::write(format!("{q}.j2k"), v).unwrap();
                    std::fs::copy(p.with_extension("raw"), format!("{q}.raw")).unwrap();
                    std::fs::copy(p.with_extension("txt"), format!("{q}.txt")).unwrap();
                    n += 1;
                }
            }
        }
        eprintln!("{n} repacked streams");
    }

    #[test]
    fn styled_and_packed_streams_survive_damage() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let iters: usize = std::env::var("LEGATOROCR_FUZZ_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150);
        let mut streams: Vec<Vec<u8>> = VECTORS.iter().map(|v| unhex(v.2)).collect();
        streams.extend(PACKED.iter().map(|v| unhex(v.1)));
        streams.push(unhex(LOSSY_BYPASS));
        let mut slowest = std::time::Duration::ZERO;
        for s in &streams {
            for _ in 0..iters {
                let mut d = s.clone();
                for _ in 0..1 + rnd() % 6 {
                    let i = (rnd() as usize) % d.len();
                    match rnd() % 4 {
                        0 => d[i] = rnd() as u8,
                        1 => d[i] ^= 1 << (rnd() % 8),
                        2 => d.truncate(i.max(8)),
                        _ => {
                            // flip the style / POC / packed-header bytes near the front
                            let j = (rnd() as usize) % d.len().min(200);
                            d[j] = rnd() as u8;
                        }
                    }
                    if d.len() < 8 {
                        break;
                    }
                }
                let t = std::time::Instant::now();
                let _ = decode(&d);
                slowest = slowest.max(t.elapsed());
            }
        }
        // a damaged canvas size (up to MAX_SAMPLES) legitimately costs seconds of allocation
        assert!(
            slowest.as_secs() < 30,
            "a damaged stream took {slowest:?} to decode"
        );
    }

    /// Hostile POC lists: far too many entries are refused, a full list of absurd ranges is
    /// bounded in time.
    #[test]
    fn hostile_poc_lists_are_bounded() {
        let base = unhex(VECTORS[0].2);
        let sot = base.windows(2).position(|w| w == [0xFF, 0x90]).unwrap();
        for entries in [1usize, 255, 256, 300, 5000] {
            let mut poc = vec![0xFF, 0x5F];
            let body_len = 2 + entries * 7;
            if body_len > 0xFFFF {
                continue;
            }
            poc.extend((body_len as u16).to_be_bytes());
            for i in 0..entries {
                // RS CS LYE(2) RE CE P
                poc.extend([0, 0, 0xFF, 0xFF, 0xFF, 0, (i % 5) as u8]);
            }
            let mut d = base[..sot].to_vec();
            d.extend(&poc);
            d.extend(&base[sot..]);
            let t = std::time::Instant::now();
            let r = decode(&d);
            if entries > MAX_POCS {
                assert!(r.is_err(), "{entries} POC entries accepted");
            }
            assert!(
                t.elapsed().as_secs() < 10,
                "{entries} POC entries took {:?}",
                t.elapsed()
            );
        }
    }

    /// Many segment-producing contributions to one code-block never grow the segment list
    /// beyond its cap.
    #[test]
    fn segment_count_is_capped() {
        assert_eq!(next_seg_max(4, Some(1)), 1);
        assert_eq!(next_seg_max(1, None), 10);
        assert_eq!(next_seg_max(1, Some(10)), 2);
        assert_eq!(next_seg_max(1, Some(2)), 1);
        assert_eq!(next_seg_max(1, Some(1)), 2);
        assert_eq!(next_seg_max(0, None), 109);
    }
}
