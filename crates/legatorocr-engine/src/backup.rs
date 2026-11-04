//! Multi-destination backup (the "3-2-1" helper, roadmap 5.8): copy a folder to several
//! destinations, read every copy back and compare its SHA-256 with the source, and write a
//! `SHA256SUMS.txt` next to the copies. Optionally mark the copies read-only.
//!
//! It checks bytes, not media: that two destinations are on different disks, or that one is off
//! site, is for the person to arrange; `checklist` prints the questions.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Report {
    pub dest: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub problems: Vec<String>,
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut ents: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    ents.sort_by_key(|e| e.file_name());
    for e in ents {
        let p = e.path();
        let ft = e.file_type()?;
        if ft.is_symlink() {
            continue; // never follow links out of the source
        }
        if ft.is_dir() {
            walk(&p, base, out)?;
        } else if ft.is_file() {
            out.push(p.strip_prefix(base).unwrap_or(&p).to_path_buf());
        }
    }
    Ok(())
}

pub fn run(src: &Path, dests: &[PathBuf], read_only: bool) -> Result<Vec<Report>> {
    if !src.is_dir() {
        bail!("{} is not a folder", src.display());
    }
    if dests.is_empty() {
        bail!("give at least one --backup-to folder");
    }
    let canon_src = src.canonicalize()?;
    let mut files = Vec::new();
    walk(src, src, &mut files)?;
    if files.is_empty() {
        bail!("{} has no files", src.display());
    }
    // hash the source once
    let mut sums: Vec<(PathBuf, String, u64)> = Vec::new();
    for f in &files {
        let p = src.join(f);
        let h =
            crate::checksums::file_hash(&p).with_context(|| format!("reading {}", p.display()))?;
        sums.push((f.clone(), h, std::fs::metadata(&p)?.len()));
    }
    let manifest: String = sums
        .iter()
        .map(|(f, h, _)| format!("{h}  {}\n", f.to_string_lossy().replace('\\', "/")))
        .collect();
    let mut reports = Vec::new();
    for d in dests {
        std::fs::create_dir_all(d)?;
        if d.canonicalize()? == canon_src || d.canonicalize()?.starts_with(&canon_src) {
            bail!("{} is inside the source folder", d.display());
        }
        let mut rep = Report {
            dest: d.clone(),
            files: 0,
            bytes: 0,
            problems: Vec::new(),
        };
        for (f, h, len) in &sums {
            let to = d.join(f);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(e) = std::fs::copy(src.join(f), &to) {
                rep.problems
                    .push(format!("{}: copy failed: {e}", f.display()));
                continue;
            }
            // read back from the destination, do not trust the copy call
            match crate::checksums::file_hash(&to) {
                Ok(got) if &got == h => {
                    rep.files += 1;
                    rep.bytes += len;
                }
                Ok(_) => rep
                    .problems
                    .push(format!("{}: copy differs from the source", f.display())),
                Err(e) => rep
                    .problems
                    .push(format!("{}: cannot read back: {e}", f.display())),
            }
            if read_only {
                if let Ok(m) = std::fs::metadata(&to) {
                    let mut perm = m.permissions();
                    perm.set_readonly(true);
                    let _ = std::fs::set_permissions(&to, perm);
                }
            }
        }
        std::fs::write(d.join("SHA256SUMS.txt"), &manifest)?;
        reports.push(rep);
    }
    Ok(reports)
}

/// Questions the program cannot answer for the person.
pub fn checklist(copies: usize) -> String {
    let mut s = String::from("3-2-1 check (the program verified the bytes, not the setup):\n");
    s += &format!(
        "  [{}] at least 3 copies in total (original + {copies} backup folder(s))\n",
        if copies >= 2 { "x" } else { " " }
    );
    s += "  [ ] the copies are on at least 2 different kinds of media / disks\n";
    s += "  [ ] at least 1 copy is somewhere else (another building or a provider)\n";
    s += "  [ ] a cold copy is kept read-only or offline (use --backup-read-only)\n";
    s += "  [ ] SHA256SUMS.txt is checked again from time to time (sha256sum -c)\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_are_verified_and_listed_in_the_manifest() {
        let base = std::env::temp_dir().join(format!("lgr_bk_{}", std::process::id()));
        let src = base.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"alpha").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"beta").unwrap();
        let (d1, d2) = (base.join("d1"), base.join("d2"));
        let r = run(&src, &[d1.clone(), d2.clone()], true).unwrap();
        assert!(r
            .iter()
            .all(|x| x.problems.is_empty() && x.files == 2 && x.bytes == 9));
        assert_eq!(std::fs::read(d2.join("sub/b.txt")).unwrap(), b"beta");
        let sums = std::fs::read_to_string(d1.join("SHA256SUMS.txt")).unwrap();
        assert!(sums.contains("  sub/b.txt") && sums.lines().count() == 2);
        assert!(std::fs::metadata(d1.join("a.txt"))
            .unwrap()
            .permissions()
            .readonly());
        // a destination inside the source is refused
        assert!(run(&src, &[src.join("inner")], false).is_err());
        assert!(run(&src, &[], false).is_err());
        for d in [&d1, &d2] {
            for f in ["a.txt", "sub/b.txt"] {
                let mut p = std::fs::metadata(d.join(f)).unwrap().permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                p.set_readonly(false);
                let _ = std::fs::set_permissions(d.join(f), p);
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
