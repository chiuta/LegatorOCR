//! `--passages-jsonl` option checks that need no OCR engine (the schema itself is covered by the
//! unit tests of `legatorocr_engine::passages` and by `scripts/check-passages.py` on real runs).

use std::process::Command;

#[test]
fn passages_jsonl_needs_ocr_and_is_a_known_option() {
    let dir = std::env::temp_dir().join(format!("lgr_passages_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    image::GrayImage::from_pixel(300, 300, image::Luma([255])).save(dir.join("a.png")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&dir)
        .args(["a.png", "--dpi", "300", "--format", "none", "--passages-jsonl", "p.jsonl"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("--passages-jsonl") && err.contains("need --ocr"), "{err}");
    assert!(!dir.join("p.jsonl").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
