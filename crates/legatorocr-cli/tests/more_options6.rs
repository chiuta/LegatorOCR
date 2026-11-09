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

fn ossl_out(d: &Path, args: &[&str]) -> String {
    let o = Command::new("openssl")
        .current_dir(d)
        .args(args)
        .output()
        .expect("openssl");
    assert!(o.status.success(), "openssl {args:?}: {}", text(&o));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn find(h: &[u8], n: &[u8], from: usize) -> usize {
    from + h[from..]
        .windows(n.len())
        .position(|w| w == n)
        .unwrap_or_else(|| panic!("{:?} not found", String::from_utf8_lossy(n)))
}

/// The CMS blob of the (only) signature of `pdf` and the bytes its ByteRange covers.
fn cms_and_covered(pdf: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let br = find(pdf, b"/ByteRange [", 0) + 12;
    let end = find(pdf, b"]", br);
    let n: Vec<usize> = std::str::from_utf8(&pdf[br..end])
        .unwrap()
        .split_whitespace()
        .map(|x| x.parse().unwrap())
        .collect();
    assert_eq!(n.len(), 4);
    let c = find(pdf, b"/Contents <", end) + 11;
    let ce = find(pdf, b">", c);
    let hex = std::str::from_utf8(&pdf[c..ce]).unwrap();
    let cms: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let mut cov = pdf[n[0]..n[0] + n[1]].to_vec();
    cov.extend_from_slice(&pdf[n[2]..n[2] + n[3]]);
    (cms, cov)
}

/// First `[HEX DUMP]:` value on a line after line `from`, lowercased.
fn hex_after(lines: &[&str], from: usize) -> String {
    lines[from + 1..]
        .iter()
        .find_map(|l| l.split("[HEX DUMP]:").nth(1))
        .expect("hex dump")
        .trim()
        .to_ascii_lowercase()
}

fn sha256_of(d: &Path, file: &str) -> String {
    let o = ossl_out(d, &["dgst", "-sha256", "-r", file]);
    o.split_whitespace().next().unwrap().to_ascii_lowercase()
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

/// `--sign-pdf --sign-pades`: the signature has no signing-time attribute (the default one does),
/// `--pades-level` reports no such error for it, and the signature still verifies. RSA and EC.
#[test]
fn sign_pades_has_no_signing_time_error() {
    let d = tmp("pades");
    if !ossl(&d, &["version"]) {
        eprintln!("skipped: openssl is not installed");
        return;
    }
    let ok = |args: &[&str]| assert!(ossl(&d, args), "openssl {args:?} failed");
    ok(&[
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "k.pem", "-out", "c.pem",
        "-subj", "/CN=T", "-days", "3",
    ]);
    ok(&[
        "pkcs12", "-export", "-inkey", "k.pem", "-in", "c.pem", "-out", "rsa.p12", "-passout",
        "pass:pw",
    ]);
    ok(&[
        "req",
        "-x509",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-nodes",
        "-keyout",
        "ek.pem",
        "-out",
        "ec.pem",
        "-subj",
        "/CN=E",
        "-days",
        "3",
    ]);
    ok(&[
        "pkcs12", "-export", "-inkey", "ek.pem", "-in", "ec.pem", "-out", "ec.p12", "-passout",
        "pass:pw",
    ]);
    let made = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf",
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    // default signer: unchanged, still carries the signing-time attribute
    let s = run(
        &d,
        &[
            "--sign-pdf",
            "h.pdf",
            "--sign-p12",
            "rsa.p12",
            "--sign-pass-env",
            "LGR_TEST_PW",
            "--pdf",
            "plain.pdf",
        ],
    );
    assert!(s.status.success(), "{}", text(&s));
    let lv = run(&d, &["--pades-level", "plain.pdf"]);
    assert!(text(&lv).contains("signing-time"), "{}", text(&lv));
    for (p12, cert, out) in [
        ("rsa.p12", "c.pem", "pr.pdf"),
        ("ec.p12", "ec.pem", "pe.pdf"),
    ] {
        let s = run(
            &d,
            &[
                "--sign-pdf",
                "h.pdf",
                "--sign-p12",
                p12,
                "--sign-pass-env",
                "LGR_TEST_PW",
                "--sign-pades",
                "--pdf",
                out,
            ],
        );
        assert!(s.status.success(), "{}", text(&s));
        let lv = run(&d, &["--pades-level", out]);
        let t = text(&lv);
        assert!(t.contains("level B-B"), "{t}");
        assert!(!t.contains("signing-time"), "{t}");
        assert!(!t.contains("ERROR"), "{t}");
        let v = run(&d, &["--verify-pdf-signatures", out, "--sign-ca", cert]);
        assert!(v.status.success(), "{}", text(&v));
        // structure openssl does not check: attribute set and order, the digest the signature
        // commits to, and the signing-certificate-v2 hash
        let pdf = std::fs::read(d.join(out)).unwrap();
        let (cms, cov) = cms_and_covered(&pdf);
        std::fs::write(d.join("x.cms"), &cms).unwrap();
        std::fs::write(d.join("x.cov"), &cov).unwrap();
        let der = format!("{out}.cert.der");
        ok(&["x509", "-in", cert, "-outform", "DER", "-out", &der]);
        let dump = ossl_out(&d, &["asn1parse", "-inform", "DER", "-in", "x.cms"]);
        assert!(!dump.contains("Error"), "{dump}");
        let lines: Vec<&str> = dump.lines().collect();
        let at = |what: &str| {
            lines
                .iter()
                .position(|l| l.contains(what))
                .unwrap_or_else(|| panic!("{what} missing in {out}:\n{dump}"))
        };
        let (ct, md, sc) = (
            at(":contentType"),
            at(":messageDigest"),
            at(":id-smime-aa-signingCertificateV2"),
        );
        // DER SET OF order: by encoding, i.e. contentType, messageDigest, signingCertificateV2
        assert!(ct < md && md < sc, "{out}: attribute order\n{dump}");
        assert!(!dump.contains(":signingTime"), "{dump}");
        assert!(!dump.contains("smimeCapabilities"), "{dump}");
        assert_eq!(
            lines.iter().filter(|l| l.contains("id-smime-aa-")).count(),
            1,
            "{dump}"
        );
        assert_eq!(
            hex_after(&lines, md),
            sha256_of(&d, "x.cov"),
            "messageDigest"
        );
        assert_eq!(hex_after(&lines, sc), sha256_of(&d, &der), "certHash");
        // issuerSerial follows the hash: [4] directoryName
        assert!(
            lines[sc..].iter().any(|l| l.contains("cont [ 4 ]")),
            "{dump}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}
