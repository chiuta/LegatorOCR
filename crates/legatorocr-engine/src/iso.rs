//! Minimal ISO 9660 image writer with Joliet long Unicode names, for handing a set of files
//! over as one `.iso` (what Folder2Iso is used for in digitisation workflows). One flat root
//! directory, no El Torito, no Rock Ridge: every file shows up with its full name on Windows,
//! macOS and Linux (via the Joliet tree) and with an 8.3 name in plain ISO 9660.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::PathBuf;

const SECTOR: usize = 2048;
/// An extent size is a 32-bit field.
const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - SECTOR as u64;
const MAX_FILES: usize = 5000;

pub enum Source {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

pub struct IsoFile {
    pub name: String,
    pub source: Source,
}

fn both32(v: u32) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&v.to_le_bytes());
    o[4..].copy_from_slice(&v.to_be_bytes());
    o
}
fn both16(v: u16) -> [u8; 4] {
    let mut o = [0u8; 4];
    o[..2].copy_from_slice(&v.to_le_bytes());
    o[2..].copy_from_slice(&v.to_be_bytes());
    o
}

/// (year, month, day, hour, minute, second) in UTC from a Unix time.
fn civil(secs: u64) -> (i64, u8, u8, u8, u8, u8) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        m,
        d,
        (rem / 3600) as u8,
        (rem % 3600 / 60) as u8,
        (rem % 60) as u8,
    )
}

fn dir_date(secs: u64) -> [u8; 7] {
    let (y, mo, d, h, mi, s) = civil(secs);
    [(y - 1900).clamp(0, 255) as u8, mo, d, h, mi, s, 0]
}

fn vol_date(secs: u64) -> [u8; 17] {
    let (y, mo, d, h, mi, s) = civil(secs);
    let t = format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}00");
    let mut o = [0u8; 17];
    o[..16].copy_from_slice(t.as_bytes());
    o[16] = 0; // UTC
    o
}

/// d-characters only: A–Z, 0–9, `_`.
fn dchars(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| {
            let u = c.to_ascii_uppercase();
            if u.is_ascii_uppercase() || u.is_ascii_digit() {
                u
            } else {
                '_'
            }
        })
        .take(max)
        .collect()
}

/// Unique 8.3 ISO 9660 names (`BASE.EXT;1`) for the given file names.
fn iso_names(names: &[&str]) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    names
        .iter()
        .map(|n| {
            let (stem, ext) = n.rsplit_once('.').unwrap_or((n, ""));
            let ext = dchars(ext, 3);
            let mut base = dchars(stem, 8);
            if base.is_empty() {
                base = "FILE".into();
            }
            let mut cand = base.clone();
            let mut k = 1;
            while !used.insert(format!("{cand}.{ext}")) {
                let tag = format!("{k}");
                cand = format!("{}{tag}", &base[..base.len().min(8 - tag.len())]);
                k += 1;
            }
            format!("{cand}.{ext};1")
        })
        .collect()
}

fn ucs2(s: &str, max_units: usize) -> Vec<u8> {
    s.encode_utf16()
        .take(max_units)
        .flat_map(|u| u.to_be_bytes())
        .collect()
}

/// UTF-16 length limit of a Joliet name.
const JOLIET_MAX: usize = 64;

