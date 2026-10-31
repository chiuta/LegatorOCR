//! JPEG 2000 (ITU-T T.800) decoder, written from the standard's description:
//! JP2 boxes or raw codestreams, tiles and tile-parts, all five progression orders,
//! quality layers, precincts, SOP/EPH markers, the reversible 5/3 and irreversible 9/7
//! wavelets, RCT/ICT colour transforms, component subsampling (nearest-neighbour upsampling),
//! and the EBCOT tier-1 coder (MQ arithmetic decoder shared with the JBIG2 module) with the
//! reset, vertically-causal and segmentation-symbol code-block styles.
//!
//! Not supported (reported as errors): selective arithmetic bypass and termination on every
//! pass, packed packet headers (PPM/PPT), progression-order changes (POC), palettes.

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

const MAX_SAMPLES: u64 = 300_000_000;
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
            if len < hdr || p + len > d.len() {
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

fn build_tile_comp(cc: &CompCod, q: &Quant, ci: &CompInfo, t: (u64, u64, u64, u64)) -> R<TileComp> {
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
        if (npw as u64) * (nph as u64) > 4_000_000 {
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
                                    data: Vec::new(),
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

fn progression(
    g: &CodG,
    tcs: &[TileComp],
    comps: &[CompInfo],
    t: (u64, u64, u64, u64),
) -> R<Vec<(u32, u32, u32, u32)>> {
    let maxr = tcs.iter().map(|c| c.cc.nl).max().unwrap_or(0);
    let layers = g.layers;
    let mut total: u64 = 0;
    for c in tcs {
        for r in &c.res {
            total += (r.npw * r.nph) as u64 * layers as u64;
        }
    }
    if total > MAX_PACKETS {
        return fail("too many packets");
    }
    let mut out = Vec::with_capacity(total as usize);
    let nprec = |c: usize, r: u32| -> u32 {
        tcs[c]
            .res
            .get(r as usize)
            .map_or(0, |x| (x.npw * x.nph) as u32)
    };
    match g.prog {
        0 => {
            for l in 0..layers {
                for r in 0..=maxr {
                    for c in 0..tcs.len() {
                        for p in 0..nprec(c, r) {
                            out.push((l, r, c as u32, p));
                        }
                    }
                }
            }
        }
        1 => {
            for r in 0..=maxr {
                for l in 0..layers {
                    for c in 0..tcs.len() {
                        for p in 0..nprec(c, r) {
                            out.push((l, r, c as u32, p));
                        }
                    }
                }
            }
        }
        2..=4 => {
            // position based orders
            let mut stepx = u64::MAX;
            let mut stepy = u64::MAX;
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
            let positions = |f: &mut dyn FnMut(u64, u64)| {
                let mut y = t.1;
                while y < t.3 {
                    let mut x = t.0;
                    while x < t.2 {
                        f(x, y);
                        x = (x / stepx + 1) * stepx;
                    }
                    y = (y / stepy + 1) * stepy;
                }
            };
            match g.prog {
                2 => {
                    for r in 0..=maxr {
                        positions(&mut |x, y| {
                            for c in 0..tcs.len() {
                                if let Some(p) = hit(c, r, x, y) {
                                    for l in 0..layers {
                                        out.push((l, r, c as u32, p));
                                    }
                                }
                            }
                        });
                    }
                }
                3 => positions(&mut |x, y| {
                    for c in 0..tcs.len() {
                        for r in 0..=maxr {
                            if let Some(p) = hit(c, r, x, y) {
                                for l in 0..layers {
                                    out.push((l, r, c as u32, p));
                                }
                            }
                        }
                    }
                }),
                _ => {
                    for c in 0..tcs.len() {
                        positions(&mut |x, y| {
                            for r in 0..=maxr {
                                if let Some(p) = hit(c, r, x, y) {
                                    for l in 0..layers {
                                        out.push((l, r, c as u32, p));
                                    }
                                }
                            }
                        });
                    }
                }
            }
        }
        _ => return fail("unknown progression order"),
    }
    Ok(out)
}

fn read_packets(g: &CodG, tcs: &mut [TileComp], order: &[(u32, u32, u32, u32)], d: &[u8]) -> R<()> {
    let mut pos = 0usize;
    for &(l, r, c, p) in order {
        if pos >= d.len() {
            break; // truncated tile: keep what we have
        }
        if g.sop && d.len() >= pos + 6 && d[pos] == 0xFF && d[pos + 1] == 0x91 {
            pos += 6;
        }
        let rs = &mut tcs[c as usize].res[r as usize];
        let precs = &mut rs.precs[p as usize];
        let mut b = Bits::new(d, pos);
        let mut work: Vec<(usize, usize, usize)> = Vec::new(); // band, cb, len
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
                    let nbits = cb.lblock + (31 - np.leading_zeros());
                    if nbits > 31 {
                        return fail("bad code-block length");
                    }
                    let len = b.bits(nbits)? as usize;
                    cb.passes += np;
                    work.push((bi, ci, len));
                }
            }
        }
        b.align();
        pos = b.p;
        if g.eph && d.len() >= pos + 2 && d[pos] == 0xFF && d[pos + 1] == 0x92 {
            pos += 2;
        }
        for (bi, ci, len) in work {
            let end = (pos + len).min(d.len());
            precs[bi].cbs[ci]
                .data
                .extend_from_slice(&d[pos.min(end)..end]);
            pos += len;
        }
    }
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

/// Decode one code-block into (magnitude, negative, last-plane) per sample.
fn t1_decode(
    w: usize,
    h: usize,
    orient: u8,
    style: u8,
    nbps: u32,
    npasses: u32,
    data: &[u8],
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
    let mut mq = Mq::new(data);
    let mut cx = init_ctx();
    let mut p = nbps as i32 - 1;
    let mut kind = 2;
    for _ in 0..npasses {
        if p < 0 {
            break;
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
                                if mq.decode(&mut cx[zc[t.nb[i] as usize]]) == 1 {
                                    let s = t.sign(&mut mq, &mut cx, x, y);
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
                                let c = if t.refd[i] == 1 {
                                    16
                                } else if t.nb[i] != 0 {
                                    15
                                } else {
                                    14
                                };
                                if mq.decode(&mut cx[c]) == 1 {
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
        if kind == 2 {
            p -= 1;
            kind = 0;
        } else {
            kind += 1;
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
        v[i] -= (l + r + 2) >> 2;
        i += 2;
    }
    let mut i = 1 - first_even;
    while i < n {
        let (l, r) = (v[mirror(i as isize - 1, n)], v[mirror(i as isize + 1, n)]);
        v[i] += (l + r) >> 1;
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
                if n == 0 || n > 16384 {
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
            0xFF5F => return fail("progression order changes (POC) are not supported"),
            0xFF60 => return fail("packed packet headers (PPM) are not supported"),
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
                        0xFF5F => return fail("POC is not supported"),
                        0xFF61 => return fail("PPT is not supported"),
                        _ => handle_cod_like(mk2, seg, s.comps.len(), &mut tp)?,
                    }
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
    let nc = siz.comps.len();
    // component planes on their own grids
    let mut dims = Vec::new();
    let mut planes = Vec::new();
    for c in &siz.comps {
        let w = (ceil_div(siz.x1, c.dx) - ceil_div(siz.x0, c.dx)) as usize;
        let h = (ceil_div(siz.y1, c.dy) - ceil_div(siz.y0, c.dy)) as usize;
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
        decode_tile(&siz, tp, &tile.data, t, &mut planes, &dims)?;
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
    t: (u64, u64, u64, u64),
    planes: &mut [Vec<f32>],
    dims: &[(usize, usize)],
) -> R<()> {
    let nc = siz.comps.len();
    let mut tcs = Vec::with_capacity(nc);
    for c in 0..nc {
        tcs.push(build_tile_comp(&tp.cc[c], &tp.q[c], &siz.comps[c], t)?);
    }
    let order = progression(&tp.g, &tcs, &siz.comps, t)?;
    read_packets(&tp.g, &mut tcs, &order, data)?;
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
        if tc.cc.style & 0x05 != 0 {
            return fail("selective arithmetic bypass / per-pass termination are not supported");
        }
        for (ri, rs) in tc.res.iter().enumerate() {
            for precs in &rs.precs {
                for (bi, pb) in precs.iter().enumerate() {
                    let b = &rs.bands[bi];
                    for cb in &pb.cbs {
                        if cb.included && cb.passes > 0 && !cb.data.is_empty() {
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
    let results: Vec<Vec<f32>> = jobs
        .par_iter()
        .map(|j| {
            let (w, h) = (j.cb.x1 - j.cb.x0, j.cb.y1 - j.cb.y0);
            let nbps = j.mb.saturating_sub(j.cb.zbp);
            if nbps == 0 || nbps > 31 || w == 0 || h == 0 {
                return vec![0f32; w * h];
            }
            let (mag, neg, last) =
                t1_decode(w, h, j.orient, j.style, nbps, j.cb.passes, &j.cb.data);
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
                let g = a0 - ((a1 + a2) >> 2);
                y0[i] = (a2 + g) as f32;
                y1[i] = g as f32;
                y2[i] = (a1 + g) as f32;
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
                    let i = (rnd() as usize) % d.len();
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
}
