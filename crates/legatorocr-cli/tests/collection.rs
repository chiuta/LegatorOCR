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
    for w in [
        "new", "add", "list", "run", "search", "set", "remove", "reindex", ".lgrcol",
    ] {
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

/// A one-page-per-entry PDF with a text layer (as the engine writes after OCR).
fn text_pdf(pages: &[&[&str]]) -> Vec<u8> {
    use legatorocr_core::{text::Word, Page, PageImage};
    use legatorocr_pdf::{Compress, PdfWriter};
    let mut w = PdfWriter::new(Compress::Flate);
    for words in pages {
        let page = Page {
            image: PageImage::Rgb(image::RgbImage::new(200, 100)),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        };
        let ws: Vec<Word> = words
            .iter()
            .enumerate()
            .map(|(i, t)| Word {
                text: (*t).into(),
                x: 10,
                y: 5 + 30 * i as u32,
                w: 80,
                h: 18,
                conf: 90.0,
                line: i as u32,
            })
            .collect();
        w.add_page(&page, &ws);
    }
    w.finish()
}

#[test]
fn set_changes_the_given_fields_only_and_refuses_bad_values() {
    let d = work("set");
    std::fs::write(d.join("scinteia_tineretului_1977_09_8801.pdf"), b"x").unwrap();
    assert!(bin(&d, &["new", ".", "--name", "C"]).status.success());
    let f = "c.lgrcol";
    assert!(
        bin(&d, &["add", f, "scinteia_tineretului_1977_09_8801.pdf"])
            .status
            .success()
    );
    let o = bin(
        &d,
        &[
            "set",
            f,
            "i1",
            "--title",
            "Scânteia tineretului",
            "--issue",
            "8802",
            "--date",
            "1977-09-02",
            "--kind",
            "magazine",
            "--status",
            "checked",
            "--tags",
            "tineret, ziar,,tineret",
            "--notes",
            "verificat de mână",
        ],
    );
    assert!(o.status.success(), "{}", err(&o));
    let t = out(&bin(&d, &["list", f]));
    assert!(
        t.contains("Scânteia tineretului")
            && t.contains("[1977-09-02 · no. 8802]")
            && t.contains("checked"),
        "{t}"
    );
    let saved = std::fs::read_to_string(d.join(f)).unwrap();
    assert!(
        saved.contains("\"magazine\"") && saved.contains("verificat de mână"),
        "{saved}"
    );
    assert!(
        saved.contains("\"tineret\",\"ziar\"") || saved.contains("\"tineret\", \"ziar\""),
        "{saved}"
    );
    // a change of status back to todo and a cleared field
    assert!(
        bin(&d, &["set", f, "i1", "--status", "todo", "--issue", ""])
            .status
            .success()
    );
    let t = out(&bin(&d, &["list", f, "--status", "todo"]));
    assert!(
        t.contains("Scânteia tineretului") && !t.contains("no. 8802"),
        "{t}"
    );
    // bad values change nothing and say why
    let before = std::fs::read_to_string(d.join(f)).unwrap();
    for (args, why) in [
        (
            vec!["set", f, "i1", "--status", "gata", "--title", "X"],
            "unknown status",
        ),
        (vec!["set", f, "i1", "--kind", "ziar"], "unknown kind"),
        (vec!["set", f, "i9", "--title", "X"], "no item 'i9'"),
        (vec!["set", f, "i1"], "nothing to change"),
        (
            vec!["set", f, "i1", "--profile", "nu-exista-profil-x"],
            "profile",
        ),
    ] {
        let o = bin(&d, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(err(&o).contains(why), "{args:?}: {}", err(&o));
    }
    assert_eq!(std::fs::read_to_string(d.join(f)).unwrap(), before);
}

#[test]
fn remove_takes_the_item_out_of_the_collection_and_the_index_only() {
    let d = work("remove");
    std::fs::write(d.join("a_1950.pdf"), text_pdf(&[&["Argeș"]])).unwrap();
    std::fs::write(d.join("b_1951.pdf"), text_pdf(&[&["Banat"]])).unwrap();
    let f = "c.lgrcol";
    assert!(bin(&d, &["new", ".", "--name", "C"]).status.success());
    assert!(bin(&d, &["add", f, "a_1950.pdf", "b_1951.pdf"])
        .status
        .success());
    let o = bin(&d, &["reindex", f]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("2 indexed"), "{}", out(&o));
    assert!(out(&bin(&d, &["search", f, "argeș"])).contains("1 hit(s)"));
    let o = bin(&d, &["remove", f, "i1"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("1 item(s) removed"));
    let t = out(&bin(&d, &["list", f]));
    assert!(
        t.contains("1 item(s)") && t.contains("B 1951") || t.contains("B"),
        "{t}"
    );
    assert!(!t.contains("A 1950"), "{t}");
    assert!(out(&bin(&d, &["search", f, "argeș"])).contains("0 hit(s)"));
    assert!(out(&bin(&d, &["search", f, "banat"])).contains("1 hit(s)"));
    assert!(d.join("a_1950.pdf").is_file(), "the PDF is left alone");
    // unknown item: error, nothing removed
    let o = bin(&d, &["remove", f, "i2", "i7"]);
    assert!(
        !o.status.success() && err(&o).contains("no item 'i7'"),
        "{}",
        err(&o)
    );
    assert!(out(&bin(&d, &["list", f])).contains("1 item(s)"));
}

#[test]
fn reindex_reads_the_text_layer_of_a_source_pdf_without_processing() {
    let d = work("reindex");
    std::fs::write(
        d.join("carte_veche_1938.pdf"),
        text_pdf(&[&["Argeș"], &["Ornamente", "populare"], &["Scânteia"]]),
    )
    .unwrap();
    std::fs::write(d.join("scan_1950.pdf"), text_pdf(&[&[], &[]])).unwrap();
    let f = "c.lgrcol";
    assert!(bin(&d, &["new", ".", "--name", "C"]).status.success());
    assert!(
        bin(&d, &["add", f, "carte_veche_1938.pdf", "scan_1950.pdf"])
            .status
            .success()
    );
    let o = bin(&d, &["search", f, "ornamente"]);
    assert!(
        out(&o).contains("0 hit(s)") && err(&o).contains("reindex"),
        "{} {}",
        out(&o),
        err(&o)
    );
    let o = bin(&d, &["reindex", f]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("1 indexed (3 pages), 1 without text, 0 failed"),
        "{}",
        out(&o)
    );
    assert!(err(&o).contains("no text layer"), "{}", err(&o));
    let o = bin(&d, &["search", f, "ornamente populare"]);
    let t = out(&o);
    assert!(t.contains("p. 2") && t.contains("1 hit(s)"), "{t}");
    assert!(out(&bin(&d, &["search", f, "Scînteia"])).contains("p. 3"));
    // nothing was processed: still to do, no PDF made, index next to the collection
    let t = out(&bin(&d, &["list", f, "--status", "todo"]));
    assert!(t.contains("Carte Veche") && t.contains("Scan"), "{t}");
    assert!(d.join("c.lgrindex").is_file() && !d.join("carte_veche_1938_1.pdf").exists());
    // one item by id, the alias, and the errors
    let o = bin(&d, &["index", f, "--item", "i1"]);
    assert!(
        o.status.success() && out(&o).contains("1 indexed"),
        "{}",
        out(&o)
    );
    let o = bin(&d, &["reindex", f, "--item", "i9"]);
    assert!(!o.status.success() && err(&o).contains("no item 'i9'"));
    std::fs::write(d.join("scan_1950.pdf"), b"damaged").unwrap();
    let o = bin(&d, &["reindex", f, "--item", "i2"]);
    assert_eq!(o.status.code(), Some(2), "{} {}", out(&o), err(&o));
    assert!(
        err(&o).contains("FAILED") && err(&o).contains("not a readable PDF"),
        "{}",
        err(&o)
    );
}
