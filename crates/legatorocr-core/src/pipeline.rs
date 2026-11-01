use crate::binarize::Threshold;
use crate::cleanup::{self, Dropout};
use crate::layers::{self, Layers, Patch};
use crate::{
    binarize, content, deskew, dewarp, orient, pictures, split, transform, BitImage, Rect,
};
use image::{imageops, DynamicImage, GrayImage, Rgb, RgbImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Color,
    Gray,
    /// Adaptive black & white (see [`Threshold`]).
    BlackWhite,
    /// Text as a sharp 1-bit layer; detected pictures kept as colour patches.
    Mixed,
    /// Like `Mixed`, plus a low-resolution paper background: colour book pages at a
    /// fraction of the size, text still razor sharp.
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitMode {
    Off,
    /// Detect the gutter of a two-page spread; leave single pages alone.
    Auto,
    /// Always cut in the exact middle.
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    Top,
    #[default]
    Center,
    Bottom,
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
    /// Manual skew correction in degrees (clockwise positive). When set it
    /// replaces the automatic estimate, even if `deskew` is off.
    pub manual_skew_deg: Option<f32>,
    pub max_skew_deg: f32,
    /// Straighten curved text lines (page curl near the spine).
    pub dewarp: bool,
    /// User-marked guide lines for dewarping: each polyline of `(x, y)` points (fractions of the
    /// deskewed page's width and height) becomes a straight horizontal line. When present they
    /// replace the automatic estimate, even if `dewarp` is off.
    pub dewarp_guides: Vec<Vec<(f32, f32)>>,
    /// Like `dewarp_guides` but drawn on the *scanned sheet as loaded* (fractions of its width
    /// and height); they follow rotation, page split and deskew.
    pub sheet_dewarp_guides: Vec<Vec<(f32, f32)>>,
    /// Margins around the detected content, millimetres: left, top, right, bottom.
    pub margins_mm: [f32; 4],
    /// Per-page correction of the detected content box, mm: how far each side
    /// (left, top, right, bottom) moves *outwards* (negative = inwards).
    pub content_adjust_mm: [f32; 4],
    /// Fixed output page size (mm). Content is placed per `align`; grows if content is larger.
    pub page_size_mm: Option<(f32, f32)>,
    pub align: (HAlign, VAlign),
    pub mode: OutputMode,
    /// Binarisation method for black & white / mixed / hybrid output.
    pub threshold: Threshold,
    /// Binarisation sensitivity 0–1 (higher = more ink).
    pub sauvola_k: f32,
    pub despeckle: bool,
    /// Even out lighting (shadows, darkened corners) in colour/gray/hybrid output.
    pub normalize_bg: bool,
    /// Resample the result to this resolution when lower than the scan's.
    pub output_dpi: Option<u32>,
    /// Picture detection sensitivity 0–1 for `Mixed` / `Hybrid`.
    pub picture_sensitivity: f32,
    /// Resolution of the `Hybrid` background layer.
    pub hybrid_bg_dpi: u32,
    /// Invert negative scans (light text on dark paper).
    pub auto_invert: bool,
    /// Stretch contrast so paper becomes white and ink black.
    pub auto_levels: bool,
    /// Unsharp-mask amount 0–2 (0 = off) for colour / gray / hybrid output.
    pub sharpen: f32,
    /// Erase binder / punch holes near the page edge.
    pub punch_holes: bool,
    /// Erase fingers holding the page at the border (skin-coloured regions).
    pub fingers: bool,
    /// Erase ruled lines (1-bit output only).
    pub remove_lines: bool,
    /// Rectangles (x, y, width, height in mm from the top-left of each page) painted with
    /// the paper colour before anything else is measured (stamps, notes, scanner marks).
    pub erase_zones: Vec<[f32; 4]>,
    /// Rectangles (x, y, width, height in mm, same frame as `erase_zones`) that are always
    /// kept as colour pictures in mixed / hybrid output, whatever the automatic detector says.
    pub picture_zones: Vec<[f32; 4]>,
    /// Like `picture_zones` but in millimetres of the *scanned sheet as loaded* (before
    /// rotation, splitting and deskew); drawn with the mouse in the GUI, stored per sheet.
    /// Followed through those steps, so they stay on the picture. Not followed through
    /// perspective correction (then they are dropped) or dewarping (position approximate).
    pub sheet_picture_zones: Vec<[f32; 4]>,
    /// Like `erase_zones` but in millimetres of the *scanned sheet as loaded* (before
    /// rotation and page splitting); drawn with the mouse in the GUI, stored per sheet.
    pub sheet_erase_zones: Vec<[f32; 4]>,
    /// Drop one printed colour (form rules, stamps).
    pub dropout: Dropout,
    /// Despeckle strength 0.25–4 (1 = default speck size).
    pub despeckle_strength: f32,
    /// Fade print showing through thin paper (also levels the lighting).
    pub show_through: bool,
    /// Smooth halftone screens of printed photographs.
    pub descreen: bool,
    /// Straighten pages photographed at an angle (finds the page quadrilateral).
    pub perspective: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dpi: 300,
            rotate_quarters: 0,
            auto_orient: true,
            split: SplitMode::Auto,
            deskew: true,
            manual_skew_deg: None,
            max_skew_deg: 7.0,
            dewarp: false,
            dewarp_guides: Vec::new(),
            sheet_dewarp_guides: Vec::new(),
            margins_mm: [5.0; 4],
            content_adjust_mm: [0.0; 4],
            page_size_mm: None,
            align: (HAlign::Center, VAlign::Center),
            mode: OutputMode::Color,
            threshold: Threshold::Sauvola,
            sauvola_k: 0.3,
            despeckle: true,
            normalize_bg: false,
            output_dpi: None,
            picture_sensitivity: 0.5,
            hybrid_bg_dpi: 100,
            auto_invert: false,
            auto_levels: false,
            sharpen: 0.0,
            punch_holes: false,
            fingers: false,
            remove_lines: false,
            erase_zones: Vec::new(),
            picture_zones: Vec::new(),
            sheet_picture_zones: Vec::new(),
            sheet_erase_zones: Vec::new(),
            dropout: Dropout::Off,
            despeckle_strength: 1.0,
            show_through: false,
            descreen: false,
            perspective: false,
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
    /// The flattened page (what you see; what OCR reads; what image export writes).
    pub image: PageImage,
    /// Resolution of `image` (the output dpi when resampling was requested).
    pub dpi: u32,
    /// Skew that was corrected, in degrees (clockwise positive).
    pub skew_deg: f32,
    /// `true` if no content was found.
    pub blank: bool,
    /// Which half of a split spread this is: `Some(0)` left, `Some(1)` right.
    pub split_index: Option<u8>,
    /// Where the detected content box (after the per-page adjustment) lies on this page:
    /// x, y, width, height in pixels of `image`. `None` for blank pages. Lets a viewer draw
    /// the box and turn a drag of its edges into `Settings::content_adjust_mm`.
    pub content_box: Option<[u32; 4]>,
    /// Layered form (`Mixed`/`Hybrid`): the PDF writer stores these instead of `image`.
    pub layers: Option<Layers>,
}

fn mm_to_px(mm: f32, dpi: u32) -> u32 {
    (mm / 25.4 * dpi as f32).round().max(0.0) as u32
}

/// Ink map for *analysis* only, computed on a copy of at most ≈150 dpi (a 600 dpi
/// scan is analysed on 1/16 of the pixels). Returns the map and the factor by which
/// it is smaller than the source, plus the dpi it corresponds to.
struct Analysis {
    ink: BitImage,
    factor: u32,
    dpi: u32,
}

fn analyse(gray: &GrayImage, dpi: u32) -> Analysis {
    let factor = (dpi / 150).max(1);
    let small = transform::downscale_gray(gray, factor);
    let eff = (dpi / factor).max(1);
    Analysis {
        ink: binarize::sauvola(&small, (eff / 8).max(15), 0.3),
        factor,
        dpi: eff,
    }
}

/// Largest output canvas side and area the pipeline will build.
const MAX_SIDE_PX: u32 = 16_000;
const MAX_AREA_PX: f64 = 150e6;

impl Settings {
    /// Copy with every value forced into a range the pipeline can handle:
    /// non-finite numbers fall back to defaults, sizes are bounded so that a
    /// typo (`--margin 100000`) cannot request a multi-gigabyte canvas.
    pub fn sanitized(&self) -> Settings {
        let d = Settings::default();
        let fin = |v: f32, lo: f32, hi: f32, dflt: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                dflt
            }
        };
        let mut s = self.clone();
        s.dpi = s.dpi.clamp(36, 2400);
        s.rotate_quarters %= 4;
        s.dewarp_guides.truncate(64);
        s.sheet_dewarp_guides.truncate(64);
        for g in s.sheet_dewarp_guides.iter_mut() {
            g.truncate(256);
            g.retain(|p| p.0.is_finite() && p.1.is_finite());
        }
        for g in &mut s.dewarp_guides {
            g.truncate(256);
        }
        s.manual_skew_deg = s
            .manual_skew_deg
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(-45.0, 45.0));
        s.max_skew_deg = fin(s.max_skew_deg, 0.0, 15.0, d.max_skew_deg);
        // No canvas side may exceed MAX_SIDE_PX and the area is capped, whatever the
        // dpi: a typo like `--page-size 2000x2000 --dpi 2400` must not ask for 35 gigapixels.
        let max_mm = MAX_SIDE_PX as f32 * 25.4 / s.dpi as f32;
        for (m, dm) in s.margins_mm.iter_mut().zip(d.margins_mm) {
            *m = fin(*m, 0.0, 200.0f32.min(max_mm / 4.0), dm.min(max_mm / 4.0));
        }
        for a in s.content_adjust_mm.iter_mut() {
            *a = fin(*a, -100.0, 100.0, 0.0);
        }
        s.page_size_mm = s.page_size_mm.and_then(|(w, h)| {
            (w.is_finite() && h.is_finite() && w >= 10.0 && h >= 10.0).then(|| {
                let (mut w, mut h) = (w.min(2000.0).min(max_mm), h.min(2000.0).min(max_mm));
                let px = (w as f64 * s.dpi as f64 / 25.4) * (h as f64 * s.dpi as f64 / 25.4);
                if px > MAX_AREA_PX {
                    let k = (MAX_AREA_PX / px).sqrt() as f32;
                    w *= k;
                    h *= k;
                }
                (w, h)
            })
        });
        s.sauvola_k = fin(s.sauvola_k, 0.0, 1.0, d.sauvola_k);
        s.picture_sensitivity = fin(s.picture_sensitivity, 0.0, 1.0, d.picture_sensitivity);
        s.hybrid_bg_dpi = s.hybrid_bg_dpi.clamp(36, 300);
        s.sharpen = fin(s.sharpen, 0.0, 2.0, 0.0);
        s.despeckle_strength = fin(s.despeckle_strength, 0.25, 4.0, 1.0);
        s.sheet_erase_zones = s
            .sheet_erase_zones
            .iter()
            .filter(|z| z.iter().all(|v| v.is_finite()) && z[2] > 0.0 && z[3] > 0.0)
            .take(64)
            .map(|z| {
                [
                    z[0].clamp(-5000.0, 5000.0),
                    z[1].clamp(-5000.0, 5000.0),
                    z[2].min(5000.0),
                    z[3].min(5000.0),
                ]
            })
            .collect();
        s.sheet_picture_zones = s
            .sheet_picture_zones
            .iter()
            .filter(|z| z.iter().all(|v| v.is_finite()) && z[2] > 0.0 && z[3] > 0.0)
            .take(64)
            .map(|z| {
                [
                    z[0].clamp(-5000.0, 5000.0),
                    z[1].clamp(-5000.0, 5000.0),
                    z[2].min(5000.0),
                    z[3].min(5000.0),
                ]
            })
            .collect();
        s.picture_zones = s
            .picture_zones
            .iter()
            .filter(|z| z.iter().all(|v| v.is_finite()) && z[2] > 0.0 && z[3] > 0.0)
            .take(64)
            .map(|z| {
                [
                    z[0].clamp(-5000.0, 5000.0),
                    z[1].clamp(-5000.0, 5000.0),
                    z[2].min(5000.0),
                    z[3].min(5000.0),
                ]
            })
            .collect();
        s.erase_zones = s
            .erase_zones
            .iter()
            .filter(|z| z.iter().all(|v| v.is_finite()) && z[2] > 0.0 && z[3] > 0.0)
            .take(64)
            .map(|z| {
                [
                    z[0].clamp(-5000.0, 5000.0),
                    z[1].clamp(-5000.0, 5000.0),
                    z[2].min(5000.0),
                    z[3].min(5000.0),
                ]
            })
            .collect();
        s.output_dpi = s.output_dpi.filter(|&o| o >= 36).map(|o| o.min(s.dpi));
        s
    }
}

