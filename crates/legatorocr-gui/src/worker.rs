//! Background preview rendering: decode → run the real pipeline → downscale for display.

use image::{imageops, DynamicImage, RgbaImage};
use legatorocr_core::{process_page, PageImage, Settings};
use std::path::PathBuf;

/// Longest edge of a preview texture (keeps GPU upload and memory small).
pub const PREVIEW_MAX: u32 = 1800;

pub struct PreviewRequest {
    pub id: u64,
    pub key: u64,
    pub path: PathBuf,
    pub settings: Settings,
    pub osd: Option<legatorocr_engine::ocr::OcrOptions>,
}

pub struct Thumb {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

pub struct PageInfo {
    /// Full-resolution size of the processed page.
    pub w: u32,
    pub h: u32,
    pub skew_deg: f32,
    pub blank: bool,
    /// Content box on the page (x, y, w, h in full-resolution pixels) and the page's dpi.
    pub content_box: Option<[u32; 4]>,
    /// The block of text on the page (x, y, w, h in full-resolution pixels), found from the
    /// picture: without binder shadows, page borders, long rules and specks.
    pub text_box: Option<[u32; 4]>,
    pub dpi: u32,
    /// Which half of a cut sheet this is (`Some(0)` left, `Some(1)` right); `None` when not cut.
    pub split_index: Option<u8>,
}

pub struct Preview {
    pub original: Thumb,
    pub original_size: (u32, u32),
    pub pages: Vec<(Thumb, PageInfo)>,
}

pub struct PreviewResult {
    pub id: u64,
    pub key: u64,
    pub outcome: Result<Preview, String>,
}

pub fn to_thumb(img: &RgbaImage, max: u32) -> Thumb {
    let (w, h) = img.dimensions();
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let small;
    let src = if scale < 1.0 {
        small = imageops::resize(
            img,
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
            imageops::FilterType::Triangle,
        );
        &small
    } else {
        img
    };
    Thumb {
        w: src.width() as usize,
        h: src.height() as usize,
        rgba: src.as_raw().clone(),
    }
}

fn page_rgba(p: &PageImage) -> RgbaImage {
    match p {
        PageImage::Rgb(i) => DynamicImage::ImageRgb8(i.clone()).to_rgba8(),
        other => DynamicImage::ImageLuma8(other.to_gray()).to_rgba8(),
    }
}

/// The block of text on a page picture (x, y, w, h in the picture's pixels), or `None` when there
/// is no text. Ink is what is clearly darker than the paper; columns that hold a long unbroken
/// vertical run of ink (binder shadow, page border, a dark edge) and rows with a long horizontal
/// run (rules, shadows along the top or bottom) are not text and are left out, as are the outer
/// 1 % of the picture and the stray specks at the ends (the extent is read between the 0.4 % and
/// 99.6 % points of the ink).
pub fn text_block(t: &Thumb) -> Option<[u32; 4]> {
    let (w, h) = (t.w, t.h);
    if w < 16 || h < 16 || t.rgba.len() < w * h * 4 {
        return None;
    }
    let gray: Vec<u8> = (0..w * h)
        .map(|i| {
            let p = &t.rgba[i * 4..i * 4 + 3];
            ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
        })
        .collect();
    let mut sorted = gray.clone();
    sorted.sort_unstable();
    let paper = sorted[sorted.len() * 9 / 10] as f32;
    let limit = (paper * 0.6).min(160.0) as u8;
    let mut ink: Vec<bool> = gray.iter().map(|&g| g < limit).collect();
    // the outer 1 % is the scan's edge, not text
    let (mx, my) = ((w / 100).max(1), (h / 100).max(1));
    for y in 0..h {
        for x in 0..w {
            if x < mx || y < my || x >= w - mx || y >= h - my {
                ink[y * w + x] = false;
            }
        }
    }
    // long runs are not text
    let max_v = (h as f32 * 0.15) as usize;
    let max_h = (w as f32 * 0.5) as usize;
    let mut drop = vec![false; w * h];
    for x in 0..w {
        let mut run = 0;
        for y in 0..=h {
            if y < h && ink[y * w + x] {
                run += 1;
            } else {
                if run > max_v {
                    for k in y - run..y {
                        drop[k * w + x] = true;
                    }
                }
                run = 0;
            }
        }
    }
    for y in 0..h {
        let mut run = 0;
        for x in 0..=w {
            if x < w && ink[y * w + x] {
                run += 1;
            } else {
                if run > max_h {
                    for k in x - run..x {
                        drop[y * w + k] = true;
                    }
                }
                run = 0;
            }
        }
    }
    // solid dark areas (a shadow with a soft edge has no long run of pure ink): where a 9x9
    // window is almost all ink, the area and a few pixels around it are not text
    {
        let iw = w + 1;
        let integral = |m: &dyn Fn(usize) -> bool| -> Vec<u32> {
            let mut t = vec![0u32; iw * (h + 1)];
            for y in 0..h {
                let mut row = 0;
                for x in 0..w {
                    row += u32::from(m(y * w + x));
                    t[(y + 1) * iw + x + 1] = t[y * iw + x + 1] + row;
                }
            }
            t
        };
        let boxsum = |t: &[u32], x: usize, y: usize, r: usize| -> u32 {
            let (x0, y0) = (x.saturating_sub(r), y.saturating_sub(r));
            let (x1, y1) = ((x + r + 1).min(w), (y + r + 1).min(h));
            t[y1 * iw + x1] + t[y0 * iw + x0] - t[y0 * iw + x1] - t[y1 * iw + x0]
        };
        let it = integral(&|i| ink[i]);
        let dense: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let r = 4;
                let area = ((x + r + 1).min(w) - x.saturating_sub(r))
                    * ((y + r + 1).min(h) - y.saturating_sub(r));
                boxsum(&it, x, y, r) as f32 > 0.75 * area as f32
            })
            .collect();
        let dt = integral(&|i| dense[i]);
        for (i, d) in drop.iter_mut().enumerate() {
            if boxsum(&dt, i % w, i / w, 8) > 0 {
                *d = true;
            }
        }
    }
    let mut cols = vec![0u32; w];
    let mut rows = vec![0u32; h];
    for y in 0..h {
        for x in 0..w {
            if ink[y * w + x] && !drop[y * w + x] {
                cols[x] += 1;
                rows[y] += 1;
            }
        }
    }
    let total: u64 = cols.iter().map(|&c| c as u64).sum();
    if total < (w * h / 2000).max(20) as u64 {
        return None;
    }
    let span = |v: &[u32]| -> (usize, usize) {
        let cut = (total as f64 * 0.004) as u64;
        let (mut acc, mut a) = (0u64, 0);
        for (i, &c) in v.iter().enumerate() {
            acc += c as u64;
            if acc > cut {
                a = i;
                break;
            }
        }
        let (mut acc, mut b) = (0u64, v.len() - 1);
        for (i, &c) in v.iter().enumerate().rev() {
            acc += c as u64;
            if acc > cut {
                b = i;
                break;
            }
        }
        (a, b)
    };
    let (x0, x1) = span(&cols);
    let (y0, y1) = span(&rows);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // a little air around the text
    let (px, py) = ((w as f32 * 0.008) as usize, (h as f32 * 0.008) as usize);
    let (x0, y0) = (x0.saturating_sub(px), y0.saturating_sub(py));
    let (x1, y1) = ((x1 + px).min(w - 1), (y1 + py).min(h - 1));
    Some([
        x0 as u32,
        y0 as u32,
        (x1 - x0 + 1) as u32,
        (y1 - y0 + 1) as u32,
    ])
}