/// Joliet forbids `* / : ; ? \` in names, allows 64 UTF-16 units and is compared without regard
/// to case. Names are cleaned, cut so the extension survives, and made unique (`~2`, `~3` …),
/// so two long names sharing a prefix, or `a:b.pdf` and `a_b.pdf`, never end up identical.
fn joliet_names(names: &[&str]) -> Vec<Vec<u8>> {
    let mut used = std::collections::HashSet::new();
    names
        .iter()
        .map(|n| {
            let clean: String = n
                .chars()
                .map(|c| {
                    if "*/:;?\\".contains(c) || (c as u32) < 0x20 {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let (stem, ext) = match clean.rfind('.') {
                Some(i) if i > 0 && clean[i..].encode_utf16().count() <= 20 => {
                    (clean[..i].to_string(), clean[i..].to_string())
                }
                _ => (clean.clone(), String::new()),
            };
            let units = |s: &str| s.encode_utf16().count();
            let fit = |stem: &str, tag: &str| -> String {
                let room = JOLIET_MAX.saturating_sub(units(&ext) + units(tag)).max(1);
                let mut out = String::new();
                for c in stem.chars() {
                    if units(&out) + c.len_utf16() > room {
                        break;
                    }
                    out.push(c);
                }
                format!("{out}{tag}{ext}")
            };
            let mut cand = fit(&stem, "");
            let mut k = 2;
            while !used.insert(cand.to_lowercase()) {
                cand = fit(&stem, &format!("~{k}"));
                k += 1;
            }
            ucs2(&cand, JOLIET_MAX)
        })
        .collect()
}

fn record(lba: u32, size: u32, flags: u8, name: &[u8], when: u64) -> Vec<u8> {
    let mut len = 33 + name.len();
    if len % 2 == 1 {
        len += 1;
    }
    let mut r = vec![0u8; len];
    r[0] = len as u8;
    r[2..10].copy_from_slice(&both32(lba));
    r[10..18].copy_from_slice(&both32(size));
    r[18..25].copy_from_slice(&dir_date(when));
    r[25] = flags;
    r[28..32].copy_from_slice(&both16(1));
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

/// Records packed into whole sectors (a record never crosses a sector boundary).
fn pack(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for r in records {
        let used = out.len() % SECTOR;
        if used + r.len() > SECTOR {
            out.resize(out.len() + SECTOR - used, 0);
        }
        out.extend_from_slice(r);
    }
    let rem = out.len() % SECTOR;
    if rem != 0 || out.is_empty() {
        out.resize(out.len() + SECTOR - rem, 0);
    }
    out
}

fn path_table(root_lba: u32, big: bool) -> Vec<u8> {
    let mut t = vec![0u8; 10];
    t[0] = 1; // identifier length
    let lba = if big {
        root_lba.to_be_bytes()
    } else {
        root_lba.to_le_bytes()
    };
    t[2..6].copy_from_slice(&lba);
    let parent = if big {
        1u16.to_be_bytes()
    } else {
        1u16.to_le_bytes()
    };
    t[6..8].copy_from_slice(&parent);
    t[8] = 0; // root
    t
}

fn pad_field(s: &str, n: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().take(n).collect();
    v.resize(n, b' ');
    v
}

fn pad_field_ucs2(s: &str, n_bytes: usize) -> Vec<u8> {
    let mut v = ucs2(s, n_bytes / 2);
    while v.len() < n_bytes {
        v.extend([0x00, 0x20]);
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn descriptor(
    joliet: bool,
    label: &str,
    total_sectors: u32,
    l_path: u32,
    m_path: u32,
    root: &[u8],
    when: u64,
) -> Vec<u8> {
    let mut d = vec![0u8; SECTOR];
    d[0] = if joliet { 2 } else { 1 };
    d[1..6].copy_from_slice(b"CD001");
    d[6] = 1;
    let text = |s: &str, n: usize| {
        if joliet {
            pad_field_ucs2(s, n)
        } else {
            pad_field(s, n)
        }
    };
    d[8..40].copy_from_slice(&text("", 32));
    d[40..72].copy_from_slice(&text(label, 32));
    d[80..88].copy_from_slice(&both32(total_sectors));
    if joliet {
        d[88..91].copy_from_slice(b"%/E"); // UCS-2 level 3
    }
    d[120..124].copy_from_slice(&both16(1)); // volume set size
    d[124..128].copy_from_slice(&both16(1)); // sequence number
    d[128..132].copy_from_slice(&both16(SECTOR as u16));
    d[132..140].copy_from_slice(&both32(10)); // path table size
    d[140..144].copy_from_slice(&l_path.to_le_bytes());
    d[148..152].copy_from_slice(&m_path.to_be_bytes());
    d[156..190].copy_from_slice(&root[..34]);
    for (off, n) in [(190usize, 128usize), (318, 128), (446, 128), (574, 128)] {
        d[off..off + n].copy_from_slice(&text("", n));
    }
    let app = "LegatorOCR";
    d[574..702].copy_from_slice(&text(app, 128));
    for off in [702usize, 739, 776] {
        // copyright / abstract / bibliographic file identifiers: none
        d[off..off + 37].copy_from_slice(&if joliet {
            pad_field_ucs2("", 36)
                .into_iter()
                .chain([0])
                .collect::<Vec<u8>>()
        } else {
            pad_field("", 37)
        });
    }
    let dt = vol_date(when);
    d[813..830].copy_from_slice(&dt); // created
    d[830..847].copy_from_slice(&dt); // modified
    d[847..864].copy_from_slice(b"0000000000000000\0"); // expires: not specified
    d[864..881].copy_from_slice(&dt); // effective
    d[881] = 1; // file structure version
    d
}

/// Write the image. `label` is the volume name (letters, digits, `_`; at most 16 characters).
pub fn write_iso(dest: &std::path::Path, label: &str, when: u64, files: &[IsoFile]) -> Result<u64> {
    if files.len() > MAX_FILES {
        bail!(
            "too many files for one ISO image ({} > {MAX_FILES})",
            files.len()
        );
    }
    let mut sizes = Vec::new();
    for f in files {
        let n = match &f.source {
            Source::Path(p) => std::fs::metadata(p)
                .with_context(|| format!("{}: cannot read", p.display()))?
                .len(),
            Source::Bytes(b) => b.len() as u64,
        };
        if n > MAX_FILE {
            bail!("{}: larger than 4 GiB, which ISO 9660 cannot hold", f.name);
        }
        sizes.push(n as u32);
    }
    let label = {
        let l = dchars(label, 16);
        if l.is_empty() {
            "LEGATOROCR".to_string()
        } else {
            l
        }
    };
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    let iso_n = iso_names(&names);
    let jol_n: Vec<Vec<u8>> = joliet_names(&names);

    // directory entries in each tree's own sort order
    let mut iso_order: Vec<usize> = (0..files.len()).collect();
    iso_order.sort_by(|&a, &b| iso_n[a].cmp(&iso_n[b]));
    let mut jol_order: Vec<usize> = (0..files.len()).collect();
    jol_order.sort_by(|&a, &b| jol_n[a].cmp(&jol_n[b]));

    // layout: 16 PVD, 17 SVD, 18 end, 19–22 path tables, then the two directories, then data
    let (l1, m1, l2, m2) = (19u32, 20u32, 21u32, 22u32);
    let dir1 = 23u32;
    let build_dir = |lba: u32,
                     size_hint: u32,
                     ext: &dyn Fn(usize) -> Vec<u8>,
                     order: &[usize],
                     extents: &[u32]| {
        let mut recs = vec![
            record(lba, size_hint, 2, &[0], when),
            record(dir1, size_hint, 2, &[1], when),
        ];
        for &i in order {
            recs.push(record(extents[i], sizes[i], 0, &ext(i), when));
        }
        recs
    };
    let zero = vec![0u32; files.len()];
    let iso_ext = |i: usize| iso_n[i].as_bytes().to_vec();
    let jol_ext = |i: usize| jol_n[i].clone();
    let n1 = (pack(&build_dir(dir1, 0, &iso_ext, &iso_order, &zero)).len() / SECTOR) as u32;
    let dir2 = dir1 + n1;
    let n2 = (pack(&build_dir(dir2, 0, &jol_ext, &jol_order, &zero)).len() / SECTOR) as u32;
    let mut next = dir2 + n2;
    let mut extents = Vec::new();
    for &s in &sizes {
        extents.push(next);
        next += (s as usize).div_ceil(SECTOR) as u32;
    }
    let total = next;
    // final directories, now that extents are known; "." and ".." of the Joliet root point
    // at the Joliet root
    let mut d1 = build_dir(dir1, n1 * SECTOR as u32, &iso_ext, &iso_order, &extents);
    d1[1] = record(dir1, n1 * SECTOR as u32, 2, &[1], when);
    let mut d2 = build_dir(dir2, n2 * SECTOR as u32, &jol_ext, &jol_order, &extents);
    d2[1] = record(dir2, n2 * SECTOR as u32, 2, &[1], when);
    let (dir1_bytes, dir2_bytes) = (pack(&d1), pack(&d2));
    let root1 = record(dir1, n1 * SECTOR as u32, 2, &[0], when);
    let root2 = record(dir2, n2 * SECTOR as u32, 2, &[0], when);

    let tmp = dest.with_extension("iso.part");
    // an unfinished image never stays behind, whatever makes the writing stop
    struct Part(PathBuf, bool);
    impl Drop for Part {
        fn drop(&mut self) {
            if !self.1 {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
    let mut part = Part(tmp.clone(), false);
    let mut out = std::io::BufWriter::new(
        std::fs::File::create(&tmp).with_context(|| format!("{}: cannot create", tmp.display()))?,
    );
    let mut wr = |b: &[u8]| out.write_all(b);
    wr(&vec![0u8; 16 * SECTOR])?;
    wr(&descriptor(false, &label, total, l1, m1, &root1, when))?;
    wr(&descriptor(true, &label, total, l2, m2, &root2, when))?;
    let mut end = vec![0u8; SECTOR];
    end[0] = 255;
    end[1..6].copy_from_slice(b"CD001");
    end[6] = 1;
    wr(&end)?;
    for t in [
        path_table(dir1, false),
        path_table(dir1, true),
        path_table(dir2, false),
        path_table(dir2, true),
    ] {
        let mut sec = vec![0u8; SECTOR];
        sec[..t.len()].copy_from_slice(&t);
        wr(&sec)?;
    }
    wr(&dir1_bytes)?;
    wr(&dir2_bytes)?;
    for f in files {
        let mut written = 0usize;
        match &f.source {
            Source::Bytes(b) => {
                wr(b)?;
                written = b.len();
            }
            Source::Path(p) => {
                let mut r = std::fs::File::open(p)
                    .with_context(|| format!("{}: cannot read", p.display()))?;
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = r.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    wr(&buf[..n])?;
                    written += n;
                }
            }
        }
        let rem = written % SECTOR;
        if rem != 0 {
            wr(&vec![0u8; SECTOR - rem])?;
        }
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, dest).with_context(|| format!("{}: cannot finish", dest.display()))?;
    part.1 = true;
    Ok(total as u64 * SECTOR as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_8_3_unique() {
        let n = iso_names(&[
            "Raport anual.pdf",
            "SHA256SUMS.txt",
            "raport_anual_2.pdf",
            "ă.txt",
            "a",
        ]);
        assert_eq!(n[0], "RAPORT_A.PDF;1");
        assert_eq!(n[1], "SHA256SU.TXT;1");
        assert_ne!(n[0], n[2]);
        assert_eq!(n[3], "_.TXT;1");
        assert_eq!(n[4], "A.;1");
        let set: std::collections::HashSet<_> = n.iter().collect();
        assert_eq!(set.len(), n.len());
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_793_000_000).0, 2026);
    }

    fn u32le(b: &[u8], o: usize) -> usize {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize
    }

    /// Read the image back with an independent minimal parser (primary tree and Joliet tree).
    #[test]
    fn image_reads_back_in_both_trees() {
        let d = std::env::temp_dir().join(format!("lgr-iso-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(d.join("big.bin"), &big).unwrap();
        let files = vec![
            IsoFile {
                name: "Zeta ăț.txt".into(),
                source: Source::Bytes(b"hello".to_vec()),
            },
            IsoFile {
                name: "big.bin".into(),
                source: Source::Path(d.join("big.bin")),
            },
            IsoFile {
                name: "empty".into(),
                source: Source::Bytes(vec![]),
            },
        ];
        let out = d.join("t.iso");
        let n = write_iso(&out, "my label!", 1_793_000_000, &files).unwrap();
        let img = std::fs::read(&out).unwrap();
        assert_eq!(img.len() as u64, n);
        assert_eq!(img.len() % SECTOR, 0);
        for (sec, ty) in [(16usize, 1u8), (17, 2), (18, 255)] {
            assert_eq!(&img[sec * SECTOR + 1..sec * SECTOR + 6], b"CD001");
            assert_eq!(img[sec * SECTOR], ty);
        }
        assert_eq!(u32le(&img, 16 * SECTOR + 80), img.len() / SECTOR);
        let find = |vd: usize, joliet: bool, want: &[u8]| -> Option<Vec<u8>> {
            let root = &img[vd * SECTOR + 156..vd * SECTOR + 190];
            let (lba, size) = (u32le(root, 2), u32le(root, 10));
            let dir = &img[lba * SECTOR..lba * SECTOR + size];
            let mut o = 0;
            while o < dir.len() {
                let len = dir[o] as usize;
                if len == 0 {
                    o = (o / SECTOR + 1) * SECTOR;
                    continue;
                }
                let name = &dir[o + 33..o + 33 + dir[o + 32] as usize];
                if name == want {
                    let (l, s) = (u32le(dir, o + 2), u32le(dir, o + 10));
                    return Some(img[l * SECTOR..l * SECTOR + s].to_vec());
                }
                o += len;
            }
            let _ = joliet;
            None
        };
        assert_eq!(find(16, false, b"ZETA___.TXT;1").unwrap(), b"hello");
        assert_eq!(find(16, false, b"BIG.BIN;1").unwrap(), big);
        assert_eq!(find(16, false, b"EMPTY.;1").unwrap(), b"");
        let j: Vec<u8> = "Zeta ăț.txt"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(find(17, true, &j).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn joliet_names_stay_unique_and_keep_their_extension() {
        let long = "x".repeat(70);
        let (n1, n2) = (format!("{long}_001.pdf"), format!("{long}_002.pdf"));
        let names = vec![n1.as_str(), n2.as_str(), "a:b.pdf", "a_b.pdf", "A_B.PDF"];
        let out = joliet_names(&names);
        let text: Vec<String> = out
            .iter()
            .map(|b| {
                let u: Vec<u16> = b
                    .chunks(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16(&u).unwrap()
            })
            .collect();
        let lower: std::collections::HashSet<_> = text.iter().map(|t| t.to_lowercase()).collect();
        assert_eq!(lower.len(), names.len(), "{text:?}");
        assert!(text.iter().all(|t| t.encode_utf16().count() <= 64));
        assert!(
            text.iter()
                .all(|t| t.ends_with(".pdf") || t.ends_with(".PDF")),
            "{text:?}"
        );
    }
}
