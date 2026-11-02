//! --djvu: the file is a DjVu document; when DjVuLibre is installed it must accept it and
//! decode the black-and-white page to exactly the pixels we wrote.

use std::process::Command;

#[test]
fn djvu_output_is_accepted_by_djvulibre_and_bw_is_lossless() {
    let d = std::env::temp_dir().join(format!("lgr_djvu_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut img = image::GrayImage::from_pixel(400, 300, image::Luma([255]));
    for y in 60..70 {
        for x in 40..300 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    for y in 120..200 {
        for x in 100..110 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(d.join("a.png")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(&d)
        .args(["a.png", "a.png", "--dpi", "300", "--mode", "bw", "--djvu", "o.djvu", "-o", "pages"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(d.join("o.djvu")).unwrap();
    assert_eq!(&bytes[..8], b"AT&TFORM");
    assert_eq!(&bytes[12..16], b"DJVM", "two pages are bundled");
    if Command::new("djvudump").arg("--help").output().is_ok() {
        let dump = Command::new("djvudump").arg(d.join("o.djvu")).output().unwrap();
        assert!(dump.status.success(), "{}", String::from_utf8_lossy(&dump.stderr));
        let text = String::from_utf8_lossy(&dump.stdout);
        assert!(text.contains("2 pages") && text.contains("JB2 bilevel data"), "{text}");
        let ok = Command::new("ddjvu").args(["-format=pbm", "-page=1"]).arg(d.join("o.djvu")).arg(d.join("p1.pbm")).status().unwrap();
        assert!(ok.success());
        let pbm = std::fs::read(d.join("p1.pbm")).unwrap();
        // P4 raw bitmap: "P4\nW H\n" then packed rows, 1 = black; compared with the page image the
        // pipeline itself wrote (the page is cropped to its content, so its size is not the input's)
        let nl: Vec<usize> = pbm.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i).take(2).collect();
        let dims = String::from_utf8_lossy(&pbm[nl[0] + 1..nl[1]]).to_string();
        let (w, h): (usize, usize) = {
            let mut it = dims.split_whitespace().map(|v| v.parse::<usize>().unwrap());
            (it.next().unwrap(), it.next().unwrap())
        };
        let rows = &pbm[nl[1] + 1..];
        let stride = w.div_ceil(8);
        let page = std::fs::read_dir(d.join("pages")).unwrap().map(|e| e.unwrap().path()).min().unwrap();
        let src = image::open(&page).unwrap().to_luma8();
        assert_eq!((src.width() as usize, src.height() as usize), (w, h));
        let mut wrong = 0;
        for y in 0..h {
            for x in 0..w {
                let black = rows[y * stride + x / 8] >> (7 - x % 8) & 1 == 1;
                wrong += (black != (src.get_pixel(x as u32, y as u32)[0] < 128)) as u32;
            }
        }
        assert_eq!(wrong, 0, "lossless bilevel page");
    }
    let _ = std::fs::remove_dir_all(&d);
}
