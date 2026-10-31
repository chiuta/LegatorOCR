use crate::{binarize, content, deskew, orient, split, transform, BitImage, Rect};
use image::{imageops, DynamicImage, GrayImage, Rgb, RgbImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Color,
    Gray,
    /// Adaptive (Sauvola) black & white.
    BlackWhite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitMode {
    Off,
    /// Detect the gutter of a two-page spread; leave single pages alone.
    Auto,
    /// Always cut in the exact middle.
    Middle,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub dpi: u32,
    /// Manual rotation, in quarter turns clockwise (0..=3), applied first.
    pub rotate_quarters: u8,
    /// Detect pages lying on their side (0°/90° only).
    pub auto_orient: bool,
    pub split: SplitMode,
    pub deskew: bool,
    pub max_skew_deg: f32,
    /// Margins around the detected content, millimetres: left, top, right, bottom.
    pub margins_mm: [f32; 4],
    /// Fixed output page size (mm). Content is centred; grows if content is larger.
    pub page_size_mm: Option<(f32, f32)>,
    pub mode: OutputMode,
    pub sauvola_k: f32,
    pub despeckle: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dpi: 300,
            rotate_quarters: 0,
            auto_orient: true,
            split: SplitMode::Auto,
            deskew: true,
            max_skew_deg: 7.0,
            margins_mm: [5.0; 4],
            page_size_mm: None,
            mode: OutputMode::Color,
            sauvola_k: 0.3,
            despeckle: true,
        }
    }
}

#[derive(Clone)]
pub enum PageImage {
    Rgb(RgbImage),
    Gray(GrayImage),
    Bw(BitImage),
}

impl PageImage {
    pub fn width(&self) -> u32 {
        match self {
            PageImage::Rgb(i) => i.width(),
            PageImage::Gray(i) => i.width(),
            PageImage::Bw(i) => i.w,
        }
    }
    pub fn height(&self) -> u32 {
        match self {
            PageImage::Rgb(i) => i.height(),
            PageImage::Gray(i) => i.height(),
            PageImage::Bw(i) => i.h,
        }
    }
    pub fn to_gray(&self) -> GrayImage {
        match self {
            PageImage::Rgb(i) => imageops::grayscale(i),
            PageImage::Gray(i) => i.clone(),
            PageImage::Bw(b) => b.to_gray(),
        }
    }
    pub fn to_dynamic(&self) -> DynamicImage {
        match self {
            PageImage::Rgb(i) => DynamicImage::ImageRgb8(i.clone()),
            _ => DynamicImage::ImageLuma8(self.to_gray()),
        }
    }
}

#[derive(Clone)]
pub struct Page {
    pub image: PageImage,
    pub dpi: u32,
    /// Skew that was corrected, in degrees (clockwise positive).
    pub skew_deg: f32,
    /// `true` if no content was found.
    pub blank: bool,
    /// Which half of a split spread this is: `Some(0)` left, `Some(1)` right.
    pub split_index: Option<u8>,
}

fn mm_to_px(mm: f32, dpi: u32) -> u32 {
    (mm / 25.4 * dpi as f32).round().max(0.0) as u32
}

/// Ink map used for *analysis* only (not for the final B/W output).
fn analysis_ink(gray: &GrayImage, dpi: u32) -> BitImage {
    binarize::sauvola(gray, (dpi / 8).max(15), 0.3)
}

/// Run the whole pipeline on one scanned sheet. A two-page spread yields two pages.
pub fn process_page(src: &DynamicImage, s: &Settings) -> Vec<Page> {
    let mut rgb = transform::rot90(&src.to_rgb8(), s.rotate_quarters);

    if s.auto_orient {
        let ink = analysis_ink(&imageops::grayscale(&rgb), s.dpi);
        if orient::needs_quarter_turn(&ink) {
            rgb = transform::rot90(&rgb, 1);
        }
    }

    // --- page split ---------------------------------------------------
    let mut parts: Vec<(RgbImage, Option<u8>)> = vec![(rgb.clone(), None)];
    if s.split != SplitMode::Off {
        let cut = match s.split {
            SplitMode::Middle => (rgb.width() as f32 >= 1.15 * rgb.height() as f32)
                .then_some(rgb.width() / 2),
            _ => {
                let ink = analysis_ink(&imageops::grayscale(&rgb), s.dpi);
                split::find_gutter(&ink)
            }
        };
        if let Some(x) = cut {
            let l = imageops::crop_imm(&rgb, 0, 0, x, rgb.height()).to_image();
            let r = imageops::crop_imm(&rgb, x, 0, rgb.width() - x, rgb.height()).to_image();
            parts = vec![(l, Some(0)), (r, Some(1))];
        }
    }

    parts
        .into_iter()
        .map(|(img, idx)| finish_page(img, idx, s))
        .collect()
}

