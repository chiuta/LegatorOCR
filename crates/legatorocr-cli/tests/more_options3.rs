//! Third batch: options that need recognised text. A tiny shell script stands in for Tesseract (the external-engine
//! hook speaks Tesseract's command line), so the words are known exactly. Unix only (the stand-in is a `sh` script).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ENGINE: &str = r#"#!/bin/sh
case "$*" in *--version*) echo "fake-engine 1.0"; exit 0;; esac
case "$*" in *--list-langs*) printf 'List of available languages (1):\nron\n'; exit 0;; esac
cat >/dev/null
printf 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
i=0
for w in sînt ion@example.com cînd; do i=$((i+1)); printf '5\t1\t1\t1\t1\t%s\t%s\t10\t100\t30\t90\t%s\n' $i $((i*120)) "$w"; done
"#;

fn setup(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_more3_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    image::GrayImage::from_pixel(600, 800, image::Luma([240]))
        .save(d.join("a.png"))
        .unwrap();
    let e = d.join("eng.sh");
    std::fs::write(&e, ENGINE).unwrap();
    std::fs::set_permissions(&e, std::fs::Permissions::from_mode(0o755)).unwrap();
    d
}

fn ocr(dir: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "a.png",
        "--dpi",
        "300",
        "--format",
        "none",
        "--ocr",
        "--engine",
        "tesseract",
        "--tesseract",
        "./eng.sh",
        "--lang",
        "ron",
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
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

#[test]
fn the_stand_in_engine_text_reaches_the_text_file() {
    let d = setup("base");
    let o = ocr(&d, &["--txt", "o.txt"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(
        std::fs::read_to_string(d.join("o.txt")).unwrap().trim(),
        "sînt ion@example.com cînd"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ocr_modern_spelling_rewrites_old_forms() {
    let d = setup("modern");
    let o = ocr(&d, &["--ocr-modern-spelling", "--txt", "m.txt"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(
        std::fs::read_to_string(d.join("m.txt")).unwrap().trim(),
        "sunt ion@example.com când"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn redact_regex_removes_the_matching_text() {
    let d = setup("redact");
    let o = ocr(&d, &["--redact-regex", r"ion@\S+", "--txt", "r.txt"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = std::fs::read_to_string(d.join("r.txt")).unwrap();
    assert!(
        !t.contains("ion@") && t.contains("sînt") && t.contains("cînd"),
        "{t}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn redact_reports_what_each_pattern_covered_and_warns_when_one_matched_nothing() {
    let d = setup("redactrep");
    let o = ocr(
        &d,
        &[
            "--redact-regex",
            r"ion@\S+",
            "--redact-regex",
            "zzzqqq",
            "--txt",
            "r.txt",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("redaction:") && t.contains("1 covered"), "{t}");
    assert!(t.contains("matched nothing"), "{t}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn passages_title_is_used_in_the_cite_string() {
    let d = setup("passages");
    let o = ocr(
        &d,
        &[
            "--pdf",
            "q.pdf",
            "--passages-jsonl",
            "p.jsonl",
            "--passages-title",
            "Carte",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let j = std::fs::read_to_string(d.join("p.jsonl")).unwrap();
    assert!(j.contains("\"cite\":\"Carte p.1\""), "{j}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn fff_title_and_year_land_in_the_envelope() {
    let d = setup("fff");
    let o = ocr(
        &d,
        &[
            "--fff-out",
            "f.fff.json",
            "--fff-title",
            "Titlu Test",
            "--fff-year",
            "1975",
        ],
    );
    assert!(o.status.success(), "{}", text(&o));
    let j = std::fs::read_to_string(d.join("f.fff.json")).unwrap();
    assert!(
        j.contains("\"title\":\"Titlu Test\"") && j.contains("\"year\":\"1975\""),
        "{j}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn tessdata_directory_without_the_language_makes_ocr_fail() {
    let d = setup("tessdata");
    let empty = d.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .env("RUST_BACKTRACE", "0")
        .args([
            "a.png",
            "--dpi",
            "300",
            "--format",
            "none",
            "--ocr",
            "--engine",
            "tesseract",
            "--lang",
            "ron",
            "--tessdata",
            "empty",
            "--txt",
            "n.txt",
        ])
        .output()
        .unwrap();
    let t = text(&o);
    // a machine without Tesseract fails for another reason, so only the failure itself is pinned there
    assert!(!o.status.success(), "{t}");
    assert!(!d.join("n.txt").exists(), "{t}");
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn install_modules_prints_commands_and_runs_them_only_with_yes() {
    use std::os::unix::fs::PermissionsExt;
    // a fake brew first (no sudo involved, so the test does not depend on who runs it) on PATH: records what it was asked and fails for one package
    let dir = std::env::temp_dir().join(format!("lor-inst-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("log.txt");
    let fake = dir.join("brew");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\necho \"$@\" | grep -q mupdf && exit 100\nexit 0\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_legatorocr"))
            .args(args)
            .env("PATH", &path)
            .output()
            .unwrap()
    };
    // dry run: commands shown, nothing executed
    let o = run(&["--install-modules", "qpdf", "--package-manager", "brew"]);
    let t = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success());
    assert!(
        t.contains("brew install qpdf") && t.contains("Nothing was run"),
        "{t}"
    );
    assert!(!log.exists());
    // --yes alone is refused (it belongs to --install-modules)
    assert!(!run(&["--yes"]).status.success());
    // with --yes: runs, a failing package is reported, the later step still runs, exit 1
    let o = run(&[
        "--install-modules",
        "renderers,qpdf",
        "--yes",
        "--package-manager",
        "brew",
    ]);
    let t = String::from_utf8_lossy(&o.stdout);
    let ran = std::fs::read_to_string(&log).unwrap();
    assert!(ran.contains("install qpdf"), "{ran}");
    assert!(t.contains("[FAILED]") && t.contains("mupdf-tools"), "{t}");
    assert_eq!(o.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}
