//! Input-file helpers shared by the CLI and the GUI.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "pdf"];

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// "page2" < "page10": digit runs compare numerically.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                let mut nb = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let (na, nb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Image files from a mix of files and folders (folders are not recursed), natural order.
/// `scan.tif#3` names page 3 (1-based) of a multi-page TIFF. A real file with that exact
/// name always wins, so odd file names keep working.
pub fn split_frame(path: &Path) -> (PathBuf, Option<u32>) {
    if !path.exists() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some((base, n)) = name.rsplit_once('#') {
                if let Ok(n) = n.parse::<u32>() {
                    let b = path.with_file_name(base);
                    if n >= 1 && b.is_file() {
                        return (b, Some(n));
                    }
                }
            }
        }
    }
    (path.to_path_buf(), None)
}

fn is_tiff(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tif") || e.eq_ignore_ascii_case("tiff"))
}

/// Number of pages (IFDs) in a TIFF; 1 when it cannot be read as one.
pub fn tiff_frame_count(path: &Path) -> u32 {
    let Ok(f) = std::fs::File::open(path) else {
        return 1;
    };
    let Ok(mut d) = tiff::decoder::Decoder::new(std::io::BufReader::new(f)) else {
        return 1;
    };
    let mut n = 1;
    while d.more_images() && n < 100_000 {
        if d.next_image().is_err() {
            break;
        }
        n += 1;
    }
    n
}

pub fn is_pdf(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// A multi-page TIFF or a PDF becomes one entry per page (`scan.tif#1`, `book.pdf#2`, …).
fn expand(p: &Path, out: &mut Vec<PathBuf>) {
    let n = if is_tiff(p) {
        tiff_frame_count(p)
    } else if is_pdf(p) {
        crate::pdf_page_count(p)
    } else {
        1
    };
    if n > 1 {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.extend((1..=n).map(|i| p.with_file_name(format!("{name}#{i}"))));
    } else {
        out.push(p.to_path_buf());
    }
}

/// Pages of the images and PDFs inside an archive, in natural order of their member path.
/// An unreadable archive contributes nothing (the reason goes to stderr).
fn from_archive(p: &Path, out: &mut Vec<PathBuf>) {
    let want = |n: &str| {
        // a member needs a real extension: a file literally called "pdf" or "png" is not an image
        let base = n.rsplit(['/', '\\']).next().unwrap_or("");
        base.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty() && IMAGE_EXT.contains(&ext.to_ascii_lowercase().as_str())
        })
    };
    match crate::archive::extract(p, &want) {
        Ok(mut v) => {
            v.sort_by(|a, b| natural_cmp(&a.0, &b.0));
            for (_, f) in v {
                expand(&f, out);
            }
        }
        Err(e) => eprintln!("{}: {e:#}", p.display()),
    }
}

pub fn collect_images(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_file() && crate::archive::is_archive(p) {
            from_archive(p, &mut out);
        } else if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p)
                .map(|rd| {
                    rd.filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| is_image(p) || crate::archive::is_archive(p))
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
            for f in v {
                if crate::archive::is_archive(&f) {
                    from_archive(&f, &mut out);
                } else {
                    expand(&f, &mut out);
                }
            }
        } else if split_frame(p).1.is_some() {
            out.push(p.clone()); // an explicit page of a multi-page TIFF or PDF
        } else if is_image(p) {
            expand(p, &mut out);
        }
    }
    out
}

/// Resolution stored in a PNG (`pHYs`) or JPEG (JFIF) header, if it looks like a
/// real scan resolution. 150–1200 dpi is accepted; 50–149 only when the implied page size
/// is plausible and it is not 72/96 dpi (what screenshots and many editors write by default).
pub fn detect_dpi(path: &Path) -> Option<u32> {
    use std::io::Read;
    let (path, frame) = &split_frame(path);
    if is_pdf(path) {
        let doc = crate::pdf_doc(path).ok()?;
        let dpi = doc.page_info(frame.unwrap_or(1) as usize - 1).ok()?.dpi?;
        return (150..=1200).contains(&dpi).then_some(dpi);
    }
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut head)
        .ok()?;
    let dpi = dpi_from_header(&head)?;
    if (150..=1200).contains(&dpi) {
        return Some(dpi);
    }
    // Low resolutions (e.g. a 100 dpi A4 export) are real too, but 72 and 96 are editor
    // defaults, so accept the 50–149 range only when the physical page size is plausible.
    if (50..150).contains(&dpi) && dpi != 72 && dpi != 96 {
        let (w, h) = image::image_dimensions(path).ok()?;
        return plausible_page(w, h, dpi).then_some(dpi);
    }
    None
}

