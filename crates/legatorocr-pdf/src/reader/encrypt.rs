//! Rewrite a finished PDF: password-protect it (AES-256, standard security handler V5/R6: every
//! string and stream is encrypted with the file key, an /Encrypt dictionary and a file /ID are
//! added) and/or pack its plain objects into compressed object streams with a cross-reference
//! stream (PDF 1.5, usually much smaller). Works on the files this program writes (classic
//! cross-reference table); an already encrypted file is refused.

use super::edit::esc_name;
use super::{fail, Dict, PdfDoc, Val, P, R};
use crate::crypt::{aes_cbc_encrypt, make_v5};
use flate2::{write::ZlibEncoder, Compression};
use std::fmt::Write as _;
use std::io::Write as _;

/// Permission bits of /P (ISO 32000-2, table 22).
pub mod perm {
    pub const PRINT: i32 = 1 << 2;
    pub const MODIFY: i32 = 1 << 3;
    pub const COPY: i32 = 1 << 4;
    pub const ANNOTATE: i32 = 1 << 5;
    pub const FORMS: i32 = 1 << 8;
    pub const ACCESSIBILITY: i32 = 1 << 9;
    pub const ASSEMBLE: i32 = 1 << 10;
    pub const HIGH_RES: i32 = 1 << 11;
    pub const ALL: i32 =
        PRINT | MODIFY | COPY | ANNOTATE | FORMS | ACCESSIBILITY | ASSEMBLE | HIGH_RES;
}

/// The /P value for the allowed `bits` (the reserved bits are set as the standard requires).
pub fn p_value(bits: i32) -> i32 {
    (0xFFFF_F0C0u32 | (bits & perm::ALL) as u32) as i32
}

pub(super) struct Ctx<'a> {
    /// The file key; `None` writes strings and streams as they are.
    pub(super) key: Option<[u8; 32]>,
    pub(super) rand: &'a mut dyn FnMut(&mut [u8]),
    /// New number of each old object number (references are renumbered); `None` keeps them.
    pub(super) map: Option<&'a [u32]>,
}

impl Ctx<'_> {
    pub(super) fn enc(&mut self, data: &[u8]) -> Vec<u8> {
        let Some(key) = self.key else {
            return data.to_vec();
        };
        let mut iv = [0u8; 16];
        (self.rand)(&mut iv);
        aes_cbc_encrypt(&key, iv, data).unwrap_or_default()
    }

    pub(super) fn ser(&mut self, v: &Val, s: &mut Vec<u8>) {
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
                for x in self.enc(b) {
                    let _ = write!(Hex(s), "{x:02X}");
                }
                s.push(b'>');
            }
            Val::Arr(a) => {
                s.push(b'[');
                for x in a {
                    self.ser(x, s);
                    s.push(b' ');
                }
                s.push(b']');
            }
            Val::Dict(d) | Val::Stream(d, _, _) => self.dict(d, None, s),
            Val::Ref(n) => {
                let n = self
                    .map
                    .map_or(*n, |m| m.get(*n as usize).copied().unwrap_or(*n));
                let _ = write!(Hex(s), "{n} 0 R");
            }
        }
    }

    pub(super) fn dict(&mut self, d: &Dict, len: Option<usize>, s: &mut Vec<u8>) {
        s.extend_from_slice(b"<<");
        let mut keys: Vec<_> = d.keys().collect();
        keys.sort();
        for k in keys {
            if len.is_some() && k == "Length" {
                continue;
            }
            s.extend_from_slice(esc_name(k).as_bytes());
            s.push(b' ');
            self.ser(&d[k], s);
            s.push(b' ');
        }
        if let Some(l) = len {
            let _ = write!(Hex(s), "/Length {l} ");
        }
        s.extend_from_slice(b">>");
    }
}

