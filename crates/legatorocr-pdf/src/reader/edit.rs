//! Object-level page editing of existing PDFs: pick pages from one or more files, write them
//! into a new file, and optionally stamp text (watermark, page numbers) on top.
//!
//! Nothing is rasterised: page dictionaries and everything they reach (fonts, images, content
//! streams) are copied byte for byte (stream data stays in its original encoding). References to
//! other pages (bookmarks, link destinations, `/Parent`) are cut, so a split file never drags in
//! the rest of the book. Encrypted input is refused. Outlines, forms and tagged structure of the
//! source are not carried over.

use super::{fail, Dict, PdfDoc, Val, R};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Where a stamp sits on the displayed page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    BottomCenter,
    BottomRight,
    BottomLeft,
    TopCenter,
    TopLeft,
    TopRight,
    Center,
}

/// A line of text laid over a page.
#[derive(Debug, Clone)]
pub struct Stamp {
    pub text: String,
    pub size: f64,
    pub anchor: Anchor,
    /// 0 = black, 1 = white.
    pub gray: f64,
    /// Counter-clockwise degrees (a diagonal watermark uses 45).
    pub angle: f64,
    /// 1 = opaque.
    pub opacity: f64,
}

const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Text as WinAnsi bytes; characters outside Latin-1 become their ASCII base letter or `?`.
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            c if (c as u32) < 256 => c as u32 as u8,
            'ă' | 'Ă' => {
                if c == 'ă' {
                    b'a'
                } else {
                    b'A'
                }
            }
            'ș' | 'ş' => b's',
            'Ș' | 'Ş' => b'S',
            'ț' | 'ţ' => b't',
            'Ț' | 'Ţ' => b'T',
            _ => b'?',
        })
        .collect()
}

fn text_width(bytes: &[u8], size: f64) -> f64 {
    bytes
        .iter()
        .map(|&b| {
            if (32..=126).contains(&b) {
                HELV[(b - 32) as usize] as f64
            } else {
                556.0
            }
        })
        .sum::<f64>()
        * size
        / 1000.0
}

fn pdf_str(bytes: &[u8]) -> String {
    let mut s = String::from("(");
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(b as char);
            }
            32..=126 => s.push(b as char),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// Stamp content for a page whose unrotated box is `w` x `h` at `(llx, lly)` with `/Rotate` `rot`.
fn stamp_content(st: &Stamp, w: f64, h: f64, llx: f64, lly: f64, rot: i64, gs: bool) -> String {
    let bytes = win_ansi(&st.text);
    let tw = text_width(&bytes, st.size);
    let (dw, dh) = if rot % 180 == 0 { (w, h) } else { (h, w) };
    let margin = 28.0;
    let (cx, cy) = match st.anchor {
        Anchor::BottomCenter => (dw / 2.0, margin),
        Anchor::BottomRight => (dw - margin - tw / 2.0, margin),
        Anchor::BottomLeft => (margin + tw / 2.0, margin),
        Anchor::TopCenter => (dw / 2.0, dh - margin - st.size),
        Anchor::TopLeft => (margin + tw / 2.0, dh - margin - st.size),
        Anchor::TopRight => (dw - margin - tw / 2.0, dh - margin - st.size),
        Anchor::Center => (dw / 2.0, dh / 2.0),
    };
    // displayed frame -> unrotated page space
    let (a, b, c, d, e, f) = match rot.rem_euclid(360) {
        90 => (0.0, 1.0, -1.0, 0.0, w, 0.0),
        180 => (-1.0, 0.0, 0.0, -1.0, w, h),
        270 => (0.0, -1.0, 1.0, 0.0, 0.0, h),
        _ => (1.0, 0.0, 0.0, 1.0, 0.0, 0.0),
    };
    let (sin, cos) = st.angle.to_radians().sin_cos();
    let mut s = String::from("q\n");
    if gs {
        s.push_str("/LGRGS gs\n");
    }
    let _ = write!(
        s,
        "{a} {b} {c} {d} {} {} cm\n{} g\n{cos:.5} {sin:.5} {:.5} {cos:.5} {cx:.2} {cy:.2} cm\nBT /LGRF {} Tf {:.2} {:.2} Td {} Tj ET\nQ\n",
        e + llx,
        f + lly,
        st.gray,
        -sin,
        st.size,
        -tw / 2.0,
        -st.size * 0.3,
        pdf_str(&bytes)
    );
    s
}