/// Paper size in millimetres from a name (`A3`…`A6`, `B5`, `Letter`, `Legal`) or `WIDTHxHEIGHT`.
pub fn paper_mm(name: &str) -> Option<(f64, f64)> {
    let n = name.trim().to_ascii_lowercase();
    let known = match n.as_str() {
        "a3" => Some((297.0, 420.0)),
        "a4" => Some((210.0, 297.0)),
        "a5" => Some((148.0, 210.0)),
        "a6" => Some((105.0, 148.0)),
        "b5" => Some((176.0, 250.0)),
        "letter" => Some((215.9, 279.4)),
        "legal" => Some((215.9, 355.6)),
        _ => None,
    };
    if known.is_some() {
        return known;
    }
    let (w, h) = n.split_once('x')?;
    let (w, h): (f64, f64) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    ((30.0..=2000.0).contains(&w) && (30.0..=2000.0).contains(&h)).then_some((w, h))
}

/// Resolution of a photograph that has none (phone cameras write no usable dpi): the shot is
/// assumed to be framed on a page of `paper`, so the long side of the picture is the long side
/// of the paper. Only an estimate, good to the margin left around the page (say 10–15 %); the
/// result is kept within 100–1200 dpi.
pub fn dpi_from_paper(path: &Path, paper: (f64, f64)) -> Option<u32> {
    let (path, _) = &split_frame(path);
    let (w, h) = image::image_dimensions(path).ok()?;
    let long_px = w.max(h) as f64;
    let long_mm = paper.0.max(paper.1);
    let dpi = (long_px / (long_mm / 25.4)).round();
    dpi.is_finite().then(|| (dpi as u32).clamp(100, 1200))
}

/// Is `w`×`h` pixels at `dpi` a believable sheet of paper (2–24 in, at most ~2:1 beyond A-series)?
fn plausible_page(w: u32, h: u32, dpi: u32) -> bool {
    let (a, b) = (w as f64 / dpi as f64, h as f64 / dpi as f64);
    let (short, long) = if a < b { (a, b) } else { (b, a) };
    (2.0..=24.0).contains(&short) && (3.0..=36.0).contains(&long)
}

