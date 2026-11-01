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

/// One line per file, names relative to `base`. A file that is not below `base` has no
/// honest relative name (the sums file would point at something that is not next to it), so
/// it is an error; callers split their list with [`below`] first.
pub fn render(base: &Path, files: &[PathBuf]) -> Result<String> {
    let mut out = String::new();
    for f in files {
        let rel = f
            .strip_prefix(base)
            .with_context(|| format!("{}: is not inside {}", f.display(), base.display()))?;
        let name = rel.to_string_lossy().replace('\\', "/");
        out.push_str(&format!("{}  {name}\n", file_hash(f)?));
    }
    Ok(out)
}

/// The files of `files` that lie below `base`, and those that do not.
pub fn below(base: &Path, files: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    files
        .iter()
        .cloned()
        .partition(|f| f.strip_prefix(base).is_ok())
}

/// Add `new` lines to an `existing` sums file: an entry for the same name is replaced, the other
/// entries stay, so a folder that collects many documents keeps all their hashes.
pub fn merge(existing: &str, new: &str) -> String {
    let name_of = |l: &str| l.split_once("  ").map(|(_, n)| n.to_string());
    let fresh: std::collections::HashSet<String> = new.lines().filter_map(name_of).collect();
    let mut out = String::new();
    for l in existing.lines().filter(|l| !l.trim().is_empty()) {
        if !name_of(l).is_some_and(|n| fresh.contains(&n)) {
            out.push_str(l);
            out.push('\n');
        }
    }
    out.push_str(new);
    out
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
        // (a ':' elsewhere in a name is an ordinary character: "Vol 1: Intro.pdf")
        let drive_letter = name.len() >= 2
            && name.as_bytes()[1] == b':'
            && name.as_bytes()[0].is_ascii_alphabetic();
        let unsafe_text = name.starts_with(['/', '\\']) || drive_letter;
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
            format!("{h}  c:evil.txt\n"),
            format!("{h}  \\\\server\\share\\x\n"),
            "xyz  a\n".to_string(),
            String::new(),
        ] {
            std::fs::write(d.join("S.txt"), bad).unwrap();
            assert!(verify(&d.join("S.txt")).is_err());
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn colons_inside_names_are_fine_and_sums_files_merge() {
        let d = std::env::temp_dir().join(format!("lgr-sums3-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Vol 1: Intro.pdf"), b"x").unwrap();
        let sums = render(&d, &[d.join("Vol 1: Intro.pdf")]).unwrap();
        std::fs::write(d.join("S.txt"), &sums).unwrap();
        assert!(verify(&d.join("S.txt")).unwrap().all_good());
        // a file elsewhere is not given a misleading short name
        let other = std::env::temp_dir().join("elsewhere.txt");
        assert!(render(&d, std::slice::from_ref(&other)).is_err());
        let (inside, outside) = below(&d, &[d.join("a"), other.clone()]);
        assert_eq!((inside.len(), outside.len()), (1, 1));
        // merge keeps old entries and replaces same-named ones
        let h = "0".repeat(64);
        let g = "1".repeat(64);
        let m = merge(
            &format!("{h}  a.pdf\n{h}  b.pdf\n"),
            &format!("{g}  b.pdf\n"),
        );
        assert_eq!(m, format!("{h}  a.pdf\n{g}  b.pdf\n"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
