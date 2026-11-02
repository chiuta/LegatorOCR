//! `--doctor`: what is installed, what is missing, and how to fix it.
//!
//! Nothing here is required to run (the native OCR engine is built in); each line says what a
//! missing helper costs. Pure reporting: it changes nothing on the machine.

use crate::{extpdf, ocr};
use std::path::Path;

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// What to do when `ok` is false, or what is lost without it.
    pub hint: String,
}

fn tool(name: &str, probe: &[&str], missing_cost: &str) -> Check {
    match extpdf::find_tool(name, probe) {
        Some(p) => Check {
            name: name.to_string(),
            ok: true,
            detail: p.display().to_string(),
            hint: String::new(),
        },
        None => Check {
            name: name.to_string(),
            ok: false,
            detail: "not found".into(),
            hint: missing_cost.to_string(),
        },
    }
}

/// Run every check. `tesseract` is the configured Tesseract path (`--tesseract`).
pub fn run(tesseract: &Path) -> Vec<Check> {
    let mut v = vec![Check {
        name: "legatorocr".into(),
        ok: true,
        detail: env!("CARGO_PKG_VERSION").into(),
        hint: String::new(),
    }];
    v.push(Check {
        name: "native OCR engine".into(),
        ok: ocr::native_available(),
        detail: if ocr::native_available() {
            "built in".into()
        } else {
            "not available".into()
        },
        hint: "built-in engine missing from this build; install Tesseract instead".into(),
    });

    let o = ocr::OcrOptions {
        tesseract: tesseract.to_path_buf(),
        lang: "eng".into(),
        ..ocr::OcrOptions::default()
    };
    match ocr::check_engine(&ocr::OcrOptions { engine: ocr::Engine::Tesseract, ..o.clone() }) {
        Ok(ver) => {
            v.push(Check {
                name: "tesseract".into(),
                ok: true,
                detail: ver.lines().next().unwrap_or("").trim().to_string(),
                hint: String::new(),
            });
            if let Ok(have) = ocr::list_languages(&o) {
                for (iso, tess, name) in ocr::UI_LANGUAGES {
                    let ok = have.iter().any(|h| h == tess);
                    v.push(Check {
                        name: format!("tesseract model {iso} ({name})"),
                        ok,
                        detail: if ok { "installed".into() } else { "missing".into() },
                        hint: if ok { String::new() } else { ocr::install_hint(tess) },
                    });
                }
            }
        }
        Err(_) => v.push(Check {
            name: "tesseract".into(),
            ok: false,
            detail: "not found".into(),
            hint: "optional: only needed for --engine tesseract and automatic page-orientation detection".into(),
        }),
    }

    v.push(tool(
        "pdftoppm",
        &["-v"],
        "optional: renders the PDF/JPEG 2000 cases that are not decoded natively",
    ));
    v.push(tool(
        "mutool",
        &["-v"],
        "optional: alternative renderer for those cases",
    ));
    v.push(tool(
        "gs",
        &["-v"],
        "optional: second alternative renderer (Ghostscript)",
    ));
    v.push(tool(
        "qpdf",
        &["--version"],
        "optional: page counts of damaged or encrypted PDFs",
    ));
    v
}

/// Plain-text report; the last line summarises how many optional helpers are missing.
pub fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let mark = if c.ok { "ok     " } else { "missing" };
        out.push_str(&format!("[{mark}] {:<34} {}\n", c.name, c.detail));
        if !c.ok && !c.hint.is_empty() {
            out.push_str(&format!("          -> {}\n", c.hint));
        }
    }
    let miss = checks.iter().filter(|c| !c.ok).count();
    out.push_str(&format!(
        "{} check(s) ok, {} missing. Missing helpers are optional unless a hint says otherwise.\n",
        checks.len() - miss,
        miss
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_version_and_native_engine_lines() {
        let r = run(Path::new("tesseract-definitely-not-installed"));
        assert_eq!(r[0].name, "legatorocr");
        assert_eq!(r[0].detail, env!("CARGO_PKG_VERSION"));
        assert!(r.iter().any(|c| c.name == "native OCR engine"));
    }

    #[test]
    fn missing_tool_has_a_hint_and_render_mentions_it() {
        let checks = vec![
            Check {
                name: "a".into(),
                ok: true,
                detail: "x".into(),
                hint: String::new(),
            },
            Check {
                name: "b".into(),
                ok: false,
                detail: "not found".into(),
                hint: "install b".into(),
            },
        ];
        let t = render(&checks);
        assert!(t.contains("[ok     ] a"));
        assert!(t.contains("[missing] b"));
        assert!(t.contains("-> install b"));
        assert!(t.contains("1 check(s) ok, 1 missing"));
    }

    #[test]
    fn nonexistent_tesseract_is_reported_not_fatal() {
        let r = run(Path::new("/nonexistent/tesseract"));
        let t = r.iter().find(|c| c.name == "tesseract").unwrap();
        assert!(!t.ok);
    }
}
