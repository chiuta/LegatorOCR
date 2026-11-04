//! Covers options that had no test: PDF metadata (`--author`, `--subject`, `--keywords`),
//! `--jobs` and `--doctor`.

use std::process::Command;

fn page(dir: &std::path::Path) {
    let mut img = image::GrayImage::from_pixel(400, 400, image::Luma([255]));
    for y in 100..110 {
        for x in 60..340 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(dir.join("a.png")).unwrap();
}

#[test]
fn pdf_metadata_options_reach_the_info_dictionary() {
    let dir = std::env::temp_dir().join(format!("lgr_meta_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    page(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&dir)
        .args(["a.png", "--dpi", "300", "--format", "none", "--pdf", "o.pdf", "--jobs", "1",
               "--author", "Ana Pop", "--subject", "Subiect de test", "--keywords", "unu, doi"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let pdf = std::fs::read(dir.join("o.pdf")).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    for needle in ["Ana Pop", "Subiect de test", "unu, doi"] {
        assert!(text.contains(needle) || contains_utf16(&pdf, needle), "missing {needle} in PDF");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn contains_utf16(hay: &[u8], s: &str) -> bool {
    let mut n = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        n.extend_from_slice(&u.to_be_bytes());
    }
    hay.windows(n.len()).any(|w| w == n.as_slice())
}

#[test]
fn doctor_runs_and_exits_cleanly() {
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr")).arg("--doctor").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!out.stdout.is_empty() || !out.stderr.is_empty());
}
