//! Huffman tables and bit reader for JBIG2 (ITU-T T.88 Annex B). The fifteen standard tables
//! are transcribed from the standard's data (checked in the tests: every table's codes fill the
//! code space exactly and its ranges are contiguous); user tables (segment type 53) are parsed
//! with the same code assignment.

use std::fmt;

#[derive(Debug)]
pub struct HuffError(pub String);
impl fmt::Display for HuffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
type R<T> = Result<T, HuffError>;
fn err<T>(m: &str) -> R<T> {
    Err(HuffError(m.into()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Normal,
    Lower,
    Upper,
    Oob,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub low: i64,
    pub preflen: u8,
    pub rangelen: u8,
    pub kind: Kind,
}

/// MSB-first bit reader.
pub struct Bits<'a> {
    d: &'a [u8],
    pub pos: usize, // in bits
}
impl<'a> Bits<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0 }
    }
    pub fn bit(&mut self) -> R<u32> {
        let b = *self
            .d
            .get(self.pos / 8)
            .ok_or_else(|| HuffError("truncated JBIG2 Huffman data".into()))?;
        let v = (b >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(v as u32)
    }
    pub fn bits(&mut self, n: u32) -> R<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }
    /// Skip to the next byte boundary.
    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
    /// Byte offset (after `align`).
    pub fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }
    pub fn skip_bytes(&mut self, n: usize) -> R<()> {
        self.align();
        let end = self.pos / 8 + n;
        if end > self.d.len() {
            return err("truncated JBIG2 Huffman data");
        }
        self.pos = end * 8;
        Ok(())
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.d[(self.pos / 8).min(self.d.len())..]
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    lines: Vec<Line>,
    /// (prefix length, code) → line: decoding costs one lookup per bit, however many lines
    index: std::collections::HashMap<(u8, u32), usize>,
}

