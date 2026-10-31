//! CCITT Group 4 (ITU-T T.6) encoder for 1-bit pages. Text pages shrink to roughly
//! a third of Flate+predictor, and every PDF viewer decodes it natively.
//! Ink = black. Tables are from ITU-T T.4 / T.6.

const WHITE_TERM: [&str; 64] = [
    "00110101","000111","0111","1000","1011","1100","1110","1111","10011","10100","00111","01000","001000","000011","110100","110101",
    "101010","101011","0100111","0001100","0001000","0010111","0000011","0000100","0101000","0101011","0010011","0100100","0011000","00000010","00000011","00011010",
    "00011011","00010010","00010011","00010100","00010101","00010110","00010111","00101000","00101001","00101010","00101011","00101100","00101101","00000100","00000101","00001010",
    "00001011","01010010","01010011","01010100","01010101","00100100","00100101","01011000","01011001","01011010","01011011","01001010","01001011","00110010","00110011","00110100",
];
const WHITE_MAKEUP: [&str; 27] = [
    "11011","10010","010111","0110111","00110110","00110111","01100100","01100101","01101000","01100111","011001100","011001101","011010010","011010011",
    "011010100","011010101","011010110","011010111","011011000","011011001","011011010","011011011","010011000","010011001","010011010","011000","010011011",
];
const BLACK_TERM: [&str; 64] = [
    "0000110111","010","11","10","011","0011","0010","00011","000101","000100","0000100","0000101","0000111","00000100","00000111","000011000",
    "0000010111","0000011000","0000001000","00001100111","00001101000","00001101100","00000110111","00000101000","00000010111","00000011000","000011001010","000011001011","000011001100","000011001101","000001101000","000001101001",
    "000001101010","000001101011","000011010010","000011010011","000011010100","000011010101","000011010110","000011010111","000001101100","000001101101","000011011010","000011011011","000001010100","000001010101","000001010110","000001010111",
    "000001100100","000001100101","000001010010","000001010011","000000100100","000000110111","000000111000","000000100111","000000101000","000001011000","000001011001","000000101011","000000101100","000001011010","000001100110","000001100111",
];
const BLACK_MAKEUP: [&str; 27] = [
    "0000001111","000011001000","000011001001","000001011011","000000110011","000000110100","000000110101","0000001101100","0000001101101","0000001001010","0000001001011","0000001001100","0000001001101","0000001110010",
    "0000001110011","0000001110100","0000001110101","0000001110110","0000001110111","0000001010010","0000001010011","0000001010100","0000001010101","0000001011010","0000001011011","0000001100100","0000001100101",
];
const EXT_MAKEUP: [&str; 13] = [
    "00000001000","00000001100","00000001101","000000010010","000000010011","000000010100","000000010101","000000010110","000000010111","000000011100","000000011101","000000011110","000000011111",
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
        b.put(if black { BLACK_MAKEUP[i] } else { WHITE_MAKEUP[i] });
        len %= 64;
    }
    b.put(if black { BLACK_TERM[len] } else { WHITE_TERM[len] });
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
    let mut bits = Bits { out: Vec::new(), acc: 0, n: 0 };
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
    encode(w, (0..b.h as usize).map(|y| b.data[y * w..(y + 1) * w].iter().map(|&v| v != 0).collect()))
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
        (259, 3, 1, 4),    // CCITT Group 4
        (262, 3, 1, 0),    // WhiteIsZero
        (273, 4, 1, 8),    // strip offset
        (277, 3, 1, 1),    // samples per pixel
        (278, 4, 1, b.h),  // rows per strip
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
