//! CCITT Group 4 (ITU-T T.6) encoder for 1-bit pages. Text pages shrink to roughly
//! a third of Flate+predictor, and every PDF viewer decodes it natively.
//! Ink = black. Tables are from ITU-T T.4 / T.6.

const WHITE_TERM: [&str; 64] = [
    "00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111", "10011", "10100",
    "00111", "01000", "001000", "000011", "110100", "110101", "101010", "101011", "0100111",
    "0001100", "0001000", "0010111", "0000011", "0000100", "0101000", "0101011", "0010011",
    "0100100", "0011000", "00000010", "00000011", "00011010", "00011011", "00010010", "00010011",
    "00010100", "00010101", "00010110", "00010111", "00101000", "00101001", "00101010", "00101011",
    "00101100", "00101101", "00000100", "00000101", "00001010", "00001011", "01010010", "01010011",
    "01010100", "01010101", "00100100", "00100101", "01011000", "01011001", "01011010", "01011011",
    "01001010", "01001011", "00110010", "00110011", "00110100",
];
const WHITE_MAKEUP: [&str; 27] = [
    "11011",
    "10010",
    "010111",
    "0110111",
    "00110110",
    "00110111",
    "01100100",
    "01100101",
    "01101000",
    "01100111",
    "011001100",
    "011001101",
    "011010010",
    "011010011",
    "011010100",
    "011010101",
    "011010110",
    "011010111",
    "011011000",
    "011011001",
    "011011010",
    "011011011",
    "010011000",
    "010011001",
    "010011010",
    "011000",
    "010011011",
];
const BLACK_TERM: [&str; 64] = [
    "0000110111",
    "010",
    "11",
    "10",
    "011",
    "0011",
    "0010",
    "00011",
    "000101",
    "000100",
    "0000100",
    "0000101",
    "0000111",
    "00000100",
    "00000111",
    "000011000",
    "0000010111",
    "0000011000",
    "0000001000",
    "00001100111",
    "00001101000",
    "00001101100",
    "00000110111",
    "00000101000",
    "00000010111",
    "00000011000",
    "000011001010",
    "000011001011",
    "000011001100",
    "000011001101",
    "000001101000",
    "000001101001",
    "000001101010",
    "000001101011",
    "000011010010",
    "000011010011",
    "000011010100",
    "000011010101",
    "000011010110",
    "000011010111",
    "000001101100",
    "000001101101",
    "000011011010",
    "000011011011",
    "000001010100",
    "000001010101",
    "000001010110",
    "000001010111",
    "000001100100",
    "000001100101",
    "000001010010",
    "000001010011",
    "000000100100",
    "000000110111",
    "000000111000",
    "000000100111",
    "000000101000",
    "000001011000",
    "000001011001",
    "000000101011",
    "000000101100",
    "000001011010",
    "000001100110",
    "000001100111",
];
const BLACK_MAKEUP: [&str; 27] = [
    "0000001111",
    "000011001000",
    "000011001001",
    "000001011011",
    "000000110011",
    "000000110100",
    "000000110101",
    "0000001101100",
    "0000001101101",
    "0000001001010",
    "0000001001011",
    "0000001001100",
    "0000001001101",
    "0000001110010",
    "0000001110011",
    "0000001110100",
    "0000001110101",
    "0000001110110",
    "0000001110111",
    "0000001010010",
    "0000001010011",
    "0000001010100",
    "0000001010101",
    "0000001011010",
    "0000001011011",
    "0000001100100",
    "0000001100101",
];
const EXT_MAKEUP: [&str; 13] = [
    "00000001000",
    "00000001100",
    "00000001101",
    "000000010010",
    "000000010011",
    "000000010100",
    "000000010101",
    "000000010110",
    "000000010111",
    "000000011100",
    "000000011101",
    "000000011110",
    "000000011111",
];

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u8,
}

