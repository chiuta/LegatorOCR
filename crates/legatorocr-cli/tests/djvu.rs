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

#[test]
fn shards_cover_the_list_in_order_and_join_into_the_full_document() {
    let d = std::env::temp_dir().join(format!("lgr_shard_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // five different pages (a bar of different width on each) so order is visible
    for k in 0..5u32 {
        let mut img = image::GrayImage::from_pixel(300, 200, image::Luma([255]));
        for y in 80..90 {
            for x in 20..(60 + 40 * k) {
                img.put_pixel(x, y, image::Luma([0]));
            }
        }
        img.save(d.join(format!("p{k}.png"))).unwrap();
    }
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_legatorocr")).current_dir(&d).args(args).args(["--dpi", "300", "--mode", "bw"]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stderr).to_string()
    };
    let files = ["p0.png", "p1.png", "p2.png", "p3.png", "p4.png"];
    let mut a = vec!["--shard", "1/2", "--pdf", "s1.pdf", "-o", "o1"];
    a.extend(files);
    let mut b = vec!["--shard", "2/2", "--pdf", "s2.pdf", "-o", "o2"];
    b.extend(files);
    let (la, lb) = (run(&a), run(&b));
    assert!(la.contains("pages 1..2 of 5") && lb.contains("pages 3..5 of 5"), "{la} {lb}");
    // outputs of the two shards are exactly the pages of the unsharded run
    let mut full = vec!["--pdf", "all.pdf", "-o", "o0"];
    full.extend(files);
    run(&full);
    let names = |o: &str| {
        let mut v: Vec<String> = std::fs::read_dir(d.join(o)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };
    let mut joined = names("o1");
    joined.extend(names("o2"));
    assert_eq!(joined, names("o0"));
    for n in names("o0") {
        let whole = std::fs::read(d.join("o0").join(&n)).unwrap();
        let part = std::fs::read(d.join(if names("o1").contains(&n) { "o1" } else { "o2" }).join(&n)).unwrap();
        assert_eq!(whole, part, "{n}");
    }
    let bad = Command::new(env!("CARGO_BIN_EXE_legatorocr")).current_dir(&d).args(["--shard", "3/2", "p0.png", "--pdf", "x.pdf"]).output().unwrap();
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&d);
}
