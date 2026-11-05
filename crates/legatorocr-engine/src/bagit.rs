//! BagIt 1.0 packaging (RFC 8493): the files a run wrote are copied into `DIR/data/` with a
//! SHA-256 manifest, so an archive can check later that nothing changed or went missing.
//! `verify` re-checks a bag, including the tag files.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Summary {
    pub files: usize,
    pub bytes: u64,
}

fn civil(when: u64) -> String {
    let z = (when / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn tag_manifest(dir: &Path, names: &[&str]) -> Result<String> {
    let mut s = String::new();
    for n in names {
        s.push_str(&format!(
            "{}  {n}\n",
            crate::checksums::file_hash(&dir.join(n))?
        ));
    }
    Ok(s)
}

/// Build a bag at `dest` (which must not exist or must be empty) from `files`.
pub fn write(
    dest: &Path,
    files: &[PathBuf],
    when: u64,
    description: Option<&str>,
) -> Result<Summary> {
    if dest.exists()
        && std::fs::read_dir(dest)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        bail!(
            "{} exists and is not empty; a bag is written into a new folder",
            dest.display()
        );
    }
    let data = dest.join("data");
    std::fs::create_dir_all(&data).with_context(|| format!("{}: cannot create", data.display()))?;
    let mut manifest = String::new();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for f in files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{}: unusable file name", f.display()))?;
        if !seen.insert(name.to_string()) {
            bail!("two files are named {name}; a bag keeps one flat data folder");
        }
        let target = data.join(name);
        std::fs::copy(f, &target).with_context(|| format!("copying {}", f.display()))?;
        bytes += std::fs::metadata(&target)?.len();
        manifest.push_str(&format!(
            "{}  data/{name}\n",
            crate::checksums::file_hash(&target)?
        ));
    }
    std::fs::write(
        dest.join("bagit.txt"),
        "BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n",
    )?;
    std::fs::write(dest.join("manifest-sha256.txt"), &manifest)?;
    let mut info = format!(
        "Bagging-Date: {}\nBag-Software-Agent: LegatorOCR {}\nPayload-Oxum: {}.{}\n",
        civil(when),
        env!("CARGO_PKG_VERSION"),
        bytes,
        files.len()
    );
    if let Some(d) = description {
        info.push_str(&format!("External-Description: {}\n", d.replace('\n', " ")));
    }
    std::fs::write(dest.join("bag-info.txt"), info)?;
    let tags = tag_manifest(dest, &["bagit.txt", "bag-info.txt", "manifest-sha256.txt"])?;
    std::fs::write(dest.join("tagmanifest-sha256.txt"), tags)?;
    Ok(Summary {
        files: files.len(),
        bytes,
    })
}

fn check_manifest(dir: &Path, manifest: &str, problems: &mut Vec<String>) -> usize {
    let mut n = 0;
    for l in manifest.lines().filter(|l| !l.trim().is_empty()) {
        n += 1;
        let Some((hash, name)) = l.split_once("  ") else {
            problems.push(format!("unreadable manifest line: {l}"));
            continue;
        };
        if name.contains("..") || Path::new(name).is_absolute() {
            problems.push(format!("{name}: path leaves the bag"));
            continue;
        }
        match crate::checksums::file_hash(&dir.join(name)) {
            Ok(h) if h.eq_ignore_ascii_case(hash) => {}
            Ok(_) => problems.push(format!("{name}: content changed")),
            Err(_) => problems.push(format!("{name}: missing")),
        }
    }
    n
}

/// Problems found in the bag at `dir` (empty = intact). Also notices files in `data/` that
/// the manifest does not list.
pub fn verify(dir: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let v = std::fs::read_to_string(dir.join("bagit.txt"))
        .context("not a bag: bagit.txt is missing")?;
    if !v.starts_with("BagIt-Version:") {
        problems.push("bagit.txt is not a BagIt declaration".into());
    }
    let manifest = std::fs::read_to_string(dir.join("manifest-sha256.txt"))
        .context("manifest-sha256.txt is missing")?;
    let listed = check_manifest(dir, &manifest, &mut problems);
    if let Ok(tags) = std::fs::read_to_string(dir.join("tagmanifest-sha256.txt")) {
        check_manifest(dir, &tags, &mut problems);
    } else {
        problems.push("tagmanifest-sha256.txt is missing".into());
    }
    let on_disk = std::fs::read_dir(dir.join("data"))
        .map(|d| d.flatten().filter(|e| e.path().is_file()).count())
        .unwrap_or(0);
    if on_disk > listed {
        problems.push(format!(
            "data/ holds {} file(s) the manifest does not list",
            on_disk - listed
        ));
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bag_verifies_and_tampering_is_found() {
        let root = std::env::temp_dir().join(format!("lgr_bag_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.pdf");
        let b = root.join("b.txt");
        std::fs::write(&a, b"%PDF-1.4 fake").unwrap();
        std::fs::write(&b, "text").unwrap();
        let bag = root.join("bag");
        let s = write(&bag, &[a.clone(), b.clone()], 1_793_541_789, Some("test")).unwrap();
        assert_eq!((s.files, s.bytes), (2, 17));
        assert!(std::fs::read_to_string(bag.join("bag-info.txt"))
            .unwrap()
            .contains("Bagging-Date: 2026-11-01"));
        assert!(std::fs::read_to_string(bag.join("bag-info.txt"))
            .unwrap()
            .contains("Payload-Oxum: 17.2"));
        assert!(verify(&bag).unwrap().is_empty());
        assert!(
            write(&bag, std::slice::from_ref(&a), 0, None).is_err(),
            "no writing into a used folder"
        );
        std::fs::write(bag.join("data/b.txt"), "TEXT").unwrap();
        std::fs::write(bag.join("data/extra.txt"), "x").unwrap();
        let p = verify(&bag).unwrap();
        assert!(
            p.iter().any(|x| x.contains("b.txt: content changed")),
            "{p:?}"
        );
        assert!(p.iter().any(|x| x.contains("does not list")), "{p:?}");
        std::fs::remove_file(bag.join("data/a.pdf")).unwrap();
        assert!(verify(&bag)
            .unwrap()
            .iter()
            .any(|x| x.contains("a.pdf: missing")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
