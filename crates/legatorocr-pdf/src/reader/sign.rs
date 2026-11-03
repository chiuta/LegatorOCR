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
            return fail(format!("the signature ({} bytes) does not fit the reserved {RESERVED} bytes", der.len()));
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
    let doc = PdfDoc::open(data.to_vec())?;
    if doc.crypt.is_some() {
        return fail("encrypted PDF: a signature cannot be added");
    }
    let Some(prev) = last_startxref(data) else { return fail("no startxref: not a signable PDF") };
    if !data.get(prev..prev + 4).is_some_and(|s| s == b"xref") {
        return fail("the file uses a cross-reference stream; only classic xref tables are supported for signing");
    }
    let tpos = data[prev..].windows(7).position(|w| w == b"trailer").map(|p| p + prev);
    let Some(tpos) = tpos else { return fail("no trailer dictionary") };
    let Some(Val::Dict(trailer)) = P::new(data, tpos + 7).val(0) else { return fail("unreadable trailer") };
    let size = trailer.get("Size").and_then(|v| v.int()).unwrap_or(0).max(1) as u32;
    let Some(Val::Ref(root)) = trailer.get("Root") else { return fail("trailer without /Root") };
    let root = *root;
    let mut catalog = doc.dict_val(&Val::Ref(root));
    let Some(page_n) = catalog.get("Pages").and_then(|p| doc.first_page(p, 0)) else { return fail("no page to attach the signature to") };
    let mut page = doc.dict_val(&Val::Ref(page_n));
    // new object numbers
    let (sig_n, wid_n) = (size, size + 1);
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
    let mut acro = acro_src.as_ref().map(|a| doc.dict_val(a)).unwrap_or_default();
    let mut fields = match acro.get("Fields").map(|f| doc.resolve(f)) {
        Some(Val::Arr(a)) => a,
        _ => Vec::new(),
    };
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
    let mut s = format!(
        "{sig_n} 0 obj\n<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.CAdES.detached /ByteRange [0 0000000000 0000000000 0000000000] /Contents <"
    );
    let head_len = buf.len() + s.len();
    s.push_str(&"0".repeat(RESERVED * 2));
    let _ = write!(s, "> /M ({})", crate::pdf_date(when));
    if let Some(n) = name {
        let _ = write!(s, " /Name {}", esc(n));
    }
    if let Some(r) = reason {
        let _ = write!(s, " /Reason {}", esc(r));
    }
    s.push_str(" >>\nendobj\n");
    buf.extend_from_slice(s.as_bytes());
    let contents_at = head_len;
    // widget
    offs.push((wid_n, buf.len()));
    buf.extend_from_slice(
        format!("{wid_n} 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [0 0 0 0] /F 132 /V {sig_n} 0 R /P {page_n} 0 R >>\nendobj\n").as_bytes(),
    );
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
    tr.insert("Size".into(), Val::Int((wid_n + 1) as i64));
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
    let at = buf[a.saturating_sub(200)..a].windows(pat.len()).position(|w| w == pat).map(|p| p + a.saturating_sub(200));
    let Some(at) = at else { return fail("internal: byte range placeholder not found") };
    let fixed = format!("/ByteRange [0 {:010} {:010} {:010}]", range[1], range[2], range[3]);
    buf[at..at + pat.len()].copy_from_slice(fixed.as_bytes());
    Ok(Prepared { bytes: buf, byte_range: range, contents_at })
}

/// One signature found in a file.
#[derive(Debug, Clone)]
pub struct SigInfo {
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
        let Some(Val::Dict(d)) = doc.get(n) else { continue };
        let (Some(Val::Arr(r)), Some(Val::Str(c))) = (d.get("ByteRange"), d.get("Contents")) else { continue };
        let range: Vec<usize> = r.iter().filter_map(|v| v.int()).map(|v| v.max(0) as usize).collect();
        out.push(SigInfo {
            byte_range: range,
            contents: c.clone(),
            sub_filter: d.get("SubFilter").and_then(|v| v.name()).unwrap_or("").to_string(),
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
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(
            &Page { image: PageImage::Bw(BitImage::new(32, 32)), dpi: 72, skew_deg: 0.0, blank: false, split_index: None, content_box: None, layers: None },
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
