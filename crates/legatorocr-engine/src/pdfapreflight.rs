//! `--pdfa-check FILE`: a quick PDF/A preflight, and veraPDF when it is installed.
//!
//! The built-in part reads the bytes of the file (not the object streams, so it is meant for the
//! files this program writes and for other classic-structure PDFs) and looks for the things that
//! most often make a file fail PDF/A: no PDF/A identification in the XMP packet, no output
//! intent, encryption, JavaScript or launch actions, LZW compression, a font without an embedded
//! font file, a transparency group, a missing file identifier. It is a **subset of the standard,
//! not a validation**: a file that passes here can still fail veraPDF. When a `verapdf`
//! command is found on the PATH (or named with `--verapdf`), it is run too, and its verdict is
//! what counts.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub ok: bool,
    pub what: String,
}

fn has(b: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && b.windows(n.len()).any(|w| w == n)
}

fn count(b: &[u8], needle: &str) -> usize {
    let n = needle.as_bytes();
    if n.is_empty() {
        return 0;
    }
    let (mut i, mut c) = (0, 0);
    while i + n.len() <= b.len() {
        if &b[i..i + n.len()] == n {
            c += 1;
            i += n.len();
        } else {
            i += 1;
        }
    }
    c
}

/// Declared `pdfaid:part` and `pdfaid:conformance`, as written in the XMP packet (element or
/// attribute form).
fn declared(b: &[u8]) -> Option<(String, String)> {
    let s = String::from_utf8_lossy(b);
    let grab = |key: &str| -> Option<String> {
        let e = format!("<pdfaid:{key}>");
        if let Some(i) = s.find(&e) {
            let r = &s[i + e.len()..];
            return r.find('<').map(|j| r[..j].trim().to_string());
        }
        let a = format!("pdfaid:{key}=\"");
        let i = s.find(&a)?;
        let r = &s[i + a.len()..];
        r.find('"').map(|j| r[..j].to_string())
    };
    Some((grab("part")?, grab("conformance")?))
}

/// Built-in checks on the file's bytes.
pub fn check_bytes(b: &[u8]) -> Vec<Finding> {
    let mut v = Vec::new();
    let mut add = |ok: bool, what: String| v.push(Finding { ok, what });
    add(
        b.starts_with(b"%PDF-"),
        "starts with the %PDF- header".into(),
    );
    let decl = declared(b);
    match &decl {
        Some((p, c)) => add(true, format!("XMP declares PDF/A-{p}{}", c.to_lowercase())),
        None => add(
            false,
            "no pdfaid:part / pdfaid:conformance in an XMP packet".into(),
        ),
    }
    if let Some((p, c)) = &decl {
        let c = c.to_ascii_uppercase();
        if c == "U" || c == "A" {
            let type0 = count(b, "/Subtype /Type0") + count(b, "/Subtype/Type0");
            let tou = count(b, "/ToUnicode");
            add(
                tou >= type0,
                format!("every composite font has a ToUnicode map ({tou} of {type0}), as level {c} requires"),
            );
        }
        if c == "U" {
            add(p != "1", "level U exists in PDF/A-2 and -3 only".into());
        }
        if c == "A" {
            add(
                has(b, "/StructTreeRoot") && has(b, "/Marked true"),
                "level A: a structure tree and /MarkInfo /Marked true".into(),
            );
            add(
                has(b, "/Lang ") || has(b, "/Lang("),
                "level A: the document language (/Lang) is set".into(),
            );
        }
    }
    add(
        has(b, "/OutputIntents") && has(b, "/GTS_PDFA1"),
        "an output intent of type GTS_PDFA1".into(),
    );
    add(!has(b, "/Encrypt"), "not encrypted".into());
    add(
        !has(b, "/JavaScript") && !has(b, "/JS ") && !has(b, "/Launch"),
        "no JavaScript or launch action".into(),
    );
    add(!has(b, "/LZWDecode"), "no LZW compression".into());
    let fonts = count(b, "/Type /FontDescriptor") + count(b, "/Type/FontDescriptor");
    let embedded = count(b, "/FontFile");
    add(
        embedded >= fonts,
        format!("every font descriptor has a font file ({embedded} of {fonts})"),
    );
    add(
        !has(b, "/S /Transparency") && !has(b, "/S/Transparency"),
        "no transparency group (PDF/A-1)".into(),
    );
    add(
        has(b, "/ID ["),
        "a file identifier (/ID) in the trailer".into(),
    );
    v
}