impl Table {
    /// Assign prefix codes (T.88 B.3): shorter codes first, lines of equal length in order.
    pub fn new(lines: Vec<Line>) -> R<Table> {
        let maxlen = lines.iter().map(|l| l.preflen).max().unwrap_or(0) as usize;
        if maxlen > 32 {
            return err("JBIG2 Huffman prefix too long");
        }
        let mut count = vec![0u32; maxlen + 2];
        for l in &lines {
            count[l.preflen as usize] += 1;
        }
        count[0] = 0;
        let mut first = vec![0u64; maxlen + 2];
        let mut codes = vec![0u32; lines.len()];
        for len in 1..=maxlen {
            first[len] = (first[len - 1] + count[len - 1] as u64) << 1;
            let mut cur = first[len];
            for (i, l) in lines.iter().enumerate() {
                if l.preflen as usize == len {
                    if cur >= 1u64 << len {
                        return err("JBIG2 Huffman table is over-subscribed");
                    }
                    codes[i] = cur as u32;
                    cur += 1;
                }
            }
        }
        let index = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.preflen > 0)
            .map(|(i, l)| ((l.preflen, codes[i]), i))
            .collect();
        Ok(Table { lines, index })
    }

    /// Decode one value; `None` is the out-of-band value.
    pub fn decode(&self, r: &mut Bits) -> R<Option<i64>> {
        let (mut code, mut len) = (0u32, 0u8);
        loop {
            code = (code << 1) | r.bit()?;
            len += 1;
            if len > 32 {
                return err("bad JBIG2 Huffman code");
            }
            if let Some(&i) = self.index.get(&(len, code)) {
                let l = &self.lines[i];
                return match l.kind {
                    Kind::Oob => Ok(None),
                    Kind::Normal => Ok(Some(l.low + r.bits(l.rangelen as u32)? as i64)),
                    Kind::Upper => Ok(Some(l.low + r.bits(32)? as i64)),
                    Kind::Lower => Ok(Some(l.low - r.bits(32)? as i64)),
                };
            }
        }
    }

    /// Like `decode`, but out-of-band is an error.
    pub fn int(&self, r: &mut Bits) -> R<i64> {
        self.decode(r)?
            .ok_or_else(|| HuffError("unexpected JBIG2 out-of-band value".into()))
    }

    /// A table whose lines are the integers `0..lens.len()` with the given prefix lengths
    /// (symbol-ID codes and the run-length codes of the symbol-ID table).
    pub fn from_lengths(lens: &[u8]) -> R<Table> {
        Table::new(
            lens.iter()
                .enumerate()
                .map(|(i, &l)| Line {
                    low: i as i64,
                    preflen: l,
                    rangelen: 0,
                    kind: Kind::Normal,
                })
                .collect(),
        )
    }

    /// A user-supplied table (segment type 53).
    pub fn parse_custom(d: &[u8]) -> R<Table> {
        let flags = *d
            .first()
            .ok_or_else(|| HuffError("truncated table".into()))?;
        let oob = flags & 1 != 0;
        let ps = ((flags >> 1) & 7) as u32 + 1;
        let rs = ((flags >> 4) & 7) as u32 + 1;
        let rd = |at: usize| {
            d.get(at..at + 4)
                .map(|s| i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as i64)
                .ok_or_else(|| HuffError("truncated table".into()))
        };
        let (low, high) = (rd(1)?, rd(5)?);
        if high < low || high - low > 1 << 20 {
            return err("bad JBIG2 table range");
        }
        let mut r = Bits::new(&d[9..]);
        let mut lines = Vec::new();
        let mut cur = low;
        while cur < high {
            let pl = r.bits(ps)? as u8;
            let rl = r.bits(rs)? as u8;
            if rl > 32 {
                return err("bad JBIG2 table range length");
            }
            lines.push(Line {
                low: cur,
                preflen: pl,
                rangelen: rl,
                kind: Kind::Normal,
            });
            cur += 1i64 << rl;
            if lines.len() > 65_536 {
                return err("JBIG2 table too large");
            }
        }
        lines.push(Line {
            low: low - 1,
            preflen: r.bits(ps)? as u8,
            rangelen: 32,
            kind: Kind::Lower,
        });
        lines.push(Line {
            low: high,
            preflen: r.bits(ps)? as u8,
            rangelen: 32,
            kind: Kind::Upper,
        });
        if oob {
            lines.push(Line {
                low: 0,
                preflen: r.bits(ps)? as u8,
                rangelen: 0,
                kind: Kind::Oob,
            });
        }
        Table::new(lines)
    }

    /// Standard table B.`n` (1..=15).
    pub fn standard(n: u8) -> Table {
        let (rows, oob): (&[(u8, u8, i64)], bool) = STD[(n - 1) as usize];
        let mut lines: Vec<Line> = Vec::new();
        let m = rows.len();
        // layout of the rows: normal lines, then (lower, upper) range lines, then OOB if any
        let tail = 2 + usize::from(oob);
        for (i, &(pl, rl, low)) in rows.iter().enumerate() {
            let kind = if i + tail == m {
                Kind::Lower
            } else if i + tail == m + 1 {
                Kind::Upper
            } else if oob && i + 1 == m {
                Kind::Oob
            } else {
                Kind::Normal
            };
            lines.push(Line {
                low,
                preflen: pl,
                rangelen: rl,
                kind,
            });
        }
        Table::new(lines).expect("standard JBIG2 table")
    }
}

