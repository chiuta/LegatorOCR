//! Input-file helpers shared by the CLI and the GUI.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp"];

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
pub fn collect_images(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p)
                .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| is_image(p)).collect())
                .unwrap_or_default();
            v.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
            out.extend(v);
        } else if is_image(p) {
            out.push(p.clone());
        }
    }
    out
}

/// Resolution stored in a PNG (`pHYs`) or JPEG (JFIF) header, if it looks like a
/// real scan resolution. Values outside 150–1200 dpi are ignored: 72/96 dpi is
/// what screenshots and many editors write by default and would be wrong here.
pub fn detect_dpi(path: &Path) -> Option<u32> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(64 * 1024).read_to_end(&mut head).ok()?;
    let dpi = dpi_from_header(&head)?;
    (150..=1200).contains(&dpi).then_some(dpi)
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
}
