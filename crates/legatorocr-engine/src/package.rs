//! A delivery package: the produced files, a `SHA256SUMS.txt` and a human-readable
//! `MANIFEST.txt`, in a new folder, one `.zip` or one `.iso` image (for handing over to an archive or a
//! library). Nothing is overwritten: an existing destination is an error.

use crate::checksums::file_hash;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// ZIP entries here have 32-bit sizes; stay well below 4 GiB.
const MAX_ZIP_BYTES: u64 = 3 * 1024 * 1024 * 1024;

pub struct Packaged {
    pub dest: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

/// ISO 8601 UTC time from a Unix time.
pub fn iso_utc(secs: u64) -> String {
    // `D:YYYYMMDDHHmmSSZ`
    let d = legatorocr_pdf::pdf_date(secs);
    let d = d.trim_start_matches("D:").trim_end_matches('Z');
    if d.len() < 14 {
        return d.to_string();
    }
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &d[0..4],
        &d[4..6],
        &d[6..8],
        &d[8..10],
        &d[10..12],
        &d[12..14]
    )
}

fn manifest_text(names: &[(String, u64, String)], when: u64, note: &str) -> String {
    let mut m = format!(
        "LegatorOCR delivery package\nCreated: {}\nProgram: LegatorOCR {}\nFiles: {}\n",
        iso_utc(when),
        env!("CARGO_PKG_VERSION"),
        names.len()
    );
    if !note.trim().is_empty() {
        m.push_str(&format!("Notes: {}\n", note.trim().replace('\n', " ")));
    }
    m.push_str("\nSHA-256                                                           bytes  file\n");
    for (n, size, h) in names {
        m.push_str(&format!("{h}  {size:>10}  {n}\n"));
    }
    m.push_str("\nVerify with: legatorocr --verify-sums SHA256SUMS.txt   (or: sha256sum -c SHA256SUMS.txt)\n");
    m
}

/// Collect `files` (flat, by file name) plus `SHA256SUMS.txt` and `MANIFEST.txt` into `dest`
/// (`.iso` makes a Joliet ISO 9660 image, `.zip` a zip, anything else a folder). `note` goes into the manifest
/// (for example the rights statement).
pub fn build(dest: &Path, files: &[PathBuf], when: u64, note: &str) -> Result<Packaged> {
    if dest.exists() {
        bail!("{}: already exists, not overwriting", dest.display());
    }
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for f in files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{}: unusable file name", f.display()))?
            .to_string();
        if !seen.insert(name.clone()) {
            bail!("two files are called {name}: cannot package them flat");
        }
        entries.push((name, f.clone()));
    }
    let mut listing = Vec::new();
    let mut sums = String::new();
    let mut total = 0u64;
    for (n, p) in &entries {
        let size = std::fs::metadata(p)
            .with_context(|| format!("{}: cannot read", p.display()))?
            .len();
        let h = file_hash(p)?;
        sums.push_str(&format!("{h}  {n}\n"));
        listing.push((n.clone(), size, h));
        total += size;
    }
    let manifest = manifest_text(&listing, when, note);
    let is_iso = dest
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("iso"));
    let is_zip = dest
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if is_iso {
        use crate::iso::{write_iso, IsoFile, Source};
        let mut list: Vec<IsoFile> = entries
            .iter()
            .map(|(n, p)| IsoFile {
                name: n.clone(),
                source: Source::Path(p.clone()),
            })
            .collect();
        list.push(IsoFile {
            name: "SHA256SUMS.txt".into(),
            source: Source::Bytes(sums.into_bytes()),
        });
        list.push(IsoFile {
            name: "MANIFEST.txt".into(),
            source: Source::Bytes(manifest.into_bytes()),
        });
        let label = dest
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("LEGATOROCR");
        write_iso(dest, label, when, &list)?;
    } else if is_zip {
        if total > MAX_ZIP_BYTES {
            bail!("the files total more than 3 GiB: package into a folder instead of a .zip");
        }
        let mut blobs: Vec<(String, Vec<u8>)> = Vec::new();
        for (n, p) in &entries {
            blobs.push((n.clone(), std::fs::read(p)?));
        }
        blobs.push(("SHA256SUMS.txt".into(), sums.into_bytes()));
        blobs.push(("MANIFEST.txt".into(), manifest.into_bytes()));
        let refs: Vec<(&str, &[u8])> = blobs
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let z = crate::zipmini::write(&refs);
        std::fs::write(dest, &z).with_context(|| format!("{}: cannot write", dest.display()))?;
    } else {
        std::fs::create_dir_all(dest)?;
        for (n, p) in &entries {
            std::fs::copy(p, dest.join(n)).with_context(|| format!("copying {}", p.display()))?;
        }
        std::fs::write(dest.join("SHA256SUMS.txt"), sums)?;
        std::fs::write(dest.join("MANIFEST.txt"), manifest)?;
    }
    Ok(Packaged {
        dest: dest.to_path_buf(),
        files: entries.len() + 2,
        bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_and_zip_packages_verify() {
        let d = std::env::temp_dir().join(format!("lgr-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.pdf"), b"%PDF fake").unwrap();
        std::fs::write(d.join("a.txt"), "text").unwrap();
        let files = [d.join("a.pdf"), d.join("a.txt")];
        let p = build(&d.join("out"), &files, 1_793_000_000, "© X, contract 1").unwrap();
        assert_eq!(p.files, 4);
        let v = crate::checksums::verify(&d.join("out/SHA256SUMS.txt")).unwrap();
        assert!(v.all_good() && v.ok == 2);
        let m = std::fs::read_to_string(d.join("out/MANIFEST.txt")).unwrap();
        assert!(m.contains("a.pdf") && m.contains("contract 1") && m.contains("2026-"));
        // zip
        let z = d.join("out.zip");
        build(&z, &files, 0, "").unwrap();
        let bytes = std::fs::read(&z).unwrap();
        let list = crate::zipmini::list(&bytes).unwrap();
        let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.pdf", "a.txt", "SHA256SUMS.txt", "MANIFEST.txt"]);
        // never overwrites; refuses colliding names
        assert!(build(&z, &files, 0, "").is_err());
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/a.pdf"), b"x").unwrap();
        assert!(build(
            &d.join("o2"),
            &[d.join("a.pdf"), d.join("sub/a.pdf")],
            0,
            ""
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
