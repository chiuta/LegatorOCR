//! Password-protect a finished PDF (AES-256, standard security handler V5/R6): every string and
//! stream is encrypted with the file key, an /Encrypt dictionary and a file /ID are added.
//! Works on the files this program writes (classic cross-reference table, no object streams
//! needed); an already encrypted file is refused.

use super::edit::esc_name;
use super::{fail, Dict, PdfDoc, Val, P, R};
use crate::crypt::{aes_cbc_encrypt, make_v5};
use std::fmt::Write as _;

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

struct Ctx<'a> {
    key: &'a [u8; 32],
    rand: &'a mut dyn FnMut(&mut [u8]),
}

impl Ctx<'_> {
    fn enc(&mut self, data: &[u8]) -> Vec<u8> {
        let mut iv = [0u8; 16];
        (self.rand)(&mut iv);
        aes_cbc_encrypt(self.key, iv, data).unwrap_or_default()
    }

    fn ser(&mut self, v: &Val, s: &mut Vec<u8>) {
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
                let _ = write!(Hex(s), "{n} 0 R");
            }
        }
    }

    fn dict(&mut self, d: &Dict, len: Option<usize>, s: &mut Vec<u8>) {
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

struct Hex<'a>(&'a mut Vec<u8>);
impl std::fmt::Write for Hex<'_> {
    fn write_str(&mut self, t: &str) -> std::fmt::Result {
        self.0.extend_from_slice(t.as_bytes());
        Ok(())
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::from("<");
    for x in b {
        let _ = write!(s, "{x:02X}");
    }
    s.push('>');
    s
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
    let doc = PdfDoc::open(data.to_vec())?;
    // the last trailer of our own classic file
    let Some(t) = data.windows(7).rposition(|w| w == b"trailer") else {
        return fail("only files with a classic cross-reference table can be encrypted");
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
    let p = p_value(allowed);
    let owner = if owner.is_empty() { user } else { owner };
    let setup = make_v5(user.as_bytes(), owner.as_bytes(), p, true, rand);
    let mut cx = Ctx {
        key: &setup.key,
        rand,
    };
    let mut buf: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offs = Vec::new();
    for n in 1..size {
        offs.push(buf.len());
        let _ = writeln!(Hex(&mut buf), "{n} 0 obj");
        match doc.get(n) {
            Some(Val::Stream(d, a, b)) => {
                let raw = data.get(a..b).unwrap_or_default();
                let body = cx.enc(raw);
                cx.dict(&d, Some(body.len()), &mut buf);
                buf.extend_from_slice(b"\nstream\n");
                buf.extend_from_slice(&body);
                buf.extend_from_slice(b"\nendstream");
            }
            Some(v) => cx.ser(&v, &mut buf),
            None => buf.extend_from_slice(b"null"),
        }
        buf.extend_from_slice(b"\nendobj\n");
    }
    // the Encrypt dictionary: its strings are not encrypted
    let enc_n = size;
    offs.push(buf.len());
    let _ = write!(
        Hex(&mut buf),
        "{enc_n} 0 obj\n<< /Filter /Standard /V 5 /R 6 /Length 256 /P {p} /O {} /U {} /OE {} /UE {} /Perms {} /EncryptMetadata true /CF << /StdCF << /AuthEvent /DocOpen /CFM /AESV3 /Length 32 >> >> /StmF /StdCF /StrF /StdCF >>\nendobj\n",
        hex(&setup.o),
        hex(&setup.u),
        hex(&setup.oe),
        hex(&setup.ue),
        hex(&setup.perms)
    );
    let mut id = [0u8; 16];
    (cx.rand)(&mut id);
    let xref = buf.len();
    let _ = write!(
        Hex(&mut buf),
        "xref\n0 {}\n0000000000 65535 f \n",
        offs.len() + 1
    );
    for o in &offs {
        let _ = writeln!(Hex(&mut buf), "{o:010} 00000 n ");
    }
    let mut trailer = Vec::new();
    cx_plain_ref(&root, &mut trailer);
    let info = match tr.get("Info") {
        Some(Val::Ref(n)) => format!(" /Info {n} 0 R"),
        _ => String::new(),
    };
    let _ = write!(
        Hex(&mut buf),
        "trailer\n<< /Size {} /Root {} /Encrypt {enc_n} 0 R{info} /ID [{} {}] >>\nstartxref\n{xref}\n%%EOF\n",
        offs.len() + 1,
        String::from_utf8_lossy(&trailer),
        hex(&id),
        hex(&id)
    );
    Ok(buf)
}

fn cx_plain_ref(v: &Val, s: &mut Vec<u8>) {
    if let Val::Ref(n) = v {
        let _ = write!(Hex(s), "{n} 0 R");
    }
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
}