/// Stages shared by [`process_page`] and [`measure_page`]: orientation and page split.
/// One page cut from a sheet: the image, which half it is, and the hand-marked picture zones
/// (x, y, width, height in pixels of this image).
type Guides = Vec<Vec<(f32, f32)>>;
type SheetPart = (RgbImage, Option<u8>, Vec<[f32; 4]>, Guides);

/// Rotate a pixel rectangle together with an image of `w`×`h` by `turns` quarter turns
/// clockwise; returns the new image size too.
fn rot90_rects(rects: &mut [[f32; 4]], w: u32, h: u32, turns: u8) -> (u32, u32) {
    let (mut w, mut h) = (w, h);
    for _ in 0..turns % 4 {
        for r in rects.iter_mut() {
            // (x, y) -> (H - y, x) for a point, so a rectangle keeps its corner nearest the origin
            *r = [h as f32 - r[1] - r[3], r[0], r[3], r[2]];
        }
        std::mem::swap(&mut w, &mut h);
    }
    (w, h)
}

/// The same quarter turns for points: (x, y) -> (H - y, x).
fn rot90_points(g: &mut Guides, w: u32, h: u32, turns: u8) {
    let (mut w, mut h) = (w, h);
    for _ in 0..turns % 4 {
        for l in g.iter_mut() {
            for p in l.iter_mut() {
                *p = (h as f32 - p.1, p.0);
            }
        }
        std::mem::swap(&mut w, &mut h);
    }
}

