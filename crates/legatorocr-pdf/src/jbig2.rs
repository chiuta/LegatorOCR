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
    let offs = template(t, &at);
    let sltp: usize = [0x9B25, 0x0795, 0x00E5, 0x0195][t as usize & 3];
    let mut cx = vec![(0u8, 0u8); 1 << offs.len()];
    let mut mq = Mq::new(data);
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

struct Seg<'a> {
    kind: u8,
    data: &'a [u8],
}

fn be32(b: &[u8], at: usize) -> R<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| Jbig2Error("truncated JBIG2 data".into()))
}

fn parse_segments(mut d: &[u8], out: &mut Vec<(u8, Vec<u8>)>) -> R<()> {
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
        p += count
            .checked_mul(ref_size)
            .ok_or_else(|| Jbig2Error("bad segment".into()))?;
        p += if big_page { 4 } else { 1 };
        let len = be32(d, p)? as usize;
        p += 4;
        if len == 0xFFFF_FFFF {
            return fail("JBIG2 segment of unknown length is not supported");
        }
        let body = d
            .get(p..p.saturating_add(len))
            .ok_or_else(|| Jbig2Error("truncated JBIG2 segment".into()))?;
        out.push((kind, body.to_vec()));
        d = &d[p + len..];
        if out.len() > 10_000 {
            return fail("too many JBIG2 segments");
        }
    }
    Ok(())
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
    for (kind, body) in &segs {
        let s = Seg {
            kind: *kind,
            data: body,
        };
        match s.kind {
            48 => {
                // page information: pages w/h may be 0xFFFFFFFF (striped); the PDF dictionary wins
                if s.data.len() >= 17 && s.data[16] & 4 != 0 {
                    page.iter_mut().for_each(|p| *p = 1);
                }
            }
            36 | 38 | 39 => {
                if s.data.len() < 18 {
                    return fail("truncated JBIG2 region");
                }
                let (rw, rh) = (be32(s.data, 0)? as usize, be32(s.data, 4)? as usize);
                let (rx, ry) = (be32(s.data, 8)? as usize, be32(s.data, 12)? as usize);
                let op = s.data[16] & 7;
                if rw == 0 || rh == 0 || rw > w || rh > h {
                    return fail("invalid JBIG2 region size");
                }
                let f = s.data[17];
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
                    let (rows, _) = crate::g4::decode(&s.data[p..], rw, rh, false);
                    rows.iter()
                        .flat_map(|r| r.iter().map(|&b| b as u8))
                        .collect()
                } else {
                    let n = if t == 0 { 4 } else { 1 };
                    if f & 0x10 != 0 {
                        return fail("JBIG2 extended templates are not supported");
                    }
                    let raw = s
                        .data
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
                    generic_arith(&s.data[p..], rw, rh, t, tpgdon, at)
                };
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
                regions += 1;
            }
            0 => return fail(
                "JBIG2 symbol dictionaries are not supported (this file uses text/symbol coding)",
            ),
            6 | 7 => return fail("JBIG2 text regions are not supported"),
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
        let offs = template(t, &at);
        let sltp: usize = [0x9B25, 0x0795, 0x00E5, 0x0195][t as usize & 3];
        let mut e = Mqe::new(1 << offs.len());
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
        e.finish()
    }

    fn seg(num: u32, kind: u8, page: u8, body: &[u8]) -> Vec<u8> {
        let mut v = num.to_be_bytes().to_vec();
        v.push(kind);
        v.push(0); // no referred-to segments
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
}

#[cfg(test)]
mod tests {
    use super::enc::*;
    use super::*;

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
    fn unsupported_and_damaged_input_is_reported_not_panicked() {
        let e = |d: &[u8]| decode(d, None, 8, 8).err().map(|e| e.0);
        // symbol dictionary segment
        let mut v = 0u32.to_be_bytes().to_vec();
        v.extend([0, 0, 1, 0, 0, 0, 0]);
        assert!(e(&v).unwrap().contains("symbol"));
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
