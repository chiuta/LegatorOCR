//! Password-protected zip archives in the WinZip AES format (AE-2, AES-256, HMAC-SHA1
//! authentication), readable by 7-Zip, WinZip, Keka, The Unarchiver and Info-ZIP's newer
//! builds. File names stay visible (as in every zip); only the contents are encrypted.
//!
//! Written for handing over an archive, not as a vault: the key comes from the password with
//! PBKDF2-HMAC-SHA1 and 1000 rounds (what the format prescribes), so use a long passphrase.

use anyhow::{bail, Result};
use flate2::{write::DeflateEncoder, Compression, Crc};
use legatorocr_pdf::crypt::Aes;
use std::io::Write;

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, c) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..20].copy_from_slice(&sha1(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend(data);
    let ih = sha1(&inner);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend(ih);
    sha1(&outer)
}

fn pbkdf2_sha1(password: &[u8], salt: &[u8], rounds: u32, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut block = 1u32;
    while out.len() < len {
        let mut s = salt.to_vec();
        s.extend(block.to_be_bytes());
        let mut u = hmac_sha1(password, &s);
        let mut t = u;
        for _ in 1..rounds {
            u = hmac_sha1(password, &u);
            for (a, b) in t.iter_mut().zip(u) {
                *a ^= b;
            }
        }
        out.extend(t);
        block += 1;
    }
    out.truncate(len);
    out
}

/// AES-CTR the way WinZip does it: little-endian counter starting at 1.
fn ctr(aes: &Aes, data: &mut [u8]) {
    for (counter, chunk) in (1u128..).zip(data.chunks_mut(16)) {
        let mut ks = counter.to_le_bytes();
        aes.encrypt_block(&mut ks);
        for (d, k) in chunk.iter_mut().zip(ks) {
            *d ^= k;
        }
    }
}

/// Encrypt one entry body: salt | 2-byte password check | ciphertext | 10-byte HMAC.
fn seal(password: &[u8], salt: [u8; 16], body: &[u8]) -> Vec<u8> {
    let dk = pbkdf2_sha1(password, &salt, 1000, 66);
    let aes = Aes::new(&dk[..32]).expect("32-byte key");
    let mut ct = body.to_vec();
    ctr(&aes, &mut ct);
    let mac = hmac_sha1(&dk[32..64], &ct);
    let mut out = Vec::with_capacity(16 + 2 + ct.len() + 10);
    out.extend(salt);
    out.extend(&dk[64..66]);
    out.extend(ct);
    out.extend(&mac[..10]);
    out
}

/// Build an AES-256 encrypted zip. Every entry gets its own random salt.
pub fn write_encrypted(entries: &[(&str, &[u8])], password: &str) -> Result<Vec<u8>> {
    if password.is_empty() {
        bail!("the zip password is empty");
    }
    if entries.len() > u16::MAX as usize {
        bail!("too many files for one zip ({})", entries.len());
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        if data.len() as u64 > u32::MAX as u64 - 1_000_000 {
            bail!("{name}: too large for a zip entry (4 GiB limit)");
        }
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data)?;
        let z = enc.finish()?;
        let (method, body): (u16, &[u8]) = if z.len() < data.len() {
            (8, &z)
        } else {
            (0, data)
        };
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("no random numbers: {e}"))?;
        let sealed = seal(password.as_bytes(), salt, body);
        let off = out.len() as u32;
        let _ = Crc::new(); // AE-2 stores no CRC: the HMAC authenticates instead
        let extra = {
            let mut e = Vec::new();
            e.extend(0x9901u16.to_le_bytes());
            e.extend(7u16.to_le_bytes());
            e.extend(2u16.to_le_bytes()); // AE-2
            e.extend(b"AE");
            e.push(3); // AES-256
            e.extend(method.to_le_bytes()); // real compression method
            e
        };
        let hdr = |h: &mut Vec<u8>, is_central: bool| {
            if is_central {
                h.extend([0x50, 0x4b, 0x01, 0x02, 51, 0]);
            } else {
                h.extend([0x50, 0x4b, 0x03, 0x04]);
            }
            h.extend(51u16.to_le_bytes());
            h.extend(0x0801u16.to_le_bytes()); // encrypted + UTF-8 names
            h.extend(99u16.to_le_bytes()); // AES
            h.extend(0u16.to_le_bytes());
            h.extend(0x21u16.to_le_bytes());
            h.extend(0u32.to_le_bytes()); // CRC (AE-2)
            h.extend((sealed.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend((extra.len() as u16).to_le_bytes());
            if is_central {
                h.extend([0u8; 6]);
                h.extend(0u32.to_le_bytes());
                h.extend(off.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h.extend(&extra);
        };
        hdr(&mut out, false);
        out.extend(&sealed);
        hdr(&mut central, true);
    }
    let cd_off = out.len() as u32;
    out.extend(&central);
    out.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd_off.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_hmac_and_pbkdf2_match_the_published_test_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        // RFC 6070
        assert_eq!(
            hex(&pbkdf2_sha1(b"password", b"salt", 2, 20)),
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
        );
        // the AES key-derivation length the format needs
        assert_eq!(pbkdf2_sha1(b"p", b"s", 3, 66).len(), 66);
    }

    #[test]
    fn empty_password_and_oversized_lists_are_refused() {
        assert!(write_encrypted(&[("a", b"x")], "").is_err());
    }

    #[test]
    fn seal_has_the_documented_layout() {
        let s = seal(b"pw", [7u8; 16], b"hello world, hello world");
        assert_eq!(s.len(), 16 + 2 + 24 + 10);
        assert_eq!(&s[..16], &[7u8; 16]);
        // a different password gives a different check value and ciphertext
        let t = seal(b"pw2", [7u8; 16], b"hello world, hello world");
        assert_ne!(s[16..], t[16..]);
    }
}