/// Result of running veraPDF: `(command, compliant?, one line of its report)`.
pub fn run_verapdf(cmd: &str, file: &Path, flavour: &str) -> Result<(bool, String), String> {
    let out = Command::new(cmd)
        .arg("--format")
        .arg("text")
        .arg("--flavour")
        .arg(flavour)
        .arg(file)
        .output()
        .map_err(|e| format!("cannot run {cmd}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let first = text.lines().next().unwrap_or("").trim().to_string();
    // veraPDF's text format starts with `PASS file ...` or `FAIL file ...`
    Ok((first.starts_with("PASS"), first))
}

/// Is a command of this name runnable (`--version` exits)?
pub fn have_verapdf(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "%PDF-1.7\n1 0 obj << /OutputIntents [<< /S /GTS_PDFA1 >>] >> endobj\n<pdfaid:part>2</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>\n/Type /FontDescriptor /FontFile2 3 0 R\ntrailer << /ID [<aa><aa>] >>";

    #[test]
    fn a_file_with_the_usual_parts_passes() {
        let f = check_bytes(GOOD.as_bytes());
        assert!(f.iter().all(|x| x.ok), "{f:?}");
        assert!(f.iter().any(|x| x.what == "XMP declares PDF/A-2b"));
    }

    #[test]
    fn levels_u_and_a_have_their_own_checks() {
        let u = GOOD.replace(">B<", ">U<");
        let f = check_bytes(format!("{u}\n/Subtype /Type0").as_bytes());
        assert!(
            f.iter().any(|x| !x.ok && x.what.contains("ToUnicode")),
            "{f:?}"
        );
        let f = check_bytes(format!("{u}\n/Subtype /Type0 /ToUnicode 9 0 R").as_bytes());
        assert!(f.iter().all(|x| x.ok), "{f:?}");
        let u1 = u.replace(">2<", ">1<");
        assert!(check_bytes(u1.as_bytes())
            .iter()
            .any(|x| !x.ok && x.what.contains("level U")));
        let a = GOOD.replace(">B<", ">A<");
        let f = check_bytes(a.as_bytes());
        assert!(f.iter().filter(|x| !x.ok).count() >= 2, "{f:?}");
        let f =
            check_bytes(format!("{a}\n/StructTreeRoot 4 0 R /Marked true /Lang (en)").as_bytes());
        assert!(f.iter().all(|x| x.ok), "{f:?}");
    }

    #[test]
    fn each_common_problem_is_reported() {
        for (bad, key) in [
            ("/Encrypt 5 0 R", "not encrypted"),
            ("/JavaScript (x)", "no JavaScript"),
            ("/LZWDecode", "no LZW"),
            ("/Type /FontDescriptor", "font descriptor"),
        ] {
            let f = check_bytes(format!("{GOOD}\n{bad}").as_bytes());
            assert!(
                f.iter().any(|x| !x.ok && x.what.contains(key)),
                "{bad}: {f:?}"
            );
        }
        let f = check_bytes(b"%PDF-1.4\n");
        assert!(f.iter().filter(|x| !x.ok).count() >= 3);
        assert!(!check_bytes(b"hello").first().unwrap().ok);
    }

    #[test]
    fn a_missing_command_is_an_error_not_a_pass() {
        assert!(!have_verapdf("definitely-not-a-real-verapdf"));
        assert!(run_verapdf("definitely-not-a-real-verapdf", Path::new("x.pdf"), "2b").is_err());
    }
}