impl Bits {
    fn put(&mut self, code: &str) {
        for c in code.bytes() {
            self.acc = (self.acc << 1) | (c - b'0') as u32;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

fn put_run(b: &mut Bits, mut len: usize, black: bool) {
    while len >= 2560 {
        b.put(EXT_MAKEUP[12]);
        len -= 2560;
    }
    if len >= 1792 {
        let i = (len - 1792) / 64;
        b.put(EXT_MAKEUP[i]);
        len -= 1792 + i * 64;
    } else if len >= 64 {
        let i = len / 64 - 1;
        b.put(if black {
            BLACK_MAKEUP[i]
        } else {
            WHITE_MAKEUP[i]
        });
        len %= 64;
    }
    b.put(if black {
        BLACK_TERM[len]
    } else {
        WHITE_TERM[len]
    });
}

/// First changing element strictly right of `from` with colour opposite to `colour`
/// (b1 search); `row[x]` true = black. Returns `w` when none.
fn next_change(row: &[bool], from: isize, colour: bool) -> usize {
    let w = row.len();
    let mut x = (from + 1).max(0) as usize;
    // find first position >= x where pixel != previous pixel and pixel == !colour
    while x < w {
        let prev = if x == 0 { false } else { row[x - 1] };
        if row[x] != prev && row[x] != colour {
            return x;
        }
        x += 1;
    }
    w
}

/// Encode `rows` (each `true` = ink) of equal width. No EOFB is needed in PDF
/// (`/EndOfBlock true` default tolerates it); one is appended for standalone TIFF use.
pub fn encode(width: usize, rows: impl Iterator<Item = Vec<bool>>) -> Vec<u8> {
    let mut bits = Bits {
        out: Vec::new(),
        acc: 0,
        n: 0,
    };
    let mut reference = vec![false; width];
    for cur in rows {
        let mut a0: isize = -1;
        let mut colour = false;
        while (a0 as i64) < width as i64 {
            // a1: next changing element on the coding line right of a0
            let a1 = {
                let mut x = (a0 + 1).max(0) as usize;
                while x < width && cur[x] == colour {
                    x += 1;
                }
                x
            };
            let b1 = next_change(&reference, a0, colour);
            let b2 = {
                let mut x = b1;
                if b1 < width {
                    x += 1;
                    while x < width && reference[x] != colour {
                        x += 1;
                    }
                }
                x.min(width)
            };
            if b2 < a1 {
                bits.put("0001"); // pass
                a0 = b2 as isize;
            } else if (a1 as isize - b1 as isize).abs() <= 3 {
                bits.put(match a1 as isize - b1 as isize {
                    0 => "1",
                    1 => "011",
                    2 => "000011",
                    3 => "0000011",
                    -1 => "010",
                    -2 => "000010",
                    _ => "0000010",
                });
                a0 = a1 as isize;
                colour = !colour;
            } else {
                let a2 = {
                    let mut x = a1;
                    if a1 < width {
                        x += 1;
                        while x < width && cur[x] != colour {
                            x += 1;
                        }
                    }
                    x.min(width)
                };
                bits.put("001");
                let start = a0.max(0) as usize;
                put_run(&mut bits, a1 - start, colour);
                put_run(&mut bits, a2 - a1, !colour);
                a0 = a2 as isize;
            }
        }
        reference = cur;
    }
    bits.put("000000000001000000000001"); // EOFB
    bits.finish()
}

/// Encode a [`legatorocr_core::BitImage`] page.
pub fn encode_bitimage(b: &legatorocr_core::BitImage) -> Vec<u8> {
    let w = b.w as usize;
    encode(
        w,
        (0..b.h as usize).map(|y| b.data[y * w..(y + 1) * w].iter().map(|&v| v != 0).collect()),
    )
}

/// A complete single-strip, Group 4 compressed TIFF (readable by every TIFF tool).
pub fn tiff_g4(b: &legatorocr_core::BitImage, dpi: u32) -> Vec<u8> {
    let strip = encode_bitimage(b);
    let mut out: Vec<u8> = Vec::with_capacity(strip.len() + 200);
    out.extend_from_slice(b"II\x2a\x00");
    out.extend_from_slice(&8u32.to_le_bytes()); // placeholder, patched below
    out.extend_from_slice(&strip);
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let res_off = out.len() as u32 + 2 + 12 * 12 + 4; // after the IFD
    let ifd_off = out.len() as u32;
    out[4..8].copy_from_slice(&ifd_off.to_le_bytes());
    // (tag, type, count, value) — type 3 SHORT, 4 LONG, 5 RATIONAL
    let entries: [(u16, u16, u32, u32); 12] = [
        (256, 4, 1, b.w),
        (257, 4, 1, b.h),
        (258, 3, 1, 1),
        (259, 3, 1, 4),   // CCITT Group 4
        (262, 3, 1, 0),   // WhiteIsZero
        (273, 4, 1, 8),   // strip offset
        (277, 3, 1, 1),   // samples per pixel
        (278, 4, 1, b.h), // rows per strip
        (279, 4, 1, strip.len() as u32),
        (282, 5, 1, res_off),
        (283, 5, 1, res_off + 8),
        (296, 3, 1, 2), // inch
    ];
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, ty, count, val) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&val.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    for _ in 0..2 {
        out.extend_from_slice(&dpi.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out
}

/// One Group 4 compressed page for [`tiff_g4_multi`].
pub struct G4Strip {
    pub w: u32,
    pub h: u32,
    pub dpi: u32,
    /// Output of [`encode_bitimage`].
    pub data: Vec<u8>,
}

impl G4Strip {
    pub fn from_bitimage(b: &legatorocr_core::BitImage, dpi: u32) -> Self {
        G4Strip {
            w: b.w,
            h: b.h,
            dpi,
            data: encode_bitimage(b),
        }
    }
}

/// A multi-page Group 4 TIFF (one IFD per page, chained), the archival format for
/// black & white books. Pages are stored in the order given.
pub fn tiff_g4_multi(pages: &[G4Strip]) -> Vec<u8> {
    let mut out: Vec<u8> =
        Vec::with_capacity(pages.iter().map(|p| p.data.len() + 200).sum::<usize>() + 8);
    out.extend_from_slice(b"II\x2a\x00");
    out.extend_from_slice(&0u32.to_le_bytes()); // first IFD, patched
    let mut next_field = 4usize;
    for p in pages {
        let strip_off = out.len() as u32;
        out.extend_from_slice(&p.data);
        if out.len() % 2 == 1 {
            out.push(0);
        }
        let ifd_off = out.len() as u32;
        out[next_field..next_field + 4].copy_from_slice(&ifd_off.to_le_bytes());
        let res_off = ifd_off + 2 + 12 * 12 + 4;
        let entries: [(u16, u16, u32, u32); 12] = [
            (256, 4, 1, p.w),
            (257, 4, 1, p.h),
            (258, 3, 1, 1),
            (259, 3, 1, 4),
            (262, 3, 1, 0),
            (273, 4, 1, strip_off),
            (277, 3, 1, 1),
            (278, 4, 1, p.h),
            (279, 4, 1, p.data.len() as u32),
            (282, 5, 1, res_off),
            (283, 5, 1, res_off + 8),
            (296, 3, 1, 2),
        ];
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, ty, count, val) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&ty.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&val.to_le_bytes());
        }
        next_field = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..2 {
            out.extend_from_slice(&p.dpi.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Decoder (T.6 / Group 4), tolerant: a damaged stream yields white rows for the
// rest of the image instead of an error, like a viewer would show it.

use std::collections::HashMap;
use std::sync::OnceLock;

type RunTable = HashMap<(u8, u16), usize>;

fn run_tables() -> &'static (RunTable, RunTable) {
    static T: OnceLock<(RunTable, RunTable)> = OnceLock::new();
    T.get_or_init(|| {
        let build = |term: &[&str; 64], makeup: &[&str; 27]| {
            let mut m = RunTable::new();
            let mut add = |code: &str, v: usize| {
                m.insert((code.len() as u8, u16::from_str_radix(code, 2).unwrap()), v);
            };
            for (i, c) in term.iter().enumerate() {
                add(c, i);
            }
            for (i, c) in makeup.iter().enumerate() {
                add(c, 64 * (i + 1));
            }
            for (i, c) in EXT_MAKEUP.iter().enumerate() {
                add(c, 1792 + 64 * i);
            }
            m
        };
        (
            build(&WHITE_TERM, &WHITE_MAKEUP),
            build(&BLACK_TERM, &BLACK_MAKEUP),
        )
    })
}

struct BitReader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn bit(&mut self) -> Option<u16> {
        let b = *self.d.get(self.pos / 8)?;
        let v = (b >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(v as u16)
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

fn read_run(r: &mut BitReader, table: &RunTable) -> Option<usize> {
    let mut total = 0;
    loop {
        let (mut code, mut len) = (0u16, 0u8);
        let v = loop {
            code = (code << 1) | r.bit()?;
            len += 1;
            if let Some(&v) = table.get(&(len, code)) {
                break v;
            }
            if len > 13 {
                return None;
            }
        };
        total += v;
        if v < 64 {
            return Some(total);
        }
    }
}

/// Decode a Group 4 stream into `rows` rows of `width` pixels (`true` = black run).
/// Returns the rows and how many were decoded before an error (the rest are white).
/// `byte_align` = the PDF `EncodedByteAlign` flag.
/// Decode one two-dimensional line against `reference`. `None` on an error / end of data.
fn row_2d(
    r: &mut BitReader,
    reference: &[bool],
    width: usize,
    white: &RunTable,
    black: &RunTable,
) -> Option<Vec<bool>> {
    let mut cur = vec![false; width];
    let mut a0: isize = -1;
    let mut colour = false;
    let mut stalled = 0u32;
    while a0 < width as isize {
        let before = a0;
        let mut x = (a0 + 1).max(0) as usize;
        let b1 = loop {
            if x >= width {
                break width;
            }
            let prev = if x == 0 { false } else { reference[x - 1] };
            if reference[x] != prev && reference[x] != colour {
                break x;
            }
            x += 1;
        };
        let b2 = if b1 >= width {
            width
        } else {
            let mut x = b1 + 1;
            while x < width && reference[x] != colour {
                x += 1;
            }
            x
        };
        // mode code (prefix-free, at most 7 bits)
        let (mut code, mut len) = (0u16, 0u8);
        let mode = loop {
            let b = r.bit()?;
            code = (code << 1) | b;
            len += 1;
            match (len, code) {
                (1, 1) => break 0i32,
                (3, 0b011) => break 1,
                (3, 0b010) => break -1,
                (3, 0b001) => break 100,
                (4, 0b0001) => break 200,
                (6, 0b000011) => break 2,
                (6, 0b000010) => break -2,
                (7, 0b0000011) => break 3,
                (7, 0b0000010) => break -3,
                (7.., _) => return None, // EOFB, extension codes or garbage
                _ => {}
            }
        };
        let fill = |cur: &mut Vec<bool>, from: usize, to: usize, c: bool| {
            if c {
                for v in cur.iter_mut().take(to.min(width)).skip(from) {
                    *v = true;
                }
            }
        };
        match mode {
            200 => {
                fill(&mut cur, a0.max(0) as usize, b2, colour);
                a0 = b2 as isize;
            }
            100 => {
                let (t1, t2) = if colour {
                    (black, white)
                } else {
                    (white, black)
                };
                let (Some(r1), Some(r2)) = (read_run(r, t1), read_run(r, t2)) else {
                    return None;
                };
                let s = a0.max(0) as usize;
                fill(&mut cur, s, s + r1, colour);
                fill(&mut cur, s + r1, s + r1 + r2, !colour);
                a0 = (s + r1 + r2) as isize;
            }
            d => {
                let a1 = b1 as isize + d as isize;
                if a1 < a0.max(0) || a1 as usize > width {
                    return None;
                }
                fill(&mut cur, a0.max(0) as usize, a1 as usize, colour);
                a0 = a1;
                colour = !colour;
            }
        }
        // every step rescans the reference row: a stream of zero-length steps would be quadratic
        if a0 <= before {
            stalled += 1;
            if stalled > 4 {
                return None;
            }
        } else {
            stalled = 0;
        }
    }
    Some(cur)
}

pub fn decode(data: &[u8], width: usize, rows: usize, byte_align: bool) -> (Vec<Vec<bool>>, usize) {
    let (white, black) = run_tables();
    let mut r = BitReader { d: data, pos: 0 };
    let mut reference = vec![false; width];
    let mut out: Vec<Vec<bool>> = Vec::with_capacity(rows.min(1 << 16));
    let mut ok = 0;
    'rows: while out.len() < rows {
        if byte_align {
            r.align();
        }
        let Some(cur) = row_2d(&mut r, &reference, width, white, black) else {
            break 'rows;
        };
        reference = cur.clone();
        out.push(cur);
        ok += 1;
    }
    out.resize(rows, vec![false; width]);
    (out, ok)
}

/// Consume an EOL (at least 11 zero bits, any number of fill zeros, then a 1).
fn take_eol(r: &mut BitReader) -> bool {
    let save = r.pos;
    let mut zeros = 0;
    loop {
        match r.bit() {
            Some(0) => zeros += 1,
            Some(_) => {
                if zeros >= 11 {
                    return true;
                }
                break;
            }
            None => break,
        }
    }
    r.pos = save;
    false
}

/// Linear scan for the next EOL (>= 11 zeros then a 1); leaves the reader at the start of it.
fn find_eol(r: &mut BitReader) -> bool {
    let mut zeros = 0usize;
    let mut at = r.pos;
    loop {
        match r.bit() {
            Some(0) => {
                if zeros == 0 {
                    at = r.pos - 1;
                }
                zeros += 1;
            }
            Some(_) => {
                if zeros >= 11 {
                    r.pos = at;
                    return true;
                }
                zeros = 0;
            }
            None => return false,
        }
    }
}

/// One-dimensional (modified Huffman) line.
fn row_1d(
    r: &mut BitReader,
    width: usize,
    white: &RunTable,
    black: &RunTable,
) -> Option<Vec<bool>> {
    let mut cur = vec![false; width];
    let (mut x, mut colour) = (0usize, false);
    while x < width {
        let n = read_run(r, if colour { black } else { white })?;
        if x + n > width {
            return None;
        }
        if colour {
            for v in cur.iter_mut().skip(x).take(n) {
                *v = true;
            }
        }
        x += n;
        colour = !colour;
    }
    Some(cur)
}

/// Decode a CCITT Group 3 stream (PDF `K >= 0`): `k == 0` is purely one-dimensional,
/// `k > 0` mixes 1-D and 2-D lines, each introduced by an EOL and a tag bit.
/// Damaged lines are skipped by resynchronising on the next EOL. Same result shape as
/// [`decode`].
pub fn decode_g3(
    data: &[u8],
    width: usize,
    rows: usize,
    k: i64,
    byte_align: bool,
) -> (Vec<Vec<bool>>, usize) {
    let (white, black) = run_tables();
    let mut r = BitReader { d: data, pos: 0 };
    let mut reference = vec![false; width];
    let mut out: Vec<Vec<bool>> = Vec::with_capacity(rows.min(1 << 16));
    let mut ok = 0;
    let mut guard = 0usize;
    while out.len() < rows && r.pos < data.len() * 8 && guard < rows * 2 + 64 {
        guard += 1;
        let had_eol = take_eol(&mut r);
        if had_eol && take_eol(&mut r) {
            break; // RTC: consecutive EOLs
        }
        if byte_align && !had_eol {
            r.align();
        }
        let two_d = if k > 0 {
            if !had_eol && !out.is_empty() {
                // 2-D streams always carry EOLs; without one we are lost
                None
            } else {
                match r.bit() {
                    Some(1) => Some(false),
                    Some(_) => Some(true),
                    None => break,
                }
            }
        } else {
            Some(false)
        };
        let row = match two_d {
            Some(false) => row_1d(&mut r, width, white, black),
            Some(true) => row_2d(&mut r, &reference, width, white, black),
            None => None,
        };
        match row {
            Some(row) => {
                reference.clone_from(&row);
                out.push(row);
                ok += 1;
            }
            None => {
                // resynchronise: skip to the next EOL, emit a blank line for the lost one
                if !find_eol(&mut r) {
                    break;
                }
                reference = vec![false; width];
                out.push(reference.clone());
            }
        }
    }
    out.resize(rows, vec![false; width]);
    (out, ok)
}

#[cfg(test)]
#[allow(clippy::manual_is_multiple_of, clippy::needless_range_loop)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Reader<'a> {
        d: &'a [u8],
        pos: usize, // bit position
    }
    impl Reader<'_> {
        fn bit(&mut self) -> Option<u8> {
            let b = *self.d.get(self.pos / 8)?;
            let v = (b >> (7 - self.pos % 8)) & 1;
            self.pos += 1;
            Some(v)
        }
    }

    fn run_table(black: bool) -> HashMap<String, usize> {
        let (term, makeup) = if black {
            (&BLACK_TERM, &BLACK_MAKEUP)
        } else {
            (&WHITE_TERM, &WHITE_MAKEUP)
        };
        let mut m = HashMap::new();
        for (i, c) in term.iter().enumerate() {
            m.insert(c.to_string(), i);
        }
        for (i, c) in makeup.iter().enumerate() {
            m.insert(c.to_string(), 64 * (i + 1));
        }
        for (i, c) in EXT_MAKEUP.iter().enumerate() {
            m.insert(c.to_string(), 1792 + 64 * i);
        }
        m
    }

    fn read_run(r: &mut Reader, table: &HashMap<String, usize>) -> Option<usize> {
        let mut total = 0;
        loop {
            let mut code = String::new();
            let v = loop {
                code.push(if r.bit()? == 1 { '1' } else { '0' });
                if let Some(&v) = table.get(&code) {
                    break v;
                }
                if code.len() > 13 {
                    return None;
                }
            };
            total += v;
            if v < 64 {
                return Some(total);
            }
        }
    }

    /// Independent reference decoder (T.6), written from the spec, not from the encoder.
    fn decode(data: &[u8], width: usize, rows: usize) -> Option<Vec<Vec<bool>>> {
        let (white, black) = (run_table(false), run_table(true));
        let mut r = Reader { d: data, pos: 0 };
        let mut reference = vec![false; width];
        let mut out = Vec::new();
        for _ in 0..rows {
            let mut cur = vec![false; width];
            let mut a0: isize = -1;
            let mut colour = false;
            while a0 < width as isize {
                // b1/b2 on the reference line
                let find_b1 = |a0: isize, colour: bool| -> usize {
                    let mut x = (a0 + 1).max(0) as usize;
                    while x < width {
                        let prev = if x == 0 { false } else { reference[x - 1] };
                        if reference[x] != prev && reference[x] != colour {
                            return x;
                        }
                        x += 1;
                    }
                    width
                };
                let b1 = find_b1(a0, colour);
                let b2 = if b1 >= width {
                    width
                } else {
                    let mut x = b1 + 1;
                    while x < width && reference[x] != colour {
                        x += 1;
                    }
                    x
                };
                // read mode
                let mut code = String::new();
                let mode = loop {
                    code.push(if r.bit()? == 1 { '1' } else { '0' });
                    match code.as_str() {
                        "1" => break 0i32,
                        "011" => break 1,
                        "010" => break -1,
                        "001" => break 100,  // horizontal
                        "0001" => break 200, // pass
                        "000011" => break 2,
                        "000010" => break -2,
                        "0000011" => break 3,
                        "0000010" => break -3,
                        _ if code.len() > 7 => return None,
                        _ => {}
                    }
                };
                let fill = |cur: &mut Vec<bool>, from: usize, to: usize, c: bool| {
                    if c {
                        for x in from..to.min(width) {
                            cur[x] = true;
                        }
                    }
                };
                match mode {
                    200 => {
                        fill(&mut cur, a0.max(0) as usize, b2, colour);
                        a0 = b2 as isize;
                    }
                    100 => {
                        let t1 = if colour { &black } else { &white };
                        let t2 = if colour { &white } else { &black };
                        let r1 = read_run(&mut r, t1)?;
                        let r2 = read_run(&mut r, t2)?;
                        let s = a0.max(0) as usize;
                        fill(&mut cur, s, s + r1, colour);
                        fill(&mut cur, s + r1, s + r1 + r2, !colour);
                        a0 = (s + r1 + r2) as isize;
                    }
                    d => {
                        let a1 = (b1 as isize + d as isize) as usize;
                        if a1 > width {
                            return None;
                        }
                        fill(&mut cur, a0.max(0) as usize, a1, colour);
                        a0 = a1 as isize;
                        colour = !colour;
                    }
                }
            }
            reference = cur.clone();
            out.push(cur);
        }
        Some(out)
    }

    fn rnd(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    fn make(kind: u64, w: usize, h: usize, s: &mut u64) -> Vec<Vec<bool>> {
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| match kind {
                        0 => rnd(s) % 2 == 0,
                        1 => rnd(s) % 50 == 0,
                        2 => rnd(s) % 50 != 0,
                        3 => (x / (1 + (y % 40) * 100)) % 2 == 0, // very long runs, shifting edges
                        4 => (x + y) % 7 < 3,
                        5 => x % 2 == 0,
                        6 => false,
                        7 => true,
                        8 => (x * y) % 13 < 5,
                        9 => x == (y * 7) % w.max(1) || x == w - 1,
                        10 => y % 3 == 0 || (x / 64) % 2 == y % 2, // 64-boundary make-up codes
                        _ => x.is_multiple_of(2560) || (x + 1) % 1792 == 0, // 2560/1792 boundaries
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn independent_decoder_agrees_on_extreme_pages() {
        let mut s = 0x1234_5678_9ABC_DEF1u64;
        let n: u64 = std::env::var("LEGATOROCR_FUZZ_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        for it in 0..n {
            let kind = it % 12;
            let w = match rnd(&mut s) % 7 {
                0 => 1,
                1 => 2 + (rnd(&mut s) % 6) as usize,
                2 => 63 + (rnd(&mut s) % 3) as usize,
                3 => 1790 + (rnd(&mut s) % 5) as usize,
                4 => 2558 + (rnd(&mut s) % 6) as usize,
                5 => 5000 + (rnd(&mut s) % 3000) as usize,
                _ => 1 + (rnd(&mut s) % 900) as usize,
            };
            let h = 1 + (rnd(&mut s) % 40) as usize;
            let img = make(kind, w, h, &mut s);
            let enc = encode(w, img.iter().cloned());
            let dec = decode(&enc, w, h)
                .unwrap_or_else(|| panic!("decode failed: it={it} kind={kind} {w}x{h}"));
            assert_eq!(dec, img, "mismatch it={it} kind={kind} {w}x{h}");
        }
    }

    #[test]
    fn production_decoder_round_trips_and_survives_damage() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        for it in 0..300u64 {
            let w = 1 + (rnd(&mut seed) % 400) as usize;
            let h = 1 + (rnd(&mut seed) % 60) as usize;
            let img = make(it % 6, w, h, &mut seed);
            let enc = encode(w, img.iter().cloned());
            let (back, ok) = super::decode(&enc, w, h, false);
            assert_eq!(ok, h, "iteration {it}");
            assert_eq!(back, img, "iteration {it} {w}x{h}");
            // damaged / truncated streams must not panic and must keep the shape
            let mut bad = enc.clone();
            if !bad.is_empty() {
                let i = (rnd(&mut seed) as usize) % bad.len();
                bad[i] ^= 0xFF;
                bad.truncate(1 + (rnd(&mut seed) as usize) % bad.len());
            }
            let (b2, _) = super::decode(&bad, w, h, false);
            assert_eq!((b2.len(), b2[0].len()), (h, w));
        }
        assert_eq!(super::decode(&[], 5, 3, true).0, vec![vec![false; 5]; 3]);
        assert_eq!(super::decode(&[0xFF], 0, 4, false).0.len(), 4);
    }

    #[test]
    fn tiff_wrapper_is_well_formed() {
        let mut b = legatorocr_core::BitImage::new(37, 11);
        for i in 0..37 {
            b.set(i, i % 11, true);
        }
        let t = tiff_g4(&b, 300);
        assert_eq!(&t[..4], b"II\x2a\x00");
        let ifd = u32::from_le_bytes(t[4..8].try_into().unwrap()) as usize;
        assert_eq!(ifd % 2, 0, "IFD must be word aligned");
        let n = u16::from_le_bytes(t[ifd..ifd + 2].try_into().unwrap()) as usize;
        assert_eq!(n, 12);
        let end = ifd + 2 + n * 12 + 4 + 16;
        assert_eq!(t.len(), end, "no trailing garbage");
        // tags strictly ascending (a TIFF requirement)
        let tags: Vec<u16> = (0..n)
            .map(|i| u16::from_le_bytes(t[ifd + 2 + i * 12..ifd + 4 + i * 12].try_into().unwrap()))
            .collect();
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "{tags:?}");
    }
}

/// Group 3 streams written by libtiff (via Pillow): 1-D, 2-D (K>0) and byte-aligned EOLs.
#[cfg(test)]
mod g3_tests {
    use super::*;

    const ROWS: [&str; 30] = [
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000110000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000110000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000011110000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000011110000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111111110000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111111111111000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111111111111000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111111111111000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000001111111110000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000",
    ];
    const CASES: [(&str, &str, i64); 3] = [
        ("g3_1d", "0013503c350004d40f0d40013503e70690013503e70690013503c350004d40f0d40013503c350004d40f0d40013503c350004d4162c35c004d4162c35c004d40f0d40013503c350004d40f0d40013503c350004d40f560d20026a07ab06900135058a0690009a82c060d00026a0b018340009a82c060d00026a0b140d20013503c350004d40f0d40013503c350004d40f6e0d00026a07b70680013503db8340009a81edc1a0004d40f6e0d00", 0),
        ("g3_2d", "0019a81e1a8002c0066a07ce0d2002f0019a81e1a8002c0066a0786a000b0019a81e1a80029058b800cd4162c35c0051800cd40f0d400160033503c350005207ab800cd40f560d20029058a40066a0b01834000bc0066a0b01834000b340d20019a81e1a8002c0066a0786a000a40f6f0019a81edc1a0005e0033503db834000bc", 1),
        ("g3_align", "00013503c35000013503c35000013503e70690013503e70690013503c35000013503c35000013503c35000013503c35000013503c350000135058b0d700135058b0d70013503c35000013503c35000013503c35000013503c35000013503d5834800013503d58348000135058a06900001350580c1a00001350580c1a00001350580c1a0000135058a069000013503c35000013503c35000013503c35000013503db834000013503db834000013503db834000013503db834000013503db8340", 4),
    ];

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn decodes_libtiff_group3_streams() {
        for (name, hex, t4) in CASES {
            let k = if t4 & 1 == 1 { 4 } else { 0 };
            let (bits, ok) = decode_g3(&unhex(hex), 100, 30, k, t4 & 4 != 0);
            assert_eq!(ok, 30, "{name}");
            for (y, want) in ROWS.iter().enumerate() {
                let got: String = bits[y].iter().map(|&b| if b { '1' } else { '0' }).collect();
                // Pillow stores the page inverted (white pixel = black run in the stream)
                let want: String = want
                    .chars()
                    .map(|c| if c == '1' { '0' } else { '1' })
                    .collect();
                assert_eq!(got, want, "{name} row {y}");
            }
        }
    }

    #[test]
    fn damaged_group3_never_panics_and_resynchronises() {
        let good = unhex(CASES[0].1);
        let mut s = 0x1234_5678_9abc_def1u64;
        for _ in 0..2000 {
            let mut d = good.clone();
            for _ in 0..1 + s % 5 {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let i = (s as usize) % d.len();
                d[i] ^= 1 << (s >> 40 & 7);
            }
            for k in [0, 4] {
                let (rows, _) = decode_g3(&d, 100, 30, k, false);
                assert_eq!(rows.len(), 30);
            }
        }
        // truncated and empty input
        assert_eq!(decode_g3(&[], 100, 5, 0, false).0.len(), 5);
        assert_eq!(decode_g3(&good[..10], 100, 30, 0, false).0.len(), 30);
    }
}
