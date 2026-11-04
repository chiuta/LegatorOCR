//! `--integrity-hints`: the option writes a neutral JSON list, skips what does not apply and
//! refuses unknown signal names. Detection quality is measured separately
//! (docs/experiments/integrity_eval.py, ROADMAP 7.5c) and is synthetic only.

use std::process::Command;

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_integ_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Busy texture with a patch copied to another place.
fn copied_texture() -> image::GrayImage {
    let mut s = 12345u64;
    let mut g = image::GrayImage::new(640, 480);
    for p in g.pixels_mut() {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        p[0] = (s >> 56) as u8;
    }
    let mut g = image::imageops::blur(&g, 1.2);
    let patch = image::imageops::crop_imm(&g, 40, 40, 160, 120).to_image();
    image::imageops::overlay(&mut g, &patch, 380, 300);
    g
}

#[test]
fn integrity_hints_writes_neutral_json_with_the_copied_patch() {
    let d = dir("ok");
    copied_texture().save(d.join("a.png")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args([
            "a.png",
            "--integrity-hints",
            "h.json",
            "--integrity-signals",
            "copy_move,jpeg_grid,line_size",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let j = std::fs::read_to_string(d.join("h.json")).unwrap();
    assert!(j.contains("\"signal\": \"copy_move\""), "{j}");
    assert!(j.contains("common_innocent_causes") && j.contains("not a verdict"));
    // PNG input: the JPEG signal is skipped with a reason; no OCR: line_size is skipped
    assert!(
        j.contains("input is not a JPEG file") && j.contains("needs OCR word boxes"),
        "{j}"
    );
    let low = j.to_lowercase();
    for bad in ["forg", "fake", "tamper", "fraud", "counterfeit"] {
        assert!(!low.contains(bad), "output uses '{bad}'");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn integrity_hints_is_opt_in_and_rejects_unknown_signals() {
    let d = dir("bad");
    copied_texture().save(d.join("a.png")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args([
            "a.png",
            "--integrity-hints",
            "h.json",
            "--integrity-signals",
            "verdict",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown signal"));
    assert!(!d.join("h.json").exists());
    // without the option nothing is written
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args(["a.png", "--pdf", "o.pdf"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!d.join("h.json").exists());
    let _ = std::fs::remove_dir_all(&d);
}