pub(super) fn esc_name(n: &str) -> String {
    let mut s = String::from("/");
    for &b in n.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+') {
            s.push(b as char);
        } else {
            let _ = write!(s, "#{b:02X}");
        }
    }
    s
}

struct Out {
    /// serialized bodies, index = object number - 1
    objs: Vec<Option<Vec<u8>>>,
    map: HashMap<(usize, u32), u32>,
    queue: Vec<(usize, u32, u32)>,
}

impl Out {
    fn alloc(&mut self) -> u32 {
        self.objs.push(None);
        self.objs.len() as u32
    }
}

fn is_page_node(v: &Val) -> bool {
    match v {
        Val::Dict(d) => matches!(d.get("Type"), Some(Val::Name(n)) if n == "Page" || n == "Pages"),
        _ => false,
    }
}

impl PdfDoc {
    /// Serialise a value; references are renumbered into `out` and queued for copying.
    fn ser(&self, di: usize, v: &Val, out: &mut Out, s: &mut Vec<u8>) {
        match v {
            Val::Null => s.extend_from_slice(b"null"),
            Val::Bool(b) => s.extend_from_slice(if *b { b"true" } else { b"false" }),
            Val::Int(i) => s.extend_from_slice(i.to_string().as_bytes()),
            Val::Real(r) => {
                let t = if r.is_finite() {
                    format!("{r:.6}")
                } else {
                    "0".into()
                };
                s.extend_from_slice(t.trim_end_matches('0').trim_end_matches('.').as_bytes());
            }
            Val::Name(n) => s.extend_from_slice(esc_name(n).as_bytes()),
            Val::Str(b) => {
                s.push(b'<');
                for x in b {
                    let _ = write!(UnsafeVec(s), "{x:02X}");
                }
                s.push(b'>');
            }
            Val::Arr(a) => {
                s.push(b'[');
                for x in a {
                    self.ser(di, x, out, s);
                    s.push(b' ');
                }
                s.push(b']');
            }
            Val::Dict(d) => self.ser_dict(di, d, out, s),
            Val::Ref(n) => {
                // other pages (and page-tree nodes) are cut off
                if self.get(*n).is_some_and(|o| is_page_node(&o)) {
                    s.extend_from_slice(b"null");
                    return;
                }
                let id = match out.map.get(&(di, *n)) {
                    Some(&id) => id,
                    None => {
                        let id = out.alloc();
                        out.map.insert((di, *n), id);
                        out.queue.push((di, *n, id));
                        id
                    }
                };
                let _ = write!(UnsafeVec(s), "{id} 0 R");
            }
            Val::Stream(d, _, _) => self.ser_dict(di, d, out, s),
        }
    }

    fn ser_dict(&self, di: usize, d: &Dict, out: &mut Out, s: &mut Vec<u8>) {
        s.extend_from_slice(b"<<");
        let mut keys: Vec<_> = d.keys().collect();
        keys.sort();
        for k in keys {
            if matches!(
                k.as_str(),
                "Parent" | "P" | "Length" | "StructParent" | "StructParents"
            ) {
                continue;
            }
            s.extend_from_slice(esc_name(k).as_bytes());
            s.push(b' ');
            self.ser(di, &d[k], out, s);
            s.push(b' ');
        }
        s.extend_from_slice(b">>");
    }

    /// Body of object `n` as it will be written (stream objects carry their raw data).
    fn ser_object(&self, di: usize, n: u32, out: &mut Out) -> Vec<u8> {
        let mut s = Vec::new();
        match self.get(n) {
            Some(Val::Stream(d, a, b)) => {
                self.ser_dict_with_len(di, &d, b.saturating_sub(a), out, &mut s);
                s.extend_from_slice(b"\nstream\n");
                if let Some(raw) = self.data.get(a..b) {
                    s.extend_from_slice(raw);
                }
                s.extend_from_slice(b"\nendstream");
            }
            Some(v) => self.ser(di, &v, out, &mut s),
            None => s.extend_from_slice(b"null"),
        }
        s
    }