fn split_sheet(src: &DynamicImage, s: &Settings) -> Vec<SheetPart> {
    let base = src.to_rgb8();
    let (w0, h0) = base.dimensions();
    let px = |mm: f32| mm / 25.4 * s.dpi as f32;
    let mut zones: Vec<[f32; 4]> = s
        .sheet_picture_zones
        .iter()
        .map(|z| [px(z[0]), px(z[1]), px(z[2]), px(z[3])])
        .collect();
    rot90_rects(&mut zones, w0, h0, s.rotate_quarters);
    // mouse-drawn dewarp guides: fractions of the sheet as loaded -> pixels, then followed
    let mut guides: Guides = s
        .sheet_dewarp_guides
        .iter()
        .map(|g| {
            g.iter()
                .map(|p| (p.0 * w0 as f32, p.1 * h0 as f32))
                .collect()
        })
        .collect();
    rot90_points(&mut guides, w0, h0, s.rotate_quarters);
    let mut rgb = if s.rotate_quarters.is_multiple_of(4) {
        base // nothing turned: no copy at all
    } else {
        transform::rot90(&base, s.rotate_quarters)
    };
    if s.perspective {
        if let Some(flat) = crate::perspective::correct(&rgb) {
            rgb = flat;
            zones.clear(); // a projective warp is not followed
            guides.clear();
        }
    }

    if s.auto_orient {
        let a = analyse(&imageops::grayscale(&rgb), s.dpi);
        if orient::needs_quarter_turn(&a.ink, a.dpi) {
            rot90_rects(&mut zones, rgb.width(), rgb.height(), 1);
            rot90_points(&mut guides, rgb.width(), rgb.height(), 1);
            rgb = transform::rot90(&rgb, 1);
        }
    }

    let mut cut_at = None;
    if s.split != SplitMode::Off {
        let cut = match s.split {
            SplitMode::Middle => {
                (rgb.width() as f32 >= 1.15 * rgb.height() as f32).then_some(rgb.width() / 2)
            }
            _ => {
                let a = analyse(&imageops::grayscale(&rgb), s.dpi);
                // a spread scanned crooked has a leaning gutter: look for it on the levelled ink
                let ink = match crate::deskew::detect_skew(&a.ink, s.max_skew_deg) {
                    Some(ang) if ang.abs() > 0.1f32.to_radians() => split::level(&a.ink, ang),
                    _ => a.ink,
                };
                split::find_gutter(&ink).map(|x| (x * a.factor).min(rgb.width().saturating_sub(1)))
            }
        };
        cut_at = cut.filter(|&x| x > 0 && x < rgb.width());
    }
    match cut_at {
        Some(x) => {
            let l = imageops::crop_imm(&rgb, 0, 0, x, rgb.height()).to_image();
            let r = imageops::crop_imm(&rgb, x, 0, rgb.width() - x, rgb.height()).to_image();
            let clip = |z: &[f32; 4], x0: f32, x1: f32| {
                let (a, b) = (z[0].max(x0), (z[0] + z[2]).min(x1));
                (b > a).then_some([a - x0, z[1], b - a, z[3]])
            };
            let xf = x as f32;
            let lz = zones.iter().filter_map(|z| clip(z, 0.0, xf)).collect();
            let rz = zones
                .iter()
                .filter_map(|z| clip(z, xf, rgb.width() as f32))
                .collect();
            // keep the points that fall on each half (a guide needs two of them)
            let part = |x0: f32, x1: f32| -> Guides {
                guides
                    .iter()
                    .map(|g| {
                        g.iter()
                            .filter(|p| p.0 >= x0 && p.0 < x1)
                            .map(|p| (p.0 - x0, p.1))
                            .collect::<Vec<_>>()
                    })
                    .filter(|g| g.len() >= 2)
                    .collect()
            };
            let (lg, rg) = (part(0.0, xf), part(xf, rgb.width() as f32));
            vec![(l, Some(0), lz, lg), (r, Some(1), rz, rg)]
        }
        None => vec![(rgb, None, zones, guides)],
    }
}

/// Run the whole pipeline on one scanned sheet. A two-page spread yields two pages.
/// The sheet with its hand-drawn erase zones painted over (borrowed when there are none).
fn erase_on_sheet<'a>(src: &'a DynamicImage, s: &Settings) -> std::borrow::Cow<'a, DynamicImage> {
    if s.sheet_erase_zones.is_empty() {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut rgb = src.to_rgb8();
    let paper = transform::estimate_background(&rgb);
    cleanup::erase_zones(&mut rgb, s.dpi, &s.sheet_erase_zones, paper);
    std::borrow::Cow::Owned(DynamicImage::ImageRgb8(rgb))
}

pub fn process_page(src: &DynamicImage, s: &Settings) -> Vec<Page> {
    let s = &s.sanitized();
    let src = &*erase_on_sheet(src, s);
    split_sheet(src, s)
        .into_iter()
        .map(|(img, idx, zones, guides)| render(prepare(img, s, zones, guides), idx, s))
        .collect()
}

/// Size in millimetres (content + margins) of every page the sheet yields, or `None`
/// for a blank page. Used to make all pages of a book the same size without keeping
/// them in memory: measure first, render second.
pub fn measure_page(src: &DynamicImage, s: &Settings) -> Vec<Option<(f32, f32)>> {
    let s = &s.sanitized();
    let src = &*erase_on_sheet(src, s);
    split_sheet(src, s)
        .into_iter()
        .map(|(img, _, zones, guides)| {
            let p = prepare(img, s, zones, guides);
            p.content.map(|r| {
                let [ml, mt, mr, mb] = s.margins_mm;
                let mm = |px: u32| px as f32 * 25.4 / s.dpi as f32;
                (mm(r.w) + ml + mr, mm(r.h) + mt + mb)
            })
        })
        .collect()
}

/// A deskewed (and optionally dewarped) page plus where its content is.
struct Prepared {
    img: RgbImage,
    /// Paper colour estimate.
    bg: Rgb<u8>,
    skew_deg: f32,
    /// Content box in `img` coordinates, after the per-page adjustment.
    content: Option<Rect>,
    /// Hand-marked picture zones (x, y, w, h) in `img` pixels: typed ones and mouse-drawn ones.
    pictures: Vec<[f32; 4]>,
}