type Std = (&'static [(u8, u8, i64)], bool);
// (PREFLEN, RANGELEN, RANGELOW); a PREFLEN of 0 marks a line that does not exist.
const STD: [Std; 15] = [
    (
        &[
            (1, 4, 0),
            (2, 8, 16),
            (3, 16, 272),
            (0, 32, -1),
            (3, 32, 65808),
        ],
        false,
    ),
    (
        &[
            (1, 0, 0),
            (2, 0, 1),
            (3, 0, 2),
            (4, 3, 3),
            (5, 6, 11),
            (0, 32, -1),
            (6, 32, 75),
            (6, 0, 0),
        ],
        true,
    ),
    (
        &[
            (8, 8, -256),
            (1, 0, 0),
            (2, 0, 1),
            (3, 0, 2),
            (4, 3, 3),
            (5, 6, 11),
            (8, 32, -257),
            (7, 32, 75),
            (6, 0, 0),
        ],
        true,
    ),
    (
        &[
            (1, 0, 1),
            (2, 0, 2),
            (3, 0, 3),
            (4, 3, 4),
            (5, 6, 12),
            (0, 32, -1),
            (5, 32, 76),
        ],
        false,
    ),
    (
        &[
            (7, 8, -255),
            (1, 0, 1),
            (2, 0, 2),
            (3, 0, 3),
            (4, 3, 4),
            (5, 6, 12),
            (7, 32, -256),
            (6, 32, 76),
        ],
        false,
    ),
    (
        &[
            (5, 10, -2048),
            (4, 9, -1024),
            (4, 8, -512),
            (4, 7, -256),
            (5, 6, -128),
            (5, 5, -64),
            (4, 5, -32),
            (2, 7, 0),
            (3, 7, 128),
            (3, 8, 256),
            (4, 9, 512),
            (4, 10, 1024),
            (6, 32, -2049),
            (6, 32, 2048),
        ],
        false,
    ),
    (
        &[
            (4, 9, -1024),
            (3, 8, -512),
            (4, 7, -256),
            (5, 6, -128),
            (5, 5, -64),
            (4, 5, -32),
            (4, 5, 0),
            (5, 5, 32),
            (5, 6, 64),
            (4, 7, 128),
            (3, 8, 256),
            (3, 9, 512),
            (3, 10, 1024),
            (5, 32, -1025),
            (5, 32, 2048),
        ],
        false,
    ),
    (
        &[
            (8, 3, -15),
            (9, 1, -7),
            (8, 1, -5),
            (9, 0, -3),
            (7, 0, -2),
            (4, 0, -1),
            (2, 1, 0),
            (5, 0, 2),
            (6, 0, 3),
            (3, 4, 4),
            (6, 1, 20),
            (4, 4, 22),
            (4, 5, 38),
            (5, 6, 70),
            (5, 7, 134),
            (6, 7, 262),
            (7, 8, 390),
            (6, 10, 646),
            (9, 32, -16),
            (9, 32, 1670),
            (2, 0, 0),
        ],
        true,
    ),
    (
        &[
            (8, 4, -31),
            (9, 2, -15),
            (8, 2, -11),
            (9, 1, -7),
            (7, 1, -5),
            (4, 1, -3),
            (3, 1, -1),
            (3, 1, 1),
            (5, 1, 3),
            (6, 1, 5),
            (3, 5, 7),
            (6, 2, 39),
            (4, 5, 43),
            (4, 6, 75),
            (5, 7, 139),
            (5, 8, 267),
            (6, 8, 523),
            (7, 9, 779),
            (6, 11, 1291),
            (9, 32, -32),
            (9, 32, 3339),
            (2, 0, 0),
        ],
        true,
    ),
    (
        &[
            (7, 4, -21),
            (8, 0, -5),
            (7, 0, -4),
            (5, 0, -3),
            (2, 2, -2),
            (5, 0, 2),
            (6, 0, 3),
            (7, 0, 4),
            (8, 0, 5),
            (2, 6, 6),
            (5, 5, 70),
            (6, 5, 102),
            (6, 6, 134),
            (6, 7, 198),
            (6, 8, 326),
            (6, 9, 582),
            (6, 10, 1094),
            (7, 11, 2118),
            (8, 32, -22),
            (8, 32, 4166),
            (2, 0, 0),
        ],
        true,
    ),
    (
        &[
            (1, 0, 1),
            (2, 1, 2),
            (4, 0, 4),
            (4, 1, 5),
            (5, 1, 7),
            (5, 2, 9),
            (6, 2, 13),
            (7, 2, 17),
            (7, 3, 21),
            (7, 4, 29),
            (7, 5, 45),
            (7, 6, 77),
            (0, 32, 0),
            (7, 32, 141),
        ],
        false,
    ),
    (
        &[
            (1, 0, 1),
            (2, 0, 2),
            (3, 1, 3),
            (5, 0, 5),
            (5, 1, 6),
            (6, 1, 8),
            (7, 0, 10),
            (7, 1, 11),
            (7, 2, 13),
            (7, 3, 17),
            (7, 4, 25),
            (8, 5, 41),
            (0, 32, 0),
            (8, 32, 73),
        ],
        false,
    ),
    (
        &[
            (1, 0, 1),
            (3, 0, 2),
            (4, 0, 3),
            (5, 0, 4),
            (4, 1, 5),
            (3, 3, 7),
            (6, 1, 15),
            (6, 2, 17),
            (6, 3, 21),
            (6, 4, 29),
            (6, 5, 45),
            (7, 6, 77),
            (0, 32, 0),
            (7, 32, 141),
        ],
        false,
    ),
    (
        &[
            (3, 0, -2),
            (3, 0, -1),
            (1, 0, 0),
            (3, 0, 1),
            (3, 0, 2),
            (0, 32, -3),
            (0, 32, 3),
        ],
        false,
    ),
    (
        &[
            (7, 4, -24),
            (6, 2, -8),
            (5, 1, -4),
            (4, 0, -2),
            (3, 0, -1),
            (1, 0, 0),
            (3, 0, 1),
            (4, 0, 2),
            (5, 1, 3),
            (6, 2, 5),
            (7, 4, 9),
            (7, 32, -25),
            (7, 32, 25),
        ],
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_tables_are_complete_prefix_codes_with_contiguous_ranges() {
        for n in 1..=15u8 {
            let t = Table::standard(n);
            // Kraft sum: the codes of the lines that exist fill the code space exactly
            let sum: f64 = t
                .lines
                .iter()
                .filter(|l| l.preflen > 0)
                .map(|l| 0.5f64.powi(l.preflen as i32))
                .sum();
            assert!((sum - 1.0).abs() < 1e-9, "B.{n}: Kraft sum {sum}");
            // normal lines follow one another without gaps
            let normals: Vec<&Line> = t.lines.iter().filter(|l| l.kind == Kind::Normal).collect();
            for w in normals.windows(2) {
                assert_eq!(
                    w[0].low + (1i64 << w[0].rangelen),
                    w[1].low,
                    "B.{n}: gap after {}",
                    w[0].low
                );
            }
            // lower range ends just below the first line, upper starts right after the last
            let first = normals[0].low;
            let last = normals[normals.len() - 1];
            for l in &t.lines {
                if l.preflen == 0 {
                    continue;
                }
                match l.kind {
                    Kind::Lower => assert_eq!(l.low, first - 1, "B.{n}: lower"),
                    Kind::Upper => {
                        assert_eq!(l.low, last.low + (1i64 << last.rangelen), "B.{n}: upper")
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn decodes_known_codes() {
        // B.1: '0' + 4 bits = 0..15; '10' + 8 bits = 16..271
        let t = Table::standard(1);
        let mut r = Bits::new(&[0b0010_1100, 0b0000_0010, 0b0000_0000]);
        assert_eq!(t.decode(&mut r).unwrap(), Some(5));
        assert_eq!(t.decode(&mut r).unwrap(), Some(16 + 1));
        // B.2: '0' → 0, '10' → 1, '110' → 2; OOB is the 6-bit code 111111
        let t = Table::standard(2);
        let mut r = Bits::new(&[0b0101_1011, 0b1111_0000]);
        assert_eq!(t.decode(&mut r).unwrap(), Some(0));
        assert_eq!(t.decode(&mut r).unwrap(), Some(1));
        assert_eq!(t.decode(&mut r).unwrap(), Some(2));
        assert_eq!(t.decode(&mut r).unwrap(), None);
    }

    #[test]
    fn run_length_style_tables_assign_codes_by_length_then_order() {
        let t = Table::from_lengths(&[2, 1, 3, 3]).unwrap();
        // lengths 1:{1}, 2:{0}, 3:{2,3} -> 0 | 10 | 110 | 111
        let mut r = Bits::new(&[0b0101_1011, 0b1000_0000]);
        assert_eq!(t.int(&mut r).unwrap(), 1);
        assert_eq!(t.int(&mut r).unwrap(), 0);
        assert_eq!(t.int(&mut r).unwrap(), 2);
        assert_eq!(t.int(&mut r).unwrap(), 3);
        assert!(Table::from_lengths(&[1, 1, 1]).is_err());
    }
}