    fn ser_dict_with_len(&self, di: usize, d: &Dict, len: usize, out: &mut Out, s: &mut Vec<u8>) {
        let mut tmp = Vec::new();
        self.ser_dict(di, d, out, &mut tmp);
        // splice /Length before the closing >>
        tmp.truncate(tmp.len() - 2);
        let _ = write!(UnsafeVec(&mut tmp), "/Length {len} >>");
        s.extend_from_slice(&tmp);
    }

    pub(super) fn page_box(&self, page: &Dict) -> (f64, f64, f64, f64) {
        if let Val::Arr(a) = self.resolve(page.get("MediaBox").unwrap_or(&Val::Null)) {
            let n: Vec<f64> = a.iter().filter_map(|v| self.resolve(v).num()).collect();
            if n.len() == 4 {
                return (
                    n[0].min(n[2]),
                    n[1].min(n[3]),
                    (n[2] - n[0]).abs(),
                    (n[3] - n[1]).abs(),
                );
            }
        }
        (0.0, 0.0, 612.0, 792.0)
    }
}

/// `write!` target over a byte vector.
struct UnsafeVec<'a>(&'a mut Vec<u8>);
impl std::fmt::Write for UnsafeVec<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// What else may happen to the pages on the way through [`assemble_opts`].
#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// Drop everything that is not page content: the document information (also the producer),
    /// annotations (links, actions), page actions, per-page metadata, thumbnails, transitions.
    pub sanitize: bool,
    /// Turn pages clockwise by this many degrees (a multiple of 90); `None` in the second slot
    /// means every page, otherwise the output pages (0-based) listed.
    pub rotate: Option<(i64, Option<Vec<usize>>)>,
    /// Cut this much off each side of every page, in points, as the page is *displayed*:
    /// `[left, bottom, right, top]`. The page box shrinks; content outside stays in the file.
    pub crop_pt: Option<[f64; 4]>,
    /// Document information to set (`Author`, `Subject`, `Keywords`, `Title`); entries the first
    /// source file has are kept unless listed here.
    pub info: Vec<(String, String)>,
}

/// Write the chosen pages (`(file, page indices)` in order) as a new PDF. `stamps(i, total)`
/// gives the stamps for output page `i` (0-based) of `total`.
pub fn assemble(
    parts: &[(&PdfDoc, Vec<usize>)],
    stamps: &dyn Fn(usize, usize) -> Vec<Stamp>,
) -> R<Vec<u8>> {
    assemble_opts(parts, stamps, &Opts::default())
}

