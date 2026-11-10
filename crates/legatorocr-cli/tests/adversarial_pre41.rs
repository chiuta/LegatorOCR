//! Hostile command-line values for the options of pre.41. Each must end in a clear error (exit 1
//! with a message naming the option), never a panic (exit 101) and never a half-written output.
use std::process::{Command, Output};

struct Tmp(std::path::PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("lgr_adv41_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn page(dir: &std::path::Path) {
    let mut g = image::GrayImage::from_pixel(800, 600, image::Luma([255]));
    for y in (60..540).step_by(26) {
        for x in 60..740 {
            for dy in 0..10 {
                g.put_pixel(x, y + dy, image::Luma([20]));
            }
        }
    }
    g.save(dir.join("in.png")).unwrap();
}

fn run(dir: &std::path::Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(dir)
        .env("RUST_BACKTRACE", "0")
        .args(["in.png", "--out-dir", "out"])
        .args(extra)
        .output()
        .unwrap()
}

fn err_text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn hostile_values_end_in_clear_errors() {
    let d = Tmp::new(&format!("{}", line!()));
    page(d.path());
    let cases: &[(&[&str], &str)] = &[
        (&["--cut-angle", "nan"], "cut-angle"),
        (&["--cut-angle", "inf"], "cut-angle"),
        (&["--cut-angle=-inf"], "cut-angle"),
        (&["--cut-angle", "20.5"], "cut-angle"),
        (&["--cut-angle=-20.5"], "cut-angle"),
        (&["--cut-angle", "1e39"], "cut-angle"),
        (&["--keystone-v", "30.1"], "keystone"),
        (&["--keystone-h=-30.1"], "keystone"),
        (&["--keystone-v", "nan"], "keystone"),
        (&["--keystone-h", "inf"], "keystone"),
        (&["--keystone-v", "1e39", "--keystone-h", "0"], "keystone"),
        (&["--page-size", "0x0"], "page-size"),
        (&["--page-size", "9.9x100"], "page-size"),
        (&["--page-size", "100x2000.1"], "page-size"),
        (&["--page-size", "nanxnan"], "page-size"),
        (&["--page-size", "infxinf"], "page-size"),
        (&["--page-size", "a99"], "page size"),
        (&["--page-size", "A4x"], "page-size"),
        (&["--page-size", "x"], "page-size"),
        (&["--page-size", "1x2x3"], "page-size"),
        (&["--page-size", "ａ４"], "page"),
        (&["--ocr", "--ocr-zone", "#table"], "ocr-zone"),
        (&["--ocr", "--ocr-zone", "1,2,3,4#table#table"], "ocr-zone"),
        (&["--ocr", "--ocr-zone", "1,2,0,4#table"], "ocr-zone"),
        (&["--ocr", "--ocr-zone", "nan,1,2,3#table"], "ocr-zone"),
        (&["--ocr", "--ocr-zone", "1,2,3#table"], "ocr-zone"),
        (&["--ocr", "--ocr-ignore", "1,2,3,4#table"], "ocr-ignore"),
    ];
    for (args, word) in cases {
        let o = run(d.path(), args);
        let e = err_text(&o);
        assert_eq!(
            o.status.code(),
            Some(1),
            "{args:?}: exit {:?}, stderr {e}",
            o.status.code()
        );
        assert!(!e.contains("panicked"), "{args:?}: {e}");
        assert!(
            e.contains(word),
            "{args:?}: message does not name the option: {e}"
        );
        let wrote = std::fs::read_dir(d.path().join("out"))
            .map(|r| r.count())
            .unwrap_or(0);
        assert_eq!(wrote, 0, "{args:?}: wrote an output");
    }
}

#[test]
fn good_values_at_the_edges_work() {
    let d = Tmp::new(&format!("{}", line!()));
    page(d.path());
    for args in [
        vec![
            "--cut-angle",
            "20",
            "--keystone-v",
            "30",
            "--keystone-h",
            "-30",
        ],
        vec!["--cut-angle=-20", "--split", "middle"],
        vec!["--cut-angle=-0", "--keystone-v", "0.04"],
        vec!["--page-size", "A2"],
        vec!["--page-size", " b6 "],
        vec!["--page-size", "TABLOID"],
        vec!["--page-size", "10x2000"],
        vec!["--page-size", "2000X10"],
        vec![
            "--ocr",
            "--ocr-zone",
            "20,20,100,50=ron#TABLE",
            "--ocr-ignore",
            "0,0,5,5",
        ],
    ] {
        let o = run(d.path(), &args);
        assert!(o.status.success(), "{args:?}: {}", err_text(&o));
    }
}

#[test]
fn a_saved_project_keeps_the_new_settings() {
    let d = Tmp::new(&format!("{}", line!()));
    page(d.path());
    let o = run(
        d.path(),
        &[
            "--cut-angle",
            "7.5",
            "--keystone-v",
            "2",
            "--keystone-h=-1.5",
            "--save-project",
            "p.lgr",
        ],
    );
    assert!(o.status.success(), "{}", err_text(&o));
    let text = std::fs::read_to_string(d.path().join("p.lgr")).unwrap();
    assert!(
        text.contains("cut_angle = 7.5") && text.contains("keystone = 2,-1.5"),
        "{text}"
    );
    // a project edited into nonsense is refused with the line number, not a panic
    let bad = text.replace("keystone = 2,-1.5", "keystone = nan,1");
    std::fs::write(d.path().join("bad.lgr"), bad).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(d.path())
        .env("RUST_BACKTRACE", "0")
        .args(["--project", "bad.lgr", "--out-dir", "out2"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", err_text(&o));
    assert!(
        !err_text(&o).contains("panicked") && err_text(&o).contains("line"),
        "{}",
        err_text(&o)
    );
    // and the untouched project loads and gives the same settings back
    let o = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(d.path())
        .env("RUST_BACKTRACE", "0")
        .args([
            "--project",
            "p.lgr",
            "--out-dir",
            "out3",
            "--save-project",
            "p2.lgr",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err_text(&o));
    let again = std::fs::read_to_string(d.path().join("p2.lgr")).unwrap();
    assert!(
        again.contains("cut_angle = 7.5") && again.contains("keystone = 2,-1.5"),
        "{again}"
    );
}
