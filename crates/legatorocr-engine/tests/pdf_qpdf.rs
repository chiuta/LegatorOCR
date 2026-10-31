//! Cross-check the PDF reader on files rewritten by qpdf (object streams, QDF form).
use std::process::Command;

#[test]
fn reads_qpdf_object_streams() {
    if Command::new("qpdf").arg("--version").output().is_err() {
        eprintln!("qpdf not installed: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("lgr-qpdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut w = legatorocr_pdf::PdfWriter::new(legatorocr_pdf::Compress::Jpeg(85));
    for i in 0..4u8 {
        let img = image::RgbImage::from_pixel(100, 140, image::Rgb([200, 200 - i * 10, 120]));
        w.add_page(
            &legatorocr_core::Page {
                image: legatorocr_core::PageImage::Rgb(img),
                dpi: 150,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                layers: None,
            },
            &[],
        );
    }
    let src = dir.join("a.pdf");
    std::fs::write(&src, w.finish()).unwrap();
    for args in [
        vec!["--object-streams=generate"],
        vec!["--qdf"],
        vec!["--linearize"],
    ] {
        let out = dir.join("b.pdf");
        let st = Command::new("qpdf")
            .args(&args)
            .arg(&src)
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.code().is_some_and(|c| c == 0 || c == 3), "{args:?}");
        let pages = legatorocr_engine::files::collect_images(std::slice::from_ref(&out));
        assert_eq!(pages.len(), 4, "{args:?}");
        for p in &pages {
            assert_eq!(
                legatorocr_engine::load_image(p).unwrap().width(),
                100,
                "{args:?}"
            );
        }
        std::fs::remove_file(&out).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}