pub(super) struct Hex<'a>(pub(super) &'a mut Vec<u8>);
impl std::fmt::Write for Hex<'_> {
    fn write_str(&mut self, t: &str) -> std::fmt::Result {
        self.0.extend_from_slice(t.as_bytes());
        Ok(())
    }
}

pub(super) fn hex(b: &[u8]) -> String {
    let mut s = String::from("<");
    for x in b {
        let _ = write!(s, "{x:02X}");
    }
    s.push('>');
    s
}

/// The Encrypt dictionary as object `n` (its strings are not encrypted).
pub(super) fn encrypt_object(n: u32, allowed: i32, s: &crate::crypt::V5Setup) -> String {
    format!(
        "{n} 0 obj\n<< /Filter /Standard /V 5 /R 6 /Length 256 /P {} /O {} /U {} /OE {} /UE {} /Perms {} /EncryptMetadata true /CF << /StdCF << /AuthEvent /DocOpen /CFM /AESV3 /Length 32 >> >> /StmF /StdCF /StrF /StdCF >>\nendobj\n",
        p_value(allowed),
        hex(&s.o),
        hex(&s.u),
        hex(&s.oe),
        hex(&s.ue),
        hex(&s.perms)
    )
}

/// The setup of a V5/R6 security handler for `p`.
pub(super) fn setup_for(p: &Protect, rand: &mut dyn FnMut(&mut [u8])) -> crate::crypt::V5Setup {
    make_v5(
        p.user.as_bytes(),
        if p.owner.is_empty() { p.user } else { p.owner }.as_bytes(),
        p_value(p.allowed),
        true,
        rand,
    )
}

/// Who may open a file and what they may do (see [`encrypt`]).
pub struct Protect<'a> {
    pub user: &'a str,
    pub owner: &'a str,
    pub allowed: i32,
}

/// Encrypt `data` so that `user` opens it (an empty user password opens without asking, only the
/// permissions then apply) and `owner` may do everything. `allowed` is the set of [`perm`] bits
/// that the user password grants. `rand` fills a buffer with random bytes.
pub fn encrypt(
    data: &[u8],
    user: &str,
    owner: &str,
    allowed: i32,
    rand: &mut dyn FnMut(&mut [u8]),
) -> R<Vec<u8>> {
    rewrite(
        data,
        false,
        Some(&Protect {
            user,
            owner,
            allowed,
        }),
        rand,
    )
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    let _ = z.write_all(data);
    z.finish().unwrap_or_default()
}

/// Objects per object stream.
const PER_OBJSTM: usize = 100;

