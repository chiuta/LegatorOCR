//! `legatorocr collection …`: the whole loop through the real binary (no OCR engine needed:
//! the batch runs with --no-ocr; search is checked against a hand-made index).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .arg("collection")
        .args(args)
        .output()
        .unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn work(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lgr_cli_col_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn help_lists_the_subcommands() {
    let d = work("help");
    let o = bin(&d, &["--help"]);
    assert!(o.status.success());
    let t = out(&o);
    for w in ["new", "add", "list", "run", "search", ".lgrcol"] {
        assert!(t.contains(w), "{w} missing from help:\n{t}");
    }
    let o = bin(&d, &["add", "--help"]);
    assert!(out(&o).contains("scinteia_tineretului_1977_09_8801"));
    assert!(!bin(&d, &["bogus"]).status.success());
    // the main help points at it
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(out(&o).contains("legatorocr collection --help"));
}

#[test]
fn new_add_list_run_and_search() {
    let d = work("flow");
    std::fs::create_dir_all(d.join("scans/gazeta_1914_05_12")).unwrap();
    image::GrayImage::from_pixel(300, 400, image::Luma([255]))
        .save(d.join("scans/gazeta_1914_05_12/p1.png"))
        .unwrap();
    image::GrayImage::from_pixel(300, 400, image::Luma([255]))
        .save(d.join("scans/gazeta_1914_05_12/p2.png"))
        .unwrap();
    std::fs::write(d.join("scans/notes.txt"), "x").unwrap();
    let o = bin(&d, &["new", ".", "--name", "Ziare vechi"]);
    assert!(o.status.success(), "{}", err(&o));
    let f = d.join("ziare_vechi.lgrcol");
    assert!(f.is_file());
    assert!(
        !bin(&d, &["new", f.to_str().unwrap()]).status.success(),
        "no overwrite"
    );

    let o = bin(
        &d,
        &[
            "add",
            "ziare_vechi.lgrcol",
            "scans",
            "--folder",
            "--lang",
            "ro",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("1 item(s) added"), "{}", out(&o));
    assert!(out(&o).contains("skipped"));
    let o = bin(&d, &["add", "ziare_vechi.lgrcol", "scans", "--folder"]);
    assert!(out(&o).contains("already in the collection"));

    let o = bin(&d, &["list", "ziare_vechi.lgrcol"]);
    let t = out(&o);
    assert!(
        t.contains("Gazeta") && t.contains("1914-05-12") && t.contains("todo"),
        "{t}"
    );
    assert!(t.contains("2p"), "{t}");
    assert!(std::fs::read_to_string(&f)
        .unwrap()
        .contains("\"scans/gazeta_1914_05_12\""));

    // a second item that cannot be processed does not stop the first
    let o = bin(&d, &["add", "ziare_vechi.lgrcol", "scans/notes.txt"]);
    assert!(!o.status.success());
    std::fs::write(d.join("broken.pdf"), b"not a pdf at all").unwrap();
    assert!(bin(&d, &["add", "ziare_vechi.lgrcol", "broken.pdf"])
        .status
        .success());

    let o = bin(&d, &["run", "ziare_vechi.lgrcol", "--no-ocr"]);
    assert_eq!(o.status.code(), Some(2), "{} {}", out(&o), err(&o));
    assert!(out(&o).contains("1 processed, 1 failed"), "{}", out(&o));
    assert!(err(&o).contains("FAILED"));
    assert!(d.join("gazeta_1914_05_12.pdf").is_file());
    let t = out(&bin(
        &d,
        &["list", "ziare_vechi.lgrcol", "--status", "processed"],
    ));
    assert!(t.contains("Gazeta") && !t.contains("Broken"), "{t}");
    // nothing pending now except the failed one, which is not retried unasked
    let o = bin(&d, &["run", "ziare_vechi.lgrcol", "--no-ocr"]);
    assert!(out(&o).contains("nothing to do"), "{}", out(&o));
    assert!(!bin(&d, &["run", "ziare_vechi.lgrcol", "--item", "i99"])
        .status
        .success());

    // search over a hand-made index
    std::fs::write(
        d.join("ziare_vechi.lgrindex"),
        "{\"format\":\"lgrindex\",\"version\":1,\"items\":[{\"id\":\"i1\",\"pages\":[\"nimic\",\"Ziarul Scânteia anunță\"]}]}",
    )
    .unwrap();
    let o = bin(&d, &["search", "ziare_vechi.lgrcol", "Scînteia"]);
    let t = out(&o);
    assert!(
        t.contains("p. 2") && t.contains("Scânteia anunță") && t.contains("1 hit(s)"),
        "{t}"
    );

    // damaged file: clear error, no panic
    std::fs::write(&f, "{\"items\":[").unwrap();
    let o = bin(&d, &["list", "ziare_vechi.lgrcol"]);
    assert!(!o.status.success());
    let e = err(&o);
    assert!(
        e.contains("damaged") && e.contains("ziare_vechi.lgrcol") && !e.contains("panicked"),
        "{e}"
    );
}
