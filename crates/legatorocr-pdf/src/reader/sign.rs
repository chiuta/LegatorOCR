//! PDF signature plumbing (roadmap 5.7): add a signature placeholder to an existing PDF as an
//! incremental update, and read the signatures of a PDF. The cryptography (CMS / CAdES) is done
//! by the caller (the engine runs OpenSSL); this module only prepares the bytes to be signed,
//! puts the signature in, and extracts what a verifier needs.
//!
//! Limits: the file's last cross-reference section must be a classic `xref` table (files with
//! cross-reference streams are refused rather than risk a broken update).

use super::{fail, Dict, PdfDoc, Val, P, R};
use std::fmt::Write as _;

/// Space reserved for the signature, in bytes (twice that in hex digits).
pub const RESERVED: usize = 20000;

/// Where and how the signature is shown on a page (a signature without it is invisible).
#[derive(Debug, Clone)]
pub struct Visible {
    /// Page number from 1.
    pub page: usize,
    /// `[x, y, width, height]` in millimetres from the top-left corner of the page as stored.
    pub rect_mm: [f64; 4],
    /// Lines of text in the box, top to bottom.
    pub lines: Vec<String>,
}

pub struct Prepared {
    /// The file with the update appended and a zero-filled `/Contents`.
    pub bytes: Vec<u8>,
    /// `[0, a, b, len - b]`: the two signed ranges.
    pub byte_range: [usize; 4],
    /// Offset of the first hex digit inside `/Contents <...>`.
    contents_at: usize,
}

impl Prepared {
    /// The bytes the signature covers.
    pub fn signed_data(&self) -> Vec<u8> {
        let [a0, a1, b0, b1] = self.byte_range;
        [&self.bytes[a0..a0 + a1], &self.bytes[b0..b0 + b1]].concat()
    }

    /// Put the DER signature into the reserved space.
    pub fn embed(mut self, der: &[u8]) -> R<Vec<u8>> {
        if der.len() > RESERVED {
            return fail(format!(
                "the signature ({} bytes) does not fit the reserved {RESERVED} bytes",
                der.len()
            ));
        }
        let mut hex = String::with_capacity(der.len() * 2);
        for b in der {
            let _ = write!(hex, "{b:02X}");
        }
        self.bytes[self.contents_at..self.contents_at + hex.len()].copy_from_slice(hex.as_bytes());
        Ok(self.bytes)
    }
}

fn ser(v: &Val, out: &mut String) {
    match v {
        Val::Null => out.push_str("null"),
        Val::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Val::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Val::Real(r) => {
            let _ = write!(out, "{r}");
        }
        Val::Name(n) => {
            out.push('/');
            for &b in n.as_bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+') {
                    out.push(b as char);
                } else {
                    let _ = write!(out, "#{b:02X}");
                }
            }
        }
        Val::Str(b) => {
            out.push('<');
            for x in b {
                let _ = write!(out, "{x:02X}");
            }
            out.push('>');
        }
        Val::Arr(a) => {
            out.push('[');
            for x in a {
                ser(x, out);
                out.push(' ');
            }
            out.push(']');
        }
        Val::Dict(d) | Val::Stream(d, _, _) => ser_dict(d, out),
        Val::Ref(n) => {
            let _ = write!(out, "{n} 0 R");
        }
    }
}

fn ser_dict(d: &Dict, out: &mut String) {
    out.push_str("<< ");
    let mut keys: Vec<_> = d.keys().collect();
    keys.sort();
    for k in keys {
        ser(&Val::Name(k.clone()), out);
        out.push(' ');
        ser(&d[k], out);
        out.push(' ');
    }
    out.push_str(">>");
}

/// ASCII text as a PDF literal string; anything else becomes `?`.
fn pdf_ascii(s: &str) -> String {
    let mut o = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if c.is_ascii() && !c.is_control() => o.push(c),
            _ => o.push('?'),
        }
    }
    o.push(')');
    o
}

fn last_startxref(d: &[u8]) -> Option<usize> {
    let tail = d.len().saturating_sub(2048);
    let i = d[tail..].windows(9).rposition(|w| w == b"startxref")? + tail;
    let mut p = P::new(d, i + 9);
    p.int_word().map(|v| v as usize)
}