fn dpi_from_header(b: &[u8]) -> Option<u32> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut i = 8;
        while i + 12 <= b.len() {
            let len = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as usize;
            match &b[i + 4..i + 8] {
                b"pHYs" if len == 9 && i + 8 + 9 <= b.len() => {
                    let ppu = u32::from_be_bytes(b[i + 8..i + 12].try_into().ok()?);
                    return (b[i + 16] == 1).then(|| (ppu as f64 * 0.0254).round() as u32);
                }
                b"IDAT" | b"IEND" => return None,
                _ => {}
            }
            i += 12 + len;
        }
        None
    } else if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 4 <= b.len() && b[i] == 0xFF {
            let marker = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if marker == 0xE0 && i + 14 <= b.len() && &b[i + 4..i + 9] == b"JFIF\0" {
                let (units, xd) = (b[i + 11], u16::from_be_bytes([b[i + 12], b[i + 13]]) as f64);
                return match units {
                    1 => Some(xd.round() as u32),
                    2 => Some((xd * 2.54).round() as u32),
                    _ => None,
                };
            }
            if marker == 0xDA {
                return None;
            }
            i += 2 + len;
        }
        None
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_names_and_dpi_estimate() {
        assert_eq!(paper_mm("A4"), Some((210.0, 297.0)));
        assert_eq!(paper_mm("200x300"), Some((200.0, 300.0)));
        assert_eq!(paper_mm("A9"), None);
        assert_eq!(paper_mm("5x5"), None);
        let d = std::env::temp_dir().join(format!("lgr-paper-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("phone.png");
        image::RgbImage::new(4000, 3000).save(&p).unwrap();
        // 4000 px over 297 mm = 342 dpi
        assert_eq!(dpi_from_paper(&p, (210.0, 297.0)), Some(342));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dpi_from_png_and_jpeg_headers() {
        // PNG with pHYs = 23622 px/m (≈ 600 dpi)
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&[0; 13]);
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&[0, 0, 0, 9]);
        png.extend_from_slice(b"pHYs");
        png.extend_from_slice(&23622u32.to_be_bytes());
        png.extend_from_slice(&23622u32.to_be_bytes());
        png.push(1);
        png.extend_from_slice(&[0; 4]);
        assert_eq!(dpi_from_header(&png), Some(600));
        // JFIF, units = dots per inch, 400
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpg.extend_from_slice(b"JFIF\0\x01\x01\x01");
        jpg.extend_from_slice(&400u16.to_be_bytes());
        jpg.extend_from_slice(&400u16.to_be_bytes());
        jpg.extend_from_slice(&[0, 0]);
        assert_eq!(dpi_from_header(&jpg), Some(400));
        assert_eq!(dpi_from_header(b"garbage"), None);
        assert_eq!(dpi_from_header(&[]), None);
    }

    #[test]
    fn low_dpi_is_accepted_only_for_plausible_pages() {
        use image::codecs::jpeg::{JpegEncoder, PixelDensity};
        let dir = std::env::temp_dir().join(format!("lgr-dpi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, w: u32, h: u32, dpi: u16| {
            let path = dir.join(name);
            let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 200, 200]));
            let mut f = std::fs::File::create(&path).unwrap();
            let mut enc = JpegEncoder::new_with_quality(&mut f, 80);
            enc.set_pixel_density(PixelDensity::dpi(dpi));
            enc.encode_image(&img).unwrap();
            path
        };
        // A4 at 100 dpi: believable
        assert_eq!(detect_dpi(&mk("a.jpg", 826, 1169, 100)), Some(100));
        // the same pixel size claiming 72/96 dpi: editor defaults, ignored
        assert_eq!(detect_dpi(&mk("b.jpg", 826, 1169, 72)), None);
        assert_eq!(detect_dpi(&mk("c.jpg", 826, 1169, 96)), None);
        // 100 dpi but 25 × 35 inches: not a sheet of paper
        assert_eq!(detect_dpi(&mk("d.jpg", 2480, 3508, 100)), None);
        // unchanged: 300 dpi
        assert_eq!(detect_dpi(&mk("e.jpg", 2480, 3508, 300)), Some(300));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A path that is safe to hand to an external program as an argument: a relative name that
/// starts with `-` or `@` would be read as an option or a list file (`-oX.7z`, `@list.7z`), so it
/// is made explicitly relative (`./-oX.7z`). Absolute paths and ordinary names are unchanged.
pub fn safe_path(p: &std::path::Path) -> std::path::PathBuf {
    let first = p.components().next();
    let risky = p.is_relative()
        && matches!(first, Some(std::path::Component::Normal(n))
            if n.to_string_lossy().starts_with(['-', '@']));
    if risky {
        std::path::Path::new(".").join(p)
    } else {
        p.to_path_buf()
    }
}

#[cfg(test)]
mod safe_path_tests {
    use super::safe_path;
    use std::path::Path;
    #[test]
    fn option_like_names_are_made_explicitly_relative() {
        assert_eq!(safe_path(Path::new("-oX.7z")), Path::new("./-oX.7z"));
        assert_eq!(safe_path(Path::new("@list.7z")), Path::new("./@list.7z"));
        assert_eq!(safe_path(Path::new("-dir/a.pdf")), Path::new("./-dir/a.pdf"));
        assert_eq!(safe_path(Path::new("book.pdf")), Path::new("book.pdf"));
        assert_eq!(safe_path(Path::new("/abs/-x.pdf")), Path::new("/abs/-x.pdf"));
        assert_eq!(safe_path(Path::new("dir/-x.pdf")), Path::new("dir/-x.pdf"));
    }
}

#[cfg(test)]
mod archive_member_tests {
    #[test]
    fn archive_members_need_a_real_extension() {
        // the filter used when listing the pages of an archive
        let want = |n: &str| {
            let base = n.rsplit(['/', '\\']).next().unwrap_or("");
            base.rsplit_once('.').is_some_and(|(stem, ext)| {
                !stem.is_empty() && super::IMAGE_EXT.contains(&ext.to_ascii_lowercase().as_str())
            })
        };
        assert!(want("a/page1.PNG") && want("x.tif") && want("dir.v2/p.pdf"));
        assert!(!want("png") && !want("dir/pdf") && !want(".png") && !want("notes.txt") && !want("dir.png/readme"));
    }
}