/// [`assemble`] with [`Opts`].
pub fn assemble_opts(
    parts: &[(&PdfDoc, Vec<usize>)],
    stamps: &dyn Fn(usize, usize) -> Vec<Stamp>,
    opts: &Opts,
) -> R<Vec<u8>> {
    let total: usize = parts.iter().map(|p| p.1.len()).sum();
    if total == 0 {
        return fail("no pages selected");
    }
    let mut out = Out {
        objs: Vec::new(),
        map: HashMap::new(),
        queue: Vec::new(),
    };
    let catalog = out.alloc();
    let pages_id = out.alloc();
    let mut kids = Vec::new();
    let mut n_out = 0usize;
    for (di, (doc, idxs)) in parts.iter().enumerate() {
        if doc.crypt.is_some() {
            return fail("encrypted PDF: decrypt it first");
        }
        for &pi in idxs {
            let Some(page) = doc.pages.get(pi) else {
                return fail(format!(
                    "page {} does not exist (file has {})",
                    pi + 1,
                    doc.pages.len()
                ));
            };
            let mut page = page.clone();
            if opts.sanitize {
                for k in [
                    "Annots",
                    "AA",
                    "PieceInfo",
                    "Metadata",
                    "Thumb",
                    "B",
                    "Dur",
                    "Trans",
                    "LastModified",
                ] {
                    page.remove(k);
                }
            }
            if let Some([l, b, r, t]) = opts.crop_pt {
                let (llx, lly, w, h) = doc.page_box(&page);
                let rot = doc
                    .resolve(page.get("Rotate").unwrap_or(&Val::Null))
                    .int()
                    .unwrap_or(0);
                // displayed (left, bottom, right, top) -> the page as stored
                let (sl, sb, sr, st_) = match rot.rem_euclid(360) {
                    90 => (t, l, b, r),
                    180 => (r, t, l, b),
                    270 => (b, r, t, l),
                    _ => (l, b, r, t),
                };
                if sl + sr >= w - 10.0 || sb + st_ >= h - 10.0 {
                    return fail(format!(
                        "page {}: the cut is as large as the page",
                        n_out + 1
                    ));
                }
                page.insert(
                    "MediaBox".into(),
                    Val::Arr(
                        [llx + sl, lly + sb, llx + w - sr, lly + h - st_]
                            .iter()
                            .map(|x| Val::Real(*x))
                            .collect(),
                    ),
                );
                page.remove("CropBox");
            }
            if let Some((deg, which)) = &opts.rotate {
                if which.as_ref().is_none_or(|v| v.contains(&n_out)) {
                    let cur = doc
                        .resolve(page.get("Rotate").unwrap_or(&Val::Null))
                        .int()
                        .unwrap_or(0);
                    page.insert("Rotate".into(), Val::Int((cur + deg).rem_euclid(360)));
                }
            }
            let st = stamps(n_out, total);
            let mut stamp_ids: Vec<u32> = Vec::new();
            if !st.is_empty() {
                let (llx, lly, w, h) = doc.page_box(&page);
                let rot = doc
                    .resolve(page.get("Rotate").unwrap_or(&Val::Null))
                    .int()
                    .unwrap_or(0);
                let mut body = String::new();
                let mut any_alpha = false;
                for s in &st {
                    any_alpha |= s.opacity < 1.0;
                    body += &stamp_content(s, w, h, llx, lly, rot, s.opacity < 1.0);
                }
                let alpha = st.iter().map(|s| s.opacity).fold(1.0, f64::min);
                // resources: the page's own, plus our font (and transparency) entry
                let mut res = doc
                    .dict_of(page.get("Resources").unwrap_or(&Val::Null))
                    .unwrap_or_default();
                let mut fonts = doc
                    .dict_of(res.get("Font").unwrap_or(&Val::Null))
                    .unwrap_or_default();
                let mut f = Dict::new();
                f.insert("Type".into(), Val::Name("Font".into()));
                f.insert("Subtype".into(), Val::Name("Type1".into()));
                f.insert("BaseFont".into(), Val::Name("Helvetica".into()));
                f.insert("Encoding".into(), Val::Name("WinAnsiEncoding".into()));
                fonts.insert("LGRF".into(), Val::Dict(f));
                res.insert("Font".into(), Val::Dict(fonts));
                if any_alpha {
                    let mut egs = doc
                        .dict_of(res.get("ExtGState").unwrap_or(&Val::Null))
                        .unwrap_or_default();
                    let mut g = Dict::new();
                    g.insert("ca".into(), Val::Real(alpha));
                    g.insert("CA".into(), Val::Real(alpha));
                    egs.insert("LGRGS".into(), Val::Dict(g));
                    res.insert("ExtGState".into(), Val::Dict(egs));
                }
                page.insert("Resources".into(), Val::Dict(res));
                for text in ["q\n", "Q\n"] {
                    let id = out.alloc();
                    out.objs[id as usize - 1] = Some(raw_stream(text.as_bytes()));
                    stamp_ids.push(id);
                }
                let id = out.alloc();
                out.objs[id as usize - 1] = Some(raw_stream(body.as_bytes()));
                stamp_ids.push(id);
            }
            // Contents: [q] original... [Q] [stamp]
            let orig: Vec<Val> = match page.remove("Contents") {
                Some(Val::Arr(a)) => a,
                Some(Val::Null) | None => vec![],
                Some(v) => match doc.resolve(&v) {
                    Val::Arr(a) => a,
                    _ => vec![v],
                },
            };
            page.remove("Parent");
            page.insert("Type".into(), Val::Name("Page".into()));
            let mut body = Vec::new();
            doc.ser_dict(di, &page, &mut out, &mut body);
            body.truncate(body.len() - 2); // drop the closing >>
            body.extend_from_slice(b"/Contents [");
            if let [q, _, _] = stamp_ids[..] {
                let _ = write!(UnsafeVec(&mut body), "{q} 0 R ");
            }
            for c in &orig {
                doc.ser(di, c, &mut out, &mut body);
                body.push(b' ');
            }
            if let [_, big_q, st_id] = stamp_ids[..] {
                let _ = write!(UnsafeVec(&mut body), "{big_q} 0 R {st_id} 0 R");
            }
            let pid = out.alloc();
            let _ = write!(UnsafeVec(&mut body), "] /Parent {pages_id} 0 R >>");
            out.objs[pid as usize - 1] = Some(body);
            kids.push(pid);
            n_out += 1;
        }
        // copy everything the pages reached
        while let Some((qdi, n, id)) = out.queue.pop() {
            let d = parts[qdi].0;
            let body = d.ser_object(qdi, n, &mut out);
            out.objs[id as usize - 1] = Some(body);
        }
    }
    // pages tree and catalog
    let kid_list: String = kids.iter().map(|k| format!("{k} 0 R ")).collect();
    out.objs[pages_id as usize - 1] = Some(
        format!(
            "<< /Type /Pages /Count {} /Kids [{kid_list}] >>",
            kids.len()
        )
        .into_bytes(),
    );
    out.objs[catalog as usize - 1] =
        Some(format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    let info = if opts.sanitize {
        0
    } else {
        let info = out.alloc();
        let mut text = String::new();
        for key in ["Title", "Author", "Subject", "Keywords"] {
            let v = opts
                .info
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .or_else(|| parts.first().and_then(|p| p.0.info_text(key)));
            if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
                text.push_str(&format!(" /{key} {}", crate::pdf_text_string(&v)));
            }
        }
        out.objs[info as usize - 1] = Some(
            format!(
                "<< /Producer (LegatorOCR {}){text} >>",
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes(),
        );
        info
    };
    let mut buf: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in out.objs.iter().enumerate() {
        offs.push(buf.len());
        let _ = writeln!(UnsafeVec(&mut buf), "{} 0 obj", i + 1);
        buf.extend_from_slice(o.as_deref().unwrap_or(b"null"));
        buf.extend_from_slice(b"\nendobj\n");
    }
    let xref = buf.len();
    let _ = write!(
        UnsafeVec(&mut buf),
        "xref\n0 {}\n0000000000 65535 f \n",
        offs.len() + 1
    );
    for o in &offs {
        let _ = writeln!(UnsafeVec(&mut buf), "{o:010} 00000 n ");
    }
    let _ = write!(
        UnsafeVec(&mut buf),
        "trailer\n<< /Size {} /Root {catalog} 0 R{} >>\nstartxref\n{xref}\n%%EOF\n",
        offs.len() + 1,
        if info == 0 {
            String::new()
        } else {
            format!(" /Info {info} 0 R")
        }
    );
    Ok(buf)
}

/// Booklet order for `n` pages: the page count rounded up to a multiple of 4 (`None` = blank),
/// and for each side of each sheet its `(left, right)` pages. Folded and stapled, the sheets read
/// in order: front `(n-1, 0)`, back `(1, n-2)`, front `(n-3, 2)`, back `(3, n-4)`, …
pub fn booklet_sides(n: usize) -> Vec<(Option<usize>, Option<usize>)> {
    let n4 = n.div_ceil(4) * 4;
    let pg = |i: usize| (i < n).then_some(i);
    let mut v = Vec::new();
    for k in 0..n4 / 4 {
        v.push((pg(n4 - 1 - 2 * k), pg(2 * k)));
        v.push((pg(2 * k + 1), pg(n4 - 2 - 2 * k)));
    }
    v
}

/// Impose `doc` as a booklet: every output page is a landscape sheet side that carries two of the
/// source pages side by side (each centred in its half, scaled never), in the order
/// [`booklet_sides`] gives. Pages become form XObjects with their content streams decoded and
/// joined, so nothing else of the source (links, forms, outlines) survives. Rotated pages are
/// refused: rotate them for real first.
pub fn booklet(doc: &PdfDoc) -> R<Vec<u8>> {
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write as _;
    if doc.crypt.is_some() {
        return fail("encrypted PDF: decrypt it first");
    }
    let n = doc.pages.len();
    if n == 0 {
        return fail("no pages");
    }
    let (mut sw, mut sh) = (0f64, 0f64);
    for p in &doc.pages {
        let rot = doc
            .resolve(p.get("Rotate").unwrap_or(&Val::Null))
            .int()
            .unwrap_or(0);
        if rot.rem_euclid(360) != 0 {
            return fail(
                "a page has /Rotate; use --rotate-pdf first (it cannot be imposed rotated)",
            );
        }
        let (_, _, w, h) = doc.page_box(p);
        sw = sw.max(w);
        sh = sh.max(h);
    }
    let mut out = Out {
        objs: Vec::new(),
        map: HashMap::new(),
        queue: Vec::new(),
    };
    let catalog = out.alloc();
    let pages_id = out.alloc();
    // one form XObject per source page
    let mut forms: Vec<u32> = Vec::new();
    for p in &doc.pages {
        let (llx, lly, w, h) = doc.page_box(p);
        let mut content = Vec::new();
        let conts: Vec<Val> = match p.get("Contents").map(|c| doc.resolve(c)) {
            Some(Val::Arr(a)) => a,
            Some(Val::Null) | None => vec![],
            Some(v) => vec![v],
        };
        for c in &conts {
            // an array element may be a reference; resolve to the stream
            let target = match c {
                Val::Ref(num) => doc.get(*num),
                other => Some(other.clone()),
            };
            if let Some(Val::Stream(d, a, b)) = target {
                let bytes = doc.decode_stream(&d, a, b, &mut |_| {})?;
                content.extend_from_slice(&bytes);
                content.push(b'\n');
            }
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&content)
            .map_err(|e| super::ReadError(e.to_string()))?;
        let z = z.finish().map_err(|e| super::ReadError(e.to_string()))?;
        let id = out.alloc();
        let mut body = Vec::new();
        let _ = write!(
            UnsafeVec(&mut body),
            "<< /Type /XObject /Subtype /Form /BBox [{llx} {lly} {} {}] /Filter /FlateDecode /Length {} ",
            llx + w,
            lly + h,
            z.len()
        );
        if let Some(res) = p.get("Resources") {
            body.extend_from_slice(b"/Resources ");
            doc.ser(0, res, &mut out, &mut body);
        }
        body.extend_from_slice(b">>\nstream\n");
        body.extend_from_slice(&z);
        body.extend_from_slice(b"\nendstream");
        out.objs[id as usize - 1] = Some(body);
        forms.push(id);
    }
    let mut kids = Vec::new();
    for (left, right) in booklet_sides(n) {
        let mut res = String::from("<< /XObject <<");
        let mut cs = String::new();
        for (slot, pg) in [(0, left), (1, right)] {
            let Some(i) = pg else { continue };
            let (llx, lly, w, h) = doc.page_box(&doc.pages[i]);
            let tx = slot as f64 * sw + (sw - w) / 2.0 - llx;
            let ty = (sh - h) / 2.0 - lly;
            let _ = write!(res, " /P{slot} {} 0 R", forms[i]);
            let _ = writeln!(cs, "q 1 0 0 1 {tx:.3} {ty:.3} cm /P{slot} Do Q");
        }
        res.push_str(" >> >>");
        let cid = out.alloc();
        out.objs[cid as usize - 1] = Some(raw_stream(cs.as_bytes()));
        let pid = out.alloc();
        out.objs[pid as usize - 1] = Some(
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {res} /Contents {cid} 0 R >>",
                sw * 2.0,
                sh
            )
            .into_bytes(),
        );
        kids.push(pid);
    }
    // copy what the forms reached (fonts, images, …)
    while let Some((qdi, num, id)) = out.queue.pop() {
        let body = doc.ser_object(qdi, num, &mut out);
        out.objs[id as usize - 1] = Some(body);
    }
    let kid_list: String = kids.iter().map(|k| format!("{k} 0 R ")).collect();
    out.objs[pages_id as usize - 1] = Some(
        format!(
            "<< /Type /Pages /Count {} /Kids [{kid_list}] >>",
            kids.len()
        )
        .into_bytes(),
    );
    out.objs[catalog as usize - 1] =
        Some(format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    let info = out.alloc();
    out.objs[info as usize - 1] =
        Some(format!("<< /Producer (LegatorOCR {}) >>", env!("CARGO_PKG_VERSION")).into_bytes());
    Ok(write_file(&out, catalog, info))
}

/// Classic xref file from the numbered objects.
fn write_file(out: &Out, catalog: u32, info: u32) -> Vec<u8> {
    let mut buf: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in out.objs.iter().enumerate() {
        offs.push(buf.len());
        let _ = writeln!(UnsafeVec(&mut buf), "{} 0 obj", i + 1);
        buf.extend_from_slice(o.as_deref().unwrap_or(b"null"));
        buf.extend_from_slice(b"\nendobj\n");
    }
    let xref = buf.len();
    let _ = write!(
        UnsafeVec(&mut buf),
        "xref\n0 {}\n0000000000 65535 f \n",
        offs.len() + 1
    );
    for o in &offs {
        let _ = writeln!(UnsafeVec(&mut buf), "{o:010} 00000 n ");
    }
    let _ = write!(
        UnsafeVec(&mut buf),
        "trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offs.len() + 1
    );
    buf
}

fn raw_stream(data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

/// `"1-3,7,10-"` → zero-based indices, in the order written. `n` is the page count.
pub fn parse_ranges(spec: &str, n: usize) -> Result<Vec<usize>, String> {
    let mut v = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let num = |t: &str| {
            t.trim()
                .parse::<usize>()
                .map_err(|_| format!("not a page number: {t:?}"))
        };
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (
                if a.trim().is_empty() { 1 } else { num(a)? },
                if b.trim().is_empty() { n } else { num(b)? },
            ),
            None => {
                let x = num(part)?;
                (x, x)
            }
        };
        if a == 0 || b == 0 || a > n || b > n {
            return Err(format!("range {part:?} is outside 1-{n}"));
        }
        if a <= b {
            v.extend(a - 1..b);
        } else {
            v.extend((b - 1..a).rev());
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BitImage, Compress, Page, PageImage, PdfWriter};

    fn sample(pages: usize) -> Vec<u8> {
        let mut w = PdfWriter::new(Compress::Flate);
        for i in 0..pages {
            let mut im = BitImage::new(64, 64);
            for y in 0..64u32 {
                for x in 0..64u32 {
                    if (x / (i as u32 + 2)).is_multiple_of(2) && y > 10 {
                        im.set(x, y, true);
                    }
                }
            }
            w.add_page(
                &Page {
                    image: PageImage::Bw(im),
                    dpi: 72,
                    skew_deg: 0.0,
                    blank: false,
                    split_index: None,
                    content_box: None,
                    layers: None,
                },
                &[],
            );
        }
        w.finish()
    }

    #[test]
    fn booklet_order_is_the_folding_order() {
        use super::booklet_sides as b;
        assert_eq!(b(4), vec![(Some(3), Some(0)), (Some(1), Some(2))]);
        assert_eq!(
            b(6),
            vec![
                (None, Some(0)),
                (Some(1), None),
                (Some(5), Some(2)),
                (Some(3), Some(4))
            ]
        );
        assert_eq!(b(1).len(), 2);
        // every page appears exactly once
        let mut seen: Vec<usize> = b(11).iter().flat_map(|(l, r)| [*l, *r]).flatten().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..11).collect::<Vec<_>>());
    }

    #[test]
    fn booklet_makes_half_as_many_double_wide_sides() {
        let src = PdfDoc::open(sample(5)).unwrap();
        let (_, _, w, h) = src.page_box(&src.pages[0]);
        let out = PdfDoc::open(booklet(&src).unwrap()).unwrap();
        assert_eq!(out.page_count(), 4); // 5 -> 8 pages -> 4 sides
        let (_, _, ow, oh) = out.page_box(&out.pages[0]);
        assert_eq!((ow, oh), (w * 2.0, h));
    }

    #[test]
    fn sanitize_rotate_and_crop_change_the_page_not_the_picture() {
        let src = PdfDoc::open(sample(3)).unwrap();
        let (_, _, w, h) = src.page_box(&src.pages[0]);
        let opts = Opts {
            sanitize: true,
            rotate: Some((90, Some(vec![1]))),
            crop_pt: Some([4.0, 2.0, 6.0, 8.0]),
        };
        let bytes = assemble_opts(&[(&src, vec![0, 1, 2])], &|_, _| vec![], &opts).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("/Info"));
        assert!(!String::from_utf8_lossy(&bytes).contains("Producer"));
        let o = PdfDoc::open(bytes).unwrap();
        let (_, _, ow, oh) = o.page_box(&o.pages[0]);
        assert_eq!((ow, oh), (w - 10.0, h - 10.0));
        let rot = |i: usize| {
            o.resolve(o.pages[i].get("Rotate").unwrap_or(&Val::Null))
                .int()
                .unwrap_or(0)
        };
        assert_eq!((rot(0), rot(1), rot(2)), (0, 90, 0));
        // too large a cut is refused
        let big = Opts {
            crop_pt: Some([w, 0.0, 0.0, 0.0]),
            ..Opts::default()
        };
        assert!(assemble_opts(&[(&src, vec![0])], &|_, _| vec![], &big).is_err());
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_ranges("1-3,7", 9).unwrap(), vec![0, 1, 2, 6]);
        assert_eq!(parse_ranges("8-", 9).unwrap(), vec![7, 8]);
        assert_eq!(parse_ranges("3-1", 9).unwrap(), vec![2, 1, 0]);
        assert!(parse_ranges("0", 9).is_err() && parse_ranges("10", 9).is_err());
    }

    #[test]
    fn split_merge_and_stamp_round_trip() {
        let a = PdfDoc::open(sample(4)).unwrap();
        let b = PdfDoc::open(sample(2)).unwrap();
        let pix = |d: &PdfDoc, i: usize| d.page_image(i).unwrap().image.width();
        // pick pages 3,1 of a, then all of b
        let out = assemble(&[(&a, vec![2, 0]), (&b, vec![0, 1])], &|_, _| vec![]).unwrap();
        let m = PdfDoc::open(out).unwrap();
        assert_eq!(m.page_count(), 4);
        assert_eq!(pix(&m, 0), pix(&a, 2));
        // the pixels of the picked pages are the same as in the source
        let src = a.page_image(2).unwrap();
        let got = m.page_image(0).unwrap();
        assert_eq!(
            src.image.to_luma8().into_raw(),
            got.image.to_luma8().into_raw()
        );
        // stamped copy still opens and keeps page count; text is on top of the image
        let st = |i: usize, n: usize| {
            vec![Stamp {
                text: format!("{} / {}", i + 1, n),
                size: 10.0,
                anchor: Anchor::BottomCenter,
                gray: 0.0,
                angle: 0.0,
                opacity: 1.0,
            }]
        };
        let out = assemble(&[(&a, vec![0, 1, 2, 3])], &st).unwrap();
        let s = String::from_utf8_lossy(&out).into_owned();
        assert!(s.contains("(4 / 4) Tj") && s.contains("/BaseFont /Helvetica"));
        let d = PdfDoc::open(out).unwrap();
        assert_eq!(d.page_count(), 4);
        assert!(d.page_image(3).is_ok());
    }

    #[test]
    fn encrypted_and_missing_pages_are_refused() {
        let a = PdfDoc::open(sample(1)).unwrap();
        assert!(assemble(&[(&a, vec![5])], &|_, _| vec![]).is_err());
        assert!(assemble(&[(&a, vec![])], &|_, _| vec![]).is_err());
    }
}
