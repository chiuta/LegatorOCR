//! A small, tolerant PDF *reader* for scanned documents: it finds the pages, picks the
//! scan image of each page and decodes it (Flate / LZW / ASCII85 / hex / run-length
//! filters with PNG and TIFF predictors, JPEG via the `image` crate, CCITT Group 4
//! via [`crate::g4::decode`]). Written from the PDF specification; it does not try to
//! render text or vector graphics, and it refuses encrypted files, JBIG2 and JPEG 2000
//! with a clear message.
//!
//! The reader never trusts the cross-reference table (damaged and hybrid files are
//! common): it scans the file for `N G obj` markers, so repaired and incrementally
//! updated files open as well. Every size read from the file is bounded.

use image::{DynamicImage, GrayImage, RgbImage};
use std::collections::{HashMap, HashSet};
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError(pub String);

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ReadError {}

type R<T> = Result<T, ReadError>;

fn fail<T>(m: impl Into<String>) -> R<T> {
    Err(ReadError(m.into()))
}

/// Largest decoded image / stream we accept.
const MAX_PIXELS: u64 = 500_000_000;
const MAX_STREAM_BYTES: u64 = 1 << 31;
const MAX_PAGES: usize = 200_000;

// ---------------------------------------------------------------------------
// objects

type Dict = HashMap<String, Val>;

#[derive(Debug, Clone)]
enum Val {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    Arr(Vec<Val>),
    Dict(Dict),
    Ref(u32),
    /// Stream: dictionary and the byte range of its raw data inside the file
    /// (or, for objects of an object stream, never used).
    Stream(Dict, usize, usize),
}

impl Val {
    fn num(&self) -> Option<f64> {
        match self {
            Val::Int(i) => Some(*i as f64),
            Val::Real(r) => Some(*r),
            _ => None,
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            Val::Real(r) if r.is_finite() => Some(*r as i64),
            _ => None,
        }
    }
    fn name(&self) -> Option<&str> {
        match self {
            Val::Name(n) => Some(n),
            _ => None,
        }
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}
fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

struct P<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> P<'a> {
    fn new(d: &'a [u8], pos: usize) -> Self {
        P { d, pos }
    }
    fn peek(&self) -> Option<u8> {
        self.d.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_ws(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
    fn word(&mut self) -> &'a [u8] {
        let s = self.pos;
        while let Some(b) = self.peek() {
            if is_ws(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        &self.d[s..self.pos]
    }
    fn int_word(&mut self) -> Option<u64> {
        let save = self.pos;
        self.skip_ws();
        let w = self.word();
        if !w.is_empty() && w.len() <= 10 && w.iter().all(|b| b.is_ascii_digit()) {
            std::str::from_utf8(w).ok()?.parse().ok()
        } else {
            self.pos = save;
            None
        }
    }

    fn val(&mut self, depth: u32) -> Option<Val> {
        if depth > 48 {
            return None;
        }
        self.skip_ws();
        match self.peek()? {
            b'<' if self.d.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut m = Dict::new();
                loop {
                    self.skip_ws();
                    match self.peek()? {
                        b'>' => {
                            self.pos += if self.d.get(self.pos + 1) == Some(&b'>') {
                                2
                            } else {
                                1
                            };
                            return Some(Val::Dict(m));
                        }
                        b'/' => {
                            let k = self.name()?;
                            let v = self.val(depth + 1)?;
                            m.insert(k, v);
                        }
                        _ => return None,
                    }
                }
            }
            b'<' => {
                self.pos += 1;
                let mut out = Vec::new();
                let mut hi: Option<u8> = None;
                loop {
                    let b = self.peek()?;
                    self.pos += 1;
                    if b == b'>' {
                        if let Some(h) = hi {
                            out.push(h << 4);
                        }
                        return Some(Val::Str(out));
                    }
                    if let Some(v) = (b as char).to_digit(16) {
                        match hi.take() {
                            Some(h) => out.push((h << 4) | v as u8),
                            None => hi = Some(v as u8),
                        }
                    }
                }
            }
            b'(' => {
                self.pos += 1;
                let mut out = Vec::new();
                let mut nest = 1;
                loop {
                    let b = self.peek()?;
                    self.pos += 1;
                    match b {
                        b'(' => {
                            nest += 1;
                            out.push(b);
                        }
                        b')' => {
                            nest -= 1;
                            if nest == 0 {
                                return Some(Val::Str(out));
                            }
                            out.push(b);
                        }
                        b'\\' => {
                            let c = self.peek()?;
                            self.pos += 1;
                            match c {
                                b'n' => out.push(10),
                                b'r' => out.push(13),
                                b't' => out.push(9),
                                b'b' => out.push(8),
                                b'f' => out.push(12),
                                b'0'..=b'7' => {
                                    let mut v = (c - b'0') as u32;
                                    for _ in 0..2 {
                                        match self.peek() {
                                            Some(o @ b'0'..=b'7') => {
                                                v = v * 8 + (o - b'0') as u32;
                                                self.pos += 1;
                                            }
                                            _ => break,
                                        }
                                    }
                                    out.push(v as u8);
                                }
                                b'\r' => {
                                    if self.peek() == Some(b'\n') {
                                        self.pos += 1;
                                    }
                                }
                                b'\n' => {}
                                other => out.push(other),
                            }
                        }
                        _ => out.push(b),
                    }
                }
            }
            b'/' => Some(Val::Name(self.name()?)),
            b'[' => {
                self.pos += 1;
                let mut v = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        return Some(Val::Arr(v));
                    }
                    v.push(self.val(depth + 1)?);
                }
            }
            b')' | b'>' | b']' | b'{' | b'}' => None,
            _ => {
                let w = self.word();
                match w {
                    b"true" => Some(Val::Bool(true)),
                    b"false" => Some(Val::Bool(false)),
                    b"null" => Some(Val::Null),
                    _ => {
                        let s = std::str::from_utf8(w).ok()?;
                        if s.is_empty()
                            || !s
                                .bytes()
                                .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
                        {
                            return None;
                        }
                        if s.contains('.') {
                            let v: f64 = s
                                .parse()
                                .ok()
                                .or_else(|| s.trim_end_matches('.').parse().ok())?;
                            return Some(Val::Real(v));
                        }
                        let n: i64 = s.parse().ok()?;
                        // `N G R` is an indirect reference
                        if n >= 0 && n <= u32::MAX as i64 && !s.starts_with(['+', '-']) {
                            let save = self.pos;
                            if self.int_word().is_some() {
                                self.skip_ws();
                                if self.peek() == Some(b'R')
                                    && self
                                        .d
                                        .get(self.pos + 1)
                                        .is_none_or(|&b| is_ws(b) || is_delim(b))
                                {
                                    self.pos += 1;
                                    return Some(Val::Ref(n as u32));
                                }
                            }
                            self.pos = save;
                        }
                        Some(Val::Int(n))
                    }
                }
            }
        }
    }

    fn name(&mut self) -> Option<String> {
        if self.peek()? != b'/' {
            return None;
        }
        self.pos += 1;
        let w = self.word();
        let mut out = Vec::with_capacity(w.len());
        let mut i = 0;
        while i < w.len() {
            if w[i] == b'#' && i + 2 < w.len() {
                let h = std::str::from_utf8(&w[i + 1..i + 3])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok());
                if let Some(h) = h {
                    out.push(h);
                    i += 3;
                    continue;
                }
            }
            out.push(w[i]);
            i += 1;
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Parse `N G obj <value> [stream…endstream]` at `off`.
fn parse_object(d: &[u8], off: usize) -> Option<Val> {
    let mut p = P::new(d, off);
    p.int_word()?;
    p.int_word()?;
    p.skip_ws();
    if p.word() != b"obj" {
        return None;
    }
    let v = p.val(0)?;
    p.skip_ws();
    if let Val::Dict(dict) = &v {
        if d[p.pos..].starts_with(b"stream") {
            p.pos += 6;
            if d.get(p.pos) == Some(&b'\r') {
                p.pos += 1;
            }
            if d.get(p.pos) == Some(&b'\n') {
                p.pos += 1;
            }
            let start = p.pos.min(d.len());
            // trust /Length only when "endstream" really follows it
            let by_len = dict
                .get("Length")
                .and_then(|l| l.int())
                .filter(|&l| l >= 0)
                .and_then(|l| {
                    let end = start.checked_add(l as usize)?;
                    if end > d.len() {
                        return None;
                    }
                    let mut q = P::new(d, end);
                    q.skip_ws();
                    d[q.pos..].starts_with(b"endstream").then_some(end)
                });
            let end = match by_len {
                Some(e) => e,
                None => {
                    let e = find(d, b"endstream", start).unwrap_or(d.len());
                    let mut e2 = e;
                    if e2 > start && d[e2 - 1] == b'\n' {
                        e2 -= 1;
                    }
                    if e2 > start && d[e2 - 1] == b'\r' {
                        e2 -= 1;
                    }
                    e2
                }
            };
            return Some(Val::Stream(dict.clone(), start, end));
        }
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// filters

fn limited_read(mut r: impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = (&mut r).take(MAX_STREAM_BYTES).read_to_end(&mut out); // keep what was readable
    out
}

fn flate(data: &[u8]) -> Vec<u8> {
    let out = limited_read(flate2::read::ZlibDecoder::new(data));
    if out.is_empty() && !data.is_empty() {
        // some writers omit the zlib header
        return limited_read(flate2::read::DeflateDecoder::new(data));
    }
    out
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        if let Some(v) = (b as char).to_digit(16) {
            match hi.take() {
                Some(h) => out.push((h << 4) | v as u8),
                None => hi = Some(v as u8),
            }
        }
    }
    if let Some(h) = hi {
        out.push(h << 4);
    }
    out
}

fn ascii85(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut group, mut n) = ([0u32; 5], 0usize);
    let mut i = 0;
    // optional <~ prefix
    if data.starts_with(b"<~") {
        i = 2;
    }
    let flush = |group: &[u32; 5], n: usize, out: &mut Vec<u8>| {
        if n < 2 {
            return;
        }
        let mut g = *group;
        for v in g.iter_mut().skip(n) {
            *v = 84;
        }
        let v = g.iter().fold(0u64, |a, &x| a * 85 + x as u64) as u32;
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    };
    while i < data.len() {
        let b = data[i];
        i += 1;
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = (b - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |a, &x| a * 85 + x as u64) as u32;
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    flush(&group, n, &mut out);
    out
}

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let l = data[i] as usize;
        i += 1;
        if l == 128 {
            break;
        }
        if l < 128 {
            let n = (l + 1).min(data.len() - i);
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        } else if i < data.len() {
            out.extend(std::iter::repeat_n(data[i], 257 - l));
            i += 1;
        }
        if out.len() as u64 > MAX_STREAM_BYTES {
            break;
        }
    }
    out
}

fn lzw(data: &[u8], early_change: bool) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut table: Vec<(u32, u8)> = Vec::new(); // (prefix code, byte); codes 258.. map to index-258
    let (mut bits, mut nbits, mut width) = (0u32, 0u32, 9u32);
    let mut prev: Option<u32> = None;
    let early = early_change as usize;
    let expand = |code: u32, table: &[(u32, u8)], buf: &mut Vec<u8>| {
        buf.clear();
        let mut c = code;
        while c >= 258 {
            let Some(&(p, b)) = table.get((c - 258) as usize) else {
                break;
            };
            buf.push(b);
            c = p;
        }
        if c < 256 {
            buf.push(c as u8);
        }
        buf.reverse();
    };
    let mut buf = Vec::new();
    for &byte in data {
        bits = (bits << 8) | byte as u32;
        nbits += 8;
        while nbits >= width {
            let code = (bits >> (nbits - width)) & ((1 << width) - 1);
            nbits -= width;
            match code {
                256 => {
                    table.clear();
                    width = 9;
                    prev = None;
                }
                257 => return out,
                _ => {
                    let next = 258 + table.len() as u32;
                    if code < 256 || code < next {
                        expand(code, &table, &mut buf);
                    } else if code == next {
                        let Some(p) = prev else { return out };
                        expand(p, &table, &mut buf);
                        let first = buf[0];
                        buf.push(first);
                    } else {
                        return out;
                    }
                    if let Some(p) = prev {
                        table.push((p, buf[0]));
                    }
                    out.extend_from_slice(&buf);
                    prev = Some(code);
                    let n = 258 + table.len() + early;
                    width = match n {
                        0..=511 => 9,
                        512..=1023 => 10,
                        1024..=2047 => 11,
                        _ => 12,
                    };
                    if table.len() > 4096 - 258 {
                        return out;
                    }
                }
            }
            if out.len() as u64 > MAX_STREAM_BYTES {
                return out;
            }
        }
    }
    out
}

