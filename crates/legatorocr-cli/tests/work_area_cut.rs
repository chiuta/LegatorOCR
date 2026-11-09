//! `--work-area` (blank everything outside a rectangle) and `--cut-at` (cut a two-page sheet at a
//! chosen fraction of its width), end to end through the command line.
use std::process::Command;

fn sheet(dir: &std::path::Path) {
    // 1800 x 900 px at 300 dpi: text bars on both halves and a dark block (a neighbour page's edge) at x 800-880 px, still left of the middle
    let mut g = image::GrayImage::from_pixel(1800, 900, image::Luma([255]));
    for y in (100..800).step_by(40) {
        for x in 100..500 {
            for dy in 0..14 {
                g.put_pixel(x, y + dy, image::Luma([0]));
            }
        }
        for x in 1100..1650 {
            for dy in 0..14 {
                g.put_pixel(x, y + dy, image::Luma([0]));
            }
        }
    }
    for y in 100..800 {
        for x in 800..880 {
            g.put_pixel(x, y, image::Luma([0]));
        }
    }
    g.save(dir.join("s.png")).unwrap();
}

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .arg("s.png")
        .args(["--dpi", "300", "--no-deskew", "--format", "png"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn work_area_blanks_the_stray_block_and_cut_at_moves_the_cut() {
    let dir = std::env::temp_dir().join(format!("lgr_workcut_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    sheet(&dir);
    // without the rectangle the block is part of the left page's content
    let out = run(&dir, &["--split", "middle", "-o", "plain"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plain = image::open(dir.join("plain/s_L.png")).unwrap().to_luma8();
    let dark = |im: &image::GrayImage| im.pixels().filter(|p| p.0[0] < 60).count();
    // with a rectangle 8-63 mm wide (94-744 px) the block is outside it and is gone
    let out = run(
        &dir,
        &[
            "--split",
            "middle",
            "--work-area",
            "8,0,55,76",
            "-o",
            "clean",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let clean = image::open(dir.join("clean/s_L.png")).unwrap().to_luma8();
    assert!(
        dark(&clean) < dark(&plain),
        "{} !< {}",
        dark(&clean),
        dark(&plain)
    );
    // cut at 40 % (720 px) instead of the middle (900 px): the left page gets narrower
    let out = run(&dir, &["--split", "middle", "--cut-at", "0.4", "-o", "cut"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cut = image::open(dir.join("cut/s_L.png")).unwrap().to_luma8();
    assert!(
        cut.width() < plain.width(),
        "{} !< {}",
        cut.width(),
        plain.width()
    );
    // bad values are refused with a message
    for bad in [
        &["--cut-at", "2"][..],
        &["--work-area", "1,2,3"][..],
        &["--work-area", "0,0,0,5"][..],
    ] {
        let out = run(&dir, bad);
        assert!(!out.status.success());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("--cut-at") || err.contains("--work-area"),
            "{err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slanted_cut_keystone_and_paper_names_work_from_the_command_line() {
    let dir = std::env::temp_dir().join(format!("lgr_slant_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    sheet(&dir);
    // a cut at the middle leaning 8 degrees: each page grows by the lean over half the height
    let out = run(
        &dir,
        &[
            "--split",
            "middle",
            "--cut-at",
            "0.5",
            "--cut-angle",
            "8",
            "--page-size",
            "b5",
            "-o",
            "slant",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // B5 at 300 dpi: 176 x 250 mm = 2079 x 2953 px
    let l = image::open(dir.join("slant/s_L.png")).unwrap();
    assert_eq!((l.width(), l.height()), (2079, 2953));
    // values outside the range are refused with a message that names the option
    let out = run(&dir, &["--cut-angle", "45", "-o", "bad"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cut-angle"));
    let out = run(&dir, &["--keystone-v", "99", "-o", "bad"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("keystone"));
    // a keystone correction runs end to end
    let out = run(
        &dir,
        &["--keystone-v", "6", "--keystone-h", "-4", "-o", "key"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
