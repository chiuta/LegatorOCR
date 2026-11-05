//! `--no-picture-zone` and `--no-auto-pictures`: the user can overrule the automatic picture
//! detector in mixed output, and hand-marked pictures still work.

use std::path::PathBuf;
use std::process::Command;

fn page() -> image::RgbImage {
    let (w, h) = (1000u32, 1300u32);
    let mut im = image::RgbImage::from_pixel(w, h, image::Rgb([240, 240, 235]));
    for line in 0..16u32 {
        for x0 in (100..900u32).step_by(14) {
            if (x0 / 14 + line) % 5 == 4 {
                continue;
            }
            for dx in 0..9 {
                for dy in 0..6 {
                    im.put_pixel(x0 + dx, 100 + line * 30 + dy, image::Rgb([20, 20, 20]));
                }
            }
        }
    }
    let mut seed = 12345u32;
    for y in 700..1100u32 {
        for x in 250..750u32 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (seed >> 24) as i32 % 50 - 25;
            let v = (70 + (x - 250) as i32 / 6 + n).clamp(0, 255) as u8;
            im.put_pixel(x, y, image::Rgb([v / 2, v / 2, v.saturating_add(50)]));
        }
    }
    im
}

fn run(name: &str, extra: &[&str]) -> usize {
    let d: PathBuf = std::env::temp_dir().join(format!("lgr_pz_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    page().save(d.join("a.png")).unwrap();
    let out = d.join("out");
    let mut args = vec![
        "a.png".to_string(),
        "--dpi".into(),
        "200".into(),
        "--mode".into(),
        "mixed".into(),
        "--no-auto-orient".into(),
        "-o".into(),
        out.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args(&args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let f = std::fs::read_dir(&out)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let img = image::open(f).unwrap().to_rgb8();
    let blue = img
        .pixels()
        .filter(|p| p[2] as i32 > p[0] as i32 + 30)
        .count();
    let _ = std::fs::remove_dir_all(&d);
    blue
}

#[test]
fn the_detector_can_be_overruled_and_hand_marked_pictures_still_work() {
    let auto = run("auto", &[]);
    assert!(
        auto > 50_000,
        "the photo should be found automatically: {auto} blue pixels"
    );
    // 200 dpi: the photo is x 32..95 mm, y 89..140 mm of the page
    let vetoed = run("veto", &["--no-picture-zone", "0,80,127,80"]);
    assert!(
        vetoed * 50 < auto,
        "veto over all of it: {vetoed} vs {auto}"
    );
    let half = run("half", &["--no-picture-zone", "0,115,127,40"]);
    assert!(
        half > auto / 4 && half < auto * 3 / 4,
        "half cut: {half} vs {auto}"
    );
    let off = run("off", &["--no-auto-pictures"]);
    assert!(off * 50 < auto, "detector off: {off} vs {auto}");
    let hand = run(
        "hand",
        &["--no-auto-pictures", "--picture-zone", "40,95,40,25"],
    );
    assert!(
        hand > 10_000 && hand < auto / 2,
        "hand zone only: {hand} vs {auto}"
    );
    // a veto does not touch a picture marked by hand inside it
    let both = run(
        "both",
        &[
            "--no-auto-pictures",
            "--picture-zone",
            "40,95,40,25",
            "--no-picture-zone",
            "0,80,127,80",
        ],
    );
    assert_eq!(both, hand);
}

#[test]
fn a_bad_no_picture_zone_is_refused() {
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .args(["x.png", "--no-picture-zone", "1,2,3"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--no-picture-zone"));
}

#[test]
fn a_page_can_have_its_own_output_mode_in_a_project() {
    let d: PathBuf = std::env::temp_dir().join(format!("lgr_pm_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut im = image::RgbImage::from_pixel(600, 800, image::Rgb([240, 240, 235]));
    for y in 200..500u32 {
        for x in 150..450u32 {
            im.put_pixel(x, y, image::Rgb([200, 30, 30]));
        }
    }
    im.save(d.join("a.png")).unwrap();
    im.save(d.join("b.png")).unwrap();
    // global colour; the second page asks for grey
    std::fs::write(
        d.join("p.lgr"),
        "legatorocr-project 1\ndpi = 150\nmode = color\n\n[sheet]\npath = a.png\n\n[sheet]\npath = b.png\nmode = gray\n",
    )
    .unwrap();
    let out = d.join("out");
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args([
            "--project",
            "p.lgr",
            "--no-auto-orient",
            "-o",
            &out.display().to_string(),
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let mut files: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 2, "{files:?}");
    let coloured = |f: &PathBuf| {
        image::open(f)
            .unwrap()
            .to_rgb8()
            .pixels()
            .filter(|p| p[0] as i32 - p[2] as i32 > 60)
            .count()
    };
    assert!(coloured(&files[0]) > 10_000, "page 1 stays in colour");
    assert_eq!(coloured(&files[1]), 0, "page 2 is grey");
    let _ = std::fs::remove_dir_all(&d);
}
