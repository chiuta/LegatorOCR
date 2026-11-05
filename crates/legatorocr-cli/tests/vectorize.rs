//! `--vectorize`: a ring and a dot on a white page become two outlines plus a hole in an SVG.
use std::process::Command;

#[test]
fn a_page_becomes_an_svg_with_the_outlines() {
    let dir = std::env::temp_dir().join(format!("lor-vec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::GrayImage::from_fn(400, 400, |x, y| {
        let r = ((x as f64 - 200.0).powi(2) + (y as f64 - 200.0).powi(2)).sqrt();
        image::Luma([if (80.0..140.0).contains(&r) || r < 20.0 {
            0
        } else {
            255
        }])
    });
    let png = dir.join("p.png");
    img.save(&png).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .args([png.to_str().unwrap(), "--dpi", "254", "--vectorize"])
        .arg(dir.join("svg"))
        .args(["--pdf"])
        .arg(dir.join("x.pdf"))
        .args(["-o"])
        .arg(dir.join("pages"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let svg = std::fs::read_to_string(dir.join("svg/page_001.svg")).unwrap();
    // ring outer + ring inner + dot = 3 outlines (the page may be cropped to the content)
    assert!(svg.contains("3 outlines"), "{}", &svg[..svg.len().min(400)]);
    assert!(svg.contains("fill-rule=\"evenodd\"") && svg.contains("viewBox"));
    let _ = std::fs::remove_dir_all(&dir);
}
