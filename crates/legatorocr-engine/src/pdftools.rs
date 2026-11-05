//! PDF toolbox on existing files (roadmap 3.12): merge, extract pages, split, watermark,
//! page numbers. Works on page objects, nothing is rasterised or re-compressed.

use anyhow::{bail, Context, Result};
use legatorocr_pdf::reader::edit::{assemble, parse_ranges, Anchor, Stamp};
use legatorocr_pdf::reader::PdfDoc;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// All inputs into one file.
    pub merge: bool,
    /// Pages kept from each input, e.g. `1-3,7` (default: all).
    pub pages: Option<String>,
    /// Cut each (merged) result into files of this many pages.
    pub split_every: Option<usize>,
    pub watermark: Option<String>,
    /// Page number format, `{n}` and `{total}` replaced.
    pub number_format: Option<String>,
}

impl Plan {
    pub fn active(&self) -> bool {
        self.merge
            || self.pages.is_some()
            || self.split_every.is_some()
            || self.watermark.is_some()
            || self.number_format.is_some()
    }
}

fn stamps(plan: &Plan, i: usize, total: usize) -> Vec<Stamp> {
    let mut v = Vec::new();
    if let Some(w) = &plan.watermark {
        v.push(Stamp {
            text: w.clone(),
            size: 72.0,
            anchor: Anchor::Center,
            gray: 0.6,
            angle: 45.0,
            opacity: 0.3,
        });
    }
    if let Some(f) = &plan.number_format {
        v.push(Stamp {
            text: f
                .replace("{n}", &(i + 1).to_string())
                .replace("{total}", &total.to_string()),
            size: 10.0,
            anchor: Anchor::BottomCenter,
            gray: 0.0,
            angle: 0.0,
            opacity: 1.0,
        });
    }
    v
}

/// Run the plan; returns the files written.
pub fn run(
    inputs: &[PathBuf],
    out_dir: &Path,
    merged_name: Option<&Path>,
    plan: &Plan,
) -> Result<Vec<PathBuf>> {
    if let Some(0) = plan.split_every {
        bail!("--split-pdf-every needs a number above 0");
    }
    let docs: Vec<(PathBuf, PdfDoc)> = inputs
        .iter()
        .map(|p| {
            let data = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
            let d = PdfDoc::open(data).map_err(|e| anyhow::anyhow!("{}: {}", p.display(), e.0))?;
            Ok((p.clone(), d))
        })
        .collect::<Result<_>>()?;
    let pick = |d: &PdfDoc| -> Result<Vec<usize>> {
        match &plan.pages {
            Some(r) => parse_ranges(r, d.page_count()).map_err(|e| anyhow::anyhow!(e)),
            None => Ok((0..d.page_count()).collect()),
        }
    };
    // groups: (output stem, parts)
    type Job = (String, Vec<(usize, Vec<usize>)>);
    let mut jobs: Vec<Job> = Vec::new();
    if plan.merge {
        let stem = merged_name
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "merged".into());
        let mut parts = Vec::new();
        for (i, (_, d)) in docs.iter().enumerate() {
            parts.push((i, pick(d)?));
        }
        jobs.push((stem, parts));
    } else {
        for (i, (p, d)) in docs.iter().enumerate() {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "out".into());
            jobs.push((stem, vec![(i, pick(d)?)]));
        }
    }
    std::fs::create_dir_all(out_dir)?;
    let mut written = Vec::new();
    for (stem, parts) in jobs {
        // flat list of (doc, page)
        let flat: Vec<(usize, usize)> = parts
            .iter()
            .flat_map(|(d, v)| v.iter().map(move |&p| (*d, p)))
            .collect();
        let chunks: Vec<&[(usize, usize)]> = match plan.split_every {
            Some(n) => flat.chunks(n).collect(),
            None => vec![&flat[..]],
        };
        let many = chunks.len() > 1;
        for (ci, chunk) in chunks.iter().enumerate() {
            // regroup consecutive pages of the same document
            let mut grouped: Vec<(usize, Vec<usize>)> = Vec::new();
            for &(d, p) in chunk.iter() {
                match grouped.last_mut() {
                    Some((ld, v)) if *ld == d => v.push(p),
                    _ => grouped.push((d, vec![p])),
                }
            }
            let refs: Vec<(&PdfDoc, Vec<usize>)> = grouped
                .iter()
                .map(|(d, v)| (&docs[*d].1, v.clone()))
                .collect();
            let bytes =
                assemble(&refs, &|i, t| stamps(plan, i, t)).map_err(|e| anyhow::anyhow!(e.0))?;
            let name = if many {
                format!("{stem}_{:03}.pdf", ci + 1)
            } else {
                format!("{stem}.pdf")
            };
            let path = match (merged_name, plan.merge, many) {
                (Some(p), true, false) => p.to_path_buf(),
                _ => out_dir.join(name),
            };
            if inputs.iter().any(|i| i == &path) {
                bail!(
                    "{} would overwrite an input; choose another output folder",
                    path.display()
                );
            }
            std::fs::write(&path, bytes)?;
            written.push(path);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_flags() {
        assert!(!Plan::default().active());
        assert!(Plan {
            watermark: Some("x".into()),
            ..Plan::default()
        }
        .active());
        let s = stamps(
            &Plan {
                number_format: Some("{n}/{total}".into()),
                ..Plan::default()
            },
            2,
            9,
        );
        assert_eq!(s[0].text, "3/9");
    }
}
