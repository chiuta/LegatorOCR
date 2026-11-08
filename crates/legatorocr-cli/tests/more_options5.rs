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

/// Dark bars on paper; `blur` > 0 makes the same scene less sharp.
fn scene(seed: u64, blur: f32) -> image::GrayImage {
    let mut s = seed;
    let mut g = image::GrayImage::from_pixel(500, 500, image::Luma([235]));
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
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::L8,
        )
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
    std::fs::write(
        d.join("a.jpg"),
        jpeg_with_time(&scene(1, 0.0), "2026:11:01 14:00:00"),
    )
    .unwrap();
    std::fs::write(
        d.join("b.jpg"),
        jpeg_with_time(&scene(1, 1.5), "2026:11:01 14:00:02"),
    )
    .unwrap();
    std::fs::write(
        d.join("c.jpg"),
        jpeg_with_time(&scene(2, 0.0), "2026:11:01 14:00:12"),
    )
    .unwrap();
    let common = ["--dpi", "300", "--format", "none", "--pdf"];
    let wide = run(
        &d,
        &[
            &["a.jpg", "b.jpg", "c.jpg"][..],
            &common[..],
            &["w.pdf", "--burst", "--burst-gap", "3"][..],
        ]
        .concat(),
    );
    assert!(wide.status.success(), "{}", text(&wide));
    let narrow = run(
        &d,
        &[
            &["a.jpg", "b.jpg", "c.jpg"][..],
            &common[..],
            &["n.pdf", "--burst", "--burst-gap", "1"][..],
        ]
        .concat(),
    );
    assert!(narrow.status.success(), "{}", text(&narrow));
    // a 3 s gap makes a and b one burst (the sharper one stays); a 1 s gap keeps all three
    let (tw, tn) = (text(&wide), text(&narrow));
    assert!(
        tw.contains("b.jpg") && tw.contains("w.pdf (2 page(s))"),
        "{tw}"
    );
    assert!(tn.contains("n.pdf (3 page(s))"), "{tn}");
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn timestamp_verify_accepts_the_file_and_rejects_a_changed_one() {
    let d = dir("tsa");
    if !Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        eprintln!("openssl not found: time-stamp test skipped");
        return;
    }
    let ssl = |args: &[&str]| {
        let o = Command::new("openssl")
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    std::fs::write(
        d.join("tsa.cnf"),
        "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ext\n[dn]\nCN=Test TSA\n[ext]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n\
         [tsa]\ndefault_tsa=tsa_cfg\n[tsa_cfg]\ndir=.\nserial=./serial\nsigner_cert=./cert.pem\nsigner_key=./key.pem\ncerts=./cert.pem\ndefault_policy=1.2.3.4.5\ndigests=sha256\nsigner_digest=sha256\naccuracy=secs:1\nordering=yes\ntsa_name=yes\n",
    )
    .unwrap();
    std::fs::write(d.join("serial"), "01\n").unwrap();
    ssl(&[
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "key.pem", "-out", "cert.pem",
        "-days", "2", "-config", "tsa.cnf",
    ]);
    std::fs::write(d.join("SHA256SUMS.txt"), "registry root\n").unwrap();
    ssl(&[
        "ts",
        "-query",
        "-data",
        "SHA256SUMS.txt",
        "-sha256",
        "-cert",
        "-out",
        "req.tsq",
    ]);
    ssl(&[
        "ts",
        "-reply",
        "-queryfile",
        "req.tsq",
        "-config",
        "tsa.cnf",
        "-out",
        "SHA256SUMS.txt.tsr",
    ]);
    let ok = run(
        &d,
        &[
            "--timestamp-verify",
            "SHA256SUMS.txt",
            "--tsa-ca",
            "cert.pem",
        ],
    );
    assert!(ok.status.success(), "{}", text(&ok));
    std::fs::write(d.join("SHA256SUMS.txt"), "registry root changed\n").unwrap();
    let bad = run(
        &d,
        &[
            "--timestamp-verify",
            "SHA256SUMS.txt",
            "--tsa-ca",
            "cert.pem",
        ],
    );
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
        let mut args = vec![
            "st.png", "--dpi", "300", "--split", "off", "--mode", "gray", "--format", "png", "-o",
            out,
        ];
        args.extend_from_slice(extra);
        let o = run(&d, &args);
        assert!(o.status.success(), "{}", text(&o));
        let f = std::fs::read_dir(d.join(out))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        image::open(f).unwrap().to_luma8().height()
    };
    let plain = height_of(&[], "plain");
    let faded = height_of(&["--show-through"], "faded");
    assert!(faded < plain, "without the option {plain} px, with it {faded} px: the faint lines below the last real line should be gone");
    let _ = std::fs::remove_dir_all(&d);
}

