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
    /// Text at the top centre of every page; `{n}`, `{total}`, `{date}`, `{file}` are replaced.
    pub header: Option<String>,
    /// Text at the bottom left of every page; same fields.
    pub footer: Option<String>,
    /// Bates number at the bottom right: `PREFIX[:START[:DIGITS]]`, e.g. `ACME-:1:6` gives
    /// ACME-000001, ACME-000002, … continuing across the files of one run.
    pub bates: Option<String>,
}

/// `PREFIX[:START[:DIGITS]]` → (prefix, first number, digits).
pub fn parse_bates(spec: &str) -> Result<(String, u64, usize)> {
    let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
    let (prefix, start, digits) = match parts.as_slice() {
        [d, s, p] => (*p, *s, *d),
        [s, p] => (*p, *s, "6"),
        _ => (spec, "1", "6"),
    };
    // `ACME-` alone has one part; `ACME-:5` has two; a lone number is a prefix-less start
    let start: u64 = start
        .parse()
        .map_err(|_| anyhow::anyhow!("--bates: START must be a number in `{spec}`"))?;
    let digits: usize = digits
        .parse()
        .ok()
        .filter(|d| (1..=12).contains(d))
        .ok_or_else(|| anyhow::anyhow!("--bates: DIGITS must be 1 to 12 in `{spec}`"))?;
    Ok((prefix.to_string(), start, digits))
}

impl Plan {
    pub fn active(&self) -> bool {
        self.merge
            || self.pages.is_some()
            || self.split_every.is_some()
            || self.watermark.is_some()
            || self.number_format.is_some()
            || self.header.is_some()
            || self.footer.is_some()
            || self.bates.is_some()
    }
}

fn fields(t: &str, n: usize, total: usize, file: &str) -> String {
    t.replace("{n}", &n.to_string())
        .replace("{total}", &total.to_string())
        .replace("{date}", &crate::naming::today_iso())
        .replace("{file}", file)
}

/// `bates_no`: the Bates number of this page (already counted across the run).
fn stamps(plan: &Plan, i: usize, total: usize, file: &str, bates_no: u64) -> Vec<Stamp> {
    let mut v = Vec::new();
    let plain = |text: String, anchor: Anchor| Stamp {
        text,
        size: 10.0,
        anchor,
        gray: 0.0,
        angle: 0.0,
        opacity: 1.0,
    };
    if let Some(h) = &plan.header {
        v.push(plain(fields(h, i + 1, total, file), Anchor::TopCenter));
    }
    if let Some(f) = &plan.footer {
        v.push(plain(fields(f, i + 1, total, file), Anchor::BottomLeft));
    }
    if let Some(b) = &plan.bates {
        if let Ok((prefix, _, digits)) = parse_bates(b) {
            v.push(plain(
                format!("{prefix}{bates_no:0digits$}"),
                Anchor::BottomRight,
            ));
        }
    }
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
    let mut bates_next = match &plan.bates {
        Some(b) => parse_bates(b)?.1,
        None => 0,
    };
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
            let first_bates = bates_next;
            bates_next += chunk.len() as u64;
            let bytes = assemble(&refs, &|i, t| {
                stamps(plan, i, t, &stem, first_bates + i as u64)
            })
            .map_err(|e| anyhow::anyhow!(e.0))?;
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
            "f",
            0,
        );
        assert_eq!(s[0].text, "3/9");
    }

    #[test]
    fn bates_specs_and_header_fields() {
        assert_eq!(parse_bates("ACME-:7:4").unwrap(), ("ACME-".into(), 7, 4));
        assert_eq!(parse_bates("ACME-:7").unwrap(), ("ACME-".into(), 7, 6));
        assert_eq!(parse_bates("ACME-").unwrap(), ("ACME-".into(), 1, 6));
        assert!(parse_bates("A:x").is_err());
        assert!(parse_bates("A:1:0").is_err());
        let plan = Plan {
            header: Some("{file} {n}/{total}".into()),
            footer: Some("© {date}".into()),
            bates: Some("B-:41:3".into()),
            ..Plan::default()
        };
        let s = stamps(&plan, 1, 5, "doc", 42);
        assert_eq!(s[0].text, "doc 2/5");
        assert!(s[1].text.starts_with("© 20") && s[1].text.chars().count() == 2 + 10);
        assert_eq!(s[2].text, "B-042");
        assert!(s.iter().all(|x| x.size == 10.0));
    }
}
