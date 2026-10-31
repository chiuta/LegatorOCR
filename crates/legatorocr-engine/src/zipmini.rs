//! Just enough ZIP for reading archives to search and for writing `.docx` files:
//! stored and deflated entries, no encryption, no ZIP64.

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};
use std::io::{Read, Write};

/// Largest single entry we are willing to inflate.
const MAX_ENTRY: u64 = 256 << 20;
const MAX_ENTRIES: usize = 100_000;

fn u16le(d: &[u8], p: usize) -> Option<usize> {
    Some(u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?) as usize)
}
fn u32le(d: &[u8], p: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as usize)
}

/// One archive member (not yet inflated).
pub struct Entry {
    pub name: String,
    method: usize,
    csize: usize,
    usize_: usize,
    local: usize,
}

/// List the members of an archive. `None` when it is not a (supported) ZIP file.
pub fn list(d: &[u8]) -> Option<Vec<Entry>> {
    // End of central directory: last 22+ bytes.
    let tail = d.len().saturating_sub(22 + 65_535);
    let eocd = (tail..=d.len().checked_sub(22)?)
        .rev()
        .find(|&i| d[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))?;
    let n = u16le(d, eocd + 10)?;
    let mut p = u32le(d, eocd + 16)?;
    let mut v = Vec::new();
    for _ in 0..n.min(MAX_ENTRIES) {
        if d.get(p..p + 4)? != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let (method, csize, usize_) = (u16le(d, p + 10)?, u32le(d, p + 20)?, u32le(d, p + 24)?);
        let (nl, el, cl) = (u16le(d, p + 28)?, u16le(d, p + 30)?, u16le(d, p + 32)?);
        let local = u32le(d, p + 42)?;
        let name = String::from_utf8_lossy(d.get(p + 46..p + 46 + nl)?).into_owned();
        v.push(Entry {
            name,
            method,
            csize,
            usize_,
            local,
        });
        p += 46 + nl + el + cl;
    }
    Some(v)
}

/// Inflate one member. `None` for unsupported methods, bad offsets or oversized data.
pub fn read(d: &[u8], e: &Entry) -> Option<Vec<u8>> {
    if e.usize_ as u64 > MAX_ENTRY || d.get(e.local..e.local + 4)? != [0x50, 0x4b, 0x03, 0x04] {
        return None;
    }
    let start = e.local + 30 + u16le(d, e.local + 26)? + u16le(d, e.local + 28)?;
    let raw = d.get(start..start.checked_add(e.csize)?)?;
    match e.method {
        0 => Some(raw.to_vec()),
        8 => {
            let mut out = Vec::new();
            DeflateDecoder::new(raw)
                .take(MAX_ENTRY)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        _ => None,
    }
}

/// Build an archive (deflated entries, current-date-free: fixed DOS timestamp).
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let mut crc = Crc::new();
        crc.update(data);
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data).unwrap();
        let z = enc.finish().unwrap();
        let off = out.len() as u32;
        let (method, body): (u16, &[u8]) = if z.len() < data.len() {
            (8, &z)
        } else {
            (0, data)
        };
        let hdr = |h: &mut Vec<u8>, central: bool| {
            if central {
                h.extend([0x50, 0x4b, 0x01, 0x02, 20, 0]); // made by
            } else {
                h.extend([0x50, 0x4b, 0x03, 0x04]);
            }
            h.extend(20u16.to_le_bytes()); // version needed
            h.extend(0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend(method.to_le_bytes());
            h.extend(0u16.to_le_bytes()); // time 00:00
            h.extend(0x21u16.to_le_bytes()); // date 1980-01-01
            h.extend(crc.sum().to_le_bytes());
            h.extend((body.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend([0u8; 6]); // comment len, disk, internal attrs
                h.extend(0u32.to_le_bytes()); // external attrs
                h.extend(off.to_le_bytes());
            }
            h.extend(name.as_bytes());
        };
        hdr(&mut out, false);
        out.extend(body);
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_stored_and_deflated() {
        let big = "ab".repeat(5000);
        let z = write(&[("a.txt", b"hi"), ("dir/b.txt", big.as_bytes())]);
        let l = list(&z).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "a.txt");
        assert_eq!(read(&z, &l[0]).unwrap(), b"hi");
        assert_eq!(read(&z, &l[1]).unwrap(), big.as_bytes());
    }

    #[test]
    fn garbage_is_not_a_zip() {
        assert!(list(b"").is_none());
        assert!(list(&[0x50; 100]).is_none());
        let mut z = write(&[("a", b"x")]);
        z.truncate(z.len() - 5);
        assert!(list(&z).is_none());
    }
}