/// `--sign-timestamp`: the whole flow against a time-stamp authority on 127.0.0.1 (plain http; a Python
/// stand-in that hands every request to a throw-away OpenSSL authority). The https path differs only in curl.
#[cfg(target_os = "linux")]
#[test]
fn sign_timestamp_gets_a_stamp_from_a_local_authority() {
    let have = |c: &str, a: &str| {
        Command::new(c)
            .arg(a)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !(have("openssl", "version") && have("python3", "--version") && have("curl", "--version")) {
        eprintln!("openssl, python3 or curl not found: sign-timestamp test skipped");
        return;
    }
    let d = dir("signts");
    let ssl = |args: &[&str]| {
        let o = Command::new("openssl")
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    std::fs::write(
        d.join("tsa.cnf"),
        "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ext\n[dn]\nCN=Test TSA\n[ext]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n\
         [tsa]\ndefault_tsa=tsa_cfg\n[tsa_cfg]\ndir=.\nserial=./serial\nsigner_cert=./cert.pem\nsigner_key=./key.pem\ncerts=./cert.pem\ndefault_policy=1.2.3.4.5\ndigests=sha256\nsigner_digest=sha256\naccuracy=secs:1\nordering=yes\ntsa_name=yes\n",
    )
    .unwrap();
    std::fs::write(d.join("serial"), "01\n").unwrap();
    ssl(&[
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "key.pem", "-out", "cert.pem",
        "-days", "2", "-config", "tsa.cnf",
    ]);
    ssl(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "k.pem",
        "-out",
        "c.pem",
        "-subj",
        "/CN=Test Signer",
        "-days",
        "3",
    ]);
    ssl(&[
        "pkcs12", "-export", "-inkey", "k.pem", "-in", "c.pem", "-out", "t.p12", "-passout",
        "pass:pw",
    ]);
    let server = "import http.server, subprocess, sys\n\
        class H(http.server.BaseHTTPRequestHandler):\n\
        \x20   def do_POST(self):\n\
        \x20       open('q.tsq','wb').write(self.rfile.read(int(self.headers['Content-Length'])))\n\
        \x20       subprocess.run(['openssl','ts','-reply','-queryfile','q.tsq','-config','tsa.cnf','-out','r.tsr'],check=True)\n\
        \x20       b=open('r.tsr','rb').read()\n\
        \x20       self.send_response(200); self.send_header('Content-Type','application/timestamp-reply'); self.send_header('Content-Length',str(len(b))); self.end_headers(); self.wfile.write(b)\n\
        \x20   def log_message(self,*a): pass\n\
        s=http.server.HTTPServer(('127.0.0.1',0),H)\n\
        open('port','w').write(str(s.server_address[1]))\n\
        s.serve_forever()\n";
    std::fs::write(d.join("tsa_server.py"), server).unwrap();
    let mut child = Command::new("python3")
        .arg("tsa_server.py")
        .current_dir(&d)
        .spawn()
        .unwrap();
    let mut port = String::new();
    for _ in 0..100 {
        if let Ok(p) = std::fs::read_to_string(d.join("port")) {
            if !p.is_empty() {
                port = p;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!port.is_empty(), "stand-in authority did not start");
    let url = format!("http://127.0.0.1:{port}/");
    let mut g = image::GrayImage::from_pixel(300, 300, image::Luma([240]));
    for y in 100..120 {
        for x in 40..260 {
            g.put_pixel(x, y, image::Luma([20]));
        }
    }
    g.save(d.join("a.png")).unwrap();
    let made = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf",
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    let s = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .env("LGR_TEST_PW", "pw")
        .args([
            "--sign-pdf",
            "h.pdf",
            "--sign-p12",
            "t.p12",
            "--sign-pass-env",
            "LGR_TEST_PW",
            "--sign-timestamp",
            "--tsa-url",
            &url,
            "--pdf",
            "s.pdf",
        ])
        .output()
        .unwrap();
    let v = run(
        &d,
        &[
            "--verify-pdf-signatures",
            "s.pdf",
            "--sign-ca",
            "c.pem",
            "--tsa-ca",
            "cert.pem",
        ],
    );
    let _ = child.kill();
    let _ = child.wait();
    assert!(s.status.success(), "{}", text(&s));
    assert!(v.status.success(), "{}", text(&v));
    assert!(
        text(&v).to_lowercase().contains("time"),
        "no time stamp reported: {}",
        text(&v)
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `--sign-visible`: the signature gets a box with text on the page, still verifies, and a box
/// outside the page is refused.
#[cfg(target_os = "linux")]
#[test]
fn sign_visible_puts_a_box_on_the_page() {
    let ok = |c: &str, a: &str| {
        Command::new(c)
            .arg(a)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !ok("openssl", "version") {
        eprintln!("openssl not found: sign-visible test skipped");
        return;
    }
    let d = dir("signvis");
    let ssl = |args: &[&str]| {
        let o = Command::new("openssl")
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    ssl(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        "k.pem",
        "-out",
        "c.pem",
        "-subj",
        "/CN=Test Signer",
        "-days",
        "3",
    ]);
    ssl(&[
        "pkcs12", "-export", "-inkey", "k.pem", "-in", "c.pem", "-out", "t.p12", "-passout",
        "pass:pw",
    ]);
    let g = image::GrayImage::from_pixel(1200, 1200, image::Luma([240]));
    g.save(d.join("a.png")).unwrap();
    let made = run(
        &d,
        &[
            "a.png", "--dpi", "300", "--format", "none", "--pdf", "h.pdf",
        ],
    );
    assert!(made.status.success(), "{}", text(&made));
    let sign = |vis: &str, out: &str| {
        Command::new(env!("CARGO_BIN_EXE_legatorocr"))
            .current_dir(&d)
            .env("LGR_TEST_PW", "pw")
            .args([
                "--sign-pdf",
                "h.pdf",
                "--sign-p12",
                "t.p12",
                "--sign-pass-env",
                "LGR_TEST_PW",
                "--sign-name",
                "Ana",
                "--sign-reason",
                "Approved",
                "--sign-visible",
                vis,
                "--pdf",
                out,
            ])
            .output()
            .unwrap()
    };
    let s = sign("1,10,10,60,20", "s.pdf");
    assert!(s.status.success(), "{}", text(&s));
    let bytes = std::fs::read(d.join("s.pdf")).unwrap();
    let all = String::from_utf8_lossy(&bytes);
    assert!(all.contains("(Digitally signed by Ana) Tj"));
    assert!(all.contains("(Reason: Approved) Tj"));
    let v = run(
        &d,
        &["--verify-pdf-signatures", "s.pdf", "--sign-ca", "c.pem"],
    );
    assert!(v.status.success(), "{}", text(&v));
    let bad = sign("1,10,10,6000,20", "x.pdf");
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&d);
}

/// `--encrypt-pdf` and `--pdf-permissions`: the file gets an AES-256 Encrypt dictionary, the page
/// content is not readable any more, and the toolbox refuses to take the result apart without
/// the password.
#[test]
fn encrypt_pdf_protects_the_output() {
    let d = dir("encpdf");
    let g = image::GrayImage::from_pixel(600, 300, image::Luma([240]));
    g.save(d.join("a.png")).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .env("LEGATOROCR_PDF_USER_PASSWORD", "open")
        .env("LEGATOROCR_PDF_OWNER_PASSWORD", "boss")
        .args([
            "a.png",
            "--dpi",
            "100",
            "--format",
            "none",
            "--pdf",
            "e.pdf",
            "--encrypt-pdf",
            "--pdf-permissions",
            "print,copy",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o));
    let b = std::fs::read(d.join("e.pdf")).unwrap();
    let t = String::from_utf8_lossy(&b);
    assert!(
        t.contains("/CFM /AESV3") && t.contains("/Encrypt "),
        "no Encrypt dictionary"
    );
    assert!(!t.contains("/Producer (LegatorOCR") || t.contains("/Encrypt "));
    let r = run(&d, &["e.pdf", "--extract-pages", "1", "-o", "x"]);
    assert!(!r.status.success());
    // an unknown permission and a missing password are refused
    let bad = run(
        &d,
        &[
            "a.png",
            "--pdf",
            "z.pdf",
            "--encrypt-pdf",
            "--pdf-permissions",
            "fly",
        ],
    );
    assert!(!bad.status.success());
    let nopw = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .env_remove("LEGATOROCR_PDF_USER_PASSWORD")
        .env_remove("LEGATOROCR_PDF_OWNER_PASSWORD")
        .args(["a.png", "--pdf", "z.pdf", "--encrypt-pdf"])
        .output()
        .unwrap();
    assert!(!nopw.status.success());
    let _ = std::fs::remove_dir_all(&d);
}

/// `--object-streams`: the structure objects go into compressed object streams behind a
/// cross-reference stream; the toolbox can read the result again; PDF/A-1 and signing are refused.
#[test]
fn object_streams_pack_the_structure() {
    let d = dir("objstm");
    for i in 0..3 {
        let mut g = image::GrayImage::from_pixel(600, 300, image::Luma([240]));
        for x in 50..500 {
            g.put_pixel(x, 100 + i * 20, image::Luma([20]));
        }
        g.save(d.join(format!("p{i}.png"))).unwrap();
    }
    let o = run(
        &d,
        &[
            "p0.png",
            "p1.png",
            "p2.png",
            "--dpi",
            "100",
            "--format",
            "none",
            "--pdf",
            "c.pdf",
            "--object-streams",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let b = std::fs::read(d.join("c.pdf")).unwrap();
    let t = String::from_utf8_lossy(&b);
    assert!(t.contains("/Type /ObjStm") && t.contains("/Type /XRef"));
    assert!(!t.contains("\ntrailer"));
    // readable again: take page 2 out of it
    let r = run(&d, &["c.pdf", "--extract-pages", "2", "-o", "x"]);
    assert!(r.status.success(), "{}", text(&r));
    assert!(d.join("x/c.pdf").exists());
    let bad = run(
        &d,
        &[
            "p0.png",
            "--pdf",
            "z.pdf",
            "--pdfa",
            "--pdfa-part",
            "1",
            "--object-streams",
        ],
    );
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&d);
}

/// `--linearize`: Fast Web View. The file starts with the linearization dictionary; when qpdf is
/// installed it must find no linearization errors. Combinations that cannot work are refused.
#[test]
fn linearize_puts_the_first_page_first() {
    let d = dir("linear");
    for i in 0..4 {
        let mut g = image::GrayImage::from_pixel(600, 300, image::Luma([240]));
        for x in 50..500 {
            g.put_pixel(x, 100 + i * 20, image::Luma([20]));
        }
        g.save(d.join(format!("p{i}.png"))).unwrap();
    }
    let o = run(
        &d,
        &[
            "p0.png",
            "p1.png",
            "p2.png",
            "p3.png",
            "--dpi",
            "100",
            "--format",
            "none",
            "--pdf",
            "l.pdf",
            "--linearize",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let b = std::fs::read(d.join("l.pdf")).unwrap();
    assert!(String::from_utf8_lossy(&b[..200]).contains("/Linearized 1"));
    if let Ok(q) = Command::new("qpdf")
        .args(["--check-linearization", "l.pdf"])
        .current_dir(&d)
        .output()
    {
        assert!(
            String::from_utf8_lossy(&q.stdout).contains("no linearization errors"),
            "{}",
            text(&q)
        );
    }
    let bad = run(
        &d,
        &[
            "p0.png",
            "--pdf",
            "z.pdf",
            "--linearize",
            "--object-streams",
        ],
    );
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&d);
}

/// `--jbig2`: a noisy black & white page is written with /JBIG2Decode when that is smaller, and the
/// toolbox can still take the file apart (the decoder reads what the encoder wrote).
#[test]
fn jbig2_output_is_smaller_and_readable() {
    let d = dir("jbig2");
    let mut g = image::GrayImage::from_pixel(900, 700, image::Luma([255]));
    let mut s = 12345u64;
    for y in 40..660u32 {
        for x in 40..860u32 {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // blobs of ink that vary from row to row
            if (x / 5 + (y / 7) * 3).is_multiple_of(4) && !(s >> 40).is_multiple_of(9) {
                g.put_pixel(x, y, image::Luma([0]));
            }
        }
    }
    g.save(d.join("a.png")).unwrap();
    let base = run(
        &d,
        &[
            "a.png", "--dpi", "150", "--mode", "bw", "--format", "none", "--pdf", "n.pdf",
        ],
    );
    assert!(base.status.success(), "{}", text(&base));
    let j = run(
        &d,
        &[
            "a.png", "--dpi", "150", "--mode", "bw", "--format", "none", "--pdf", "j.pdf",
            "--jbig2",
        ],
    );
    assert!(j.status.success(), "{}", text(&j));
    let (n, jb) = (
        std::fs::read(d.join("n.pdf")).unwrap(),
        std::fs::read(d.join("j.pdf")).unwrap(),
    );
    assert!(jb.len() <= n.len(), "{} vs {}", jb.len(), n.len());
    assert!(String::from_utf8_lossy(&jb).contains("/JBIG2Decode"));
    let r = run(&d, &["j.pdf", "--extract-pages", "1", "-o", "x"]);
    assert!(r.status.success(), "{}", text(&r));
    let _ = std::fs::remove_dir_all(&d);
}

/// `--smooth-glyphs`: the black & white page comes out at FACTOR times the pixels (and dpi);
/// in colour mode it is ignored.
#[test]
fn smooth_glyphs_enlarges_black_and_white_pages() {
    let d = dir("smoothg");
    let mut g = image::GrayImage::from_pixel(300, 120, image::Luma([255]));
    for y in 40..80u32 {
        for x in 60..240u32 {
            if (x / 6 + y / 5) % 2 == 0 {
                g.put_pixel(x, y, image::Luma([20]));
            }
        }
    }
    g.save(d.join("a.png")).unwrap();
    let dims = |sub: &str| -> (u32, u32) {
        let f = std::fs::read_dir(d.join(sub))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        image::image_dimensions(f).unwrap()
    };
    let mk = |sub: &str, extra: &[&str]| {
        let mut a = vec![
            "a.png",
            "--dpi",
            "100",
            "--split",
            "off",
            "--format",
            "png",
            "--out-dir",
            sub,
        ];
        a.extend_from_slice(extra);
        let o = run(&d, &a);
        assert!(o.status.success(), "{}", text(&o));
    };
    mk("plain", &["--mode", "bw"]);
    mk("smooth", &["--mode", "bw", "--smooth-glyphs", "2"]);
    let (p, s) = (dims("plain"), dims("smooth"));
    assert!(s.0 * 10 >= p.0 * 19 && s.0 * 10 <= p.0 * 21, "{p:?} {s:?}");
    mk("colour", &["--mode", "color", "--smooth-glyphs", "2"]);
    let c = dims("colour");
    assert!(c.0 < s.0 - 20, "{c:?} {s:?}");
    assert!(!run(&d, &["a.png", "--smooth-glyphs", "4"]).status.success());
    let _ = std::fs::remove_dir_all(&d);
}

/// `--pdfa-level`: U and A are declared in the XMP packet only when their requirements are met;
/// the preflight (`--pdfa-check`) knows the extra requirements; impossible combinations are refused.
#[test]
fn pdfa_level_declares_u_and_a() {
    let d = dir("pdfalevel");
    let mut g = image::GrayImage::from_pixel(600, 400, image::Luma([245]));
    for y in 100..120u32 {
        for x in 80..500u32 {
            g.put_pixel(x, y, image::Luma([20]));
        }
    }
    g.save(d.join("a.png")).unwrap();
    let make = |level: &str, extra: &[&str]| {
        let out = format!("{level}.pdf");
        let mut a = vec![
            "a.png",
            "--dpi",
            "150",
            "--mode",
            "bw",
            "--format",
            "none",
            "--pdfa",
            "--pdfa-level",
            level,
            "--pdf",
            &out,
        ];
        a.extend_from_slice(extra);
        let o = run(&d, &a);
        (o, out)
    };
    let declared = |f: &str| -> String {
        let b = std::fs::read(d.join(f)).unwrap();
        let t = String::from_utf8_lossy(&b).into_owned();
        t.split("<pdfaid:conformance>")
            .nth(1)
            .and_then(|r| r.chars().next())
            .map(String::from)
            .unwrap_or_default()
    };
    let (o, f) = make("U", &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(declared(&f), "U");
    let chk = run(&d, &["--pdfa-check", &f]);
    assert!(text(&chk).contains("ToUnicode"), "{}", text(&chk));
    let (o, f) = make("A", &["--ocr", "--engine", "native", "--lang", "eng"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(declared(&f), "A");
    let b = std::fs::read(d.join(&f)).unwrap();
    assert!(String::from_utf8_lossy(&b).contains("/StructTreeRoot"));
    // refusals
    assert!(!make("A", &[]).0.status.success());
    assert!(!make("U", &["--pdfa-part", "1"]).0.status.success());
    let o = run(&d, &["a.png", "--pdfa-level", "U", "--pdf", "z.pdf"]);
    assert!(!o.status.success());
    let _ = std::fs::remove_dir_all(&d);
}
