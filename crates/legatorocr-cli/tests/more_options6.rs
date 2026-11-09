//! PAdES long-term options (needs openssl; skipped without it).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more6_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    image::GrayImage::from_pixel(300, 400, image::Luma([200]))
        .save(d.join("a.png"))
        .unwrap();
    d
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .env("LGR_TEST_PW", "pw")
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn ossl(d: &Path, args: &[&str]) -> bool {
    Command::new("openssl")
        .current_dir(d)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn lt_refusal_then_lt_and_level_report() {
    let d = tmp("lt");
    if !ossl(&d, &["version"]) {
        return;
    }
    assert!(ossl(
        &d,
        &[
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "k.pem", "-out", "c.pem",
            "-subj", "/CN=T", "-days", "3"
        ]
    ));
    assert!(ossl(
        &d,
        &["x509", "-in", "c.pem", "-outform", "DER", "-out", "c.der"]
    ));
    assert!(ossl(
        &d,
        &[
            "pkcs12", "-export", "-inkey", "k.pem", "-in", "c.pem", "-out", "t.p12", "-passout",
            "pass:pw"
        ]
    ));
    // stand-in revocation value: a DER element (the program does not validate it)
    std::fs::write(d.join("o.der"), [0x30, 0x03, 0x02, 0x01, 0x00]).unwrap();
    let made = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf",
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    let s = run(
        &d,
        &[
            "--sign-pdf",
            "h.pdf",
            "--sign-p12",
            "t.p12",
            "--sign-pass-env",
            "LGR_TEST_PW",
            "--pdf",
            "s.pdf",
        ],
    );
    assert!(s.status.success(), "{}", text(&s));
    // no revocation value: refused, nothing written
    let no = run(
        &d,
        &[
            "--pades-lt",
            "s.pdf",
            "--dss-cert",
            "c.der",
            "--pdf",
            "lt.pdf",
        ],
    );
    assert!(!no.status.success(), "{}", text(&no));
    assert!(!d.join("lt.pdf").exists());
    let lt = run(
        &d,
        &[
            "--pades-lt",
            "s.pdf",
            "--dss-cert",
            "c.der",
            "--dss-ocsp",
            "o.der",
            "--pdf",
            "lt.pdf",
        ],
    );
    assert!(lt.status.success(), "{}", text(&lt));
    let before = std::fs::read(d.join("s.pdf")).unwrap();
    assert!(std::fs::read(d.join("lt.pdf"))
        .unwrap()
        .starts_with(&before));
    // the level report: no time stamp, so the level stays B-B
    let lv = run(&d, &["--pades-level", "lt.pdf"]);
    assert!(text(&lv).contains("level B-B"), "{}", text(&lv));
    // the plain signature: the signer's own `-cades` output is reported as it is
    let _ = std::fs::remove_dir_all(&d);
}
