//! Options that had no test: `--ocr-languages`, `--front-matter`, `--fit-width`, `--ocfl-id`,
//! `--verify-bag` (intact and tampered), `--scan-active`, `--backup-from` with `--backup-read-only`.
//! Everything runs on one synthetic page and needs no OCR engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more_{name}_{}", std::process::id()));
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
    img.save(d.join("b.png")).unwrap();
    img.save(d.join("c.png")).unwrap();
    d
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr")).current_dir(dir).args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn ocr_languages_lists_the_interface_languages() {
    let d = tmp("langs");
    let o = run(&d, &["--ocr-languages"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("Romanian") && t.contains("English"), "{t}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn front_matter_writes_roman_page_labels_then_restarts() {
    let d = tmp("front");
    let o = run(&d, &["a.png", "b.png", "c.png", "--dpi", "300", "--format", "none", "--pdf", "f.pdf", "--front-matter", "2"]);
    assert!(o.status.success(), "{}", text(&o));
    let pdf = std::fs::read(d.join("f.pdf")).unwrap();
    assert!(contains(&pdf, b"/S /r"), "no lower-case roman label");
    assert!(contains(&pdf, b"/S /D"), "no decimal label for the pages after the front matter");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn fit_width_fills_an_a5_page() {
    let d = tmp("fit");
    let o = run(&d, &["a.png", "--dpi", "300", "--format", "none", "--page-size", "a5", "--fit-width", "--pdf", "g.pdf"]);
    assert!(o.status.success(), "{}", text(&o));
    let pdf = String::from_utf8_lossy(&std::fs::read(d.join("g.pdf")).unwrap()).into_owned();
    // only the width is pinned: A5 is 419.53 pt wide and the writer rounds to the pixel grid at 300 dpi
    assert!(pdf.contains("/MediaBox [0 0 419.760"), "{}", pdf.lines().find(|l| l.contains("MediaBox")).unwrap_or("no MediaBox"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ocfl_id_lands_in_the_inventory() {
    let d = tmp("ocfl");
    let o = run(&d, &["a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf", "--ocfl", "obj", "--ocfl-id", "urn:test:42"]);
    assert!(o.status.success(), "{}", text(&o));
    let inv = std::fs::read_to_string(d.join("obj/v1/inventory.json")).unwrap();
    assert!(inv.contains("urn:test:42"), "{inv}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn verify_bag_accepts_an_intact_bag_and_rejects_a_changed_one() {
    let d = tmp("bag");
    let o = run(&d, &["a.png", "--dpi", "300", "--format", "none", "--pdf", "i.pdf", "--bagit", "bag"]);
    assert!(o.status.success(), "{}", text(&o));
    let ok = run(&d, &["--verify-bag", "bag"]);
    assert!(ok.status.success() && text(&ok).contains("intact"), "{}", text(&ok));
    let data = d.join("bag/data");
    let f = std::fs::read_dir(&data).unwrap().next().unwrap().unwrap().path();
    let mut bytes = std::fs::read(&f).unwrap();
    bytes.push(b'!');
    std::fs::write(&f, bytes).unwrap();
    let bad = run(&d, &["--verify-bag", "bag"]);
    assert!(!bad.status.success(), "{}", text(&bad));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn scan_active_finds_nothing_in_a_plain_pdf() {
    let d = tmp("active");
    let o = run(&d, &["a.png", "--dpi", "300", "--format", "none", "--pdf", "p.pdf"]);
    assert!(o.status.success(), "{}", text(&o));
    let s = run(&d, &["--scan-active", "p.pdf"]);
    assert!(s.status.success(), "{}", text(&s));
    assert!(text(&s).contains("nothing active found"), "{}", text(&s));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn backup_copies_checks_and_can_mark_read_only() {
    let d = tmp("backup");
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join("bk")).unwrap();
    std::fs::write(d.join("src/x.txt"), "hi").unwrap();
    let o = run(&d, &["--backup-from", "src", "--backup-to", "bk", "--backup-read-only"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(std::fs::read_to_string(d.join("bk/x.txt")).unwrap(), "hi");
    assert!(d.join("bk/SHA256SUMS.txt").exists());
    assert!(std::fs::metadata(d.join("bk/x.txt")).unwrap().permissions().readonly());
    // make the tree removable again
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(d.join("bk/x.txt"), std::fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    {
        let mut p = std::fs::metadata(d.join("bk/x.txt")).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        let _ = std::fs::set_permissions(d.join("bk/x.txt"), p);
    }
    let _ = std::fs::remove_dir_all(&d);
}
