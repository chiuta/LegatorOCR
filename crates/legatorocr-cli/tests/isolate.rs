//! --isolate-decoders: a good file gives the same pages as the direct path, a corrupt one
//! fails cleanly (the run reports it and exits non-zero instead of crashing).

use std::path::PathBuf;
use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_legatorocr")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_iso_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn good_file_matches_direct_and_corrupt_file_fails_cleanly() {
    let d = tmp("a");
    let mut img = image::GrayImage::from_pixel(300, 400, image::Luma([255]));
    for y in 100..110 {
        for x in 50..250 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(d.join("a.png")).unwrap();
    let mut bytes = std::fs::read(d.join("a.png")).unwrap();
    bytes.truncate(60);
    std::fs::write(d.join("bad.png"), bytes).unwrap();
    let run = |extra: &[&str], input: &str, out: &str| {
        Command::new(exe())
            .current_dir(&d)
            .arg(input)
            .args(extra)
            .args(["--dpi", "300", "--format", "png", "-o", out])
            .output()
            .unwrap()
    };
    let direct = run(&[], "a.png", "o1");
    let iso = run(&["--isolate-decoders"], "a.png", "o2");
    assert!(
        direct.status.success() && iso.status.success(),
        "{}",
        String::from_utf8_lossy(&iso.stderr)
    );
    let name = |o: &str| {
        std::fs::read_dir(d.join(o))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    };
    assert_eq!(
        std::fs::read(name("o1")).unwrap(),
        std::fs::read(name("o2")).unwrap()
    );
    let bad = run(&["--isolate-decoders"], "bad.png", "o3");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("bad.png"));
    // nothing is left behind in the temp directory by the worker
    let left = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("lgr_dec_{}_", std::process::id()))
        })
        .count();
    assert_eq!(left, 0);
    let _ = std::fs::remove_dir_all(&d);
}