fn finish_page(mut img: RgbImage, split_index: Option<u8>, s: &Settings) -> Page {
    let mut bg = transform::estimate_background(&img);
    let mut skew_deg = 0.0;

    // --- deskew -------------------------------------------------------
    if s.deskew {
        let ink = analysis_ink(&imageops::grayscale(&img), s.dpi);
        if let Some(a) = deskew::detect_skew(&ink, s.max_skew_deg) {
            if a.abs() > 0.02f32.to_radians() {
                img = transform::rotate_cw(&img, a, bg);
                skew_deg = a.to_degrees();
            }
        }
    }

    // --- content ------------------------------------------------------
    let gray = imageops::grayscale(&img);
    let ink = analysis_ink(&gray, s.dpi);
    let content: Option<Rect> = content::detect_content(&ink, s.dpi);

    let (canvas_x, canvas_y, cw, ch, blank) = match content {
        None => (0i64, 0i64, img.width(), img.height(), true),
        Some(r) => {
            let [ml, mt, mr, mb] = s.margins_mm.map(|m| mm_to_px(m, s.dpi));
            let (mut cw, mut ch) = (r.w + ml + mr, r.h + mt + mb);
            if let Some((pw, ph)) = s.page_size_mm {
                cw = cw.max(mm_to_px(pw, s.dpi));
                ch = ch.max(mm_to_px(ph, s.dpi));
            }
            // Centre the content box (plus its own asymmetric margins) in the canvas.
            let ox = r.x as i64 - ml as i64 - ((cw - (r.w + ml + mr)) / 2) as i64;
            let oy = r.y as i64 - mt as i64 - ((ch - (r.h + mt + mb)) / 2) as i64;
            (ox, oy, cw, ch, false)
        }
    };
    if s.mode != OutputMode::Color {
        bg = Rgb([255, 255, 255]);
    }
    let cropped = match content {
        None => img,
        Some(r) => {
            // Only the content box (plus a hair of safety) is taken from the scan;
            // everything else — margins, scanner bed, edge shadows — is paper colour.
            let pad = (s.dpi / 100).max(2) as i64;
            let (wx, wy) = (r.x as i64 - pad, r.y as i64 - pad);
            let (ww, wh) = (r.w + 2 * pad as u32, r.h + 2 * pad as u32);
            let window = transform::crop_pad(&img, wx, wy, ww, wh, bg);
            let mut canvas = RgbImage::from_pixel(cw, ch, bg);
            imageops::replace(&mut canvas, &window, wx - canvas_x, wy - canvas_y);
            canvas
        }
    };

    // --- output mode --------------------------------------------------
    let image = match s.mode {
        OutputMode::Color => PageImage::Rgb(cropped),
        OutputMode::Gray => PageImage::Gray(imageops::grayscale(&cropped)),
        OutputMode::BlackWhite => {
            let g = imageops::grayscale(&cropped);
            let mut bw = binarize::sauvola(&g, (s.dpi / 3) | 1, s.sauvola_k);
            if s.despeckle {
                let min_px = ((s.dpi as f32 / 300.0).powi(2) * 6.0).round() as u32;
                bw.despeckle(min_px.max(2));
            }
            PageImage::Bw(bw)
        }
    };
    Page { image, dpi: s.dpi, skew_deg, blank, split_index }
}

#[cfg(test)]
pub(crate) mod testutil {
    use image::{GrayImage, Luma, RgbImage};