fn prepare(
    mut img: RgbImage,
    s: &Settings,
    mut pictures: Vec<[f32; 4]>,
    mut guides: Guides,
) -> Prepared {
    // --- clean-up: fixes that make every later stage's job easier ----------
    if s.auto_invert {
        cleanup::invert_if_negative(&mut img);
    }
    if s.auto_levels {
        cleanup::auto_levels(&mut img);
    }
    if s.dropout != Dropout::Off {
        let paper = transform::estimate_background(&img);
        cleanup::dropout(&mut img, s.dropout, paper);
    }
    if s.descreen {
        cleanup::descreen(&mut img, s.dpi);
    }
    if s.show_through {
        let paper = transform::estimate_background(&img);
        img = cleanup::show_through(&img, s.dpi, paper);
    }
    if !s.erase_zones.is_empty() {
        let paper = transform::estimate_background(&img);
        cleanup::erase_zones(&mut img, s.dpi, &s.erase_zones, paper);
    }
    if s.punch_holes {
        cleanup::remove_punch_holes(&mut img, s.dpi);
    }
    if s.fingers {
        let paper = transform::estimate_background(&img);
        cleanup::remove_fingers(&mut img, s.dpi, paper);
    }
    let bg = transform::estimate_background(&img);
    let mut skew_deg = 0.0;

    // --- deskew ---------------------------------------------------------
    let angle = match s.manual_skew_deg {
        Some(deg) => Some(deg.to_radians()),
        None if s.deskew => {
            let gray = imageops::grayscale(&img);
            let mut a = analyse(&gray, s.dpi);
            // Photographs have no text lines: their edges and tones would steer the estimate
            // (a mounted photo on a straight page came out rotated by 4°). Judge the skew
            // from what is left outside the picture zones.
            let f = a.factor.max(1);
            for z in pictures::detect(&gray, s.dpi, s.picture_sensitivity) {
                let pad = (z.rect.w.min(z.rect.h) / 25).max(f);
                let x0 = z.rect.x.saturating_sub(pad) / f;
                let y0 = z.rect.y.saturating_sub(pad) / f;
                let x1 = ((z.rect.x + z.rect.w + pad) / f + 1).min(a.ink.w);
                let y1 = ((z.rect.y + z.rect.h + pad) / f + 1).min(a.ink.h);
                for y in y0..y1 {
                    for x in x0..x1 {
                        a.ink.data[(y * a.ink.w + x) as usize] = 0;
                    }
                }
            }
            deskew::detect_skew(&a.ink, s.max_skew_deg)
        }
        None => None,
    };
    if let Some(a) = angle {
        // (an image too large to turn is left as it is, and nothing records a skew correction)
        if a.abs() > 0.05f32.to_radians() && transform::rotation_fits(img.width(), img.height(), a)
        {
            let (w, h) = img.dimensions();
            img = transform::rotate_cw(&img, a, bg);
            skew_deg = a.to_degrees();
            // follow the zones through the rotation (bounding box of the rotated corners)
            let (sn, cs) = (a.sin(), a.cos());
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let (ncx, ncy) = (img.width() as f32 / 2.0, img.height() as f32 / 2.0);
            for z in pictures.iter_mut() {
                let pts = [
                    (z[0], z[1]),
                    (z[0] + z[2], z[1]),
                    (z[0], z[1] + z[3]),
                    (z[0] + z[2], z[1] + z[3]),
                ]
                .map(|(x, y)| {
                    let (dx, dy) = (x - cx, y - cy);
                    (ncx + dx * cs - dy * sn, ncy + dx * sn + dy * cs)
                });
                let x0 = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min);
                let x1 = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max);
                let y0 = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
                let y1 = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max);
                *z = [x0, y0, x1 - x0, y1 - y0];
            }
            for p in guides.iter_mut().flat_map(|g| g.iter_mut()) {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                *p = (ncx + dx * cs - dy * sn, ncy + dx * sn + dy * cs);
            }
        }
    }

    // --- dewarp -----------------------------------------------------------
    // typed guides are fractions of the deskewed page, drawn ones follow the sheet to here
    let (iw, ih) = (img.width().max(1) as f32, img.height().max(1) as f32);
    let mut all: Guides = s.dewarp_guides.clone();
    all.extend(guides.iter().map(|g| {
        g.iter()
            .map(|p| ((p.0 / iw).clamp(0.0, 1.0), (p.1 / ih).clamp(0.0, 1.0)))
            .collect()
    }));
    if !all.is_empty() {
        if let Some(flat) = dewarp::dewarp_guides(&img, &all, bg) {
            img = flat;
        }
    } else if s.dewarp {
        if let Some(flat) = dewarp::dewarp(&img, s.dpi, bg) {
            img = flat;
        }
    }

    // --- content ----------------------------------------------------------
    let a = analyse(&imageops::grayscale(&img), s.dpi);
    let content = content::detect_content(&a.ink, a.dpi).map(|r| {
        // back to full resolution, grown by the analysis step so no ink is shaved
        let f = a.factor;
        let (x0, y0) = ((r.x * f).saturating_sub(f), (r.y * f).saturating_sub(f));
        let (x1, y1) = (
            ((r.x + r.w) * f + f).min(img.width()),
            ((r.y + r.h) * f + f).min(img.height()),
        );
        let mut rect = Rect {
            x: x0,
            y: y0,
            w: x1.saturating_sub(x0).max(1),
            h: y1.saturating_sub(y0).max(1),
        };
        // per-page manual correction
        let [al, at, ar, ab] = s
            .content_adjust_mm
            .map(|mm| (mm / 25.4 * s.dpi as f32).round() as i64);
        let nx0 = (rect.x as i64 - al).clamp(0, img.width() as i64 - 1);
        let ny0 = (rect.y as i64 - at).clamp(0, img.height() as i64 - 1);
        let nx1 = ((rect.x + rect.w) as i64 + ar).clamp(nx0 + 1, img.width() as i64);
        let ny1 = ((rect.y + rect.h) as i64 + ab).clamp(ny0 + 1, img.height() as i64);
        rect = Rect {
            x: nx0 as u32,
            y: ny0 as u32,
            w: (nx1 - nx0) as u32,
            h: (ny1 - ny0) as u32,
        };
        rect
    });
    // typed zones are in millimetres of the deskewed page
    let mm_px = |mm: f32| mm / 25.4 * s.dpi as f32;
    pictures.extend(
        s.picture_zones
            .iter()
            .map(|z| [mm_px(z[0]), mm_px(z[1]), mm_px(z[2]), mm_px(z[3])]),
    );
    Prepared {
        img,
        bg,
        skew_deg,
        content,
        pictures,
    }
}

/// Where the content window lands on the output canvas (all in source pixels).
struct Layout {
    /// Canvas size.
    cw: u32,
    ch: u32,
    /// Source-pixel position of the canvas's top-left corner.
    cx: i64,
    cy: i64,
}

fn layout(r: Rect, s: &Settings) -> Layout {
    let [ml, mt, mr, mb] = s.margins_mm.map(|m| mm_to_px(m, s.dpi));
    let (inner_w, inner_h) = (r.w + ml + mr, r.h + mt + mb);
    let (mut cw, mut ch) = (inner_w, inner_h);
    if let Some((pw, ph)) = s.page_size_mm {
        // ceil with a tiny tolerance: a size measured in mm must come back as the same pixels
        let px = |mm: f32| ((mm as f64 / 25.4 * s.dpi as f64) - 0.01).ceil().max(0.0) as u32;
        cw = cw.max(px(pw));
        ch = ch.max(px(ph));
    }
    let (extra_w, extra_h) = (cw - inner_w, ch - inner_h);
    let dx = match s.align.0 {
        HAlign::Left => 0,
        HAlign::Center => extra_w / 2,
        HAlign::Right => extra_w,
    };
    let dy = match s.align.1 {
        VAlign::Top => 0,
        VAlign::Center => extra_h / 2,
        VAlign::Bottom => extra_h,
    };
    Layout {
        cw,
        ch,
        cx: r.x as i64 - ml as i64 - dx as i64,
        cy: r.y as i64 - mt as i64 - dy as i64,
    }
}