/// Rewrite `data`: with `compact`, plain objects go into compressed object streams and the file
/// gets a cross-reference stream; with `protect`, everything is encrypted. Either or both.
pub fn rewrite(
    data: &[u8],
    compact: bool,
    protect: Option<&Protect>,
    rand: &mut dyn FnMut(&mut [u8]),
) -> R<Vec<u8>> {
    let doc = PdfDoc::open(data.to_vec())?;
    // the last trailer of our own classic file
    let Some(t) = data.windows(7).rposition(|w| w == b"trailer") else {
        return fail("only files with a classic cross-reference table can be rewritten");
    };
    let Some(Val::Dict(tr)) = P::new(data, t + 7).val(0) else {
        return fail("the trailer cannot be read");
    };
    if tr.contains_key("Encrypt") {
        return fail("the file is already encrypted");
    }
    let size = match tr.get("Size") {
        Some(Val::Int(n)) if (1..=50_000_000).contains(n) => *n as u32,
        _ => return fail("the trailer has no /Size"),
    };
    let Some(root) = tr.get("Root").cloned() else {
        return fail("the trailer has no /Root");
    };
    let setup = protect.map(|p| {
        make_v5(
            p.user.as_bytes(),
            if p.owner.is_empty() { p.user } else { p.owner }.as_bytes(),
            p_value(p.allowed),
            true,
            rand,
        )
    });
    let mut cx = Ctx {
        key: setup.as_ref().map(|s| s.key),
        rand,
        map: None,
    };
    let mut plain = Ctx {
        key: None,
        rand: &mut |_| {},
        map: None,
    };
    let mut buf: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    // (object number, entry): offset of a top-level object, or (object stream, index)
    #[derive(Clone, Copy)]
    enum At {
        Free,
        Off(usize),
        In(u32, usize),
    }
    let mut at: Vec<At> = vec![At::Free; size as usize];
    // numbers that become members of object streams (non-stream objects), in order
    let mut packed: Vec<(u32, Vec<u8>)> = Vec::new();
    for n in 1..size {
        match doc.get(n) {
            Some(Val::Stream(d, a, b)) => {
                at[n as usize] = At::Off(buf.len());
                let _ = writeln!(Hex(&mut buf), "{n} 0 obj");
                let raw = data.get(a..b).unwrap_or_default();
                let body = cx.enc(raw);
                cx.dict(&d, Some(body.len()), &mut buf);
                buf.extend_from_slice(b"\nstream\n");
                buf.extend_from_slice(&body);
                buf.extend_from_slice(b"\nendstream\nendobj\n");
            }
            Some(v) if compact => {
                let mut s = Vec::new();
                plain.ser(&v, &mut s);
                packed.push((n, s));
            }
            Some(v) => {
                at[n as usize] = At::Off(buf.len());
                let _ = writeln!(Hex(&mut buf), "{n} 0 obj");
                cx.ser(&v, &mut buf);
                buf.extend_from_slice(b"\nendobj\n");
            }
            None => {
                at[n as usize] = At::Off(buf.len());
                let _ = writeln!(Hex(&mut buf), "{n} 0 obj\nnull\nendobj");
            }
        }
    }
    // numbers after the original objects: object streams, the Encrypt dictionary, the xref stream
    let mut next = size;
    let mut enc_n = None;
    if let Some(s) = &setup {
        // its strings are not encrypted
        let n = next;
        next += 1;
        enc_n = Some(n);
        at.push(At::Off(buf.len()));
        buf.extend_from_slice(encrypt_object(n, protect.map_or(0, |p| p.allowed), s).as_bytes());
    }
    for chunk in packed.chunks(PER_OBJSTM) {
        let sn = next;
        next += 1;
        let mut head = String::new();
        let mut body: Vec<u8> = Vec::new();
        for (i, (n, s)) in chunk.iter().enumerate() {
            let _ = write!(head, "{n} {} ", body.len());
            body.extend_from_slice(s);
            body.push(b'\n');
            at[*n as usize] = At::In(sn, i);
        }
        let mut raw = head.clone().into_bytes();
        raw.extend_from_slice(&body);
        let z = cx.enc(&deflate(&raw));
        at.push(At::Off(buf.len()));
        let _ = write!(
            Hex(&mut buf),
            "{sn} 0 obj\n<< /Type /ObjStm /N {} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            chunk.len(),
            head.len(),
            z.len()
        );
        buf.extend_from_slice(&z);
        buf.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let mut id = [0u8; 16];
    if setup.is_some() {
        (cx.rand)(&mut id);
    }
    let mut trailer = String::new();
    let ent = |s: &mut String, k: &str, v: &Val| {
        if let Val::Ref(n) = v {
            let _ = write!(s, " /{k} {n} 0 R");
        }
    };
    ent(&mut trailer, "Root", &root);
    if let Some(v) = tr.get("Info") {
        ent(&mut trailer, "Info", v);
    }
    if let Some(n) = enc_n {
        let _ = write!(trailer, " /Encrypt {n} 0 R /ID [{} {}]", hex(&id), hex(&id));
    }
    let xref;
    if compact {
        // cross-reference stream: type(1) field2(4) field3(2)
        let xn = next;
        at.push(At::Off(buf.len()));
        xref = buf.len();
        let mut rows: Vec<u8> = Vec::new();
        for e in &at {
            let (t, a, b) = match *e {
                At::Free => (0u8, 0u32, 0xFFFFu32),
                At::Off(o) => (1, o as u32, 0),
                At::In(sn, i) => (2, sn, i as u32),
            };
            rows.push(t);
            rows.extend_from_slice(&a.to_be_bytes());
            rows.extend_from_slice(&(b as u16).to_be_bytes());
        }
        let z = deflate(&rows);
        let _ = write!(
            Hex(&mut buf),
            "{xn} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2]{trailer} /Filter /FlateDecode /Length {} >>\nstream\n",
            at.len(),
            z.len()
        );
        buf.extend_from_slice(&z);
        let _ = write!(
            Hex(&mut buf),
            "\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n"
        );
    } else {
        xref = buf.len();
        let _ = write!(Hex(&mut buf), "xref\n0 {}\n0000000000 65535 f \n", at.len());
        for e in at.iter().skip(1) {
            let o = match e {
                At::Off(o) => *o,
                _ => 0,
            };
            let _ = writeln!(Hex(&mut buf), "{o:010} 00000 n ");
        }
        let _ = write!(
            Hex(&mut buf),
            "trailer\n<< /Size {}{trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            at.len()
        );
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypt::{Crypt, EncryptParams, Method};

    fn counter() -> impl FnMut(&mut [u8]) {
        let mut x = 7u8;
        move |b: &mut [u8]| {
            for v in b {
                x = x.wrapping_mul(31).wrapping_add(17);
                *v = x;
            }
        }
    }

    fn sample() -> Vec<u8> {
        let mut o = b"%PDF-1.4\n".to_vec();
        let mut offs = Vec::new();
        for (n, body) in [
            (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>"
                    .to_string(),
            ),
            (
                4,
                "<< /Length 21 >>\nstream\n0 0 50 50 re f % x y\nendstream".to_string(),
            ),
            (5, "<< /Title (Secret title) >>".to_string()),
        ] {
            offs.push(o.len());
            o.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let x = o.len();
        o.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for f in offs {
            o.extend_from_slice(format!("{f:010} 00000 n \n").as_bytes());
        }
        o.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{x}\n%%EOF\n")
                .as_bytes(),
        );
        o
    }

    #[test]
    fn encrypted_file_needs_the_user_password_and_hides_text() {
        let src = sample();
        let enc = encrypt(&src, "open", "boss", perm::PRINT, &mut counter()).unwrap();
        let text = String::from_utf8_lossy(&enc);
        assert!(!text.contains("Secret title") && !text.contains("0 0 50 50 re f"));
        assert!(text.contains("/Encrypt 6 0 R"));
        // without a password the file is refused, with the user or the owner password it opens
        assert!(PdfDoc::open(enc.clone()).is_err());
        assert_eq!(
            PdfDoc::open_with_password(enc.clone(), b"open")
                .unwrap()
                .page_count(),
            1
        );
        assert_eq!(
            PdfDoc::open_with_password(enc.clone(), b"boss")
                .unwrap()
                .page_count(),
            1
        );
        assert!(PdfDoc::open_with_password(enc, b"wrong").is_err());
    }

    #[test]
    fn decrypting_gives_back_a_plain_file_with_the_same_content() {
        let src = sample();
        let enc = encrypt(&src, "open", "boss", perm::PRINT, &mut counter()).unwrap();
        let doc = PdfDoc::open_with_password(enc, b"open").unwrap();
        assert!(doc.is_encrypted());
        let plain = doc.decrypted().unwrap();
        let t = String::from_utf8_lossy(&plain);
        assert!(!t.contains("/Encrypt"), "still encrypted");
        assert!(
            t.contains(&hex(b"Secret title")) && t.contains("0 0 50 50 re f"),
            "{t}"
        );
        let again = PdfDoc::open(plain).unwrap();
        assert!(!again.is_encrypted());
        assert_eq!(again.page_count(), 1);
    }

    #[test]
    fn empty_user_password_opens_freely_and_keeps_restrictions_in_p() {
        let enc = encrypt(&sample(), "", "boss", perm::PRINT, &mut counter()).unwrap();
        assert!(PdfDoc::open(enc.clone()).is_ok());
        let text = String::from_utf8_lossy(&enc).into_owned();
        let p = text.split("/P ").nth(1).unwrap().split(' ').next().unwrap();
        let p: i32 = p.parse().unwrap();
        assert_eq!(p & perm::ALL, perm::PRINT);
        assert_eq!(p, p_value(perm::PRINT));
    }

    #[test]
    fn the_file_key_comes_out_of_the_setup() {
        let s = make_v5(b"u", b"o", p_value(0), true, &mut counter());
        for pw in [&b"u"[..], &b"o"[..]] {
            let c = Crypt::new(&EncryptParams {
                v: 5,
                r: 6,
                length_bits: 256,
                o: &s.o,
                u: &s.u,
                ue: &s.ue,
                oe: &s.oe,
                password: pw,
                p: p_value(0) as i64,
                id0: b"",
                encrypt_metadata: true,
                stm: Method::AesV3,
            })
            .unwrap();
            let enc = aes_cbc_encrypt(&s.key, [9; 16], b"hello").unwrap();
            assert_eq!(c.decrypt_stream(1, 0, &enc), b"hello");
        }
    }

    #[test]
    fn an_encrypted_file_is_not_encrypted_twice() {
        let enc = encrypt(&sample(), "", "boss", perm::ALL, &mut counter()).unwrap();
        assert!(encrypt(&enc, "x", "y", 0, &mut counter()).is_err());
    }

    #[test]
    fn compact_packs_plain_objects_and_still_opens() {
        let src = sample();
        let c = rewrite(&src, true, None, &mut counter()).unwrap();
        let t = String::from_utf8_lossy(&c);
        assert!(t.contains("/Type /ObjStm") && t.contains("/Type /XRef"));
        assert!(
            !t.contains("trailer"),
            "xref stream files have no trailer keyword"
        );
        // the page dictionaries are inside the compressed stream, not readable in the file
        assert!(!t.contains("/MediaBox"));
        let doc = PdfDoc::open(c.clone()).unwrap();
        assert_eq!(doc.page_count(), 1);
        // the content stream and the info string survive
        assert!(t.contains("/Length"));
        // compacting a compact file is not supported, and says so
        assert!(rewrite(&c, true, None, &mut counter()).is_err());
    }

    #[test]
    fn a_compact_file_can_be_flattened_for_signing() {
        let c = rewrite(&sample(), true, None, &mut counter()).unwrap();
        let doc = PdfDoc::open(c).unwrap();
        assert!(doc.uses_xref_stream());
        let flat = doc.flattened().unwrap();
        let t = String::from_utf8_lossy(&flat);
        assert!(t.contains("\ntrailer") && !t.contains("/ObjStm"));
        assert!(!PdfDoc::open(flat.clone()).unwrap().uses_xref_stream());
        // a classic file is signable now
        assert!(super::super::sign::prepare(&flat, 0, None, None).is_ok());
    }

    #[test]
    fn compact_and_encrypt_together() {
        let src = sample();
        let p = Protect {
            user: "open",
            owner: "boss",
            allowed: perm::PRINT,
        };
        let c = rewrite(&src, true, Some(&p), &mut counter()).unwrap();
        let t = String::from_utf8_lossy(&c);
        assert!(t.contains("/Type /ObjStm") && t.contains("/Encrypt "));
        assert!(PdfDoc::open(c.clone()).is_err());
        assert_eq!(
            PdfDoc::open_with_password(c, b"open").unwrap().page_count(),
            1
        );
    }
}