impl PdfDoc {
    fn dict_val(&self, v: &Val) -> Dict {
        self.dict_of(v).unwrap_or_default()
    }

    /// Object number of the `n`-th page (from 0) under `node`.
    fn nth_page(&self, node: &Val, n: &mut usize, depth: u32) -> Option<u32> {
        if depth > 32 {
            return None;
        }
        let Val::Ref(num) = node else { return None };
        let d = self.dict_of(node)?;
        match self.resolve(d.get("Kids").unwrap_or(&Val::Null)) {
            Val::Arr(kids) if d.get("Type").and_then(|t| t.name()) != Some("Page") => {
                kids.iter().find_map(|k| self.nth_page(k, n, depth + 1))
            }
            _ => {
                if *n == 0 {
                    Some(*num)
                } else {
                    *n -= 1;
                    None
                }
            }
        }
    }

    fn first_page(&self, node: &Val, depth: u32) -> Option<u32> {
        if depth > 32 {
            return None;
        }
        let Val::Ref(n) = node else { return None };
        let d = self.dict_of(node)?;
        match self.resolve(d.get("Kids").unwrap_or(&Val::Null)) {
            Val::Arr(kids) if d.get("Type").and_then(|t| t.name()) != Some("Page") => {
                kids.iter().find_map(|k| self.first_page(k, depth + 1))
            }
            _ => Some(*n),
        }
    }
}

/// Append an unsigned signature field (and its signature dictionary with a zero `/Contents`)
/// to `data` as an incremental update. `when` is Unix seconds for `/M`.
pub fn prepare(data: &[u8], when: u64, name: Option<&str>, reason: Option<&str>) -> R<Prepared> {
    prepare_visible(data, when, name, reason, None)
}

/// [`prepare`], optionally with a visible appearance: a box with `lines` of text on a page.
pub fn prepare_visible(
    data: &[u8],
    when: u64,
    name: Option<&str>,
    reason: Option<&str>,
    visible: Option<&Visible>,
) -> R<Prepared> {
    prepare_impl(data, when, name, reason, visible, false)
}

/// Append an invisible document time-stamp field (`/Type /DocTimeStamp`, `ETSI.RFC3161`) with a
/// zero-filled `/Contents`, as an incremental update. The caller hashes [`Prepared::signed_data`],
/// gets an RFC 3161 token for the hash and puts the token in with [`Prepared::embed`]. The
/// dictionary carries none of the entries the PAdES rules forbid (no `/Cert`, `/Reference`,
/// `/Changes`, `/R`, `/Prop_*`) and none they discourage (`/Name`, `/M`, `/Location`, `/Reason`,
/// `/ContactInfo`).
pub fn prepare_doc_timestamp(data: &[u8]) -> R<Prepared> {
    prepare_impl(data, 0, None, None, None, true)
}

