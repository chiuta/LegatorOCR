//! Second batch of option tests (no OCR engine): search flags, Merkle proof, OCFL check, rights notice,
//! e-Factura, saving a profile and PDF signing (the last two need Linux / openssl).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more2_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut img = image::GrayImage::from_pixel(600, 800, image::Luma([210]));
    for y in (100..700).step_by(40) {
        for yy in y..y + 10 {
            for x in 80..520 {
                img.put_pixel(x, yy, image::Luma([40]));
            }
        }
    }
    img.save(d.join("a.png")).unwrap();
    d
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
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

#[test]
fn search_flags_exact_max_and_old_orthography() {
    let d = tmp("search");
    std::fs::write(
        d.join("t.txt"),
        "Mobila pînză sînt cînd\nAbc 123 Mobilă\nmobila\nsecond mobila line\n",
    )
    .unwrap();
    let all = run(&d, &["--search", "mobila", "t.txt"]);
    assert!(text(&all).contains("4 hit(s)"), "{}", text(&all));
    let exact = run(&d, &["--search", "mobila", "--search-exact", "t.txt"]);
    assert!(text(&exact).contains("2 hit(s)"), "{}", text(&exact));
    let max = run(&d, &["--search", "mobila", "--search-max", "1", "t.txt"]);
    assert!(
        text(&max).contains("1 hit(s)") && text(&max).contains("stopped at the limit"),
        "{}",
        text(&max)
    );
    let new = run(&d, &["--search", "sunt", "t.txt"]);
    assert!(text(&new).contains("0 hit(s)"), "{}", text(&new));
    let old = run(
        &d,
        &["--search", "sunt", "--search-old-orthography", "t.txt"],
    );
    assert!(text(&old).contains("1 hit(s)"), "{}", text(&old));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn merkle_proof_verifies_and_a_wrong_root_does_not() {
    let d = tmp("merkle");
    std::fs::write(d.join("SHA256SUMS.txt"), "1111111111111111111111111111111111111111111111111111111111111111  one\n2222222222222222222222222222222222222222222222222222222222222222  two\n").unwrap();
    let o = run(
        &d,
        &["--merkle-root", "SHA256SUMS.txt", "--merkle-proof", "two"],
    );
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    let root = t
        .lines()
        .find_map(|l| l.strip_prefix("merkle root of 2 file(s): "))
        .expect(&t)
        .trim()
        .to_string();
    let line = t
        .lines()
        .find_map(|l| l.strip_prefix("proof: "))
        .expect(&t)
        .trim()
        .to_string();
    let good = run(&d, &["--merkle-verify", &line, "--merkle-expect", &root]);
    assert!(good.status.success(), "{}", text(&good));
    let wrong = "0".repeat(64);
    let bad = run(&d, &["--merkle-verify", &line, "--merkle-expect", &wrong]);
    assert!(!bad.status.success(), "{}", text(&bad));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn merkle_consistency_proves_an_unchanged_start_and_a_rewrite_fails() {
    let d = tmp("merkle_cons");
    let h = |c: char| c.to_string().repeat(64);
    let old = format!("{}  zeta\n{}  alpha\n{}  mid\n", h('1'), h('2'), h('3'));
    std::fs::write(d.join("old.txt"), &old).unwrap();
    std::fs::write(
        d.join("new.txt"),
        format!("{old}{}  late\n{}  last\n", h('4'), h('5')),
    )
    .unwrap();
    let o = run(&d, &["--merkle-root", "old.txt", "--merkle-keep-order"]);
    assert!(o.status.success(), "{}", text(&o));
    let old_root = text(&o)
        .lines()
        .find_map(|l| l.strip_prefix("merkle root of 3 file(s): "))
        .unwrap()
        .trim()
        .to_string();
    let n = run(
        &d,
        &["--merkle-root", "new.txt", "--merkle-consistency", "3"],
    );
    assert!(n.status.success(), "{}", text(&n));
    let t = text(&n);
    let new_root = t
        .lines()
        .find_map(|l| l.strip_prefix("merkle root of 5 file(s): "))
        .unwrap()
        .trim()
        .to_string();
    assert!(t.contains(&old_root), "{t}");
    let line = t
        .lines()
        .find(|l| l.starts_with("consistency: 3 5 "))
        .unwrap()
        .to_string();
    let args = |line: &str, old: &str, new: &str| {
        run(
            &d,
            &[
                "--merkle-verify-consistency",
                line,
                "--merkle-old-root",
                old,
                "--merkle-expect",
                new,
            ],
        )
    };
    let ok = args(&line, &old_root, &new_root);
    assert!(ok.status.success(), "{}", text(&ok));
    let wrong = "0".repeat(64);
    assert!(!args(&line, &wrong, &new_root).status.success());
    assert!(!args("consistency: 3 5 ", &old_root, &new_root)
        .status
        .success());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn verify_ocfl_accepts_an_object_and_rejects_a_changed_file() {
    let d = tmp("ocflv");
    let o = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf", "--ocfl", "obj",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let ok = run(&d, &["--verify-ocfl", "obj"]);
    assert!(
        ok.status.success() && text(&ok).contains("intact"),
        "{}",
        text(&ok)
    );
    let content = d.join("obj/v1/content");
    let f = std::fs::read_dir(&content)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&f).unwrap();
    bytes.push(b'!');
    std::fs::write(&f, bytes).unwrap();
    let bad = run(&d, &["--verify-ocfl", "obj"]);
    assert!(!bad.status.success(), "{}", text(&bad));
    let _ = std::fs::remove_dir_all(&d);
}

fn utf16_hex_strings(pdf: &[u8]) -> Vec<String> {
    // PDF text strings written as <FEFF....> (UTF-16BE with a byte-order mark)
    let s = String::from_utf8_lossy(pdf);
    let mut out = Vec::new();
    for part in s.split("<FEFF").skip(1) {
        if let Some(end) = part.find('>') {
            let hex = &part[..end];
            let units: Vec<u16> = (0..hex.len() / 4)
                .filter_map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok())
                .collect();
            out.push(String::from_utf16_lossy(&units));
        }
    }
    out
}

#[test]
fn rights_notice_names_the_author_the_year_and_the_deed() {
    let d = tmp("rights");
    let o = run(
        &d,
        &[
            "a.png",
            "--dpi",
            "300",
            "--format",
            "none",
            "--pdf",
            "r.pdf",
            "--rights-death-year",
            "1990",
            "--rights-author",
            "Ion Test",
            "--rights-deed-number",
            "12",
            "--rights-deed-date",
            "2001-05-04",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let strings = utf16_hex_strings(&std::fs::read(d.join("r.pdf")).unwrap());
    let notice = strings
        .iter()
        .find(|s| s.contains("Ion Test"))
        .unwrap_or_else(|| panic!("no rights string in {strings:?}"));
    for needle in ["1990", "2060", "2061", "deed no. 12", "2001-05-04"] {
        assert!(notice.contains(needle), "{needle} missing from: {notice}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

const INVOICE: &str = r#"<?xml version="1.0"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cbc="urn:x" xmlns:cac="urn:y">
 <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:efactura.mfinante.ro:CIUS-RO:1.0.1</cbc:CustomizationID>
 <cbc:ID>FCT 0042</cbc:ID><cbc:IssueDate>2026-10-30</cbc:IssueDate>
 <cbc:DocumentCurrencyCode>RON</cbc:DocumentCurrencyCode>
 <cac:AccountingSupplierParty><cac:Party>
   <cac:PartyTaxScheme><cbc:CompanyID>RO1000009</cbc:CompanyID></cac:PartyTaxScheme>
   <cac:PartyLegalEntity><cbc:RegistrationName>Furnizor SRL</cbc:RegistrationName></cac:PartyLegalEntity>
 </cac:Party></cac:AccountingSupplierParty>
 <cac:AccountingCustomerParty><cac:Party>
   <cac:PartyTaxScheme><cbc:CompanyID>RO1000009</cbc:CompanyID></cac:PartyTaxScheme>
   <cac:PartyLegalEntity><cbc:RegistrationName>Client SA</cbc:RegistrationName></cac:PartyLegalEntity>
 </cac:Party></cac:AccountingCustomerParty>
 <cac:TaxTotal><cbc:TaxAmount currencyID="RON">190.00</cbc:TaxAmount></cac:TaxTotal>
 <cac:LegalMonetaryTotal><cbc:LineExtensionAmount>1000.00</cbc:LineExtensionAmount>
   <cbc:TaxExclusiveAmount>1000.00</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount>__GROSS__</cbc:TaxInclusiveAmount>
   <cbc:PayableAmount>__GROSS__</cbc:PayableAmount></cac:LegalMonetaryTotal>
 <cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:LineExtensionAmount>1000.00</cbc:LineExtensionAmount></cac:InvoiceLine>
</Invoice>"#;

#[test]
fn efactura_accepts_a_consistent_invoice_and_flags_a_wrong_total() {
    let d = tmp("efactura");
    std::fs::write(d.join("ok.xml"), INVOICE.replace("__GROSS__", "1190.00")).unwrap();
    std::fs::write(d.join("bad.xml"), INVOICE.replace("__GROSS__", "1200.00")).unwrap();
    let ok = run(&d, &["--efactura", "ok.xml"]);
    assert!(ok.status.success(), "{}", text(&ok));
    assert!(text(&ok).contains("FCT 0042"), "{}", text(&ok));
    let bad = run(&d, &["--efactura", "bad.xml"]);
    assert!(!bad.status.success(), "{}", text(&bad));
    let rules_ok = run(&d, &["--efactura", "ok.xml", "--efactura-rules"]);
    assert!(rules_ok.status.success(), "{}", text(&rules_ok));
    assert!(
        text(&rules_ok).contains("no violation"),
        "{}",
        text(&rules_ok)
    );
    let rules_bad = run(&d, &["--efactura", "bad.xml", "--efactura-rules"]);
    assert!(!rules_bad.status.success());
    assert!(
        text(&rules_bad).contains("RULE BR-CO-15"),
        "{}",
        text(&rules_bad)
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(target_os = "linux")]
#[test]
fn save_profile_writes_a_user_profile_that_is_then_listed() {
    let d = tmp("profile");
    let home = d.join("home");
    let cfg = home.join(".config");
    let go = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_legatorocr"))
            .current_dir(&d)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("RUST_BACKTRACE", "0")
            .args(args)
            .output()
            .unwrap()
    };
    let o = go(&["--save-profile", "myprof", "--dpi", "300"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        cfg.join("legatorocr/profiles/myprof.lgr").exists(),
        "{}",
        text(&o)
    );
    let l = go(&["--list-profiles"]);
    assert!(text(&l).contains("myprof"), "{}", text(&l));
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(target_os = "linux")]
#[test]
fn pdf_signing_round_trip_with_a_throw_away_certificate() {
    let d = tmp("sign");
    let have_openssl = Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_openssl {
        eprintln!("openssl not found: signing test skipped");
        return;
    }
    let ok = |c: &mut Command| assert!(c.current_dir(&d).output().unwrap().status.success());
    ok(Command::new("openssl").args([
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "k.pem",
        "-out",
        "c.pem",
        "-subj",
        "/CN=Test Signer",
        "-days",
        "3",
    ]));
    ok(Command::new("openssl").args([
        "pkcs12", "-export", "-inkey", "k.pem", "-in", "c.pem", "-out", "t.p12", "-passout",
        "pass:pw",
    ]));
    let made = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf",
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    let s = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .env("LGR_TEST_PW", "pw")
        .args([
            "--sign-pdf",
            "h.pdf",
            "--sign-p12",
            "t.p12",
            "--sign-pass-env",
            "LGR_TEST_PW",
            "--sign-name",
            "Ion Test",
            "--sign-reason",
            "Verificare",
            "--pdf",
            "s.pdf",
        ])
        .output()
        .unwrap();
    assert!(s.status.success(), "{}", text(&s));
    let signed = std::fs::read(d.join("s.pdf")).unwrap();
    let as_text = String::from_utf8_lossy(&signed);
    assert!(as_text.contains("Ion Test") && as_text.contains("Verificare"));
    let v = run(
        &d,
        &["--verify-pdf-signatures", "s.pdf", "--sign-ca", "c.pem"],
    );
    assert!(
        v.status.success() && text(&v).contains("chain verified"),
        "{}",
        text(&v)
    );
    let unsigned = run(&d, &["--verify-pdf-signatures", "h.pdf"]);
    assert!(!unsigned.status.success(), "{}", text(&unsigned));
    let mut bad = signed.clone();
    bad[100] ^= 1;
    std::fs::write(d.join("bad.pdf"), bad).unwrap();
    let t = run(&d, &["--verify-pdf-signatures", "bad.pdf"]);
    assert!(!t.status.success(), "{}", text(&t));
    let _ = std::fs::remove_dir_all(&d);
}
