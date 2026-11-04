//! Fifth batch: `--burst` with `--burst-gap` (JPEGs carrying a hand-made EXIF capture time) and
//! `--timestamp-verify` against a reply made by a throw-away OpenSSL time-stamp authority.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more5_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(d: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr")).current_dir(d).args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Dark bars on paper; `blur` > 0 makes the same scene less sharp.
fn scene(seed: u64, blur: f32) -> image::GrayImage {
    let mut s = seed;
    let mut g = image::GrayImage::from_pixel(500, 500, image::Luma([235]));
    for _ in 0..200 {
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 33) as u32
        };
        let (x, y, w, h, v) = (20 + next() % 420, 20 + next() % 420, 8 + next() % 32, 4 + next() % 10, 20 + next() % 70);
        for yy in y..(y + h).min(500) {
            for xx in x..(x + w).min(500) {
                g.put_pixel(xx, yy, image::Luma([v as u8]));
            }
        }
    }
    if blur > 0.0 {
        image::imageops::blur(&g, blur)
    } else {
        g
    }
}

/// JPEG bytes of `img` with an APP1/Exif block carrying DateTimeOriginal (same layout as the unit test in burst.rs).
fn jpeg_with_time(img: &image::GrayImage, dt: &str) -> Vec<u8> {
    let mut plain = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 92)
        .encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::L8)
        .unwrap();
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0\x2A\0\0\0\x08");
    tiff.extend_from_slice(&[0, 1, 0x87, 0x69, 0, 4, 0, 0, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0]);
    tiff.extend_from_slice(&[0, 1, 0x90, 0x03, 0, 2, 0, 0, 0, 20, 0, 0, 0, 44, 0, 0, 0, 0]);
    tiff.extend_from_slice(dt.as_bytes());
    tiff.push(0);
    let mut app = b"Exif\0\0".to_vec();
    app.extend_from_slice(&tiff);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((app.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app);
    out.extend_from_slice(&plain[2..]);
    out
}

#[test]
fn burst_gap_decides_which_frames_count_as_one_burst() {
    let d = dir("burst");
    // two shots of the same page two seconds apart (the second one blurred), then another page ten seconds later
    std::fs::write(d.join("a.jpg"), jpeg_with_time(&scene(1, 0.0), "2026:11:01 14:00:00")).unwrap();
    std::fs::write(d.join("b.jpg"), jpeg_with_time(&scene(1, 1.5), "2026:11:01 14:00:02")).unwrap();
    std::fs::write(d.join("c.jpg"), jpeg_with_time(&scene(2, 0.0), "2026:11:01 14:00:12")).unwrap();
    let common = ["--dpi", "300", "--format", "none", "--pdf"];
    let wide = run(&d, &[&["a.jpg", "b.jpg", "c.jpg"][..], &common[..], &["w.pdf", "--burst", "--burst-gap", "3"][..]].concat());
    assert!(wide.status.success(), "{}", text(&wide));
    let narrow = run(&d, &[&["a.jpg", "b.jpg", "c.jpg"][..], &common[..], &["n.pdf", "--burst", "--burst-gap", "1"][..]].concat());
    assert!(narrow.status.success(), "{}", text(&narrow));
    // a 3 s gap makes a and b one burst (the sharper one stays); a 1 s gap keeps all three
    let (tw, tn) = (text(&wide), text(&narrow));
    assert!(tw.contains("b.jpg") && tw.contains("w.pdf (2 page(s))"), "{tw}");
    assert!(tn.contains("n.pdf (3 page(s))"), "{tn}");
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn timestamp_verify_accepts_the_file_and_rejects_a_changed_one() {
    let d = dir("tsa");
    if !Command::new("openssl").arg("version").output().map(|o| o.status.success()).unwrap_or(false) {
        eprintln!("openssl not found: time-stamp test skipped");
        return;
    }
    let ssl = |args: &[&str]| {
        let o = Command::new("openssl").args(args).current_dir(&d).output().unwrap();
        assert!(o.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    std::fs::write(
        d.join("tsa.cnf"),
        "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ext\n[dn]\nCN=Test TSA\n[ext]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n\
         [tsa]\ndefault_tsa=tsa_cfg\n[tsa_cfg]\ndir=.\nserial=./serial\nsigner_cert=./cert.pem\nsigner_key=./key.pem\ncerts=./cert.pem\ndefault_policy=1.2.3.4.5\ndigests=sha256\nsigner_digest=sha256\naccuracy=secs:1\nordering=yes\ntsa_name=yes\n",
    )
    .unwrap();
    std::fs::write(d.join("serial"), "01\n").unwrap();
    ssl(&["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "key.pem", "-out", "cert.pem", "-days", "2", "-config", "tsa.cnf"]);
    std::fs::write(d.join("SHA256SUMS.txt"), "registry root\n").unwrap();
    ssl(&["ts", "-query", "-data", "SHA256SUMS.txt", "-sha256", "-cert", "-out", "req.tsq"]);
    ssl(&["ts", "-reply", "-queryfile", "req.tsq", "-config", "tsa.cnf", "-out", "SHA256SUMS.txt.tsr"]);
    let ok = run(&d, &["--timestamp-verify", "SHA256SUMS.txt", "--tsa-ca", "cert.pem"]);
    assert!(ok.status.success(), "{}", text(&ok));
    std::fs::write(d.join("SHA256SUMS.txt"), "registry root changed\n").unwrap();
    let bad = run(&d, &["--timestamp-verify", "SHA256SUMS.txt", "--tsa-ca", "cert.pem"]);
    assert!(!bad.status.success(), "{}", text(&bad));
    let _ = std::fs::remove_dir_all(&d);
}

/// A page lit unevenly, with real text lines and fainter lines in between (print showing through from the other
/// side), the faint ones also running past the last real line.
fn show_through_page() -> image::GrayImage {
    let (w, h) = (600u32, 800u32);
    let mut g = image::GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            g.put_pixel(x, y, image::Luma([(150 + 100 * x / w) as u8]));
        }
    }
    for y in (100..700).step_by(40) {
        for yy in y..y + 10 {
            for x in 80..520 {
                g.put_pixel(x, yy, image::Luma([40]));
            }
        }
    }
    for y in (120..700).step_by(40) {
        for yy in y + 14..y + 22 {
            let paper = g.get_pixel(300, yy)[0]; // read once: the loop below overwrites this pixel
            for x in 100..500 {
                g.put_pixel(x, yy, image::Luma([paper.saturating_sub(35)]));
            }
        }
    }
    g
}

#[test]
fn show_through_removes_faint_print_so_the_page_ends_at_the_last_real_line() {
    let d = dir("showthrough");
    show_through_page().save(d.join("st.png")).unwrap();
    let height_of = |extra: &[&str], out: &str| {
        let mut args = vec!["st.png", "--dpi", "300", "--split", "off", "--mode", "gray", "--format", "png", "-o", out];
        args.extend_from_slice(extra);
        let o = run(&d, &args);
        assert!(o.status.success(), "{}", text(&o));
        let f = std::fs::read_dir(d.join(out)).unwrap().next().unwrap().unwrap().path();
        image::open(f).unwrap().to_luma8().height()
    };
    let plain = height_of(&[], "plain");
    let faded = height_of(&["--show-through"], "faded");
    assert!(faded < plain, "without the option {plain} px, with it {faded} px: the faint lines below the last real line should be gone");
    let _ = std::fs::remove_dir_all(&d);
}