/// Undo a PNG (`predictor >= 10`) or TIFF (`2`) predictor.
fn unpredict(data: Vec<u8>, predictor: i64, colors: usize, bpc: usize, columns: usize) -> Vec<u8> {
    if predictor < 2 || colors == 0 || columns == 0 || !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        return data;
    }
    // hostile /Colors and /Columns must not overflow or ask for gigabytes per row
    let Some(bits_per_row) = colors.checked_mul(bpc).and_then(|b| b.checked_mul(columns)) else {
        return data;
    };
    let row_bytes = bits_per_row.div_ceil(8);
    if row_bytes == 0 || row_bytes > (1 << 26) || colors > 64 {
        return data;
    }
    let bpp = (colors * bpc).div_ceil(8).max(1);
    if predictor == 2 {
        if bpc != 8 {
            return data; // sub-byte and 16-bit TIFF prediction are not supported; leave as is
        }
        let mut d = data;
        for row in d.chunks_mut(row_bytes) {
            for i in colors..row.len() {
                row[i] = row[i].wrapping_add(row[i - colors]);
            }
        }
        return d;
    }
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row_bytes];
    for chunk in data.chunks(row_bytes + 1) {
        let (ft, body) = (chunk[0], &chunk[1..]);
        let mut row = vec![0u8; row_bytes];
        row[..body.len()].copy_from_slice(body);
        for i in 0..row_bytes {
            let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let add = match ft {
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            row[i] = row[i].wrapping_add(add as u8);
        }
        out.extend_from_slice(&row);
        prev = row;
    }
    out
}

// ---------------------------------------------------------------------------
// colour spaces

#[derive(Debug, Clone)]
enum Cs {
    Gray,
    Rgb,
    Cmyk,
    Indexed {
        base: Box<Cs>,
        hival: usize,
        table: Vec<u8>,
    },
}

