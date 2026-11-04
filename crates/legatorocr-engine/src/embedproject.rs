//! The PDF as a reversible project (`--embed-project`, `--extract-project`).
//!
//! The output PDF carries, as embedded files (`/EmbeddedFiles`, `/AFRelationship /Source`, so
//! it also fits PDF/A-3), the original input files byte for byte (not recompressed) and
//! `project.lgr`: the same text the project file saves (global settings, per-sheet rotation,
//! split, skew, content box, erase / picture zones, dewarp guides, and the resolution the run
//! used for each sheet). `--extract-project PDF DIR` writes them out; `DIR/project.lgr` then
//! reloads with `--project` and the processing runs again from the originals.
//!
//! Limits: the PDF holds the originals, so it is as large as plain PDF + originals; OCR words are
//! not stored (run OCR again); output options (`--pdf-quality`, `--skip-blank`,
//! `--uniform-size`) travel in the project, other output options do not.

use crate::project::{Project, SheetAdj};
use anyhow::{bail, Context, Result};
use legatorocr_core::Settings;
use legatorocr_pdf::Attachment;
use std::path::{Path, PathBuf};

pub const PROJECT_NAME: &str = "project.lgr";
/// More than this many bytes of originals is refused: the whole set is held in memory.
pub const MAX_ORIGINALS: u64 = 1 << 30;

/// What was embedded, for the size report.
#[derive(Debug, Clone, Default)]
pub struct Embedded {
    pub files: usize,
    pub original_bytes: u64,
    pub project_bytes: u64,
}

fn safe_name(n: usize, path: &Path) -> String {
    let raw = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "input".into());
    let clean: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    format!("{n:03}_{}", clean.trim_start_matches('.'))
}

fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Build the attachments for a run: every distinct input file once, then `project.lgr`.
/// `sheets` are the inputs (`file.pdf#N` for a PDF page) with their adjustments.
pub fn build(
    settings: &Settings,
    extra: Vec<(String, String)>,
    sheets: &[(PathBuf, SheetAdj)],
) -> Result<(Vec<Attachment>, Embedded)> {
    let mut files: Vec<(PathBuf, String)> = Vec::new(); // canonical source, embedded name
    let mut project = Project { settings: settings.clone(), extra, sheets: Vec::new() };
    let mut total = 0u64;
    let mut atts = Vec::new();
    for (path, adj) in sheets {
        let (base, frame) = crate::files::split_frame(path);
        let src = std::fs::canonicalize(&base).with_context(|| format!("{}: not found", base.display()))?;
        let name = match files.iter().find(|(s, _)| *s == src) {
            Some((_, n)) => n.clone(),
            None => {
                let name = safe_name(files.len() + 1, &src);
                let len = std::fs::metadata(&src)?.len();
                total += len;
                if total > MAX_ORIGINALS {
                    bail!("--embed-project: the originals are larger than 1 GiB; embed fewer pages");
                }
                let data = std::fs::read(&src).with_context(|| format!("reading {}", src.display()))?;
                atts.push(Attachment {
                    mime: mime_of(&name).into(),
                    description: format!("LegatorOCR original input {} (byte for byte)", files.len() + 1),
                    deflate: name.to_ascii_lowercase().ends_with(".bmp"),
                    data,
                    name: name.clone(),
                });
                files.push((src, name.clone()));
                name
            }
        };
        let shown = match frame {
            Some(n) => format!("{name}#{n}"),
            None => name,
        };
        project.sheets.push((PathBuf::from(shown), adj.clone()));
    }
    let text = project.to_text(None);
    let project_bytes = text.len() as u64;
    atts.push(Attachment {
        name: PROJECT_NAME.into(),
        mime: "text/plain".into(),
        description: "LegatorOCR project: settings and per-sheet adjustments".into(),
        data: text.into_bytes(),
        deflate: true,
    });
    let info = Embedded { files: files.len(), original_bytes: total, project_bytes };
    Ok((atts, info))
}

