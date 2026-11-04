//! Reprocessable archive package (roadmap 5.6): one folder that keeps what is needed to run
//! the work again when the engines improve, and what the run produced, with hashes:
//!
//! - `originals/`  the input files, byte for byte (the source of truth);
//! - `project.lgr` the settings and per-page adjustments (rotation, split, skew, content box,
//!   picture and erase zones, dewarp guides) with paths relative to the folder;
//! - `derived/`    the outputs of this run, including `ocr.json` (every word with its box,
//!   confidence and line) so a later run can be compared word by word;
//! - `RUN.txt`     program version, time and the command line to run again;
//! - `SHA256SUMS.txt` over all of it (check with `--verify-sums`).
//!
//! Limit: the per-word provenance is what the OCR engine reported (text, box, confidence);
//! which engine version produced it is recorded once for the run, not per word.

use crate::checksums;
use crate::project::Project;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Built {
    pub files: usize,
    pub originals: usize,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if e.file_type()?.is_dir() {
            walk(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

/// Create `dir/originals` and `dir/derived` before the run (fails when `dir` exists).
pub fn prepare(dir: &Path) -> Result<()> {
    if dir.exists() {
        bail!(
            "{} already exists; --archive-package never overwrites",
            dir.display()
        );
    }
    std::fs::create_dir_all(dir.join("originals"))?;
    std::fs::create_dir_all(dir.join("derived"))?;
    Ok(())
}

/// Fill `dir` after the run. `project.sheets` are the inputs with their adjustments; `derived`
/// are the output files (those already inside `dir` are left where they are).
pub fn finish(
    dir: &Path,
    mut project: Project,
    derived: &[PathBuf],
    when: u64,
    command: &str,
) -> Result<Built> {
    let dir = std::fs::canonicalize(dir)?;
    let mut seen: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut originals = 0;
    for (n, (path, _)) in project.sheets.iter_mut().enumerate() {
        let src = std::fs::canonicalize(&*path)
            .with_context(|| format!("{}: not found", path.display()))?;
        if let Some((_, to)) = seen.iter().find(|(s, _)| *s == src) {
            *path = to.clone();
            continue;
        }
        let name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "input".into());
        let to = dir.join("originals").join(format!("{:03}_{name}", n + 1));
        std::fs::copy(&src, &to).with_context(|| format!("copying {}", src.display()))?;
        originals += 1;
        seen.push((src, to.clone()));
        *path = to;
    }
    for f in derived {
        let f = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        if f.starts_with(&dir) || !f.is_file() {
            continue;
        }
        let name = f.file_name().map(|s| s.to_os_string()).unwrap_or_default();
        std::fs::copy(&f, dir.join("derived").join(name))
            .with_context(|| format!("copying {}", f.display()))?;
    }
    let mut text = project.to_text(Some(&dir));
    if cfg!(windows) {
        // forward slashes work on every system, so a package made on Windows opens elsewhere
        text = text
            .lines()
            .map(|l| {
                if l.starts_with("path = ") {
                    l.replace('\\', "/")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
    }
    std::fs::write(dir.join("project.lgr"), text)?;
    let run = format!(
        "LegatorOCR archive package\nCreated: {}\nProgram: LegatorOCR {}\n\nRun again with the same or a newer program (paths are relative to this folder):\n\n    cd <this folder>\n    {}\n\nCompare the new ocr.json / PDF with derived/ to see what the newer engine changed.\nCheck that nothing was altered since packaging: legatorocr --verify-sums SHA256SUMS.txt\n",
        crate::package::iso_utc(when),
        env!("CARGO_PKG_VERSION"),
        command
    );
    std::fs::write(dir.join("RUN.txt"), run)?;
    let mut files = Vec::new();
    walk(&dir, &mut files)?;
    files.retain(|f| f.file_name().is_some_and(|n| n != "SHA256SUMS.txt"));
    let sums = checksums::render(&dir, &files)?;
    std::fs::write(dir.join("SHA256SUMS.txt"), sums)?;
    Ok(Built {
        files: files.len() + 1,
        originals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::SheetAdj;

    #[test]
    fn package_keeps_originals_project_and_hashes() {
        let t = std::env::temp_dir().join(format!("lgr_rp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let src = t.join("scan.png");
        std::fs::write(&src, b"original bytes").unwrap();
        let dir = t.join("pkg");
        prepare(&dir).unwrap();
        assert!(prepare(&dir).is_err(), "never overwrites");
        let out = t.join("out.txt");
        std::fs::write(&out, b"derived").unwrap();
        let project = Project {
            settings: Default::default(),
            extra: Vec::new(),
            sheets: vec![
                (src.clone(), SheetAdj::default()),
                (src.clone(), SheetAdj::default()),
            ],
        };
        let b = finish(
            &dir,
            project,
            &[out],
            1_782_864_000,
            "legatorocr --project project.lgr",
        )
        .unwrap();
        assert_eq!(b.originals, 1, "the same input is stored once");
        assert_eq!(
            std::fs::read(dir.join("originals/001_scan.png")).unwrap(),
            b"original bytes"
        );
        let text = std::fs::read_to_string(dir.join("project.lgr")).unwrap();
        assert!(text.contains("path = originals/001_scan.png"), "{text}");
        let v = checksums::verify(&dir.join("SHA256SUMS.txt")).unwrap();
        assert!(v.all_good());
        std::fs::write(dir.join("originals/001_scan.png"), b"changed").unwrap();
        assert!(!checksums::verify(&dir.join("SHA256SUMS.txt"))
            .unwrap()
            .all_good());
        let _ = std::fs::remove_dir_all(&t);
    }
}