impl Cs {
    fn comps(&self) -> usize {
        match self {
            Cs::Gray | Cs::Indexed { .. } => 1,
            Cs::Rgb => 3,
            Cs::Cmyk => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// document

struct Placement {
    dict: Dict,
    s: usize,
    e: usize,
    m: [f64; 6],
    fill: [u8; 3],
}

pub struct PdfDoc {
    data: Vec<u8>,
    offs: HashMap<u32, usize>,
    /// Objects stored in object streams: number → (stream index, byte offset in that stream's data).
    comp: HashMap<u32, (usize, usize)>,
    stms: Vec<Vec<u8>>,
    pages: Vec<Dict>,
    crypt: Option<crate::crypt::Crypt>,
    /// (file offset of `N G obj`, N) sorted, to find the object a stream belongs to.
    by_off: Vec<(usize, u32)>,
}

/// What [`PdfDoc::page_info`] reports without decoding the image.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    pub width_px: u32,
    pub height_px: u32,
    /// Scan resolution derived from the page size, when plausible (36–2400).
    pub dpi: Option<u32>,
    /// Page rotation in degrees clockwise (0/90/180/270).
    pub rotate: u16,
    /// More than one sizeable image was found on the page.
    pub layered: bool,
}

pub struct PdfPage {
    pub image: DynamicImage,
    pub info: PageInfo,
}

impl PdfDoc {
    pub fn open(data: Vec<u8>) -> R<PdfDoc> {
        if find(&data[..data.len().min(1024)], b"%PDF-", 0).is_none() {
            return fail("not a PDF file (no %PDF- header)");
        }
        let mut doc = PdfDoc {
            data,
            offs: HashMap::new(),
            comp: HashMap::new(),
            stms: Vec::new(),
            pages: Vec::new(),
            crypt: None,
            by_off: Vec::new(),
        };
        doc.scan_objects();
        doc.by_off = doc.offs.iter().map(|(n, o)| (*o, *n)).collect();
        doc.by_off.sort_unstable();
        let mut enc_val: Option<Val> = None;
        let mut id0: Vec<u8> = Vec::new();
        let mut root: Option<u32> = None;
        let mut catalog: Option<u32> = None;
        let mut page_objs: Vec<u32> = Vec::new();
        let mut objstms: Vec<u32> = Vec::new();
        let mut nums: Vec<u32> = doc.offs.keys().copied().collect();
        nums.sort_unstable();
        for n in &nums {
            let Some(Val::Dict(d) | Val::Stream(d, _, _)) = parse_object(&doc.data, doc.offs[n])
            else {
                continue;
            };
            if let Some(e) = d.get("Encrypt") {
                enc_val = Some(e.clone());
                if let Some(Val::Arr(a)) = d.get("ID") {
                    if let Some(Val::Str(s)) = a.first() {
                        id0 = s.clone();
                    }
                }
            }
            if let Some(Val::Ref(r)) = d.get("Root") {
                root = Some(*r);
            }
            match d.get("Type").and_then(|t| t.name()) {
                Some("ObjStm") => objstms.push(*n),
                Some("Catalog") => catalog = Some(*n),
                Some("Page") => page_objs.push(*n),
                _ => {}
            }
        }
        // trailers of classic files
        let mut at = 0;
        while let Some(p) = find(&doc.data, b"trailer", at) {
            at = p + 7;
            if let Some(Val::Dict(d)) = P::new(&doc.data, at).val(0) {
                if let Some(e) = d.get("Encrypt") {
                    enc_val = Some(e.clone());
                    if let Some(Val::Arr(a)) = d.get("ID") {
                        if let Some(Val::Str(s)) = a.first() {
                            id0 = s.clone();
                        }
                    }
                }
                if let Some(Val::Ref(r)) = d.get("Root") {
                    root = Some(*r);
                }
            }
        }
        if let Some(ev) = enc_val {
            doc.crypt = Some(doc.setup_crypt(&ev, &id0)?);
        }
        for n in objstms {
            doc.load_objstm(n);
        }
        // objects inside object streams may include the catalog and pages
        let mut comp_nums: Vec<u32> = doc.comp.keys().copied().collect();
        comp_nums.sort_unstable();
        for n in comp_nums {
            if let Some(Val::Dict(d)) = doc.get(n) {
                match d.get("Type").and_then(|t| t.name()) {
                    Some("Catalog") if catalog.is_none() => catalog = Some(n),
                    Some("Page") => page_objs.push(n),
                    _ => {}
                }
            }
        }
        let root = root.or(catalog);
        let mut pages = Vec::new();
        if let Some(r) = root {
            if let Some(Val::Dict(cat)) = doc.get(r) {
                if let Some(pr) = cat.get("Pages") {
                    let mut seen = HashSet::new();
                    doc.collect_pages(pr, &Dict::new(), 0, &mut seen, &mut pages);
                }
            }
        }
        if pages.is_empty() {
            // damaged page tree: fall back to every /Type /Page object, in file order
            page_objs.sort_unstable();
            page_objs.dedup();
            for n in page_objs.into_iter().take(MAX_PAGES) {
                if let Some(Val::Dict(d)) = doc.get(n) {
                    pages.push(d);
                }
            }
        }
        if pages.is_empty() {
            return fail("no pages found in the PDF");
        }
        doc.pages = pages;
        Ok(doc)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Find every `N G obj` marker; the last definition of a number wins.
    fn scan_objects(&mut self) {
        let d = &self.data;
        let mut i = 0;
        while let Some(p) = find(d, b"obj", i) {
            i = p + 3;
            // "endobj" and "objstm"-like words are not markers
            if p >= 3 && &d[p - 3..p] == b"end" {
                continue;
            }
            if d.get(p + 3).is_some_and(|&b| !(is_ws(b) || is_delim(b))) {
                continue;
            }
            // walk back over `N G `
            let mut q = p;
            let take_ws = |q: &mut usize| {
                let s = *q;
                while *q > 0 && is_ws(d[*q - 1]) {
                    *q -= 1;
                }
                *q < s
            };
            let take_int = |q: &mut usize| {
                let s = *q;
                while *q > 0 && d[*q - 1].is_ascii_digit() {
                    *q -= 1;
                }
                *q < s
            };
            if !take_ws(&mut q) || !take_int(&mut q) || !take_ws(&mut q) {
                continue;
            }
            let num_end = q;
            if !take_int(&mut q) {
                continue;
            }
            let num_start = q;
            if num_end - num_start > 10
                || (num_start > 0 && !(is_ws(d[num_start - 1]) || is_delim(d[num_start - 1])))
            {
                continue;
            }
            let Some(num) = std::str::from_utf8(&d[num_start..num_end])
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            self.offs.insert(num, num_start);
        }
    }

    fn setup_crypt(&self, ev: &Val, id0: &[u8]) -> R<crate::crypt::Crypt> {
        use crate::crypt::{Crypt, EncryptParams, Method};
        let d = match self.resolve(ev) {
            Val::Dict(d) => d,
            _ => return fail("the PDF's encryption dictionary cannot be read"),
        };
        if let Some(f) = d.get("Filter").and_then(|f| f.name()) {
            if f != "Standard" {
                return fail(format!(
                    "the PDF uses the {f} security handler, which is not supported"
                ));
            }
        }
        let int = |k: &str, dflt: i64| d.get(k).and_then(|v| self.resolve(v).int()).unwrap_or(dflt);
        let st = |k: &str| match d.get(k).map(|v| self.resolve(v)) {
            Some(Val::Str(s)) => s,
            _ => Vec::new(),
        };
        let (v, r) = (int("V", 0), int("R", 2));
        if !(1..=5).contains(&v) || !(2..=6).contains(&r) {
            return fail(format!("unsupported PDF encryption (V={v}, R={r})"));
        }
        let stm = if v >= 4 {
            let name = d
                .get("StmF")
                .and_then(|n| n.name())
                .unwrap_or("Identity")
                .to_string();
            if name == "Identity" {
                Method::None
            } else {
                let cfm = d
                    .get("CF")
                    .and_then(|c| self.dict_of(c))
                    .and_then(|cf| cf.get(&name).and_then(|f| self.dict_of(f)))
                    .and_then(|f| f.get("CFM").and_then(|m| m.name().map(str::to_string)));
                match cfm.as_deref() {
                    Some("V2") => Method::Rc4,
                    Some("AESV2") => Method::AesV2,
                    Some("AESV3") => Method::AesV3,
                    Some("None") => Method::None,
                    _ => return fail("unsupported PDF crypt filter"),
                }
            }
        } else {
            Method::Rc4
        };
        let em = !matches!(
            d.get("EncryptMetadata").map(|v| self.resolve(v)),
            Some(Val::Bool(false))
        );
        let (o, u, ue) = (st("O"), st("U"), st("UE"));
        crate::crypt::Crypt::new(&EncryptParams {
            v,
            r,
            length_bits: int("Length", 40),
            o: &o,
            u: &u,
            ue: &ue,
            p: int("P", 0),
            id0,
            encrypt_metadata: em,
            stm,
        })
        .map_err(ReadError)
        .map(|c: Crypt| c)
    }

    /// Object number and generation of the object whose stream data starts at `s`.
    fn owner_of(&self, s: usize) -> (u32, u32) {
        let i = self.by_off.partition_point(|(o, _)| *o < s);
        let Some(&(off, num)) = i.checked_sub(1).and_then(|k| self.by_off.get(k)) else {
            return (0, 0);
        };
        let d = &self.data;
        let mut q = off;
        while q < d.len() && d[q].is_ascii_digit() {
            q += 1;
        }
        while q < d.len() && is_ws(d[q]) {
            q += 1;
        }
        let mut gen = 0u32;
        while q < d.len() && d[q].is_ascii_digit() {
            gen = gen.saturating_mul(10).saturating_add((d[q] - b'0') as u32);
            q += 1;
        }
        (num, gen)
    }

    fn load_objstm(&mut self, n: u32) {
        let Some(Val::Stream(dict, s, e)) = parse_object(&self.data, self.offs[&n]) else {
            return;
        };
        let (Some(count), Some(first)) = (
            dict.get("N").and_then(|v| v.int()),
            dict.get("First").and_then(|v| v.int()),
        ) else {
            return;
        };
        if !(0..=1_000_000).contains(&count) || first < 0 {
            return;
        }
        let Ok(body) = self.decode_stream(&dict, s, e, &mut |_| {}) else {
            return;
        };
        let mut p = P::new(&body, 0);
        let idx = self.stms.len();
        let mut entries = Vec::new();
        for _ in 0..count {
            let (Some(num), Some(off)) = (p.int_word(), p.int_word()) else {
                break;
            };
            entries.push((num as u32, first as usize + off as usize));
        }
        for (num, off) in entries {
            if off < body.len() {
                self.comp.entry(num).or_insert((idx, off));
            }
        }
        self.stms.push(body);
    }

    fn get(&self, n: u32) -> Option<Val> {
        if let Some(&off) = self.offs.get(&n) {
            return parse_object(&self.data, off);
        }
        let &(i, off) = self.comp.get(&n)?;
        P::new(&self.stms[i], off).val(0)
    }

    /// Follow references (bounded).
    fn resolve(&self, v: &Val) -> Val {
        let mut cur = v.clone();
        for _ in 0..16 {
            match cur {
                Val::Ref(n) => cur = self.get(n).unwrap_or(Val::Null),
                other => return other,
            }
        }
        Val::Null
    }

    fn dict_of(&self, v: &Val) -> Option<Dict> {
        match self.resolve(v) {
            Val::Dict(d) | Val::Stream(d, _, _) => Some(d),
            _ => None,
        }
    }

    fn collect_pages(
        &self,
        node: &Val,
        inherited: &Dict,
        depth: u32,
        seen: &mut HashSet<u32>,
        out: &mut Vec<Dict>,
    ) {
        if depth > 64 || out.len() >= MAX_PAGES {
            return;
        }
        if let Val::Ref(n) = node {
            if !seen.insert(*n) {
                return; // cycle
            }
        }
        let Some(d) = self.dict_of(node) else { return };
        let mut inh = inherited.clone();
        for k in ["Resources", "MediaBox", "CropBox", "Rotate", "UserUnit"] {
            if let Some(v) = d.get(k) {
                inh.insert(k.into(), v.clone());
            }
        }
        match self.resolve(d.get("Kids").unwrap_or(&Val::Null)) {
            Val::Arr(kids) if d.get("Type").and_then(|t| t.name()) != Some("Page") => {
                for k in &kids {
                    self.collect_pages(k, &inh, depth + 1, seen, out);
                }
            }
            _ => {
                let mut page = d.clone();
                for (k, v) in inh {
                    page.entry(k).or_insert(v);
                }
                out.push(page);
            }
        }
    }

    // ---------------------------------------------------------------- images

    fn num_of(&self, d: &Dict, k: &str) -> Option<f64> {
        d.get(k).and_then(|v| self.resolve(v).num())
    }

    /// Image XObjects reachable from a resources dictionary (Form XObjects searched to depth 3).
    fn images_in(&self, res: &Val, depth: u32, out: &mut Vec<(Dict, usize, usize)>) {
        if depth > 3 {
            return;
        }
        let Some(res) = self.dict_of(res) else { return };
        let Some(xo) = res.get("XObject").and_then(|x| self.dict_of(x)) else {
            return;
        };
        for v in xo.values() {
            if let Val::Stream(d, s, e) = self.resolve(v) {
                match d.get("Subtype").and_then(|t| t.name()) {
                    Some("Image") => out.push((d, s, e)),
                    Some("Form") => {
                        if let Some(r) = d.get("Resources") {
                            self.images_in(r, depth + 1, out);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The scan image of a page: the largest image; `layered` when other large ones exist.
    fn page_image_ref(&self, page: &Dict) -> R<((Dict, usize, usize), bool)> {
        let mut imgs = Vec::new();
        if let Some(r) = page.get("Resources") {
            self.images_in(r, 0, &mut imgs);
        }
        if imgs.is_empty() {
            return fail("the page has no image (only text/vector pages are not supported)");
        }
        let area = |d: &Dict| {
            self.num_of(d, "Width").unwrap_or(0.0) * self.num_of(d, "Height").unwrap_or(0.0)
        };
        imgs.sort_by(|a, b| area(&b.0).total_cmp(&area(&a.0)));
        let top = area(&imgs[0].0);
        // images with an ImageMask or a size ≥ 10 % of the largest take part in the page
        let layered = imgs.iter().skip(1).any(|i| area(&i.0) >= top * 0.005);
        Ok((imgs.swap_remove(0), layered))
    }

    pub fn page_info(&self, index: usize) -> R<PageInfo> {
        let page = self
            .pages
            .get(index)
            .ok_or_else(|| ReadError(format!("page {} does not exist", index + 1)))?;
        let ((d, _, _), layered) = self.page_image_ref(page)?;
        let (w, h) = (
            self.num_of(&d, "Width").unwrap_or(0.0),
            self.num_of(&d, "Height").unwrap_or(0.0),
        );
        if !(1.0..=100_000.0).contains(&w) || !(1.0..=100_000.0).contains(&h) {
            return fail("the page image has an invalid size");
        }
        let mb = page
            .get("MediaBox")
            .and_then(|v| match self.resolve(v) {
                Val::Arr(a) if a.len() == 4 => {
                    let n: Vec<f64> = a.iter().filter_map(|x| self.resolve(x).num()).collect();
                    (n.len() == 4).then(|| ((n[2] - n[0]).abs(), (n[3] - n[1]).abs()))
                }
                _ => None,
            })
            .unwrap_or((612.0, 792.0));
        let unit = page
            .get("UserUnit")
            .and_then(|u| self.resolve(u).num())
            .filter(|u| *u > 0.0)
            .unwrap_or(1.0);
        let dpi = (mb.0 > 1.0)
            .then(|| (w * 72.0 / (mb.0 * unit)).round())
            .filter(|d| (36.0..=2400.0).contains(d))
            .map(|d| d as u32);
        let rot = page
            .get("Rotate")
            .and_then(|r| self.resolve(r).int())
            .unwrap_or(0)
            .rem_euclid(360);
        let rotate = (rot / 90 * 90) as u16;
        Ok(PageInfo {
            width_px: w as u32,
            height_px: h as u32,
            dpi,
            rotate,
            layered,
        })
    }

    /// Decode the scan image of page `index` (0-based), turned the way the PDF displays it.
    pub fn page_image(&self, index: usize) -> R<PdfPage> {
        let info = self.page_info(index)?;
        let page = &self.pages[index];
        let ((d, s, e), layered) = self.page_image_ref(page)?;
        let img = match layered.then(|| self.compose_page(page)).transpose()? {
            Some(Some(img)) => img,
            _ => self.decode_image(&d, s, e)?,
        };
        let info = PageInfo {
            dpi: self.composed_dpi(&img, page).or(info.dpi),
            ..info
        };
        let img = match info.rotate {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };
        Ok(PdfPage { image: img, info })
    }

    fn media_box(&self, page: &Dict) -> (f64, f64, f64, f64) {
        page.get("MediaBox")
            .and_then(|v| match self.resolve(v) {
                Val::Arr(a) if a.len() == 4 => {
                    let n: Vec<f64> = a.iter().filter_map(|x| self.resolve(x).num()).collect();
                    (n.len() == 4).then(|| {
                        (
                            n[0].min(n[2]),
                            n[1].min(n[3]),
                            n[0].max(n[2]),
                            n[1].max(n[3]),
                        )
                    })
                }
                _ => None,
            })
            .unwrap_or((0.0, 0.0, 612.0, 792.0))
    }

    fn composed_dpi(&self, img: &DynamicImage, page: &Dict) -> Option<u32> {
        let (x0, _, x1, _) = self.media_box(page);
        let unit = page
            .get("UserUnit")
            .and_then(|u| self.resolve(u).num())
            .filter(|u| *u > 0.0)
            .unwrap_or(1.0);
        let d = (img.width() as f64 * 72.0 / ((x1 - x0).max(1.0) * unit)).round();
        (36.0..=2400.0).contains(&d).then_some(d as u32)
    }

    /// Decoded content streams of a page, joined.
    fn page_content(&self, page: &Dict) -> Vec<u8> {
        let mut out = Vec::new();
        let parts: Vec<Val> = match page.get("Contents").map(|c| self.resolve(c)) {
            Some(Val::Arr(a)) => a,
            Some(v @ Val::Stream(..)) => vec![v],
            _ => vec![],
        };
        for p in parts {
            if let Val::Stream(d, s, e) = self.resolve(&p) {
                if let Ok(b) = self.decode_stream(&d, s, e, &mut |_| {}) {
                    out.extend_from_slice(&b);
                    out.push(b'\n');
                }
            }
        }
        out
    }

    /// Walk a content stream and record where each image is drawn.
    fn placements(
        &self,
        content: &[u8],
        res: &Val,
        ctm: [f64; 6],
        depth: u32,
        fill: &mut [u8; 3],
        out: &mut Vec<Placement>,
    ) {
        fn mul(m: [f64; 6], c: [f64; 6]) -> [f64; 6] {
            [
                m[0] * c[0] + m[1] * c[2],
                m[0] * c[1] + m[1] * c[3],
                m[2] * c[0] + m[3] * c[2],
                m[2] * c[1] + m[3] * c[3],
                m[4] * c[0] + m[5] * c[2] + c[4],
                m[4] * c[1] + m[5] * c[3] + c[5],
            ]
        }
        if depth > 3 || out.len() > 5000 {
            return;
        }
        let xobjs = self
            .dict_of(res)
            .and_then(|r| r.get("XObject").and_then(|x| self.dict_of(x)))
            .unwrap_or_default();
        let mut p = P::new(content, 0);
        let mut ops: Vec<Val> = Vec::new();
        let mut ctm = ctm;
        let mut stack: Vec<([f64; 6], [u8; 3])> = Vec::new();
        let nums = |ops: &[Val]| -> Vec<f64> { ops.iter().filter_map(|v| v.num()).collect() };
        loop {
            p.skip_ws();
            let Some(b) = p.peek() else { break };
            let is_op = b.is_ascii_alphabetic() || matches!(b, b'\'' | b'"' | b'*');
            if !is_op {
                match p.val(0) {
                    Some(v) => {
                        ops.push(v);
                        if ops.len() > 64 {
                            ops.remove(0);
                        }
                        continue;
                    }
                    None => {
                        p.pos += 1;
                        continue;
                    }
                }
            }
            let w = p.word();
            let word: &[u8] = w;
            if matches!(word, b"true" | b"false" | b"null") {
                ops.push(match word {
                    b"true" => Val::Bool(true),
                    b"false" => Val::Bool(false),
                    _ => Val::Null,
                });
                continue;
            }
            match word {
                b"q" => stack.push((ctm, *fill)),
                b"Q" => {
                    if let Some((c, f)) = stack.pop() {
                        ctm = c;
                        *fill = f;
                    }
                }
                b"cm" => {
                    let n = nums(&ops);
                    if n.len() >= 6 {
                        ctm = mul(
                            [
                                n[n.len() - 6],
                                n[n.len() - 5],
                                n[n.len() - 4],
                                n[n.len() - 3],
                                n[n.len() - 2],
                                n[n.len() - 1],
                            ],
                            ctm,
                        );
                    }
                }
                b"g" | b"rg" | b"k" | b"sc" | b"scn" => {
                    let n = nums(&ops);
                    let u = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    match n.len() {
                        1 => *fill = [u(n[0]); 3],
                        3 => *fill = [u(n[0]), u(n[1]), u(n[2])],
                        4 => {
                            let k = 1.0 - n[3];
                            *fill = [
                                u((1.0 - n[0]) * k),
                                u((1.0 - n[1]) * k),
                                u((1.0 - n[2]) * k),
                            ];
                        }
                        _ => {}
                    }
                }
                b"BI" => {
                    // inline image: skip to "EI"
                    let mut at = p.pos;
                    loop {
                        match find(content, b"EI", at) {
                            Some(q)
                                if q > 0
                                    && is_ws(content[q - 1])
                                    && content.get(q + 2).is_none_or(|&c| is_ws(c)) =>
                            {
                                p.pos = q + 2;
                                break;
                            }
                            Some(q) => at = q + 2,
                            None => {
                                p.pos = content.len();
                                break;
                            }
                        }
                    }
                }
                b"Do" => {
                    if let Some(name) = ops.last().and_then(|v| v.name()) {
                        if let Some(v) = xobjs.get(name) {
                            if let Val::Stream(d, s, e) = self.resolve(v) {
                                match d.get("Subtype").and_then(|t| t.name()) {
                                    Some("Image") => out.push(Placement {
                                        dict: d,
                                        s,
                                        e,
                                        m: ctm,
                                        fill: *fill,
                                    }),
                                    Some("Form") => {
                                        let m = match d.get("Matrix").map(|m| self.resolve(m)) {
                                            Some(Val::Arr(a)) if a.len() == 6 => {
                                                let n: Vec<f64> = a
                                                    .iter()
                                                    .filter_map(|x| self.resolve(x).num())
                                                    .collect();
                                                if n.len() == 6 {
                                                    [n[0], n[1], n[2], n[3], n[4], n[5]]
                                                } else {
                                                    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
                                                }
                                            }
                                            _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                                        };
                                        if let Ok(body) = self.decode_stream(&d, s, e, &mut |_| {})
                                        {
                                            let inner = d
                                                .get("Resources")
                                                .cloned()
                                                .unwrap_or_else(|| res.clone());
                                            let mut f2 = *fill;
                                            self.placements(
                                                &body,
                                                &inner,
                                                mul(m, ctm),
                                                depth + 1,
                                                &mut f2,
                                                out,
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            ops.clear();
        }
    }

    /// Composite a page built from several images (scanner "MRC" layers, our own hybrid
    /// pages): every image is placed by its transformation matrix on a white canvas at
    /// the highest resolution present; stencil masks are painted in the current fill colour.
    /// `Ok(None)` when no placement can be determined.
    fn compose_page(&self, page: &Dict) -> R<Option<DynamicImage>> {
        let Some(res) = page.get("Resources") else {
            return Ok(None);
        };
        let content = self.page_content(page);
        let mut pl = Vec::new();
        self.placements(
            &content,
            res,
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            0,
            &mut [0, 0, 0],
            &mut pl,
        );
        if pl.is_empty() {
            return Ok(None);
        }
        let (x0, y0, x1, y1) = self.media_box(page);
        // pixels per point: the sharpest image decides
        let mut ppp = 0.0f64;
        let straight = |m: &[f64; 6]| {
            !(m[1].abs() > 1e-3 * m[0].abs().max(1e-9)
                || m[2].abs() > 1e-3 * m[3].abs().max(1e-9)
                || m[0].abs() < 1e-6
                || m[3].abs() < 1e-6)
        };
        for q in &pl {
            let det = q.m[0] * q.m[3] - q.m[1] * q.m[2];
            if !det.is_finite() || det.abs() < 1e-9 {
                return fail("an image is placed with a degenerate transformation");
            }
            let width_pt = q.m[0].hypot(q.m[1]);
            ppp = ppp.max(self.num_of(&q.dict, "Width").unwrap_or(0.0) / width_pt.max(1e-6));
        }
        let (pw, ph) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        let mut scale = ppp.clamp(0.5, 33.4); // up to 2400 dpi
        let limit = (MAX_PIXELS as f64 / (pw * ph))
            .sqrt()
            .min(16_000.0 / pw.max(ph));
        scale = scale.min(limit);
        let (cw, ch) = (
            (pw * scale).round().max(1.0) as u32,
            (ph * scale).round().max(1.0) as u32,
        );
        let mut canvas = RgbImage::from_pixel(cw, ch, image::Rgb([255, 255, 255]));
        for q in &pl {
            let is_mask = matches!(
                q.dict.get("ImageMask").map(|v| self.resolve(v)),
                Some(Val::Bool(true))
            );
            let img = self.decode_image(&q.dict, q.s, q.e)?;
            if !straight(&q.m) {
                // rotated / skewed placement: inverse-map every canvas pixel of its bounding box
                let m = q.m;
                let det = m[0] * m[3] - m[1] * m[2];
                let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(u, v)| {
                    let (px, py) = (m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]);
                    ((px - x0) * scale, (y1 - py) * scale)
                });
                let bx0 = corners
                    .iter()
                    .map(|c| c.0)
                    .fold(f64::MAX, f64::min)
                    .floor()
                    .max(0.0) as i64;
                let bx1 = corners
                    .iter()
                    .map(|c| c.0)
                    .fold(f64::MIN, f64::max)
                    .ceil()
                    .min(cw as f64) as i64;
                let by0 = corners
                    .iter()
                    .map(|c| c.1)
                    .fold(f64::MAX, f64::min)
                    .floor()
                    .max(0.0) as i64;
                let by1 = corners
                    .iter()
                    .map(|c| c.1)
                    .fold(f64::MIN, f64::max)
                    .ceil()
                    .min(ch as f64) as i64;
                let (iw, ih) = (img.width() as f64, img.height() as f64);
                let rgb = img.to_rgb8();
                let g = if is_mask { Some(img.to_luma8()) } else { None };
                for cy in by0..by1 {
                    for cx in bx0..bx1 {
                        let (px, py) = (
                            x0 + (cx as f64 + 0.5) / scale,
                            y1 - (cy as f64 + 0.5) / scale,
                        );
                        let (rx, ry) = (px - m[4], py - m[5]);
                        let u = (rx * m[3] - ry * m[2]) / det;
                        let v = (ry * m[0] - rx * m[1]) / det;
                        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                            continue;
                        }
                        let (fx, fy) = (u * iw - 0.5, (1.0 - v) * ih - 0.5);
                        if let Some(g) = &g {
                            let (sx, sy) = (
                                (fx + 0.5).floor().clamp(0.0, iw - 1.0) as u32,
                                (fy + 0.5).floor().clamp(0.0, ih - 1.0) as u32,
                            );
                            if g.get_pixel(sx, sy).0[0] < 128 {
                                canvas.put_pixel(cx as u32, cy as u32, image::Rgb(q.fill));
                            }
                        } else {
                            let (x0i, y0i) = (fx.floor().max(0.0), fy.floor().max(0.0));
                            let (tx, ty) = ((fx - x0i).clamp(0.0, 1.0), (fy - y0i).clamp(0.0, 1.0));
                            let at = |x: f64, y: f64| {
                                rgb.get_pixel(x.min(iw - 1.0) as u32, y.min(ih - 1.0) as u32)
                                    .0
                            };
                            let (p00, p10, p01, p11) = (
                                at(x0i, y0i),
                                at(x0i + 1.0, y0i),
                                at(x0i, y0i + 1.0),
                                at(x0i + 1.0, y0i + 1.0),
                            );
                            let mut o = [0u8; 3];
                            for k in 0..3 {
                                let top = p00[k] as f64 * (1.0 - tx) + p10[k] as f64 * tx;
                                let bot = p01[k] as f64 * (1.0 - tx) + p11[k] as f64 * tx;
                                o[k] = (top * (1.0 - ty) + bot * ty).round() as u8;
                            }
                            canvas.put_pixel(cx as u32, cy as u32, image::Rgb(o));
                        }
                    }
                }
                continue;
            }
            let (dx, dy) = (
                (q.m[4].min(q.m[4] + q.m[0]) - x0) * scale,
                (y1 - (q.m[5] + q.m[3]).max(q.m[5])) * scale,
            );
            let (dw, dh) = (
                (q.m[0].abs() * scale).round().max(1.0) as u32,
                (q.m[3].abs() * scale).round().max(1.0) as u32,
            );
            if dw as u64 * dh as u64 > MAX_PIXELS {
                return fail("layered page is too large");
            }
            // PDF images are stored top row first; a negative d flips them
            let img = if q.m[3] < 0.0 { img.flipv() } else { img };
            let img = if q.m[0] < 0.0 { img.fliph() } else { img };
            let img = if img.width() == dw && img.height() == dh {
                img
            } else {
                img.resize_exact(
                    dw,
                    dh,
                    if is_mask {
                        image::imageops::FilterType::Nearest
                    } else {
                        image::imageops::FilterType::Triangle
                    },
                )
            };
            let (dx, dy) = (dx.round() as i64, dy.round() as i64);
            if is_mask {
                let g = img.to_luma8();
                for yy in 0..dh {
                    for xx in 0..dw {
                        if g.get_pixel(xx, yy).0[0] < 128 {
                            let (cx, cy) = (dx + xx as i64, dy + yy as i64);
                            if cx >= 0 && cy >= 0 && cx < cw as i64 && cy < ch as i64 {
                                canvas.put_pixel(cx as u32, cy as u32, image::Rgb(q.fill));
                            }
                        }
                    }
                }
            } else {
                image::imageops::replace(&mut canvas, &img.to_rgb8(), dx, dy);
            }
        }
        Ok(Some(DynamicImage::ImageRgb8(canvas)))
    }

    fn filters(&self, dict: &Dict) -> Vec<(String, Dict)> {
        let names: Vec<String> = match dict
            .get("Filter")
            .or_else(|| dict.get("F"))
            .map(|f| self.resolve(f))
        {
            Some(Val::Name(n)) => vec![n],
            Some(Val::Arr(a)) => a
                .iter()
                .filter_map(|x| self.resolve(x).name().map(str::to_string))
                .collect(),
            _ => vec![],
        };
        let parms: Vec<Dict> = match dict
            .get("DecodeParms")
            .or_else(|| dict.get("DP"))
            .map(|f| self.resolve(f))
        {
            Some(Val::Dict(d)) => vec![d],
            Some(Val::Arr(a)) => a
                .iter()
                .map(|x| self.dict_of(x).unwrap_or_default())
                .collect(),
            _ => vec![],
        };
        names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (n, parms.get(i).cloned().unwrap_or_default()))
            .collect()
    }

    /// Run all general-purpose filters. Stops at an image codec and reports it through `stop`.
    fn decode_stream(
        &self,
        dict: &Dict,
        s: usize,
        e: usize,
        stop: &mut dyn FnMut((&str, &Dict)),
    ) -> R<Vec<u8>> {
        let mut data = self.data[s.min(self.data.len())..e.min(self.data.len())].to_vec();
        if let Some(c) = &self.crypt {
            let is_xref = dict.get("Type").and_then(|t| t.name()) == Some("XRef");
            if !is_xref {
                let (num, gen) = self.owner_of(s);
                data = c.decrypt_stream(num, gen, &data);
            }
        }
        for (name, parms) in self.filters(dict) {
            let pi = |k: &str, dflt: i64| {
                parms
                    .get(k)
                    .and_then(|v| self.resolve(v).int())
                    .unwrap_or(dflt)
            };
            let predicted = |data: Vec<u8>| {
                unpredict(
                    data,
                    pi("Predictor", 1),
                    pi("Colors", 1).clamp(1, 64) as usize,
                    pi("BitsPerComponent", 8) as usize,
                    pi("Columns", 1).clamp(1, 1 << 24) as usize,
                )
            };
            data = match name.as_str() {
                "FlateDecode" | "Fl" => predicted(flate(&data)),
                "LZWDecode" | "LZW" => predicted(lzw(&data, pi("EarlyChange", 1) != 0)),
                "ASCIIHexDecode" | "AHx" => ascii_hex(&data),
                "ASCII85Decode" | "A85" => ascii85(&data),
                "RunLengthDecode" | "RL" => run_length(&data),
                "Crypt" => data,
                other => {
                    stop((other, &parms));
                    return Ok(data);
                }
            };
        }
        Ok(data)
    }

    fn colour_space(&self, v: Option<&Val>, depth: u32) -> R<Cs> {
        let Some(v) = v else { return Ok(Cs::Gray) };
        if depth > 4 {
            return fail("colour space nesting too deep");
        }
        match self.resolve(v) {
            Val::Name(n) => match n.as_str() {
                "DeviceGray" | "G" | "CalGray" => Ok(Cs::Gray),
                "DeviceRGB" | "RGB" | "CalRGB" => Ok(Cs::Rgb),
                "DeviceCMYK" | "CMYK" => Ok(Cs::Cmyk),
                other => fail(format!("unsupported colour space {other}")),
            },
            Val::Arr(a) if !a.is_empty() => match self.resolve(&a[0]).name().unwrap_or("") {
                "CalGray" => Ok(Cs::Gray),
                "CalRGB" | "Lab" => Ok(Cs::Rgb),
                "ICCBased" => {
                    let n = a
                        .get(1)
                        .and_then(|x| self.dict_of(x))
                        .and_then(|d| self.num_of(&d, "N"))
                        .unwrap_or(3.0) as i64;
                    match n {
                        1 => Ok(Cs::Gray),
                        3 => Ok(Cs::Rgb),
                        4 => Ok(Cs::Cmyk),
                        _ => fail("unsupported ICC profile"),
                    }
                }
                "Indexed" | "I" if a.len() >= 4 => {
                    let base = self.colour_space(a.get(1), depth + 1)?;
                    let hival = self.resolve(&a[2]).int().unwrap_or(255).clamp(0, 255) as usize;
                    let table = match self.resolve(&a[3]) {
                        Val::Str(s) => s,
                        Val::Stream(d, s, e) => self.decode_stream(&d, s, e, &mut |_| {})?,
                        _ => return fail("bad palette"),
                    };
                    Ok(Cs::Indexed {
                        base: Box::new(base),
                        hival,
                        table,
                    })
                }
                other => fail(format!("unsupported colour space {other}")),
            },
            _ => fail("unsupported colour space"),
        }
    }

    fn decode_image(&self, dict: &Dict, s: usize, e: usize) -> R<DynamicImage> {
        let w = self.num_of(dict, "Width").unwrap_or(0.0);
        let h = self.num_of(dict, "Height").unwrap_or(0.0);
        if !(1.0..=100_000.0).contains(&w)
            || !(1.0..=100_000.0).contains(&h)
            || (w as u64) * (h as u64) > MAX_PIXELS
        {
            return fail("the page image has an invalid or too large size");
        }
        let (w, h) = (w as usize, h as usize);
        let is_mask = matches!(
            dict.get("ImageMask").map(|v| self.resolve(v)),
            Some(Val::Bool(true))
        );
        let mut bpc = if is_mask {
            1
        } else {
            self.num_of(dict, "BitsPerComponent").unwrap_or(8.0) as usize
        };
        let mut cs = if is_mask {
            Cs::Gray
        } else {
            self.colour_space(dict.get("ColorSpace").or_else(|| dict.get("CS")), 0)
                .or_else(|err| {
                    // a JPEG / CCITT stream carries its own colour information
                    if dict.get("Filter").is_some() {
                        Ok(Cs::Rgb)
                    } else {
                        Err(err)
                    }
                })?
        };
        let mut codec: Option<(String, Dict)> = None;
        let mut raw = self.decode_stream(dict, s, e, &mut |(n, p)| {
            codec = Some((n.to_string(), p.clone()))
        })?;
        let mut row_pixels = w;
        if let Some((name, parms)) = codec {
            match name.as_str() {
                "DCTDecode" | "DCT" => {
                    return image::load_from_memory_with_format(&raw, image::ImageFormat::Jpeg)
                        .map(|i| {
                            if i.color().has_color() {
                                DynamicImage::ImageRgb8(i.to_rgb8())
                            } else {
                                DynamicImage::ImageLuma8(i.to_luma8())
                            }
                        })
                        .map_err(|err| ReadError(format!("JPEG page image: {err}")));
                }
                "CCITTFaxDecode" | "CCF" => {
                    let pi = |k: &str, d: i64| {
                        parms
                            .get(k)
                            .and_then(|v| self.resolve(v).int())
                            .unwrap_or(d)
                    };
                    let k = pi("K", 0);
                    // the decoder allocates rows × columns: tie it to the declared width
                    let columns = (pi("Columns", 1728).clamp(1, 1 << 20) as usize).min(w.saturating_add(64));
                    let black_is_1 = matches!(
                        parms.get("BlackIs1").map(|v| self.resolve(v)),
                        Some(Val::Bool(true))
                    );
                    let align = matches!(
                        parms.get("EncodedByteAlign").map(|v| self.resolve(v)),
                        Some(Val::Bool(true))
                    );
                    let rows = match pi("Rows", 0) {
                        r if r > 0 => (r as usize).min(h),
                        _ => h,
                    };
                    let (bits, _) = if k < 0 {
                        crate::g4::decode(&raw, columns, rows, align)
                    } else {
                        crate::g4::decode_g3(&raw, columns, rows, k, align)
                    };
                    // rows are at least as wide as the image, whatever /Columns said
                    let out_cols = columns.max(w);
                    let stride = out_cols.div_ceil(8);
                    let mut packed = vec![if black_is_1 { 0x00 } else { 0xFF }; stride * h];
                    for (y, row) in bits.iter().enumerate() {
                        for (x, &black) in row.iter().enumerate() {
                            // sample bit: 1 = white unless BlackIs1
                            let one = black == black_is_1;
                            let m = 0x80u8 >> (x % 8);
                            let b = &mut packed[y * stride + x / 8];
                            if one {
                                *b |= m;
                            } else {
                                *b &= !m;
                            }
                        }
                    }
                    raw = packed;
                    bpc = 1;
                    cs = Cs::Gray;
                    row_pixels = out_cols;
                }
                "JBIG2Decode" => {
                    let globals =
                        parms
                            .get("JBIG2Globals")
                            .map(|g| self.resolve(g))
                            .and_then(|v| match v {
                                Val::Stream(d, s, e) => {
                                    self.decode_stream(&d, s, e, &mut |_| {}).ok()
                                }
                                _ => None,
                            });
                    let bm = crate::jbig2::decode(&raw, globals.as_deref(), w, h)
                        .map_err(|e| ReadError(format!("JBIG2 page image: {e}")))?;
                    let stride = w.div_ceil(8);
                    // the filter's output convention: 1 = white
                    let mut packed = vec![0xFFu8; stride * h];
                    for y in 0..h {
                        for x in 0..w {
                            if bm[y * w + x] != 0 {
                                packed[y * stride + x / 8] &= !(0x80u8 >> (x % 8));
                            }
                        }
                    }
                    raw = packed;
                    bpc = 1;
                    cs = Cs::Gray;
                    row_pixels = w;
                }
                "JPXDecode" => {
                    let j = crate::jpx::decode(&raw)
                        .map_err(|e| ReadError(format!("JPEG 2000 page image: {e}")))?;
                    let (jw, jh) = (j.w as u32, j.h as u32);
                    return if j.nc == 1 {
                        image::GrayImage::from_raw(jw, jh, j.data).map(DynamicImage::ImageLuma8)
                    } else {
                        image::RgbImage::from_raw(jw, jh, j.data).map(DynamicImage::ImageRgb8)
                    }
                    .ok_or_else(|| ReadError("JPEG 2000 page image: bad size".into()));
                }
                other => return fail(format!("unsupported image filter {other}")),
            }
        }
        if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
            return fail(format!("unsupported bits per component: {bpc}"));
        }
        let nc = cs.comps();
        let stride = (row_pixels * nc * bpc).div_ceil(8);
        let need = stride
            .checked_mul(h)
            .filter(|&n| (n as u64) <= MAX_STREAM_BYTES)
            .ok_or_else(|| ReadError("image too large".into()))?;
        if raw.len() < need {
            raw.resize(
                need,
                if matches!(cs, Cs::Indexed { .. }) {
                    0
                } else {
                    0xFF
                },
            );
        }
        // /Decode: per-component inversion for the common [1 0] case
        let dec: Vec<f64> = match dict
            .get("Decode")
            .or_else(|| dict.get("D"))
            .map(|v| self.resolve(v))
        {
            Some(Val::Arr(a)) => a.iter().filter_map(|x| self.resolve(x).num()).collect(),
            _ => vec![],
        };
        let invert = |c: usize| {
            dec.get(2 * c)
                .zip(dec.get(2 * c + 1))
                .is_some_and(|(a, b)| a > b)
        };
        let maxv = (1u32 << bpc.min(8)) - 1;
        let sample = |row: &[u8], i: usize| -> u32 {
            match bpc {
                8 => row[i] as u32,
                16 => row[2 * i] as u32,
                _ => {
                    let bit = i * bpc;
                    ((row[bit / 8] >> (8 - bpc - bit % 8)) & ((1 << bpc) - 1) as u8) as u32
                }
            }
        };
        let scale = |v: u32| (v * 255 / maxv) as u8;
        let mut rgb: Vec<u8> = Vec::new();
        let mut gray: Vec<u8> = Vec::new();
        let is_gray_out = matches!(cs, Cs::Gray)
            || matches!(&cs, Cs::Indexed { base, .. } if matches!(**base, Cs::Gray));
        for y in 0..h {
            let row = &raw[y * stride..(y + 1) * stride];
            for x in 0..w {
                match &cs {
                    Cs::Gray => {
                        let v = scale(sample(row, x));
                        gray.push(if invert(0) { 255 - v } else { v });
                    }
                    Cs::Rgb => {
                        for c in 0..3 {
                            let v = scale(sample(row, x * 3 + c));
                            rgb.push(if invert(c) { 255 - v } else { v });
                        }
                    }
                    Cs::Cmyk => {
                        let q = |c: usize| {
                            let v = scale(sample(row, x * 4 + c)) as u32;
                            if invert(c) {
                                255 - v
                            } else {
                                v
                            }
                        };
                        let k = q(3);
                        for c in 0..3 {
                            rgb.push(((255 - q(c)) * (255 - k) / 255) as u8);
                        }
                    }
                    Cs::Indexed { base, hival, table } => {
                        let idx = (sample(row, x) as usize).min(*hival);
                        match **base {
                            Cs::Gray => gray.push(*table.get(idx).unwrap_or(&255)),
                            Cs::Rgb => {
                                for c in 0..3 {
                                    rgb.push(*table.get(idx * 3 + c).unwrap_or(&255));
                                }
                            }
                            Cs::Cmyk => {
                                let k = *table.get(idx * 4 + 3).unwrap_or(&0) as u32;
                                for c in 0..3 {
                                    rgb.push(
                                        ((255 - *table.get(idx * 4 + c).unwrap_or(&0) as u32)
                                            * (255 - k)
                                            / 255) as u8,
                                    );
                                }
                            }
                            Cs::Indexed { .. } => return fail("nested indexed colour space"),
                        }
                    }
                }
            }
        }
        if is_gray_out {
            GrayImage::from_raw(w as u32, h as u32, gray)
                .map(DynamicImage::ImageLuma8)
                .ok_or_else(|| ReadError("image buffer".into()))
        } else {
            RgbImage::from_raw(w as u32, h as u32, rgb)
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(|| ReadError("image buffer".into()))
        }
    }
}

// ---------------------------------------------------------------------------
// text extraction (for full-text search)

/// A font's byte-code → text map.
struct FontMap {
    /// Codes are two bytes wide (Type0 / Identity-H and friends).
    wide: bool,
    map: HashMap<u32, String>,
    /// There is a ToUnicode map (otherwise single bytes are taken as Latin-1).
    has_map: bool,
}

fn hex_tokens(t: &[u8]) -> Vec<(usize, Vec<u8>)> {
    // (position, bytes) of every <hex> token; position lets callers see `[` / `]` in between.
    let mut v = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] == b'<' && t.get(i + 1) != Some(&b'<') {
            let st = i + 1;
            let mut j = st;
            while j < t.len() && t[j] != b'>' {
                j += 1;
            }
            let digits: Vec<u8> = t[st..j.min(t.len())]
                .iter()
                .filter(|b| b.is_ascii_hexdigit())
                .copied()
                .collect();
            let bytes = digits
                .chunks(2)
                .map(|c| {
                    let h = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
                    if c.len() == 2 {
                        h(c[0]) * 16 + h(c[1])
                    } else {
                        h(c[0]) * 16
                    }
                })
                .collect();
            v.push((i, bytes));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    v
}

fn utf16_be(b: &[u8]) -> String {
    let u: Vec<u16> = b
        .chunks(2)
        .map(|c| ((c[0] as u16) << 8) | *c.get(1).unwrap_or(&0) as u16)
        .collect();
    String::from_utf16_lossy(&u)
}

fn be_num(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0, |a, &x| (a << 8) | x as u32)
}

/// Parse the `bfchar` / `bfrange` entries of a ToUnicode CMap.
fn parse_to_unicode(cmap: &[u8]) -> (HashMap<u32, String>, bool) {
    let mut map = HashMap::new();
    let mut wide = false;
    let mut pos = 0;
    while let Some(a) = find(cmap, b"beginbf", pos) {
        let is_range = cmap[a..].starts_with(b"beginbfrange");
        let Some(e) = find(
            cmap,
            if is_range {
                b"endbfrange"
            } else {
                b"endbfchar"
            },
            a,
        ) else {
            break;
        };
        let body = &cmap[a + 7..e];
        let toks = hex_tokens(body);
        if is_range {
            let mut i = 0;
            while i + 2 < toks.len() && map.len() < 70_000 {
                let (lo, hi) = (be_num(&toks[i].1), be_num(&toks[i + 1].1));
                if toks[i].1.len() > 1 {
                    wide = true;
                }
                // `[ <..> <..> ]` form: an opening bracket sits between the hi and next token.
                let between = &body[toks[i + 1].0..toks[i + 2].0];
                if between.contains(&b'[') {
                    let close = body[toks[i + 2].0..]
                        .iter()
                        .position(|&b| b == b']')
                        .map(|x| x + toks[i + 2].0)
                        .unwrap_or(body.len());
                    let mut k = i + 2;
                    let mut code = lo;
                    while k < toks.len() && toks[k].0 < close && code <= hi && code - lo < 65_536 {
                        map.insert(code, utf16_be(&toks[k].1));
                        code += 1;
                        k += 1;
                    }
                    i = k;
                } else {
                    let base = utf16_be(&toks[i + 2].1);
                    let mut chars: Vec<char> = base.chars().collect();
                    for code in lo..=hi.min(lo.saturating_add(65_535)) {
                        map.insert(code, chars.iter().collect());
                        if let Some(l) = chars.last_mut() {
                            *l = char::from_u32(*l as u32 + 1).unwrap_or(*l);
                        }
                    }
                    i += 3;
                }
            }
        } else {
            let mut i = 0;
            while i + 1 < toks.len() && map.len() < 70_000 {
                if toks[i].1.len() > 1 {
                    wide = true;
                }
                map.insert(be_num(&toks[i].1), utf16_be(&toks[i + 1].1));
                i += 2;
            }
        }
        pos = e + 1;
    }
    (map, wide)
}

impl PdfDoc {
    /// Best-effort text of a page: text drawn with `Tj`/`TJ`/`'`/`"`, decoded through each
    /// font's ToUnicode map (PDFs written by LegatorOCR, ABBYY, Tesseract, Word, … carry
    /// one). Lines are separated by `\n`. Pure scans without a text layer give an empty string.
    pub fn page_text(&self, index: usize) -> R<String> {
        let Some(page) = self.pages.get(index) else {
            return fail("page index out of range");
        };
        let res = page
            .get("Resources")
            .map(|r| self.resolve(r))
            .unwrap_or(Val::Null);
        let fonts_d = self
            .dict_of(&res)
            .and_then(|r| r.get("Font").and_then(|f| self.dict_of(f)))
            .unwrap_or_default();
        let mut fonts: HashMap<String, FontMap> = HashMap::new();
        for (name, fv) in &fonts_d {
            let Some(fd) = self.dict_of(fv) else { continue };
            let type0 = fd
                .get("Subtype")
                .and_then(|v| self.resolve(v).name().map(str::to_string))
                .as_deref()
                == Some("Type0");
            let mut fm = FontMap {
                wide: type0,
                map: HashMap::new(),
                has_map: false,
            };
            if let Some(Val::Stream(d, s, e)) = fd.get("ToUnicode").map(|t| self.resolve(t)) {
                if let Ok(b) = self.decode_stream(&d, s, e, &mut |_| {}) {
                    let (m, wide) = parse_to_unicode(&b);
                    fm.has_map = !m.is_empty();
                    fm.wide |= wide;
                    fm.map = m;
                }
            }
            fonts.insert(name.clone(), fm);
        }
        let content = self.page_content(page);
        let mut out = String::new();
        let mut cur: Option<&FontMap> = None;
        let mut last_y = f64::NAN;
        let mut p = P::new(&content, 0);
        let mut ops: Vec<Val> = Vec::new();
        let decode = |fm: Option<&FontMap>, bytes: &[u8], out: &mut String| match fm {
            Some(f) if f.has_map => {
                if f.wide {
                    for c in bytes.chunks(2) {
                        let code = ((c[0] as u32) << 8) | *c.get(1).unwrap_or(&0) as u32;
                        if let Some(t) = f.map.get(&code) {
                            out.push_str(t);
                        }
                    }
                } else {
                    for &b in bytes {
                        match f.map.get(&(b as u32)) {
                            Some(t) => out.push_str(t),
                            None => out.push(b as char),
                        }
                    }
                }
            }
            Some(f) if f.wide => {} // Identity-H without a map: nothing meaningful to read
            _ => out.extend(bytes.iter().map(|&b| b as char)),
        };
        let newline = |out: &mut String| {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        };
        let space = |out: &mut String| {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
        };
        let mut guard = 0usize;
        loop {
            p.skip_ws();
            let Some(b) = p.peek() else { break };
            guard += 1;
            if guard > 4_000_000 || out.len() > 8_000_000 {
                break;
            }
            let is_op = b.is_ascii_alphabetic() || matches!(b, b'\'' | b'"' | b'*');
            if !is_op {
                match p.val(0) {
                    Some(v) => {
                        ops.push(v);
                        if ops.len() > 64 {
                            ops.remove(0);
                        }
                    }
                    None => p.pos += 1,
                }
                continue;
            }
            let op = p.word().to_vec();
            match op.as_slice() {
                b"Tf" => {
                    if let Some(n) = ops.iter().rev().find_map(|v| v.name()) {
                        cur = fonts.get(n);
                    }
                }
                b"Tj" => {
                    if let Some(Val::Str(s)) = ops.last() {
                        decode(cur, s, &mut out);
                    }
                }
                b"'" | b"\"" => {
                    newline(&mut out);
                    if let Some(Val::Str(s)) = ops.last() {
                        decode(cur, s, &mut out);
                    }
                }
                b"TJ" => {
                    if let Some(Val::Arr(a)) = ops.last() {
                        for v in a {
                            match v {
                                Val::Str(s) => decode(cur, s, &mut out),
                                v => {
                                    if v.num().is_some_and(|n| n < -250.0) {
                                        space(&mut out);
                                    }
                                }
                            }
                        }
                    }
                }
                b"Td" | b"TD" => {
                    let n: Vec<f64> = ops.iter().filter_map(|v| v.num()).collect();
                    if n.len() >= 2 && n[n.len() - 1].abs() > 0.01 {
                        newline(&mut out);
                    } else {
                        space(&mut out);
                    }
                }
                b"T*" => newline(&mut out),
                b"Tm" => {
                    let n: Vec<f64> = ops.iter().filter_map(|v| v.num()).collect();
                    if n.len() >= 6 {
                        let y = n[n.len() - 1];
                        if last_y.is_finite() && (y - last_y).abs() > 1.0 {
                            newline(&mut out);
                        } else {
                            space(&mut out);
                        }
                        last_y = y;
                    }
                }
                b"ET" => space(&mut out),
                _ => {}
            }
            ops.clear();
        }
        let cleaned: Vec<String> = out
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect();
        Ok(cleaned.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compress, PdfWriter};
    use image::{Rgb, RgbImage};
    use legatorocr_core::{BitImage, Page, PageImage};

    fn page(img: PageImage, dpi: u32) -> Page {
        Page {
            image: img,
            dpi,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        }
    }

    fn gradient(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            Rgb([
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x + y) % 256) as u8,
            ])
        })
    }

    #[test]
    fn hostile_predictor_parameters_neither_panic_nor_allocate_gigabytes() {
        let d = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        for (pred, colors, bpc, columns) in [
            (12i64, 1_000_000_000usize, 8usize, 1usize << 24),
            (2, 1usize << 61, 8, 1),
            (15, 64, 16, 1 << 24),
            (12, 1, 8, usize::MAX),
            (12, 3, 8, 0),
        ] {
            let out = unpredict(d.clone(), pred, colors, bpc, columns);
            assert!(out.len() <= d.len() * 2 + 64, "{pred} {colors} {columns}");
        }
    }

    fn tiny_pdf_with_image(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");
        v.extend_from_slice(b"2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n");
        v.extend_from_slice(b"3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 10]/Resources<</XObject<</Im0 4 0 R>>>>/Contents 5 0 R>>endobj\n");
        v.extend_from_slice(format!("4 0 obj<</Type/XObject/Subtype/Image{dict}/Length {}>>stream\n", data.len()).as_bytes());
        v.extend_from_slice(data);
        v.extend_from_slice(b"\nendstream endobj\n5 0 obj<</Length 27>>stream\nq 100 0 0 10 0 0 cm /Im0 Do Q\nendstream endobj\ntrailer<</Root 1 0 R/Size 6>>\n");
        v
    }

    #[test]
    fn ccitt_columns_narrower_than_width_do_not_overrun_the_row() {
        for cols in [1, 8, 33, 100, 5000, 1 << 20] {
            let dict = format!("/Width 100/Height 10/BitsPerComponent 1/ColorSpace/DeviceGray/Filter/CCITTFaxDecode/DecodeParms<</K -1/Columns {cols}>>");
            let doc = PdfDoc::open(tiny_pdf_with_image(&dict, &[0x00, 0x10, 0x01, 0x00, 0x55, 0xAA])).unwrap();
            if let Ok(p) = doc.page_image(0) {
                assert_eq!((p.image.width(), p.image.height()), (100, 10), "columns {cols}");
            }
        }
    }

    #[test]
    fn hostile_flate_predictor_in_a_pdf_is_survived() {
        let dict = "/Width 8/Height 8/BitsPerComponent 8/ColorSpace/DeviceGray/Filter/FlateDecode/DecodeParms<</Predictor 12/Colors 1000000000/Columns 16777216>>";
        let doc = PdfDoc::open(tiny_pdf_with_image(dict, b"x\x9c\x03\x00\x00\x00\x00\x01")).unwrap();
        let _ = doc.page_image(0);
    }

    #[test]
    fn reads_back_flate_rgb_and_dpi() {
        let src = gradient(150, 200);
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(&page(PageImage::Rgb(src.clone()), 150), &[]);
        w.add_page(&page(PageImage::Rgb(src.clone()), 150), &[]);
        let doc = PdfDoc::open(w.finish()).unwrap();
        assert_eq!(doc.page_count(), 2);
        let p = doc.page_image(1).unwrap();
        assert_eq!(p.info.dpi, Some(150));
        assert_eq!(p.image.to_rgb8().as_raw(), src.as_raw());
        assert_eq!(doc.page_info(0).unwrap().width_px, 150);
        assert!(doc.page_image(2).is_err());
    }

    #[test]
    fn reads_back_jpeg_and_gray() {
        let src = gradient(64, 80);
        let mut w = PdfWriter::new(Compress::Jpeg(90));
        w.add_page(&page(PageImage::Rgb(src), 200), &[]);
        w.add_page(
            &page(
                PageImage::Gray(image::GrayImage::from_pixel(30, 30, image::Luma([77]))),
                200,
            ),
            &[],
        );
        let doc = PdfDoc::open(w.finish()).unwrap();
        let a = doc.page_image(0).unwrap();
        assert_eq!(
            (a.image.width(), a.image.height(), a.info.dpi),
            (64, 80, Some(200))
        );
        let g = doc.page_image(1).unwrap().image.to_luma8();
        assert!((g.get_pixel(5, 5).0[0] as i32 - 77).abs() <= 3);
    }

    #[test]
    fn reads_back_ccitt_g4_bilevel() {
        let mut b = BitImage::new(131, 97);
        for y in 0..97 {
            for x in 0..131 {
                b.set(x, y, (x / 7 + y / 5) % 3 == 0 || (x == y));
            }
        }
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(&page(PageImage::Bw(b.clone()), 300), &[]);
        let doc = PdfDoc::open(w.finish()).unwrap();
        let g = doc.page_image(0).unwrap().image.to_luma8();
        assert_eq!(g.dimensions(), (131, 97));
        for y in 0..97 {
            for x in 0..131 {
                assert_eq!(g.get_pixel(x, y).0[0] == 0, b.get(x, y), "({x},{y})");
            }
        }
    }

    fn pdf_with_image(dict_extra: &str, data: &[u8], w: u32, h: u32, page_extra: &str) -> Vec<u8> {
        let mut o = b"%PDF-1.4\n".to_vec();
        o.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        o.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        o.extend_from_slice(
            format!("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] {page_extra} /Resources << /XObject << /Im0 4 0 R >> >> >>\nendobj\n", w as f32 * 72.0 / 100.0, h as f32 * 72.0 / 100.0)
                .as_bytes(),
        );
        // deliberately wrong /Length: the reader must fall back to scanning for endstream
        o.extend_from_slice(format!("4 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} {dict_extra} /Length 999999 >>\nstream\n").as_bytes());
        o.extend_from_slice(data);
        o.extend_from_slice(b"\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        o
    }

    #[test]
    fn filters_and_colour_spaces() {
        // raw gray with Decode [1 0]
        let pdf = pdf_with_image(
            "/ColorSpace /DeviceGray /BitsPerComponent 8 /Decode [1 0]",
            &[0, 255, 100, 50],
            2,
            2,
            "",
        );
        let g = PdfDoc::open(pdf)
            .unwrap()
            .page_image(0)
            .unwrap()
            .image
            .to_luma8();
        assert_eq!(g.as_raw(), &[255, 0, 155, 205]);
        // ASCIIHex + 4-bit gray
        let pdf = pdf_with_image(
            "/ColorSpace /DeviceGray /BitsPerComponent 4 /Filter /ASCIIHexDecode",
            b"0F F0>",
            2,
            2,
            "",
        );
        let g = PdfDoc::open(pdf)
            .unwrap()
            .page_image(0)
            .unwrap()
            .image
            .to_luma8();
        assert_eq!(g.as_raw(), &[0, 255, 255, 0]);
        // indexed RGB palette, 8-bit
        let pdf = pdf_with_image(
            "/ColorSpace [/Indexed /DeviceRGB 1 <FF0000 0000FF>] /BitsPerComponent 8",
            &[0, 1],
            2,
            1,
            "",
        );
        let c = PdfDoc::open(pdf)
            .unwrap()
            .page_image(0)
            .unwrap()
            .image
            .to_rgb8();
        assert_eq!(
            (c.get_pixel(0, 0).0, c.get_pixel(1, 0).0),
            ([255, 0, 0], [0, 0, 255])
        );
        // CMYK
        let pdf = pdf_with_image(
            "/ColorSpace /DeviceCMYK /BitsPerComponent 8",
            &[0, 0, 0, 0, 255, 0, 0, 0],
            2,
            1,
            "",
        );
        let c = PdfDoc::open(pdf)
            .unwrap()
            .page_image(0)
            .unwrap()
            .image
            .to_rgb8();
        assert_eq!(
            (c.get_pixel(0, 0).0, c.get_pixel(1, 0).0),
            ([255, 255, 255], [0, 255, 255])
        );
        // page rotation turns the image
        let pdf = pdf_with_image(
            "/ColorSpace /DeviceGray /BitsPerComponent 8",
            &[1, 2, 3, 4, 5, 6],
            3,
            2,
            "/Rotate 90",
        );
        let p = PdfDoc::open(pdf).unwrap().page_image(0).unwrap();
        assert_eq!(
            (p.image.width(), p.image.height(), p.info.rotate),
            (2, 3, 90)
        );
    }

    #[test]
    fn layered_pages_are_composited() {
        use legatorocr_core::layers::{Background, Layers, Patch};
        let (w, h) = (300u32, 400u32);
        let mut mask = BitImage::new(w, h);
        for y in 50..60 {
            for x in 40..200 {
                mask.set(x, y, true);
            }
        }
        let layers = Layers {
            mask,
            fg: [200, 0, 0],
            bg: Some(Background {
                img: RgbImage::from_pixel(30, 40, Rgb([240, 230, 200])),
                dpi: 30,
            }),
            patches: vec![Patch {
                x: 100,
                y: 200,
                img: RgbImage::from_pixel(80, 60, Rgb([0, 0, 255])),
            }],
        };
        let mut pg = page(PageImage::Rgb(layers.compose()), 300);
        pg.layers = Some(layers);
        let mut wr = PdfWriter::new(Compress::Flate);
        wr.add_page(&pg, &[]);
        let doc = PdfDoc::open(wr.finish()).unwrap();
        let out = doc.page_image(0).unwrap();
        assert_eq!(out.info.dpi, Some(300));
        let img = out.image.to_rgb8();
        assert_eq!(img.dimensions(), (w, h));
        assert_eq!(img.get_pixel(100, 55).0, [200, 0, 0]); // text in ink colour
        assert_eq!(img.get_pixel(140, 230).0, [0, 0, 255]); // picture patch
        let bgp = img.get_pixel(250, 350).0; // background layer
        assert!((bgp[0] as i32 - 240).abs() < 6 && (bgp[2] as i32 - 200).abs() < 6);
    }

    #[test]
    fn jpeg2000_page_images_are_decoded() {
        let data = include_bytes!("../tests/jpx/rgb_lossless_rct.j2k");
        let want = include_bytes!("../tests/jpx/rgb_lossless_rct.raw");
        let (w, h) = (67usize, 45usize);
        let mut o = b"%PDF-1.5\n".to_vec();
        o.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        o.extend_from_slice(format!("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n").as_bytes());
        o.extend_from_slice(format!("4 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /Filter /JPXDecode /Length {} >>\nstream\n", data.len()).as_bytes());
        o.extend_from_slice(data);
        let c = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q");
        o.extend_from_slice(format!("\nendstream\nendobj\n5 0 obj\n<< /Length {} >>\nstream\n{c}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n", c.len()).as_bytes());
        let doc = PdfDoc::open(o).unwrap();
        let img = doc.page_image(0).unwrap().image.to_rgb8();
        assert_eq!(img.dimensions(), (w as u32, h as u32));
        assert_eq!(img.as_raw().as_slice(), &want[..]);
    }

    /// Decode `stream` with our reader and (when installed) with poppler; both must give `want`.
    fn jbig2_agrees(
        dir: &std::path::Path,
        tag: &str,
        (stream, poppler): (&[u8], bool),
        w: usize,
        h: usize,
        want: &[u8],
    ) {
        use crate::jbig2::enc::pdf_with_stream;
        let pdf = pdf_with_stream(stream, w, h);
        let ours = PdfDoc::open(pdf.clone())
            .unwrap()
            .page_image(0)
            .unwrap_or_else(|e| panic!("{tag}: {e}"))
            .image
            .to_luma8();
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    ours.get_pixel(x as u32, y as u32).0[0] < 128,
                    want[y * w + x] == 1,
                    "ours {tag} ({x},{y})"
                );
            }
        }
        if !poppler
            || std::process::Command::new("pdfimages")
                .arg("-v")
                .output()
                .is_err()
        {
            return;
        }
        let f = dir.join(format!("{tag}.pdf"));
        std::fs::write(&f, &pdf).unwrap();
        let prefix = dir.join(format!("o-{tag}"));
        let st = std::process::Command::new("pdfimages")
            .arg("-png")
            .arg(&f)
            .arg(&prefix)
            .output()
            .unwrap();
        assert!(
            st.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&st.stderr)
        );
        let theirs = image::open(format!("{}-000.png", prefix.display()))
            .unwrap()
            .to_luma8();
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    theirs.get_pixel(x as u32, y as u32).0[0] < 128,
                    want[y * w + x] == 1,
                    "poppler {tag} ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn jbig2_refinement_aggregate_and_halftone_match_poppler() {
        use crate::jbig2::enc::*;
        use crate::jbig2::Bitmap;
        let dir = std::env::temp_dir().join(format!("lgr-jbig2r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // blobby base picture
        let (w, h) = (61usize, 47usize);
        let mut base = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                base[y * w + x] = (((x / 7 + y / 5) % 3 == 0) as u8) ^ (rnd() % 37 == 0) as u8;
            }
        }
        let mut target = base.clone();
        for v in target.iter_mut() {
            if rnd() % 19 == 0 {
                *v ^= 1;
            }
        }
        for y in 10..20 {
            for x in 30..45 {
                target[y * w + x] = 1;
            }
        }
        for (n, (t, tp, at)) in [
            (0u8, false, [(-1, -1), (-1, -1)]),
            (0, true, [(-1, -1), (-1, -1)]),
            (0, false, [(-2, -1), (1, 1)]),
            (1, false, [(-1, -1); 2]),
            (1, true, [(-1, -1); 2]),
        ]
        .into_iter()
        .enumerate()
        {
            let st = refinement_stream(&base, &target, w, h, t, tp, at);
            // typical prediction (TPGRON) is checked against our own encoder only: poppler
            // decodes it differently and no other independent decoder is at hand
            jbig2_agrees(&dir, &format!("ref{n}"), (&st, !tp), w, h, &target);
        }
        // glyphs
        let glyph = |rnd: &mut dyn FnMut() -> u64, gw: usize, gh: usize| -> Bitmap {
            bm(
                gw,
                gh,
                (0..gw * gh)
                    .map(|i| {
                        (!(i % gw + i / gw).is_multiple_of(3)) as u8
                            ^ rnd().is_multiple_of(11) as u8
                    })
                    .collect(),
            )
        };
        let mut syms: Vec<Bitmap> = vec![
            glyph(&mut rnd, 5, 7),
            glyph(&mut rnd, 7, 7),
            glyph(&mut rnd, 6, 9),
            glyph(&mut rnd, 8, 9),
        ];
        syms.sort_by_key(|s| s.h);
        let tweak = |b: &Bitmap, nw: usize, nh: usize, rnd: &mut dyn FnMut() -> u64| -> Vec<u8> {
            (0..nw * nh)
                .map(|i| {
                    let (x, y) = (i % nw, i / nw);
                    let o = if x < b.w && y < b.h {
                        b.d[y * b.w + x]
                    } else {
                        0
                    };
                    o ^ rnd().is_multiple_of(7) as u8
                })
                .collect()
        };
        for rt in [0u8, 1] {
            let rat = [(-1, -1), (-1, -1)];
            let (pw, ph) = (80usize, 70usize);
            let mut placed = Vec::new();
            let mut want = vec![0u8; pw * ph];
            let mut y = 2i64;
            for (k, (id, nw, nh, rdx, rdy)) in [
                (0usize, 5usize, 7usize, 0i32, 0i32),
                (1, 7, 7, 0, 0),
                (2, 8, 10, 1, -1),
                (3, 8, 9, 0, 0),
                (1, 6, 8, -1, 1),
                (0, 5, 7, 0, 0),
            ]
            .into_iter()
            .enumerate()
            {
                let x = 3 + (k as i64 * 11) % 60;
                let refined = (k != 1 && k != 5).then(|| Refined {
                    w: nw,
                    h: nh,
                    d: tweak(&syms[id], nw, nh, &mut rnd),
                    rdx,
                    rdy,
                });
                let (dw, dh, dd) = match &refined {
                    Some(r) => (r.w, r.h, r.d.clone()),
                    None => (syms[id].w, syms[id].h, syms[id].d.clone()),
                };
                for yy in 0..dh {
                    for xx in 0..dw {
                        let (px, py) = (x as usize + xx, y as usize + yy);
                        if px < pw && py < ph {
                            want[py * pw + px] |= dd[yy * dw + xx];
                        }
                    }
                }
                placed.push((id, x, y, refined));
                y += 11;
            }
            let st = refined_text_stream(pw, ph, &syms, &placed, rt, rat);
            jbig2_agrees(&dir, &format!("txt{rt}"), (&st, true), pw, ph, &want);
            // aggregate dictionary
            let mut news: Vec<(usize, Refined)> = [
                (0usize, 5usize, 8usize, 0i32, 0i32),
                (1, 7, 7, 1, 0),
                (3, 9, 9, 0, 1),
                (2, 6, 9, 0, 0),
            ]
            .into_iter()
            .map(|(id, nw, nh, rdx, rdy)| {
                (
                    id,
                    Refined {
                        w: nw,
                        h: nh,
                        d: tweak(&syms[id], nw, nh, &mut rnd),
                        rdx,
                        rdy,
                    },
                )
            })
            .collect();
            news.sort_by_key(|(_, r)| r.h);
            let mut want = vec![0u8; pw * ph];
            let mut placed = Vec::new();
            for (k, (_, r)) in news.iter().enumerate() {
                let (x, y) = (4 + k as i64 * 15, 3 + k as i64 * 12);
                placed.push((k, x, y));
                for yy in 0..r.h {
                    for xx in 0..r.w {
                        want[(y as usize + yy) * pw + x as usize + xx] |= r.d[yy * r.w + xx];
                    }
                }
            }
            let st = aggregate_stream(pw, ph, &syms, &news, &placed, rt, rat);
            jbig2_agrees(&dir, &format!("agg{rt}"), (&st, true), pw, ph, &want);
        }
        // halftones
        let (pw, ph, np) = (4usize, 4usize, 8usize);
        let pats: Vec<Bitmap> = (0..np)
            .map(|k| bm(pw, ph, (0..pw * ph).map(|i| (i < 2 * k) as u8).collect()))
            .collect();
        let (gw, gh) = (10usize, 7usize);
        let gray: Vec<u32> = (0..gw * gh).map(|_| (rnd() % np as u64) as u32).collect();
        for (t, geo, skip) in [
            (0u8, (0i32, 0i32, 4u16 << 8, 0u16), false),
            (1, (0, 0, 4 << 8, 0), false),
            (2, (0, 0, 4 << 8, 0), false),
            (3, (0, 0, 4 << 8, 0), false),
            (0, (-(2 << 8), -(3 << 8), 4 << 8, 0), true),
            (2, (-(2 << 8), -(3 << 8), 4 << 8, 0), true),
        ] {
            let (rw, rh) = (gw * pw - 6, gh * ph - 5);
            let st = halftone_stream(rw, rh, &pats, &gray, gw, gh, geo, t, skip);
            let mut want = vec![0u8; rw * rh];
            for m in 0..gh {
                for nn in 0..gw {
                    let x =
                        (geo.0 as i64 + m as i64 * geo.3 as i64 + nn as i64 * geo.2 as i64) >> 8;
                    let y =
                        (geo.1 as i64 + m as i64 * geo.2 as i64 - nn as i64 * geo.3 as i64) >> 8;
                    let p = &pats[gray[m * gw + nn] as usize];
                    for yy in 0..ph {
                        for xx in 0..pw {
                            let (px, py) = (x + xx as i64, y + yy as i64);
                            if px >= 0 && py >= 0 && (px as usize) < rw && (py as usize) < rh {
                                want[py as usize * rw + px as usize] |= p.d[yy * pw + xx];
                            }
                        }
                    }
                }
            }
            // shifted grids are checked against our own encoder only (poppler places them differently)
            let plain = geo.0 == 0 && geo.1 == 0;
            jbig2_agrees(
                &dir,
                &format!("ht{t}{}", skip as u8),
                (&st, plain),
                rw,
                rh,
                &want,
            );
        }
    }

    #[test]
    fn jbig2_generic_regions_match_poppler() {
        use crate::jbig2::enc::stream;
        let (w, h) = (97usize, 61usize);
        let mut bm = vec![0u8; w * h];
        let mut s = 99u64;
        for y in 0..h {
            for x in 0..w {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                bm[y * w + x] =
                    (((y / 6) % 3 == 0 && x % 9 < 6) as u8) ^ s.is_multiple_of(41) as u8;
            }
        }
        let dir = std::env::temp_dir().join(format!("lgr-jbig2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let have_poppler = std::process::Command::new("pdfimages")
            .arg("-v")
            .output()
            .is_ok();
        let at0 = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        for (t, tp, at) in [
            (0u8, false, at0),
            (0, true, at0),
            (1, false, at0),
            (2, true, at0),
            (3, false, at0),
            (0, false, [(-4, -1), (5, -1), (1, -2), (-3, -2)]),
        ] {
            let data = stream(&bm, w, h, t, tp, {
                let mut a = at;
                match t {
                    1 => a[0] = (3, -1),
                    2 | 3 => a[0] = (2, -1),
                    _ => {}
                }
                a
            });
            let mut o = b"%PDF-1.4\n".to_vec();
            o.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
            o.extend_from_slice(format!("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n").as_bytes());
            o.extend_from_slice(format!("4 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode /Length {} >>\nstream\n", data.len()).as_bytes());
            o.extend_from_slice(&data);
            let c = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q");
            o.extend_from_slice(format!("\nendstream\nendobj\n5 0 obj\n<< /Length {} >>\nstream\n{c}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n", c.len()).as_bytes());
            let doc = PdfDoc::open(o.clone()).unwrap();
            let img = doc.page_image(0).unwrap().image.to_luma8();
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        img.get_pixel(x as u32, y as u32).0[0] < 128,
                        bm[y * w + x] == 1,
                        "ours t{t} tp{tp} ({x},{y})"
                    );
                }
            }
            if have_poppler {
                let f = dir.join(format!("j{t}{}.pdf", tp as u8));
                std::fs::write(&f, &o).unwrap();
                let prefix = dir.join(format!("o{t}{}", tp as u8));
                let st = std::process::Command::new("pdfimages")
                    .arg("-png")
                    .arg(&f)
                    .arg(&prefix)
                    .output()
                    .unwrap();
                assert!(
                    st.status.success(),
                    "{}",
                    String::from_utf8_lossy(&st.stderr)
                );
                let png = image::open(format!("{}-000.png", prefix.display()))
                    .unwrap()
                    .to_luma8();
                assert_eq!(png.dimensions(), (w as u32, h as u32));
                for y in 0..h {
                    for x in 0..w {
                        assert_eq!(
                            png.get_pixel(x as u32, y as u32).0[0] < 128,
                            bm[y * w + x] == 1,
                            "poppler t{t} tp{tp} ({x},{y})"
                        );
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_pdfs_open_with_the_empty_password() {
        let have = std::process::Command::new("qpdf")
            .arg("--version")
            .output()
            .is_ok();
        if !have {
            eprintln!("qpdf missing: skipping");
            return;
        }
        let src = gradient(90, 70);
        let mut w = PdfWriter::new(Compress::Jpeg(90));
        w.add_page(&page(PageImage::Rgb(src.clone()), 150), &[]);
        w.add_page(&page(PageImage::Rgb(gradient(50, 50)), 150), &[]);
        let plain = w.finish();
        let want = PdfDoc::open(plain.clone())
            .unwrap()
            .page_image(0)
            .unwrap()
            .image
            .to_rgb8();
        let dir = std::env::temp_dir().join(format!("lgr-enc-r-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inp = dir.join("in.pdf");
        std::fs::write(&inp, &plain).unwrap();
        let variants: [&[&str]; 7] = [
            &["--allow-weak-crypto", "--encrypt", "", "own", "40", "--"],
            &[
                "--allow-weak-crypto",
                "--encrypt",
                "",
                "own",
                "128",
                "--use-aes=n",
                "--",
            ],
            &[
                "--allow-weak-crypto",
                "--encrypt",
                "",
                "own",
                "128",
                "--use-aes=y",
                "--",
            ],
            &["--encrypt", "", "own", "256", "--force-R5", "--"],
            &["--encrypt", "", "own", "256", "--"],
            &[
                "--object-streams=generate",
                "--encrypt",
                "",
                "own",
                "256",
                "--",
            ],
            &[
                "--allow-weak-crypto",
                "--object-streams=generate",
                "--encrypt",
                "",
                "own",
                "128",
                "--use-aes=y",
                "--",
            ],
        ];
        for (i, v) in variants.iter().enumerate() {
            let out = dir.join(format!("out{i}.pdf"));
            let mut cmd = std::process::Command::new("qpdf");
            cmd.args(v.iter()).arg(&inp).arg(&out);
            let o = cmd.output().unwrap();
            assert!(
                o.status.success(),
                "qpdf variant {i}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            let enc = std::fs::read(&out).unwrap();
            assert_ne!(enc, plain);
            let doc = PdfDoc::open(enc).unwrap_or_else(|e| panic!("variant {i}: {e}"));
            assert_eq!(doc.page_count(), 2, "variant {i}");
            let got = doc
                .page_image(0)
                .unwrap_or_else(|e| panic!("variant {i}: {e}"))
                .image
                .to_rgb8();
            assert_eq!(got.as_raw(), want.as_raw(), "variant {i}");
        }
        // a password-protected file is refused with a clear message
        let o = std::process::Command::new("qpdf")
            .args(["--encrypt", "secret", "own", "256", "--"])
            .arg(&inp)
            .arg(dir.join("pw.pdf"))
            .output()
            .unwrap();
        assert!(o.status.success());
        let e = PdfDoc::open(std::fs::read(dir.join("pw.pdf")).unwrap())
            .err()
            .unwrap();
        assert!(e.0.contains("password"), "{}", e.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotated_and_skewed_placements_are_composited() {
        let mut o = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_vec();
        o.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /XObject << /A 4 0 R /B 5 0 R >> >> /Contents 6 0 R >>\nendobj\n");
        let base = vec![200u8; 100 * 100];
        o.extend_from_slice(b"4 0 obj\n<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 10000 >>\nstream\n");
        o.extend_from_slice(&base);
        o.extend_from_slice(b"\nendstream\nendobj\n");
        let red: Vec<u8> = (0..10 * 10).flat_map(|_| [255u8, 0, 0]).collect();
        o.extend_from_slice(b"5 0 obj\n<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 300 >>\nstream\n");
        o.extend_from_slice(&red);
        let c = "q 200 0 0 200 0 0 cm /A Do Q q 0 40 -40 0 100 80 cm /B Do Q q 30 0 15 30 140 20 cm /B Do Q";
        o.extend_from_slice(format!("\nendstream\nendobj\n6 0 obj\n<< /Length {} >>\nstream\n{c}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n", c.len()).as_bytes());
        let doc = PdfDoc::open(o).unwrap();
        let img = doc.page_image(0).unwrap().image.to_rgb8();
        let at = |x: f64, y: f64| {
            let k = img.width() as f64 / 200.0;
            img.get_pixel((x * k) as u32, ((200.0 - y) * k) as u32).0
        };
        assert_eq!(at(80.0, 100.0), [255, 0, 0], "rotated square");
        assert_eq!(at(50.0, 100.0), [200, 200, 200], "background");
        assert_eq!(at(150.0, 30.0), [255, 0, 0], "skewed parallelogram");
        assert_eq!(
            at(140.0, 55.0),
            [200, 200, 200],
            "outside the parallelogram"
        );
    }

    #[test]
    fn jbig2_symbol_coding_matches_poppler() {
        use crate::jbig2::enc::{pdf_with_stream, symbol_stream, TextCfg};
        use crate::jbig2::tests::symbol_case;
        if std::process::Command::new("pdfimages")
            .arg("-v")
            .output()
            .is_err()
        {
            eprintln!("pdfimages missing: skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lgr-jbig2s-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let at0 = [(3, -1), (-3, -1), (2, -2), (-2, -2)];
        let mut n = 0;
        for corner in 0..4u8 {
            for transposed in [false, true] {
                for log_strips in [0u8, 2] {
                    let (w, h, syms, placed, want) = symbol_case(5 + corner as u64);
                    let cfg = TextCfg {
                        corner,
                        transposed,
                        log_strips,
                        comb: 0,
                        dso: 0,
                    };
                    let data = symbol_stream(w, h, &syms, &placed, 0, at0, &cfg);
                    let pdf = pdf_with_stream(&data, w, h);
                    let ours = PdfDoc::open(pdf.clone())
                        .unwrap()
                        .page_image(0)
                        .unwrap()
                        .image
                        .to_luma8();
                    let f = dir.join(format!("s{n}.pdf"));
                    std::fs::write(&f, &pdf).unwrap();
                    let prefix = dir.join(format!("o{n}"));
                    let st = std::process::Command::new("pdfimages")
                        .arg("-png")
                        .arg(&f)
                        .arg(&prefix)
                        .output()
                        .unwrap();
                    assert!(
                        st.status.success(),
                        "{}",
                        String::from_utf8_lossy(&st.stderr)
                    );
                    let theirs = image::open(format!("{}-000.png", prefix.display()))
                        .unwrap()
                        .to_luma8();
                    assert_eq!(theirs.dimensions(), (w as u32, h as u32));
                    for y in 0..h {
                        for x in 0..w {
                            let k = want[y * w + x] == 1;
                            let tag = format!("corner {corner} transposed {transposed} strips {log_strips} ({x},{y})");
                            assert_eq!(
                                ours.get_pixel(x as u32, y as u32).0[0] < 128,
                                k,
                                "ours {tag}"
                            );
                            assert_eq!(
                                theirs.get_pixel(x as u32, y as u32).0[0] < 128,
                                k,
                                "poppler {tag}"
                            );
                        }
                    }
                    n += 1;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_things_say_so() {
        let e = |extra: &str| {
            PdfDoc::open(pdf_with_image(extra, &[0; 8], 2, 2, ""))
                .unwrap()
                .page_image(0)
                .err()
                .unwrap()
                .0
        };
        assert!(
            e("/ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode").contains("JBIG2")
        );
        assert!(
            e("/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode")
                .contains("JPEG 2000")
        );
        assert!(e("/ColorSpace /Pattern /BitsPerComponent 8").contains("colour space"));
        let enc = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R /Encrypt 5 0 R >>\n";
        assert!(PdfDoc::open(enc.to_vec()).is_err());
        assert!(PdfDoc::open(b"hello".to_vec()).is_err());
        assert!(PdfDoc::open(b"%PDF-1.4\n".to_vec()).is_err());
    }

    #[test]
    fn small_codecs() {
        assert_eq!(ascii_hex(b"48 65 6C6c 6F>"), b"Hello");
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo7~>"), b"Hello World");
        assert_eq!(ascii85(b"z~>"), vec![0u8; 4]);
        assert_eq!(
            run_length(&[2, 1, 2, 3, 254, 9, 128]),
            vec![1, 2, 3, 9, 9, 9]
        );
        // LZW sample from the PDF specification
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(
            lzw(&enc, true),
            vec![45, 45, 45, 45, 45, 65, 45, 45, 45, 66]
        );
        // PNG "up" predictor
        let data = vec![2, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(unpredict(data, 12, 1, 8, 3), vec![1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn text_layer_reads_back_with_diacritics_and_lines() {
        use legatorocr_core::text::Word;
        let w = |t: &str, x: u32, y: u32, line: u32| Word {
            text: t.into(),
            x,
            y,
            w: 60,
            h: 20,
            conf: 90.0,
            line,
        };
        let mut wr = PdfWriter::new(Compress::Flate);
        wr.add_page(
            &page(PageImage::Rgb(gradient(300, 200)), 150),
            &[
                w("Tehnologia", 10, 10, 0),
                w("mobilei", 90, 10, 0),
                w("șțăîâ", 10, 80, 1),
            ],
        );
        let doc = PdfDoc::open(wr.finish()).unwrap();
        assert_eq!(doc.page_text(0).unwrap(), "Tehnologia mobilei\nșțăîâ");
        assert!(doc.page_text(5).is_err());
    }

    #[test]
    fn plain_scan_has_no_text() {
        let mut wr = PdfWriter::new(Compress::Flate);
        wr.add_page(&page(PageImage::Rgb(gradient(40, 40)), 150), &[]);
        let doc = PdfDoc::open(wr.finish()).unwrap();
        assert_eq!(doc.page_text(0).unwrap(), "");
    }

    #[test]
    fn to_unicode_ranges_and_bracket_lists() {
        let (m, wide) = parse_to_unicode(
            b"1 beginbfrange <0041> <0043> <0061> endbfrange 1 beginbfrange <0050> <0051> [<0058> <0059>] endbfrange 1 beginbfchar <0001> <021B> endbfchar",
        );
        assert!(wide);
        assert_eq!(m[&0x42], "b");
        assert_eq!(m[&0x51], "Y");
        assert_eq!(m[&1], "ț");
    }

    #[test]
    fn garbage_never_panics() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(&page(PageImage::Rgb(gradient(40, 40)), 100), &[]);
        let good = w.finish();
        for _ in 0..400 {
            let mut b = good.clone();
            for _ in 0..(1 + rnd() % 12) {
                let i = (rnd() as usize) % b.len();
                match rnd() % 3 {
                    0 => b[i] = rnd() as u8,
                    1 => b.truncate(i.max(8)),
                    _ => b.insert(i, rnd() as u8),
                }
            }
            if let Ok(d) = PdfDoc::open(b) {
                for i in 0..d.page_count().min(3) {
                    let _ = d.page_info(i);
                    let _ = d.page_image(i);
                }
            }
        }
        for _ in 0..300 {
            let n = (rnd() % 400) as usize;
            let mut b = b"%PDF-1.5\n".to_vec();
            b.extend((0..n).map(|_| rnd() as u8));
            let _ = PdfDoc::open(b);
        }
    }
}