/// Scale a 1-bit image by `f` (< 1) the way a scanner driver would: average, then re-threshold.
fn scale_bw(b: &BitImage, nw: u32, nh: u32) -> BitImage {
    let g = imageops::resize(
        &b.to_gray(),
        nw.max(1),
        nh.max(1),
        imageops::FilterType::Triangle,
    );
    // a 1-px rule or hairline serif averages to well above mid-grey when shrunk: threshold at
    // 200 so that anything with a quarter of its coverage left is still ink
    binarize::global(&g, 200)
}

fn render(p: Prepared, split_index: Option<u8>, s: &Settings) -> Page {
    let Prepared {
        img,
        bg,
        skew_deg,
        content,
        pictures,
    } = p;
    let blank = content.is_none();
    // Output resolution: only ever *down*-sampled.
    let (out_dpi, f) = match s.output_dpi {
        Some(o) if o < s.dpi => (o, o as f32 / s.dpi as f32),
        _ => (s.dpi, 1.0),
    };
    let scaled = |v: u32| {
        if f == 1.0 {
            v
        } else {
            ((v as f32 * f).round() as u32).max(1)
        }
    };
    let paper = if matches!(
        s.mode,
        OutputMode::Gray | OutputMode::BlackWhite | OutputMode::Mixed
    ) {
        Rgb([255, 255, 255])
    } else {
        bg
    };

    // Window taken from the scan: the content box plus a hair of safety. Everything
    // else — margins, scanner bed, edge shadows — becomes paper.
    let pad = (s.dpi / 100).max(2) as i64;
    // A blank sheet with a fixed page size becomes an empty page of exactly that size (so a PDF
    // keeps uniform pages); without one it stays the whole scan, as before.
    let blank_canvas = blank && s.page_size_mm.is_some();
    let (lay, win) = match content {
        None if blank_canvas => (
            layout(
                Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                },
                s,
            ),
            (0i64, 0i64, 0u32, 0u32),
        ),
        None => (
            Layout {
                cw: img.width(),
                ch: img.height(),
                cx: 0,
                cy: 0,
            },
            (0i64, 0i64, img.width(), img.height()),
        ),
        Some(r) => (
            layout(r, s),
            (
                r.x as i64 - pad,
                r.y as i64 - pad,
                r.w + 2 * pad as u32,
                r.h + 2 * pad as u32,
            ),
        ),
    };
    let (cw, ch) = (scaled(lay.cw), scaled(lay.ch));
    let (wx_out, wy_out) = (
        ((win.0 - lay.cx) as f32 * f).round() as i64,
        ((win.1 - lay.cy) as f32 * f).round() as i64,
    );
    let (ww_out, wh_out) = (scaled(win.2), scaled(win.3));

    // 1-bit ink layer for BW / Mixed / Hybrid: binarise the WHOLE deskewed scan first,
    // then cut (a window thresholded on a flat canvas gets a black frame on uneven paper).
    let wants_mask = matches!(
        s.mode,
        OutputMode::BlackWhite | OutputMode::Mixed | OutputMode::Hybrid
    );
    let mask: Option<BitImage> = wants_mask.then(|| {
        let g = imageops::grayscale(&img);
        let mut bw = binarize::threshold(&g, s.threshold, (s.dpi / 3) | 1, s.sauvola_k);
        if s.despeckle {
            let min_px =
                ((s.dpi as f32 / 300.0).powi(2) * 6.0 * s.despeckle_strength).round() as u32;
            bw.despeckle(min_px.max(2));
        }
        if s.remove_lines {
            cleanup::remove_lines(&mut bw, s.dpi);
        }
        let mut canvas = BitImage::new(cw, ch);
        if blank_canvas {
            return canvas; // empty page of the requested size
        }
        if blank {
            return if f == 1.0 { bw } else { scale_bw(&bw, cw, ch) };
        }
        let mut window = BitImage::new(win.2, win.3);
        window.paste_region(&bw, (win.0, win.1, win.2, win.3), (0, 0));
        let window = if f == 1.0 {
            window
        } else {
            scale_bw(&window, ww_out, wh_out)
        };
        canvas.paste_region(&window, (0, 0, window.w, window.h), (wx_out, wy_out));
        canvas
    });

    // Colour/gray window (also the source of picture patches and the hybrid background).
    let wants_colour = !matches!(s.mode, OutputMode::BlackWhite);
    let canvas_rgb: Option<RgbImage> = wants_colour.then(|| {
        if blank_canvas {
            return RgbImage::from_pixel(cw, ch, paper);
        }
        // the strip beyond the scan's edge is paper, not (possibly tinted) page colour
        let mut window = transform::crop_pad(&img, win.0, win.1, win.2, win.3, paper);
        if s.normalize_bg && window.width() > 8 && window.height() > 8 {
            let target = if s.mode == OutputMode::Gray {
                Rgb([255, 255, 255])
            } else {
                bg
            };
            window = transform::normalize_background(&window, s.dpi, target);
        }
        if s.sharpen > 0.0 {
            window = cleanup::sharpen(&window, s.dpi, s.sharpen);
        }
        if f != 1.0 {
            window = imageops::resize(&window, ww_out, wh_out, imageops::FilterType::Triangle);
        }
        let mut canvas = RgbImage::from_pixel(cw, ch, paper);
        imageops::replace(&mut canvas, &window, wx_out, wy_out);
        canvas
    });

    let (image, layers) = match s.mode {
        OutputMode::BlackWhite => (PageImage::Bw(mask.expect("mask")), None),
        OutputMode::Color => (PageImage::Rgb(canvas_rgb.expect("rgb")), None),
        OutputMode::Gray => (
            PageImage::Gray(imageops::grayscale(&canvas_rgb.expect("rgb"))),
            None,
        ),
        OutputMode::Mixed | OutputMode::Hybrid => {
            let (rgb, mut mask) = (canvas_rgb.expect("rgb"), mask.expect("mask"));
            if blank {
                (PageImage::Bw(mask), None)
            } else {
                let gray = imageops::grayscale(&rgb);
                let mut zones = pictures::detect(&gray, out_dpi, s.picture_sensitivity);
                // Hand-marked pictures: pixels of the deskewed page -> canvas pixels.
                for z in &pictures {
                    let to = |v: f32, o: i64| ((v - o as f32) * f).round() as i64;
                    let x0 = to(z[0], lay.cx).max(0);
                    let y0 = to(z[1], lay.cy).max(0);
                    let x1 = to(z[0] + z[2], lay.cx).min(cw as i64);
                    let y1 = to(z[1] + z[3], lay.cy).min(ch as i64);
                    if x1 > x0 && y1 > y0 {
                        zones.push(pictures::Zone {
                            rect: Rect {
                                x: x0 as u32,
                                y: y0 as u32,
                                w: (x1 - x0) as u32,
                                h: (y1 - y0) as u32,
                            },
                        });
                    }
                }
                let mut patches = Vec::new();
                for z in &zones {
                    let r = z.rect;
                    patches.push(Patch {
                        x: r.x,
                        y: r.y,
                        img: imageops::crop_imm(&rgb, r.x, r.y, r.w, r.h).to_image(),
                    });
                    for y in r.y..(r.y + r.h).min(mask.h) {
                        for x in r.x..(r.x + r.w).min(mask.w) {
                            mask.set(x, y, false); // the picture patch already shows these pixels
                        }
                    }
                }
                if s.mode == OutputMode::Mixed {
                    if patches.is_empty() {
                        (PageImage::Bw(mask), None)
                    } else {
                        let l = Layers {
                            mask,
                            fg: [0, 0, 0],
                            bg: None,
                            patches,
                        };
                        (PageImage::Rgb(l.compose()), Some(l))
                    }
                } else {
                    let fg = layers::ink_colour(&rgb, &mask);
                    let down = (out_dpi / s.hybrid_bg_dpi.max(1)).max(1);
                    let bg_img = layers::build_background(&rgb, &layers::dilate(&mask), down);
                    let l = Layers {
                        mask,
                        fg,
                        bg: Some(layers::Background {
                            img: bg_img,
                            dpi: out_dpi / down,
                        }),
                        patches,
                    };
                    (PageImage::Rgb(l.compose()), Some(l))
                }
            }
        }
    };
    let content_box = content.map(|r| {
        let px = |v: i64| ((v as f32) * f).round().max(0.0) as u32;
        [
            px(r.x as i64 - lay.cx),
            px(r.y as i64 - lay.cy),
            px(r.w as i64).max(1),
            px(r.h as i64).max(1),
        ]
    });
    Page {
        image,
        dpi: out_dpi,
        skew_deg,
        blank,
        split_index,
        content_box,
        layers,
    }
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

    #[test]
    fn drawn_guide_points_follow_quarter_turns_like_pixels() {
        for turns in 1..4u8 {
            let mut img = RgbImage::from_pixel(40, 24, Rgb([255, 255, 255]));
            img.put_pixel(7, 5, Rgb([0, 0, 0]));
            let mut g: Guides = vec![vec![(7.5, 5.5), (7.5, 5.5)]];
            rot90_points(&mut g, 40, 24, turns);
            let turned = transform::rot90(&img, turns);
            let (x, y) = (g[0][0].0.floor() as u32, g[0][0].1.floor() as u32);
            assert_eq!(turned.get_pixel(x, y)[0], 0, "turns {turns}: ({x},{y})");
        }
    }

    #[test]
    fn sheet_guides_straighten_after_rotation_and_junk_is_ignored() {
        let bend = |u: f32| 26.0 * (u - 0.5) * (u - 0.5) * 4.0 - 8.0 * (u - 0.5);
        let mut page = RgbImage::from_pixel(900, 1200, Rgb([238, 238, 238]));
        for k in 0..10 {
            let y0 = 120.0 + k as f32 * 100.0;
            for x in (60..840).step_by(21) {
                let dy = bend(x as f32 / 900.0);
                for yy in 0..13 {
                    for xx in 0..14 {
                        page.put_pixel(x + xx, (y0 + dy) as u32 + yy, Rgb([25, 25, 25]));
                    }
                }
            }
        }
        let src = DynamicImage::ImageRgb8(page);
        let guide: Vec<(f32, f32)> = (0..=10)
            .map(|i| {
                let u = i as f32 / 10.0;
                (u, (126.0 + bend(u)) / 1200.0)
            })
            .collect();
        let base = Settings {
            split: SplitMode::Off,
            deskew: false,
            ..Settings::default()
        };
        let with = Settings {
            sheet_dewarp_guides: vec![guide, vec![(f32::NAN, 0.5), (0.2, 9.0)]],
            ..base.clone()
        };
        let a = process_page(&src, &base);
        let b = process_page(&src, &with);
        assert_eq!((a.len(), b.len()), (1, 1));
        let (PageImage::Rgb(x), PageImage::Rgb(y)) = (&a[0].image, &b[0].image) else {
            panic!("colour output expected");
        };
        assert_ne!(x.as_raw(), y.as_raw(), "guides had no effect");
    }
    use super::*;
    use crate::{binarize, content, deskew, orient, split, transform};
    use image::{imageops, Rgb};

    const DPI: u32 = 300;

    #[test]
    fn blank_sheet_with_a_page_size_becomes_an_empty_page_of_that_size() {
        let blank = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            900,
            1300,
            Rgb([240, 240, 240]),
        ));
        for mode in [OutputMode::BlackWhite, OutputMode::Color, OutputMode::Gray] {
            let s = Settings {
                dpi: DPI,
                mode,
                page_size_mm: Some((100.0, 150.0)),
                ..Settings::default()
            };
            let pages = process_page(&blank, &s);
            assert_eq!(pages.len(), 1);
            let (w, h) = (pages[0].image.width(), pages[0].image.height());
            // 100 × 150 mm at 300 dpi
            assert!(
                (1180..=1182).contains(&w) && (1771..=1773).contains(&h),
                "{mode:?}: {w}x{h}"
            );
            assert!(pages[0].blank);
        }
    }

    #[test]
    fn shrinking_a_bilevel_page_keeps_one_pixel_rules() {
        let mut b = BitImage::new(400, 400);
        for x in 20..380 {
            b.set(x, 200, true); // a 1-px horizontal rule
        }
        let small = scale_bw(&b, 200, 200);
        let ink: u32 = (0..200)
            .map(|x| small.get(x, 100) as u32 + small.get(x, 99) as u32)
            .sum();
        assert!(ink > 120, "the rule must survive halving, kept {ink} px");
    }

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
    fn hand_marked_picture_zone_becomes_a_colour_patch() {
        let g = text_page(1200, 900, 100, 1100);
        let mut rgb = to_rgb(&g);
        // a flat colour block the automatic detector has no reason to call a picture
        for y in 700..800 {
            for x in 150..400 {
                rgb.put_pixel(x, y, Rgb([200, 40, 40]));
            }
        }
        let src = image::DynamicImage::ImageRgb8(rgb);
        let base = Settings {
            mode: OutputMode::Mixed,
            dpi: 300,
            deskew: false,
            auto_orient: false,
            ..Settings::default()
        };
        let none = &process_page(&src, &base)[0];
        let n0 = none.layers.as_ref().map_or(0, |l| l.patches.len());
        let forced = Settings {
            // 300 dpi: 25.4 mm = 300 px. Zone covers x 100..450, y 650..850 px.
            picture_zones: vec![[8.5, 55.0, 30.0, 17.0]],
            ..base.clone()
        };
        let page = &process_page(&src, &forced)[0];
        let l = page
            .layers
            .as_ref()
            .expect("layers when a picture is forced");
        assert!(l.patches.len() > n0, "{} patches vs {n0}", l.patches.len());
    }

    fn red_share(p: &Page) -> Option<f32> {
        let l = p.layers.as_ref()?;
        let (mut red, mut n) = (0u64, 0u64);
        for pt in &l.patches {
            for px in pt.img.pixels() {
                n += 1;
                if px[0] > 150 && px[1] < 90 {
                    red += 1;
                }
            }
        }
        (n > 0).then(|| red as f32 / n as f32)
    }

    #[test]
    fn mouse_drawn_zone_follows_the_picture_through_rotation_and_skew() {
        let g = text_page(1200, 900, 100, 1100);
        let mut rgb = to_rgb(&g);
        // red block at x 150..400, y 700..800 of the loaded sheet
        for y in 700..800 {
            for x in 150..400 {
                rgb.put_pixel(x, y, Rgb([200, 40, 40]));
            }
        }
        let src = image::DynamicImage::ImageRgb8(rgb);
        let mm = |px: f32| px / 300.0 * 25.4;
        for (quarters, skew) in [
            (0u8, None),
            (1, None),
            (3, None),
            (0, Some(3.0f32)),
            (1, Some(-4.0)),
        ] {
            let s = Settings {
                mode: OutputMode::Mixed,
                dpi: 300,
                deskew: false,
                manual_skew_deg: skew,
                auto_orient: false,
                rotate_quarters: quarters,
                sheet_picture_zones: vec![[mm(140.0), mm(690.0), mm(270.0), mm(120.0)]],
                ..Settings::default()
            };
            let page = &process_page(&src, &s)[0];
            let share = red_share(page).unwrap_or(0.0);
            assert!(
                share > 0.5,
                "quarters {quarters}, skew {skew:?}: only {share} of the patch is the block"
            );
        }
    }

    #[test]
    fn mouse_drawn_zone_lands_on_the_right_half_of_a_split_spread() {
        let g = text_page(2400, 900, 100, 2300);
        let mut rgb = to_rgb(&g);
        for y in 600..800 {
            for x in 1700..2000 {
                rgb.put_pixel(x, y, Rgb([200, 40, 40]));
            }
        }
        let src = image::DynamicImage::ImageRgb8(rgb);
        let mm = |px: f32| px / 300.0 * 25.4;
        let s = Settings {
            mode: OutputMode::Mixed,
            dpi: 300,
            deskew: false,
            split: SplitMode::Middle,
            sheet_picture_zones: vec![[mm(1690.0), mm(590.0), mm(320.0), mm(220.0)]],
            ..Settings::default()
        };
        let pages = process_page(&src, &s);
        assert_eq!(pages.len(), 2);
        assert!(red_share(&pages[0]).is_none(), "left page has no picture");
        assert!(red_share(&pages[1]).unwrap_or(0.0) > 0.5);
    }

    #[test]
    fn content_box_is_reported_on_the_output_page_and_follows_the_nudge() {
        let g = text_page(1200, 900, 100, 1100);
        let src = image::DynamicImage::ImageRgb8(to_rgb(&g));
        let base = Settings {
            mode: OutputMode::BlackWhite,
            dpi: 300,
            deskew: false,
            auto_orient: false,
            margins_mm: [10.0; 4],
            ..Settings::default()
        };
        let p0 = &process_page(&src, &base)[0];
        let b0 = p0.content_box.expect("box");
        let margin = (10.0f32 / 25.4 * 300.0).round() as i64;
        assert!((b0[0] as i64 - margin).abs() <= 2, "{b0:?}");
        assert!((b0[1] as i64 - margin).abs() <= 2, "{b0:?}");
        // box + margins = page
        assert!((b0[0] as i64 + b0[2] as i64 + margin - p0.image.width() as i64).abs() <= 3);
        // grow the box by 5 mm on the left: it gets 5 mm wider
        let nudged = Settings {
            content_adjust_mm: [5.0, 0.0, 0.0, 0.0],
            ..base.clone()
        };
        let b1 = process_page(&src, &nudged)[0].content_box.unwrap();
        let five = (5.0f32 / 25.4 * 300.0).round() as i64;
        assert!(
            (b1[2] as i64 - b0[2] as i64 - five).abs() <= 2,
            "{b0:?} -> {b1:?}"
        );
        // blank page: no box
        let blank =
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(300, 300, Rgb([240; 3])));
        assert!(process_page(&blank, &base)[0].content_box.is_none());
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
        assert!(!orient::needs_quarter_turn(&up, DPI));
        let side = imageops::rotate90(&g);
        assert!(orient::needs_quarter_turn(
            &binarize::sauvola(&side, 41, 0.3),
            DPI
        ));
    }

    #[test]
    fn rotated_two_page_spread_is_detected_and_upright_one_is_not() {
        // two text pages with a gutter between them: big low-frequency structure
        let mut spread = text_page(2400, 1500, 100, 2300);
        for y in 0..1500 {
            for x in 1150..1260 {
                spread.put_pixel(x, y, Luma([235]));
            }
        }
        let upright = binarize::sauvola(&spread, 41, 0.3);
        assert!(
            !orient::needs_quarter_turn(&upright, DPI),
            "upright spread flagged sideways"
        );
        for side in [imageops::rotate90(&spread), imageops::rotate270(&spread)] {
            assert!(
                orient::needs_quarter_turn(&binarize::sauvola(&side, 41, 0.3), DPI),
                "sideways spread missed"
            );
        }
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
    fn manual_skew_overrides_auto() {
        let g = text_page(1500, 2000, 250, 1250);
        let rot = transform::rotate_cw(&to_rgb(&g), 2.0f32.to_radians(), Rgb([235; 3]));
        let src = image::DynamicImage::ImageRgb8(rot);
        let off = Settings {
            deskew: false,
            ..Default::default()
        };
        assert_eq!(process_page(&src, &off)[0].skew_deg, 0.0);
        let manual = Settings {
            deskew: false,
            manual_skew_deg: Some(-2.0),
            ..Default::default()
        };
        let p = &process_page(&src, &manual)[0];
        assert!((p.skew_deg + 2.0).abs() < 1e-3);
        // a correct manual angle must give the same geometry as auto-deskew
        let auto = &process_page(&src, &Settings::default())[0];
        assert!((p.image.width() as i32 - auto.image.width() as i32).abs() < 12);
    }

    #[test]
    fn end_to_end_page_geometry() {
        let g = text_page(1500, 2000, 250, 1250);
        let rot = transform::rotate_cw(&to_rgb(&g), 2.0f32.to_radians(), Rgb([235; 3]));
        let s = Settings {
            margins_mm: [10.0; 4],
            mode: OutputMode::BlackWhite,
            ..Default::default()
        };
        let pages = process_page(&image::DynamicImage::ImageRgb8(rot), &s);
        assert_eq!(pages.len(), 1);
        let p = &pages[0];
        assert!((p.skew_deg + 2.0).abs() < 0.2, "skew {}", p.skew_deg);
        // content (~1000 px wide) + 2×10 mm margin (118 px each) = ≈ 1236
        assert!(
            (p.image.width() as i32 - 1236).abs() < 40,
            "w {}",
            p.image.width()
        );
        assert!(matches!(p.image, PageImage::Bw(_)));
    }

    #[test]
    fn sanitized_bounds_everything() {
        let wild = Settings {
            dpi: 0,
            margins_mm: [1e9, f32::NAN, -5.0, f32::INFINITY],
            page_size_mm: Some((1e9, f32::NAN)),
            max_skew_deg: 1e9,
            manual_skew_deg: Some(1e9),
            sauvola_k: f32::NAN,
            rotate_quarters: 7,
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(wild.dpi, 36);
        assert_eq!(wild.margins_mm, [200.0, 5.0, 0.0, 5.0]);
        assert_eq!(wild.page_size_mm, None);
        assert_eq!(
            (
                wild.max_skew_deg,
                wild.manual_skew_deg,
                wild.rotate_quarters
            ),
            (15.0, Some(45.0), 3)
        );
        assert!(wild.sauvola_k.is_finite());
    }

    #[test]
    fn solid_black_page_is_not_deskewed() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(900, 1200, Rgb([0, 0, 0])));
        let pages = process_page(
            &img,
            &Settings {
                dpi: 150,
                split: SplitMode::Off,
                auto_orient: false,
                ..Settings::default()
            },
        );
        assert_eq!(pages[0].skew_deg, 0.0);
    }

    #[test]
    fn uneven_illumination_gives_no_black_frame_in_bw() {
        // Text on paper that darkens strongly toward one corner.
        let (w, h) = (900u32, 1200u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let f = 1.0 - 0.55 * (y as f32 / h as f32) - 0.25 * (x as f32 / w as f32);
                let line = (y % 40) < 12
                    && (120..780).contains(&x)
                    && (150..1050).contains(&y)
                    && (x / 22) % 3 != 0;
                let v = if line { 60.0 * f } else { 235.0 * f };
                img.put_pixel(x, y, Rgb([v as u8, v as u8, (v * 0.95) as u8]));
            }
        }
        let st = Settings {
            dpi: 150,
            mode: OutputMode::BlackWhite,
            split: SplitMode::Off,
            auto_orient: false,
            deskew: false,
            ..Settings::default()
        };
        let pages = process_page(&DynamicImage::ImageRgb8(img), &st);
        let PageImage::Bw(bw) = &pages[0].image else {
            panic!("bw expected")
        };
        let (bw_w, bw_h) = (bw.w, bw.h);
        // Outer ring (margins) must be paper.
        let ring_ink: u64 = (0..bw_h)
            .flat_map(|y| (0..bw_w).map(move |x| (x, y)))
            .filter(|&(x, y)| x < 10 || y < 10 || x + 10 >= bw_w || y + 10 >= bw_h)
            .map(|(x, y)| bw.get(x, y) as u64)
            .sum();
        assert_eq!(ring_ink, 0, "black frame around the page");
        let ink_frac = bw.count_ink() as f64 / (bw_w as f64 * bw_h as f64);
        assert!((0.03..0.5).contains(&ink_frac), "ink fraction {ink_frac}");
    }

    /// Random tiny/odd images × random settings must never panic and must stay bounded.
    #[test]
    fn fuzz_no_panics() {
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut rnd = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for it in 0..220 {
            let (w, h) = (1 + rnd(400) as u32, 1 + rnd(400) as u32);
            let kind = rnd(4);
            let mut img = RgbImage::new(w, h);
            for (x, y, p) in img.enumerate_pixels_mut() {
                let v = match kind {
                    0 => 255,
                    1 => rnd(256) as u8,
                    2 => {
                        if (y / 7) % 3 == 0 && (x / 5) % 2 == 0 {
                            20
                        } else {
                            235
                        }
                    }
                    _ => {
                        if rnd(50) == 0 {
                            0
                        } else {
                            250
                        }
                    }
                };
                *p = Rgb([v, v, v]);
            }
            let s = Settings {
                dpi: [36, 72, 100, 150, 300, 600][rnd(6) as usize],
                rotate_quarters: rnd(4) as u8,
                auto_orient: rnd(2) == 0,
                split: [SplitMode::Off, SplitMode::Auto, SplitMode::Middle][rnd(3) as usize],
                deskew: rnd(2) == 0,
                manual_skew_deg: (rnd(3) == 0).then(|| rnd(120) as f32 - 60.0),
                max_skew_deg: rnd(20) as f32,
                margins_mm: [rnd(30) as f32; 4],
                page_size_mm: (rnd(2) == 0).then(|| (rnd(300) as f32, rnd(300) as f32)),
                mode: [
                    OutputMode::Color,
                    OutputMode::Gray,
                    OutputMode::BlackWhite,
                    OutputMode::Mixed,
                    OutputMode::Hybrid,
                ][rnd(5) as usize],
                sauvola_k: rnd(100) as f32 / 100.0,
                despeckle: rnd(2) == 0,
                dewarp: rnd(2) == 0,
                dewarp_guides: if rnd(3) == 0 {
                    vec![(0..3)
                        .map(|k| (k as f32 / 2.0, rnd(100) as f32 / 50.0 - 0.5))
                        .collect()]
                } else {
                    Vec::new()
                },
                content_adjust_mm: [rnd(40) as f32 - 20.0; 4],
                align: (
                    [HAlign::Left, HAlign::Center, HAlign::Right][rnd(3) as usize],
                    [VAlign::Top, VAlign::Center, VAlign::Bottom][rnd(3) as usize],
                ),
                threshold: Threshold::ALL[rnd(Threshold::ALL.len() as u64) as usize],
                normalize_bg: rnd(2) == 0,
                output_dpi: (rnd(2) == 0).then(|| [10, 36, 72, 150][rnd(4) as usize]),
                picture_sensitivity: rnd(100) as f32 / 100.0,
                hybrid_bg_dpi: rnd(400) as u32,
                auto_invert: rnd(2) == 0,
                auto_levels: rnd(2) == 0,
                sharpen: rnd(300) as f32 / 100.0 - 0.5,
                punch_holes: rnd(2) == 0,
                fingers: rnd(2) == 0,
                remove_lines: rnd(2) == 0,
                sheet_erase_zones: vec![],
                sheet_dewarp_guides: if rnd(3) == 0 {
                    vec![(0..3)
                        .map(|k| (k as f32 / 2.0, rnd(100) as f32 / 50.0 - 0.5))
                        .collect()]
                } else {
                    Vec::new()
                },
                sheet_picture_zones: if rnd(3) == 0 {
                    vec![[
                        rnd(300) as f32 - 50.0,
                        rnd(300) as f32 - 50.0,
                        rnd(200) as f32,
                        rnd(200) as f32,
                    ]]
                } else {
                    vec![]
                },
                picture_zones: if rnd(3) == 0 {
                    vec![[
                        rnd(200) as f32 - 20.0,
                        rnd(200) as f32 - 20.0,
                        rnd(150) as f32,
                        rnd(150) as f32,
                    ]]
                } else {
                    vec![]
                },
                erase_zones: if rnd(2) == 0 {
                    vec![[
                        rnd(50) as f32,
                        rnd(50) as f32,
                        rnd(80) as f32,
                        rnd(80) as f32,
                    ]]
                } else {
                    vec![]
                },
                dropout: Dropout::ALL[rnd(4) as usize],
                despeckle_strength: rnd(500) as f32 / 100.0,
                show_through: rnd(2) == 0,
                descreen: rnd(2) == 0,
                perspective: rnd(2) == 0,
            };
            let pages = process_page(&DynamicImage::ImageRgb8(img), &s);
            assert!(!pages.is_empty() && pages.len() <= 2, "iteration {it}");
            for p in pages {
                assert!(p.image.width() >= 1 && p.image.height() >= 1);
                assert!(
                    p.image.width() < 12_000 && p.image.height() < 12_000,
                    "iteration {it}: {}x{}",
                    p.image.width(),
                    p.image.height()
                );
            }
        }
    }
}