    /// Fake "text page": dark word-blocks of irregular width on paper, ragged
    /// like real text (deterministic LCG), so columns do not line up.
    pub fn text_page(w: u32, h: u32, x0: u32, x1: u32) -> GrayImage {
        let mut g = GrayImage::from_pixel(w, h, Luma([235]));
        let mut seed: u32 = 12345;
        let mut rnd = move |m: u32| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) % m
        };
        let mut y = h / 8;
        while y + 20 < h * 7 / 8 {
            let mut x = x0 + rnd(6);
            loop {
                let bw = 14 + rnd(40);
                if x + bw >= x1 {
                    break;
                }
                for yy in y..y + 14 {
                    for xx in x..x + bw {
                        g.put_pixel(xx, yy, Luma([30]));
                    }
                }
                x += bw + 8 + rnd(10);
            }
            y += 34;
        }
        g
    }

    pub fn to_rgb(g: &GrayImage) -> RgbImage {
        image::DynamicImage::ImageLuma8(g.clone()).to_rgb8()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::{binarize, content, deskew, orient, split, transform};
    use image::{imageops, Rgb};

    const DPI: u32 = 300;

    #[test]
    fn sauvola_finds_text_on_uneven_light() {
        let mut g = text_page(600, 500, 80, 520);
        // strong illumination gradient across the page
        for (x, _y, p) in g.enumerate_pixels_mut() {
            let k = 1.0 - 0.35 * (x as f32 / 600.0);
            p.0[0] = (p.0[0] as f32 * k) as u8;
        }
        let bw = binarize::sauvola(&g, 51, 0.3);
        let ink = bw.count_ink() as f32 / (600.0 * 500.0);
        assert!(ink > 0.08 && ink < 0.45, "ink fraction {ink}");
        assert!(!bw.get(5, 5), "paper must stay white");
    }

    #[test]
    fn deskew_recovers_known_angle() {
        let g = text_page(1200, 900, 100, 1100);
        for deg in [-3.0f32, -0.8, 1.7, 4.2] {
            let rot = transform::rotate_cw(&to_rgb(&g), deg.to_radians(), Rgb([235; 3]));
            let ink = binarize::sauvola(&imageops::grayscale(&rot), 41, 0.3);
            let found = deskew::detect_skew(&ink, 7.0).unwrap().to_degrees();
            // we rotated clockwise by `deg`; correction must be the opposite
            assert!((found + deg).abs() < 0.15, "deg {deg}: found {found}");
        }
    }

    #[test]
    fn deskew_leaves_straight_page_alone() {
        let g = text_page(1200, 900, 100, 1100);
        let ink = binarize::sauvola(&g, 41, 0.3);
        let a = deskew::detect_skew(&ink, 7.0).unwrap().to_degrees();
        assert!(a.abs() < 0.1, "{a}");
    }

    #[test]
    fn splits_spread_at_gutter_not_single_page() {
        let spread = text_page(2400, 1500, 100, 2300);
        let mut img = spread.clone();
        for y in 0..1500 {
            for x in 1150..1260 {
                img.put_pixel(x, y, Luma([235]));
            }
        }
        let ink = binarize::sauvola(&img, 41, 0.3);
        let cut = split::find_gutter(&ink).expect("gutter");
        assert!((1150..1260).contains(&cut), "cut at {cut}");
        // one wide text block (no gutter) and a portrait page must not split
        assert!(split::find_gutter(&binarize::sauvola(&spread, 41, 0.3)).is_none());
        let portrait = text_page(1000, 1400, 80, 920);
        assert!(split::find_gutter(&binarize::sauvola(&portrait, 41, 0.3)).is_none());
    }

    use image::Luma;

    #[test]
    fn content_box_keeps_page_number_drops_dust_and_edge_shadow() {
        let mut g = text_page(1200, 1600, 200, 1000);
        // page number
        for y in 1500..1530 {
            for x in 580..640 {
                g.put_pixel(x, y, Luma([20]));
            }
        }
        // dust
        for &(x, y) in &[(40u32, 60u32), (1150, 90), (30, 1400)] {
            g.put_pixel(x, y, Luma([0]));
            g.put_pixel(x + 1, y, Luma([0]));
        }
        // scanner shadow strip along the left edge
        for y in 0..1600 {
            for x in 0..12 {
                g.put_pixel(x, y, Luma([10]));
            }
        }
        let ink = binarize::sauvola(&g, 41, 0.3);
        let r = content::detect_content(&ink, DPI).expect("content");
        assert!(r.x >= 150 && r.x <= 210, "x {}", r.x);
        assert!(r.x + r.w <= 1010, "right {}", r.x + r.w);
        assert!(r.y + r.h >= 1525, "page number kept: bottom {}", r.y + r.h);
        assert!(r.y > 100, "dust above the text ignored: y {}", r.y);
    }

    #[test]
    fn blank_page_has_no_content() {
        let g = image::GrayImage::from_pixel(800, 800, Luma([240]));
        let ink = binarize::sauvola(&g, 41, 0.3);
        assert!(content::detect_content(&ink, DPI).is_none());
    }

    #[test]
    fn sideways_page_detected() {
        let g = text_page(1000, 800, 80, 920);
        let up = binarize::sauvola(&g, 41, 0.3);
        assert!(!orient::needs_quarter_turn(&up));
        let side = imageops::rotate90(&g);
        assert!(orient::needs_quarter_turn(&binarize::sauvola(&side, 41, 0.3)));
    }

    #[test]
    fn despeckle_removes_only_small_components() {
        let mut b = crate::BitImage::new(50, 50);
        b.set(3, 3, true); // 1-px speck
        for y in 20..30 {
            for x in 20..30 {
                b.set(x, y, true);
            }
        }
        assert_eq!(b.despeckle(6), 1);
        assert!(!b.get(3, 3) && b.get(25, 25));
    }

    #[test]
    fn end_to_end_page_geometry() {
        let g = text_page(1500, 2000, 250, 1250);
        let rot = transform::rotate_cw(&to_rgb(&g), 2.0f32.to_radians(), Rgb([235; 3]));
        let s = Settings { margins_mm: [10.0; 4], mode: OutputMode::BlackWhite, ..Default::default() };
        let pages = process_page(&image::DynamicImage::ImageRgb8(rot), &s);
        assert_eq!(pages.len(), 1);
        let p = &pages[0];
        assert!((p.skew_deg + 2.0).abs() < 0.2, "skew {}", p.skew_deg);
        // content (~1000 px wide) + 2×10 mm margin (118 px each) = ≈ 1236
        assert!((p.image.width() as i32 - 1236).abs() < 40, "w {}", p.image.width());
        assert!(matches!(p.image, PageImage::Bw(_)));
    }
}
