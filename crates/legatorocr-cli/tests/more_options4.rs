//! Fourth batch: frame fusion limits, `--verify-document`, decoder limits and the hot folder's
//! `--watch-skip-existing`. Synthetic images only.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more4_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(d: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(d)
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

/// Scattered dark bars on paper, the same scene shifted by (dx, dy).
fn frame(dx: i64, dy: i64) -> image::GrayImage {
    let mut s = 7u64;
    let mut base = image::GrayImage::from_pixel(500, 500, image::Luma([235]));
    for _ in 0..200 {
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u32
        };
        let (x, y, w, h, v) = (
            20 + next() % 420,
            20 + next() % 420,
            8 + next() % 32,
            4 + next() % 10,
            20 + next() % 70,
        );
        for yy in y..(y + h).min(500) {
            for xx in x..(x + w).min(500) {
                base.put_pixel(xx, yy, image::Luma([v as u8]));
            }
        }
    }
    let mut out = image::GrayImage::from_pixel(500, 500, image::Luma([235]));
    image::imageops::overlay(&mut out, &base, dx, dy);
    out
}

#[test]
fn fuse_max_shift_decides_which_frames_are_kept() {
    let d = dir("fuse");
    frame(0, 0).save(d.join("f1.png")).unwrap();
    frame(4, 3).save(d.join("f2.png")).unwrap();
    frame(40, 0).save(d.join("f3.png")).unwrap();
    let wide = run(
        &d,
        &[
            "f1.png",
            "f2.png",
            "f3.png",
            "--fuse-frames",
            "w.png",
            "--fuse-max-shift",
            "50",
        ],
    );
    assert!(
        wide.status.success() && text(&wide).contains("from 3 of 3 frames"),
        "{}",
        text(&wide)
    );
    // the search window is a little wider than the number given (coarse steps of 4 px plus a fine step), so a 4 px shift
    // passes a limit of 2 while a 40 px shift does not
    let narrow = run(
        &d,
        &[
            "f1.png",
            "f2.png",
            "f3.png",
            "--fuse-frames",
            "n.png",
            "--fuse-max-shift",
            "2",
        ],
    );
    assert!(narrow.status.success(), "{}", text(&narrow));
    assert!(
        text(&narrow).contains("from 2 of 3 frames") && text(&narrow).contains("f3.png: left out"),
        "{}",
        text(&narrow)
    );
    assert!(d.join("n.png").exists());
    let _ = std::fs::remove_dir_all(&d);
}

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
fn verify_document_reports_a_copied_region_and_stays_quiet_on_a_plain_page() {
    let d = dir("vd");
    copied_texture().save(d.join("cm.png")).unwrap();
    image::GrayImage::from_pixel(300, 300, image::Luma([240]))
        .save(d.join("plain.png"))
        .unwrap();
    let hit = run(&d, &["cm.png", "--verify-document"]);
    assert!(hit.status.success(), "{}", text(&hit));
    let t = text(&hit);
    assert!(
        t.contains("copy-move") && !t.contains("copy-move: nothing found"),
        "{t}"
    );
    let quiet = run(&d, &["plain.png", "--verify-document"]);
    assert!(
        text(&quiet).contains("copy-move: nothing found"),
        "{}",
        text(&quiet)
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn isolated_decoders_accept_memory_and_time_limits() {
    let d = dir("iso");
    frame(0, 0).save(d.join("a.png")).unwrap();
    let o = run(
        &d,
        &[
            "a.png",
            "--dpi",
            "300",
            "--format",
            "none",
            "--pdf",
            "o.pdf",
            "--isolate-decoders",
            "--decoder-memory",
            "2048",
            "--decoder-timeout",
            "60",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(std::fs::metadata(d.join("o.pdf")).unwrap().len() > 500);
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn watch_skip_existing_ignores_old_files_and_processes_new_ones() {
    let d = dir("watch");
    let inbox = d.join("in");
    std::fs::create_dir_all(&inbox).unwrap();
    frame(0, 0).save(inbox.join("old.png")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args([
            "--watch",
            "in",
            "--watch-interval",
            "1",
            "--watch-skip-existing",
            "--dpi",
            "300",
            "--format",
            "none",
            "--pdf",
            "out_{name}.pdf",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(4));
    frame(5, 5).save(inbox.join("new.png")).unwrap();
    let t0 = Instant::now();
    while !d.join("out_new.pdf").exists() && t0.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        d.join("out_new.pdf").exists(),
        "the file added after the start was not processed"
    );
    assert!(
        !d.join("out_old.pdf").exists(),
        "the file present at the start must be skipped"
    );
    let _ = std::fs::remove_dir_all(&d);
}