fn prepare_impl(
    data: &[u8],
    when: u64,
    name: Option<&str>,
    reason: Option<&str>,
    visible: Option<&Visible>,
    doc_ts: bool,
) -> R<Prepared> {
    let doc = PdfDoc::open(data.to_vec())?;
    if doc.crypt.is_some() {
        return fail("encrypted PDF: a signature cannot be added");
    }
    let Some(prev) = last_startxref(data) else {
        return fail("no startxref: not a signable PDF");
    };
    if !data.get(prev..prev + 4).is_some_and(|s| s == b"xref") {
        return fail("the file uses a cross-reference stream; only classic xref tables are supported for signing");
    }
    let tpos = data[prev..]
        .windows(7)
        .position(|w| w == b"trailer")
        .map(|p| p + prev);
    let Some(tpos) = tpos else {
        return fail("no trailer dictionary");
    };
    let Some(Val::Dict(trailer)) = P::new(data, tpos + 7).val(0) else {
        return fail("unreadable trailer");
    };
    let size = trailer
        .get("Size")
        .and_then(|v| v.int())
        .unwrap_or(0)
        .max(1) as u32;
    let Some(Val::Ref(root)) = trailer.get("Root") else {
        return fail("trailer without /Root");
    };
    let root = *root;
    let mut catalog = doc.dict_val(&Val::Ref(root));
    let page_n = match visible {
        Some(v) => {
            if v.page == 0 || v.page > doc.page_count() {
                return fail(format!(
                    "the signature page {} does not exist (the file has {} pages)",
                    v.page,
                    doc.page_count()
                ));
            }
            let mut k = v.page - 1;
            catalog
                .get("Pages")
                .and_then(|p| doc.nth_page(p, &mut k, 0))
        }
        None => catalog.get("Pages").and_then(|p| doc.first_page(p, 0)),
    };
    let Some(page_n) = page_n else {
        return fail("no page to attach the signature to");
    };
    let mut page = doc.dict_val(&Val::Ref(page_n));
    // the box in page space, and the appearance stream
    let mut rect = [0.0f64; 4];
    let mut appearance: Option<String> = None;
    if let Some(v) = visible {
        let (llx, lly, pw, ph) = doc.page_box(&page);
        let rot = doc
            .resolve(page.get("Rotate").unwrap_or(&Val::Null))
            .int()
            .unwrap_or(0);
        if rot.rem_euclid(360) != 0 {
            return fail(
                "the signature page is rotated; rotate it for real before signing visibly",
            );
        }
        let mm = 72.0 / 25.4;
        let (x, y, w, h) = (
            v.rect_mm[0] * mm,
            v.rect_mm[1] * mm,
            v.rect_mm[2] * mm,
            v.rect_mm[3] * mm,
        );
        if w < 30.0 || h < 14.0 || x < 0.0 || y < 0.0 || x + w > pw + 0.5 || y + h > ph + 0.5 {
            return fail("the signature box must be at least 11 x 5 mm and lie inside the page");
        }
        rect = [llx + x, lly + ph - y - h, llx + x + w, lly + ph - y];
        let size_pt = (h / (v.lines.len().max(1) as f64 + 0.6)).clamp(5.0, 9.0);
        let mut c = format!(
            "0.5 w 0 G 0.5 0.5 {} {} re S\nBT /F1 {size_pt:.1} Tf 0 g\n",
            w - 1.0,
            h - 1.0
        );
        for (i, l) in v.lines.iter().enumerate() {
            let _ = writeln!(
                c,
                "1 0 0 1 4 {:.2} Tm {} Tj",
                h - 3.0 - size_pt * (i as f64 + 1.0),
                pdf_ascii(l)
            );
        }
        c.push_str("ET\n");
        appearance = Some(format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 {w:.2} {h:.2}] /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> /Length {} >>\nstream\n{c}endstream",
            c.len()
        ));
    }
    // new object numbers
    let (sig_n, wid_n) = (size, size + 1);
    let ap_n = size + 2;
    // page /Annots + widget
    let mut annots = match page.get("Annots").map(|a| doc.resolve(a)) {
        Some(Val::Arr(a)) => a,
        _ => Vec::new(),
    };
    annots.push(Val::Ref(wid_n));
    page.insert("Annots".into(), Val::Arr(annots));
    // AcroForm
    let mut extra_objs: Vec<(u32, String)> = Vec::new();
    let acro_src = catalog.get("AcroForm").cloned();
    let mut acro = acro_src
        .as_ref()
        .map(|a| doc.dict_val(a))
        .unwrap_or_default();
    let mut fields = match acro.get("Fields").map(|f| doc.resolve(f)) {
        Some(Val::Arr(a)) => a,
        _ => Vec::new(),
    };
    // a second signature needs a field name of its own: viewers (and validators) treat two fields
    // with one name as a forgery of the first
    let taken: Vec<Vec<u8>> = fields
        .iter()
        .filter_map(|f| match doc.dict_val(f).get("T") {
            Some(Val::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    let field_name = (1..)
        .map(|i| format!("Signature{i}"))
        .find(|n| !taken.iter().any(|t| t == n.as_bytes()))
        .unwrap_or_else(|| "Signature".into());
    fields.push(Val::Ref(wid_n));
    acro.insert("Fields".into(), Val::Arr(fields));
    acro.insert("SigFlags".into(), Val::Int(3));
    match acro_src {
        Some(Val::Ref(n)) => {
            let mut s = String::new();
            ser_dict(&acro, &mut s);
            extra_objs.push((n, s));
        }
        _ => {
            catalog.insert("AcroForm".into(), Val::Dict(acro));
        }
    }
    let mut buf = data.to_vec();
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    let mut offs: Vec<(u32, usize)> = Vec::new();
    let esc = |s: &str| -> String {
        let mut o = String::from("(");
        for c in s.chars() {
            match c {
                '(' | ')' | '\\' => {
                    o.push('\\');
                    o.push(c);
                }
                c if c.is_ascii() && !c.is_control() => o.push(c),
                _ => o.push('?'),
            }
        }
        o.push(')');
        o
    };
    // signature dictionary
    offs.push((sig_n, buf.len()));
    let (ty, sub) = if doc_ts {
        ("DocTimeStamp", "ETSI.RFC3161")
    } else {
        ("Sig", "ETSI.CAdES.detached")
    };
    let mut s = format!(
        "{sig_n} 0 obj\n<< /Type /{ty} /Filter /Adobe.PPKLite /SubFilter /{sub} /ByteRange [0 0000000000 0000000000 0000000000] /Contents <"
    );
    let head_len = buf.len() + s.len();
    s.push_str(&"0".repeat(RESERVED * 2));
    s.push('>');
    if !doc_ts {
        let _ = write!(s, " /M ({})", crate::pdf_date(when));
        if let Some(n) = name {
            let _ = write!(s, " /Name {}", esc(n));
        }
        if let Some(r) = reason {
            let _ = write!(s, " /Reason {}", esc(r));
        }
    }
    s.push_str(" >>\nendobj\n");
    buf.extend_from_slice(s.as_bytes());
    let contents_at = head_len;
    // widget
    offs.push((wid_n, buf.len()));
    let ap_ref = if appearance.is_some() {
        format!(" /AP << /N {ap_n} 0 R >>")
    } else {
        String::new()
    };
    let field = if doc_ts {
        format!("DocTimeStamp{sig_n}")
    } else {
        field_name.clone()
    };
    buf.extend_from_slice(
        format!("{wid_n} 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Sig /T ({field}) /Rect [{:.2} {:.2} {:.2} {:.2}] /F 132 /V {sig_n} 0 R /P {page_n} 0 R{ap_ref} >>\nendobj\n", rect[0], rect[1], rect[2], rect[3]).as_bytes(),
    );
    if let Some(ap) = &appearance {
        offs.push((ap_n, buf.len()));
        buf.extend_from_slice(format!("{ap_n} 0 obj\n{ap}\nendobj\n").as_bytes());
    }
    // page
    offs.push((page_n, buf.len()));
    let mut t = String::new();
    ser_dict(&page, &mut t);
    buf.extend_from_slice(format!("{page_n} 0 obj\n{t}\nendobj\n").as_bytes());
    // catalog
    offs.push((root, buf.len()));
    let mut t = String::new();
    ser_dict(&catalog, &mut t);
    buf.extend_from_slice(format!("{root} 0 obj\n{t}\nendobj\n").as_bytes());
    for (n, body) in extra_objs {
        offs.push((n, buf.len()));
        buf.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    // xref section: one subsection per object (they need not be contiguous)
    offs.sort();
    let xref_at = buf.len();
    buf.extend_from_slice(b"xref\n");
    for (n, off) in &offs {
        buf.extend_from_slice(format!("{n} 1\n{off:010} 00000 n \n").as_bytes());
    }
    let mut tr = Dict::new();
    tr.insert(
        "Size".into(),
        Val::Int((if appearance.is_some() { ap_n } else { wid_n } + 1) as i64),
    );
    tr.insert("Root".into(), Val::Ref(root));
    tr.insert("Prev".into(), Val::Int(prev as i64));
    for k in ["Info", "ID"] {
        if let Some(v) = trailer.get(k) {
            tr.insert(k.into(), v.clone());
        }
    }
    let mut t = String::new();
    ser_dict(&tr, &mut t);
    buf.extend_from_slice(format!("trailer\n{t}\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    // byte range: everything except `<` ... `>` of /Contents
    let a = contents_at - 1;
    let b = contents_at + RESERVED * 2 + 1;
    let range = [0, a, b, buf.len() - b];
    let pat = b"/ByteRange [0 0000000000 0000000000 0000000000]";
    let at = buf[a.saturating_sub(200)..a]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| p + a.saturating_sub(200));
    let Some(at) = at else {
        return fail("internal: byte range placeholder not found");
    };
    let fixed = format!(
        "/ByteRange [0 {:010} {:010} {:010}]",
        range[1], range[2], range[3]
    );
    buf[at..at + pat.len()].copy_from_slice(fixed.as_bytes());
    Ok(Prepared {
        bytes: buf,
        byte_range: range,
        contents_at,
    })
}

/// What the document security store of a PDF holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DssInfo {
    pub certs: usize,
    pub ocsps: usize,
    pub crls: usize,
    /// The store has a `/VRI` dictionary.
    pub vri: bool,
    /// Byte offset of the DSS dictionary object in the file (current cross-reference), when it
    /// is an indirect object.
    pub offset: Option<usize>,
}

/// The `/DSS` of the catalog, if there is one.
pub fn dss_info(data: &[u8]) -> R<Option<DssInfo>> {
    let doc = PdfDoc::open(data.to_vec())?;
    let Some(trailer_root) = doc.root_obj else {
        return fail("no /Root");
    };
    let catalog = doc.dict_val(&Val::Ref(trailer_root));
    let Some(dss_v) = catalog.get("DSS") else {
        return Ok(None);
    };
    let d = doc.dict_val(dss_v);
    let count = |k: &str| match d.get(k).map(|v| doc.resolve(v)) {
        Some(Val::Arr(a)) => a.len(),
        _ => 0,
    };
    Ok(Some(DssInfo {
        certs: count("Certs"),
        ocsps: count("OCSPs"),
        crls: count("CRLs"),
        vri: d.contains_key("VRI"),
        offset: match dss_v {
            Val::Ref(n) => doc.offs.get(n).copied(),
            _ => None,
        },
    }))
}

/// Add validation material (DER certificates, OCSP responses, CRLs) to the document security
/// store as an incremental update: each value becomes a stream, the store's arrays are the
/// earlier ones (kept as they are, with any `/VRI`) followed by the new streams, and the
/// catalog points to the new store. The bytes before the update are unchanged, so signatures
/// stay valid. Only classic cross-reference tables are supported.
pub fn add_dss(data: &[u8], certs: &[Vec<u8>], ocsps: &[Vec<u8>], crls: &[Vec<u8>]) -> R<Vec<u8>> {
    let doc = PdfDoc::open(data.to_vec())?;
    if doc.crypt.is_some() {
        return fail("encrypted PDF: validation material cannot be added");
    }
    let Some(prev) = last_startxref(data) else {
        return fail("no startxref: not an updatable PDF");
    };
    if !data.get(prev..prev + 4).is_some_and(|s| s == b"xref") {
        return fail(
            "the file uses a cross-reference stream; only classic xref tables are supported here",
        );
    }
    let Some(tpos) = data[prev..]
        .windows(7)
        .position(|w| w == b"trailer")
        .map(|p| p + prev)
    else {
        return fail("no trailer dictionary");
    };
    let Some(Val::Dict(trailer)) = P::new(data, tpos + 7).val(0) else {
        return fail("unreadable trailer");
    };
    let size = trailer
        .get("Size")
        .and_then(|v| v.int())
        .unwrap_or(0)
        .max(1) as u32;
    let Some(Val::Ref(root)) = trailer.get("Root") else {
        return fail("trailer without /Root");
    };
    let root = *root;
    let mut catalog = doc.dict_val(&Val::Ref(root));
    let mut dss = catalog
        .get("DSS")
        .map(|v| doc.dict_val(v))
        .unwrap_or_default();
    let mut buf = data.to_vec();
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    let mut offs: Vec<(u32, usize)> = Vec::new();
    let mut next = size;
    for (key, list) in [("Certs", certs), ("OCSPs", ocsps), ("CRLs", crls)] {
        if list.is_empty() {
            continue;
        }
        let mut arr = match dss.get(key).map(|v| doc.resolve(v)) {
            Some(Val::Arr(a)) => a,
            _ => Vec::new(),
        };
        for item in list {
            offs.push((next, buf.len()));
            buf.extend_from_slice(
                format!("{next} 0 obj\n<< /Length {} >>\nstream\n", item.len()).as_bytes(),
            );
            buf.extend_from_slice(item);
            buf.extend_from_slice(b"\nendstream\nendobj\n");
            arr.push(Val::Ref(next));
            next += 1;
        }
        dss.insert(key.into(), Val::Arr(arr));
    }
    dss.insert("Type".into(), Val::Name("DSS".into()));
    let dss_n = next;
    next += 1;
    offs.push((dss_n, buf.len()));
    let mut t = String::new();
    ser_dict(&dss, &mut t);
    buf.extend_from_slice(format!("{dss_n} 0 obj\n{t}\nendobj\n").as_bytes());
    catalog.insert("DSS".into(), Val::Ref(dss_n));
    offs.push((root, buf.len()));
    let mut t = String::new();
    ser_dict(&catalog, &mut t);
    buf.extend_from_slice(format!("{root} 0 obj\n{t}\nendobj\n").as_bytes());
    offs.sort();
    let xref_at = buf.len();
    buf.extend_from_slice(b"xref\n");
    for (n, off) in &offs {
        buf.extend_from_slice(format!("{n} 1\n{off:010} 00000 n \n").as_bytes());
    }
    let mut tr = Dict::new();
    tr.insert("Size".into(), Val::Int(next as i64));
    tr.insert("Root".into(), Val::Ref(root));
    tr.insert("Prev".into(), Val::Int(prev as i64));
    for k in ["Info", "ID"] {
        if let Some(v) = trailer.get(k) {
            tr.insert(k.into(), v.clone());
        }
    }
    let mut t = String::new();
    ser_dict(&tr, &mut t);
    buf.extend_from_slice(format!("trailer\n{t}\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    Ok(buf)
}

/// One signature found in a file.
#[derive(Debug, Clone)]
pub struct SigInfo {
    /// `/Type` of the dictionary (`Sig`, `DocTimeStamp`, ... or empty).
    pub kind: String,
    /// Every key of the signature dictionary.
    pub keys: Vec<String>,
    /// The value of `/V` when it is an integer.
    pub v: Option<i64>,
    pub byte_range: Vec<usize>,
    /// DER CMS bytes (trailing zero padding included).
    pub contents: Vec<u8>,
    pub sub_filter: String,
    pub name: Option<String>,
}

impl SigInfo {
    /// The signed bytes, or `None` when the byte range does not fit the file.
    pub fn signed_data(&self, file: &[u8]) -> Option<Vec<u8>> {
        let r = &self.byte_range;
        if r.len() != 4 || r[0] + r[1] > file.len() || r[2] + r[3] > file.len() {
            return None;
        }
        Some([&file[r[0]..r[0] + r[1]], &file[r[2]..r[2] + r[3]]].concat())
    }

    /// Does the signed range reach the end of the file (nothing added after signing)?
    pub fn covers_whole_file(&self, file: &[u8]) -> bool {
        let r = &self.byte_range;
        r.len() == 4 && r[2] + r[3] == file.len()
    }
}

/// All signature dictionaries of a PDF (any object with `/ByteRange` and `/Contents`).
pub fn signatures(data: &[u8]) -> R<Vec<SigInfo>> {
    let doc = PdfDoc::open(data.to_vec())?;
    let mut nums: Vec<u32> = doc.offs.keys().chain(doc.comp.keys()).copied().collect();
    nums.sort_unstable();
    nums.dedup();
    let mut out = Vec::new();
    for n in nums {
        let Some(Val::Dict(d)) = doc.get(n) else {
            continue;
        };
        let (Some(Val::Arr(r)), Some(Val::Str(c))) = (d.get("ByteRange"), d.get("Contents")) else {
            continue;
        };
        let range: Vec<usize> = r
            .iter()
            .filter_map(|v| v.int())
            .map(|v| v.max(0) as usize)
            .collect();
        let mut keys: Vec<String> = d.keys().cloned().collect();
        keys.sort();
        out.push(SigInfo {
            kind: d
                .get("Type")
                .and_then(|v| v.name())
                .unwrap_or("")
                .to_string(),
            keys,
            v: d.get("V").and_then(|v| v.int()),
            byte_range: range,
            contents: c.clone(),
            sub_filter: d
                .get("SubFilter")
                .and_then(|v| v.name())
                .unwrap_or("")
                .to_string(),
            name: match d.get("Name") {
                Some(Val::Str(s)) => Some(String::from_utf8_lossy(s).into_owned()),
                _ => None,
            },
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BitImage, Compress, Page, PageImage, PdfWriter};

    fn sample() -> Vec<u8> {
        sample_px(32)
    }

    fn sample_px(px: u32) -> Vec<u8> {
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(
            &Page {
                image: PageImage::Bw(BitImage::new(px, px)),
                dpi: 72,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                content_box: None,
                layers: None,
            },
            &[],
        );
        w.finish()
    }

    #[test]
    fn placeholder_is_a_valid_incremental_update() {
        let src = sample();
        let p = prepare(&src, 1_782_864_000, Some("Test"), Some("Approved")).unwrap();
        assert!(p.bytes.starts_with(&src));
        // the ranges leave out exactly the hex string with its angle brackets
        let [a0, a1, b0, b1] = p.byte_range;
        assert_eq!(a0, 0);
        assert_eq!(p.bytes[a1], b'<');
        assert_eq!(p.bytes[b0 - 1], b'>');
        assert_eq!(b0 + b1, p.bytes.len());
        assert_eq!(p.signed_data().len(), p.bytes.len() - (b0 - a1));
        // the updated file still opens and has the same page, plus a signature field
        let doc = PdfDoc::open(p.bytes.clone()).unwrap();
        assert_eq!(doc.page_count(), 1);
        let embedded = p.embed(&[0x30, 0x03, 0x02, 0x01, 0x05]).unwrap();
        let sigs = signatures(&embedded).unwrap();
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].sub_filter, "ETSI.CAdES.detached");
        assert_eq!(&sigs[0].contents[..5], &[0x30, 0x03, 0x02, 0x01, 0x05]);
        assert!(sigs[0].covers_whole_file(&embedded));
        assert_eq!(sigs[0].signed_data(&embedded).unwrap().len(), a1 + b1);
    }

    #[test]
    fn a_visible_signature_has_a_box_and_an_appearance() {
        let src = sample_px(595);
        let vis = Visible {
            page: 1,
            rect_mm: [10.0, 10.0, 60.0, 20.0],
            lines: vec!["Signed by Ana".into(), "2026-11-08".into()],
        };
        let p = prepare_visible(&src, 1_782_864_000, Some("Ana"), None, Some(&vis)).unwrap();
        let text = String::from_utf8_lossy(&p.bytes[src.len()..]).into_owned();
        assert!(text.contains("/AP << /N "), "{text}");
        assert!(text.contains("(Signed by Ana) Tj"));
        assert!(text.contains("/Subtype /Form"));
        // the file still opens, has the widget, and keeps its signed range intact
        let doc = PdfDoc::open(p.bytes.clone()).unwrap();
        assert_eq!(doc.page_count(), 1);
        let [_, a1, b0, b1] = p.byte_range;
        assert_eq!(b0 + b1, p.bytes.len());
        assert_eq!(p.bytes[a1], b'<');
        // bad boxes and pages are refused
        let too_small = Visible {
            rect_mm: [10.0, 10.0, 5.0, 2.0],
            ..vis.clone()
        };
        assert!(prepare_visible(&src, 0, None, None, Some(&too_small)).is_err());
        let outside = Visible {
            rect_mm: [10.0, 10.0, 6000.0, 20.0],
            ..vis.clone()
        };
        assert!(prepare_visible(&src, 0, None, None, Some(&outside)).is_err());
        let nopage = Visible { page: 4, ..vis };
        assert!(prepare_visible(&src, 0, None, None, Some(&nopage)).is_err());
    }

    #[test]
    fn a_mapped_file_opens_like_a_read_one() {
        let path = std::env::temp_dir().join(format!("lgr_map_{}.pdf", std::process::id()));
        std::fs::write(&path, sample()).unwrap();
        let mapped = PdfDoc::open_file_with_password(&path, b"").unwrap();
        let read = PdfDoc::open(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(mapped.page_count(), read.page_count());
        assert!(PdfDoc::open_file_with_password(&path.with_extension("missing"), b"").is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_without_classic_xref_or_encrypted_is_refused() {
        assert!(prepare(b"not a pdf", 0, None, None).is_err());
    }
}
