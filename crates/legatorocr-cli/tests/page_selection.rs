//! --pages, --duplex-adf, --reverse, --dry-run and --find-replace (options added in pre.26).

use std::process::Command;

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_legatorocr")
}

fn sheets(tag: &str, n: usize) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_ps_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    for i in 1..=n {
        image::GrayImage::from_pixel(200, 300, image::Luma([255]))
            .save(d.join(format!("p{i}.png")))
            .unwrap();
    }
    d
}

fn dry(d: &std::path::Path, extra: &[&str]) -> (bool, String, String) {
    let mut a: Vec<String> = (1..=6).map(|i| format!("p{i}.png")).collect();
    a.push("--dry-run".into());
    a.extend(extra.iter().map(|s| s.to_string()));
    let o = Command::new(exe()).current_dir(d).args(a).output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|w| w.ends_with(".png"))
        .map(String::from)
        .collect()
}

#[test]
fn duplex_adf_reverse_and_range() {
    let d = sheets("a", 6);
    let (ok, out, _) = dry(&d, &["--duplex-adf"]);
    assert!(ok);
    assert_eq!(
        names(&out),
        ["p1.png", "p6.png", "p2.png", "p5.png", "p3.png", "p4.png"]
    );
    let (_, out, _) = dry(&d, &["--reverse", "--pages", "2-3"]);
    assert_eq!(names(&out), ["p3.png", "p2.png"]);
    let (_, out, _) = dry(&d, &["--pages", "1,5-"]);
    assert_eq!(names(&out), ["p1.png", "p5.png", "p6.png"]);
    assert!(out.contains("nothing was processed"));
    let (ok, _, err) = dry(&d, &["--pages", "9"]);
    assert!(!ok && err.contains("no page left"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn duplex_adf_needs_an_even_count() {
    let d = sheets("b", 5);
    let o = Command::new(exe())
        .current_dir(&d)
        .args(["p1.png", "p2.png", "p3.png", "--duplex-adf", "--dry-run"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("even number"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn find_replace_file_errors_are_clear() {
    let d = sheets("c", 1);
    std::fs::write(d.join("r.txt"), "no arrow here\n").unwrap();
    let o = Command::new(exe())
        .current_dir(&d)
        .args(["p1.png", "--find-replace", "r.txt", "--dry-run"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("wrong => right"));
    let _ = std::fs::remove_dir_all(&d);
}