/// Write the embedded files of `pdf` into the new or empty folder `dir`; returns the project path.
/// Never overwrites. Fails when the PDF has no `project.lgr` or the project lists a missing file.
pub fn extract(pdf: &Path, dir: &Path) -> Result<(PathBuf, usize)> {
    let doc = legatorocr_pdf::reader::PdfDoc::open_file_with_password(pdf, b"")
        .map_err(|e| anyhow::anyhow!("{}: {e}", pdf.display()))?;
    let found = doc.embedded_files().map_err(|e| anyhow::anyhow!("{}: {e}", pdf.display()))?;
    if !found.iter().any(|(n, _)| n == PROJECT_NAME) {
        bail!("{}: no embedded {PROJECT_NAME} (made without --embed-project?)", pdf.display());
    }
    for (n, _) in &found {
        if n.is_empty() || n.contains(['/', '\\', '\0']) || n.starts_with('.') {
            bail!("{}: embedded file name '{n}' is not a plain file name; refusing to write it", pdf.display());
        }
    }
    std::fs::create_dir_all(dir)?;
    if std::fs::read_dir(dir)?.next().is_some() {
        bail!("{} is not empty; --extract-project never overwrites", dir.display());
    }
    for (n, data) in &found {
        std::fs::write(dir.join(n), data).with_context(|| format!("writing {n}"))?;
    }
    let project_path = dir.join(PROJECT_NAME);
    let p = Project::load(&project_path)?;
    for (s, _) in &p.sheets {
        let (base, _) = crate::files::split_frame(s);
        if !base.is_file() {
            bail!("project lists {} but the PDF does not hold it", s.display());
        }
    }
    Ok((project_path, found.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use legatorocr_pdf::{Compress, Meta, PdfWriter};

    #[test]
    fn names_are_plain_and_numbered() {
        assert_eq!(safe_name(1, Path::new("/a/b/Pagină 1.png")), "001_Pagin__1.png");
        assert_eq!(safe_name(12, Path::new("/x/.hidden")), "012_hidden");
        assert_eq!(mime_of("001_a.JPG"), "image/jpeg");
    }

    #[test]
    fn build_embeds_each_file_once_and_the_project_last() {
        let dir = std::env::temp_dir().join(format!("lgr_embed_b_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        std::fs::write(&a, b"not really a png").unwrap();
        let sheets = vec![
            (a.clone(), SheetAdj { rotate_quarters: 1, dpi: Some(150), ..Default::default() }),
            (a.clone(), SheetAdj::default()),
        ];
        let (atts, info) = build(&Settings::default(), vec![("pdf_quality".into(), "70".into())], &sheets).unwrap();
        assert_eq!(atts.len(), 2);
        assert_eq!(atts[0].data, b"not really a png");
        assert_eq!(atts[1].name, PROJECT_NAME);
        assert_eq!(info.files, 1);
        let text = String::from_utf8(atts[1].data.clone()).unwrap();
        let p = Project::parse(&text, None).unwrap();
        assert_eq!(p.sheets.len(), 2);
        assert_eq!(p.sheets[0].0, PathBuf::from("001_a.png"));
        assert_eq!(p.sheets[0].1.dpi, Some(150));
        assert_eq!(p.get("pdf_quality"), Some("70"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pdf_round_trip_returns_the_bytes_and_refuses_to_overwrite() {
        let dir = std::env::temp_dir().join(format!("lgr_embed_x_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("scan.png");
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(&a, &bytes).unwrap();
        let (atts, _) = build(&Settings::default(), Vec::new(), &[(a, SheetAdj::default())]).unwrap();
        let mut w = PdfWriter::new(Compress::Flate);
        w.set_pdfa(true);
        w.set_pdfa_part(3);
        w.set_meta(Meta { attachments: atts, created_unix: Some(0), ..Meta::default() });
        w.add_page(
            &legatorocr_core::Page {
                image: legatorocr_core::PageImage::Bw(legatorocr_core::BitImage::new(40, 40)),
                dpi: 300,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                content_box: None,
                layers: None,
            },
            &[],
        );
        let pdf = w.finish();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/AFRelationship /Source") && s.contains("/EmbeddedFiles") && s.contains("/AF ["));
        let f = dir.join("out.pdf");
        std::fs::write(&f, &pdf).unwrap();
        let out = dir.join("x");
        let (proj, n) = extract(&f, &out).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read(out.join("001_scan.png")).unwrap(), bytes);
        assert_eq!(Project::load(&proj).unwrap().sheets.len(), 1);
        assert!(extract(&f, &out).is_err(), "must not write into a folder that has files");
        std::fs::remove_dir_all(&dir).ok();
    }
}
