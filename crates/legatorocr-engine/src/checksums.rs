//! `SHA256SUMS` files in the format of `sha256sum` / `shasum -a 256`
//! (`<64 hex digits>  <file name>`), so they can be checked with standard tools too.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn sha256_hex(data: &[u8]) -> String {
    legatorocr_pdf::crypt::sha256(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Hash of a file read in full.
pub fn file_hash(p: &Path) -> Result<String> {
    let d = std::fs::read(p).with_context(|| format!("{}: cannot read", p.display()))?;
    Ok(sha256_hex(&d))
}

/// One line per file, names relative to `base` (file name only when it is not below it).
pub fn render(base: &Path, files: &[PathBuf]) -> Result<String> {
    let mut out = String::new();
    for f in files {
        let name = f
            .strip_prefix(base)
            .ok()
            .or_else(|| f.file_name().map(Path::new))
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        out.push_str(&format!("{}  {name}\n", file_hash(f)?));
    }
    Ok(out)
}

/// Result of checking a sums file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verification {
    pub ok: usize,
    pub mismatched: Vec<String>,
    pub missing: Vec<String>,
}

impl Verification {
    pub fn all_good(&self) -> bool {
        self.mismatched.is_empty() && self.missing.is_empty()
    }
}

/// Check every entry of `sums` (paths are relative to the sums file's folder).
pub fn verify(sums: &Path) -> Result<Verification> {
    let text = std::fs::read_to_string(sums)
        .with_context(|| format!("{}: cannot read", sums.display()))?;
    let dir = sums.parent().unwrap_or_else(|| Path::new("."));
    let mut v = Verification::default();
    let mut lines = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some((hash, name)) = line.split_once("  ").or_else(|| line.split_once(" *")) else {
            bail!("{}: malformed line: {line:?}", sums.display());
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("{}: not a SHA-256 digest: {hash:?}", sums.display());
        }
        // never follow a name out of the folder
        let n = Path::new(name);
        // Rejected on every platform alike: a leading slash is not "absolute" on Windows, and a
        // drive letter or `\\server` prefix is not one on Unix, so the text is checked as well.
        let unsafe_text = name.starts_with(['/', '\\']) || name.contains(':');
        let unsafe_path = n.is_absolute()
            || n.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            });
        if unsafe_text || unsafe_path {
            bail!("{}: unsafe path {name:?}", sums.display());
        }
        lines += 1;
        match std::fs::read(dir.join(n)) {
            Ok(d) if sha256_hex(&d).eq_ignore_ascii_case(hash) => v.ok += 1,
            Ok(_) => v.mismatched.push(name.to_string()),
            Err(_) => v.missing.push(name.to_string()),
        }
    }
    if lines == 0 {
        bail!("{}: no entries", sums.display());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digest_and_roundtrip() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let d = std::env::temp_dir().join(format!("lgr-sums-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.pdf"), b"one").unwrap();
        std::fs::write(d.join("sub/b.txt"), b"two").unwrap();
        let sums = render(&d, &[d.join("a.pdf"), d.join("sub/b.txt")]).unwrap();
        std::fs::write(d.join("SHA256SUMS.txt"), &sums).unwrap();
        assert!(sums.contains("  sub/b.txt\n"));
        let v = verify(&d.join("SHA256SUMS.txt")).unwrap();
        assert!(v.all_good() && v.ok == 2);
        std::fs::write(d.join("a.pdf"), b"changed").unwrap();
        std::fs::remove_file(d.join("sub/b.txt")).unwrap();
        let v = verify(&d.join("SHA256SUMS.txt")).unwrap();
        assert_eq!(
            (v.mismatched, v.missing),
            (vec!["a.pdf".into()], vec!["sub/b.txt".into()])
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_sums_files_are_rejected() {
        let d = std::env::temp_dir().join(format!("lgr-sums2-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let h = "0".repeat(64);
        for bad in [
            format!("{h}  ../etc/passwd\n"),
            format!("{h}  /etc/passwd\n"),
            format!("{h}  C:\\Windows\\win.ini\n"),
            format!("{h}  \\\\server\\share\\x\n"),
            "xyz  a\n".to_string(),
            String::new(),
        ] {
            std::fs::write(d.join("S.txt"), bad).unwrap();
            assert!(verify(&d.join("S.txt")).is_err());
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
