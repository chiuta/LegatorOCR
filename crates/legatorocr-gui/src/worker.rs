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
    pub dpi: u32,
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

pub fn render(req: &PreviewRequest) -> Result<Preview, String> {
    let img = legatorocr_engine::load_image(&req.path)
        .map_err(|e| format!("{}: {e}", req.path.display()))?;
    let original_size = (img.width(), img.height());
    let original = to_thumb(&img.to_rgba8(), PREVIEW_MAX);
    let (img, settings) = legatorocr_engine::orient_with_osd(img, &req.settings, req.osd.as_ref());
    let pages = process_page(&img, &settings)
        .into_iter()
        .map(|p| {
            let info = PageInfo {
                w: p.image.width(),
                h: p.image.height(),
                skew_deg: p.skew_deg,
                blank: p.blank,
                content_box: p.content_box,
                dpi: p.dpi,
            };
            (to_thumb(&page_rgba(&p.image), PREVIEW_MAX), info)
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