pub fn render(req: &PreviewRequest) -> Result<Preview, String> {
    let img = legatorocr_engine::load_image(&req.path)
        .map_err(|e| format!("{}: {e}", req.path.display()))?;
    let original_size = (img.width(), img.height());
    let original = to_thumb(&img.to_rgba8(), PREVIEW_MAX);
    let (img, settings) = legatorocr_engine::orient_with_osd(img, &req.settings, req.osd.as_ref());
    let pages = process_page(&img, &settings)
        .into_iter()
        .map(|p| {
            let thumb = to_thumb(&page_rgba(&p.image), PREVIEW_MAX);
            let (pw, ph) = (p.image.width(), p.image.height());
            let info = PageInfo {
                w: pw,
                h: ph,
                skew_deg: p.skew_deg,
                blank: p.blank,
                content_box: p.content_box,
                text_box: text_block(&thumb).map(|[x, y, w, h]| {
                    let (kx, ky) = (pw as f32 / thumb.w as f32, ph as f32 / thumb.h as f32);
                    [
                        (x as f32 * kx) as u32,
                        (y as f32 * ky) as u32,
                        ((w as f32 * kx) as u32).max(1),
                        ((h as f32 * ky) as u32).max(1),
                    ]
                }),
                split_index: p.split_index,
                dpi: p.dpi,
            };
            (thumb, info)
        })
        .collect();
    Ok(Preview {
        original,
        original_size,
        pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    #[test]
    fn thumb_keeps_aspect_and_never_upscales() {
        let big = RgbaImage::new(3600, 1800);
        let t = to_thumb(&big, 1800);
        assert_eq!((t.w, t.h), (1800, 900));
        assert_eq!(t.rgba.len(), 1800 * 900 * 4);
        let small = RgbaImage::new(100, 50);
        let t = to_thumb(&small, 1800);
        assert_eq!((t.w, t.h), (100, 50));
    }

    #[test]
    fn renders_original_and_processed() {
        let dir = std::env::temp_dir().join(format!("lgr-gui-prev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut g = GrayImage::from_pixel(600, 800, Luma([235]));
        for y in (100..700).step_by(30) {
            for yy in y..y + 12 {
                for x in 80..520 {
                    if (x / 30) % 2 == 0 {
                        g.put_pixel(x, yy, Luma([25]));
                    }
                }
            }
        }
        let path = dir.join("p.png");
        g.save(&path).unwrap();
        let req = PreviewRequest {
            id: 1,
            key: 7,
            path,
            settings: Settings {
                dpi: 150,
                auto_orient: false,
                deskew: false,
                ..Default::default()
            },
            osd: None,
        };
        let pv = render(&req).unwrap();
        assert_eq!(pv.original_size, (600, 800));
        assert_eq!(pv.pages.len(), 1);
        assert!(pv.pages[0].1.w < 600, "cropped to content");
        assert!(render(&PreviewRequest {
            path: "/nope.png".into(),
            ..req
        })
        .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod text_block_tests {
    use super::*;

    fn page(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Thumb {
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                if f(x, y) {
                    rgba[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(20);
                }
            }
        }
        Thumb { w, h, rgba }
    }

    /// Lines of text between (x0,y0) and (x1,y1): 4 px of ink, 6 px of space, in words.
    fn text(x: usize, y: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> bool {
        x >= x0 && x < x1 && y >= y0 && y < y1 && (y - y0) % 10 < 4 && !(x / 7).is_multiple_of(5)
    }

    #[test]
    fn finds_the_text_and_not_the_page() {
        let t = page(600, 800, |x, y| text(x, y, 120, 150, 480, 650));
        let [x, y, w, h] = text_block(&t).unwrap();
        assert!((110..=125).contains(&x), "{x}");
        assert!((140..=155).contains(&y), "{y}");
        assert!((355..=385).contains(&w), "{w}");
        assert!((495..=525).contains(&h), "{h}");
    }

    #[test]
    fn ignores_shadows_borders_rules_and_specks() {
        let t = page(600, 800, |x, y| {
            text(x, y, 120, 150, 480, 650)
                || (580..596).contains(&x) && (100..700).contains(&y) // binder shadow
                || (x == 30 && (0..800).contains(&y)) // page border
                || (y == 40 && (20..580).contains(&x)) // rule across the top
                || (x == 60 && y == 70) // speck
        });
        let [x, y, w, h] = text_block(&t).unwrap();
        assert!((110..=125).contains(&x), "{x}");
        assert!((140..=155).contains(&y), "{y}");
        assert!((355..=385).contains(&w), "{w}");
        assert!((495..=525).contains(&h), "{h}");
    }

    #[test]
    fn a_blank_or_tiny_picture_has_no_text() {
        assert!(text_block(&page(600, 800, |_, _| false)).is_none());
        assert!(text_block(&page(8, 8, |_, _| true)).is_none());
        let bad = Thumb {
            w: 600,
            h: 800,
            rgba: vec![0; 10],
        };
        assert!(text_block(&bad).is_none());
        // all black: a solid block is not text
        assert!(text_block(&page(600, 800, |_, _| true)).is_none());
    }
}
