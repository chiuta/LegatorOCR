//! --auto-retry is opt-in and needs --ocr; the options exist and are documented in --help.

use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_legatorocr")
}

#[test]
fn auto_retry_needs_ocr_and_is_listed() {
    let d = std::env::temp_dir().join(format!("lgr_ar_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let img = image::GrayImage::from_pixel(200, 300, image::Luma([255]));
    img.save(d.join("a.png")).unwrap();
    let out = Command::new(exe())
        .current_dir(&d)
        .args(["a.png", "--auto-retry", "--format", "png", "-o", "o"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--auto-retry needs --ocr"), "{}", String::from_utf8_lossy(&out.stderr));
    let help = Command::new(exe()).arg("--help").output().unwrap();
    let h = String::from_utf8_lossy(&help.stdout);
    assert!(h.contains("--auto-retry-below") && h.contains("EXPERIMENTAL"));
    let _ = std::fs::remove_dir_all(&d);
}
