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
    /// Drop one printed colour (form rules, stamps).
    pub dropout: Dropout,
    /// Despeckle strength 0.25–4 (1 = default speck size).
    pub despeckle_strength: f32,
    /// Fade print showing through thin paper (also levels the lighting).
    pub show_through: bool,
    /// Smooth halftone screens of printed photographs.
    pub descreen: bool,
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
            dropout: Dropout::Off,
            despeckle_strength: 1.0,
            show_through: false,
            descreen: false,
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
        s.output_dpi = s.output_dpi.filter(|&o| o >= 36).map(|o| o.min(s.dpi));
        s
    }
}

/// Stages shared by [`process_page`] and [`measure_page`]: orientation and page split.
fn split_sheet(src: &DynamicImage, s: &Settings) -> Vec<(RgbImage, Option<u8>)> {
    let mut rgb = transform::rot90(&src.to_rgb8(), s.rotate_quarters);

    if s.auto_orient {
        let a = analyse(&imageops::grayscale(&rgb), s.dpi);
        if orient::needs_quarter_turn(&a.ink, a.dpi) {
            rgb = transform::rot90(&rgb, 1);
        }
    }

    let mut parts: Vec<(RgbImage, Option<u8>)> = vec![(rgb.clone(), None)];
    if s.split != SplitMode::Off {
        let cut = match s.split {
            SplitMode::Middle => {
                (rgb.width() as f32 >= 1.15 * rgb.height() as f32).then_some(rgb.width() / 2)
            }
            _ => {
                let a = analyse(&imageops::grayscale(&rgb), s.dpi);
                split::find_gutter(&a.ink)
                    .map(|x| (x * a.factor).min(rgb.width().saturating_sub(1)))
            }
        };
        if let Some(x) = cut.filter(|&x| x > 0 && x < rgb.width()) {
            let l = imageops::crop_imm(&rgb, 0, 0, x, rgb.height()).to_image();
            let r = imageops::crop_imm(&rgb, x, 0, rgb.width() - x, rgb.height()).to_image();
            parts = vec![(l, Some(0)), (r, Some(1))];
        }
    }
    parts
}

/// Run the whole pipeline on one scanned sheet. A two-page spread yields two pages.
pub fn process_page(src: &DynamicImage, s: &Settings) -> Vec<Page> {
    let s = &s.sanitized();
    split_sheet(src, s)
        .into_iter()
        .map(|(img, idx)| render(prepare(img, s), idx, s))
        .collect()
}

/// Size in millimetres (content + margins) of every page the sheet yields, or `None`
/// for a blank page. Used to make all pages of a book the same size without keeping
/// them in memory: measure first, render second.
pub fn measure_page(src: &DynamicImage, s: &Settings) -> Vec<Option<(f32, f32)>> {
    let s = &s.sanitized();
    split_sheet(src, s)
        .into_iter()
        .map(|(img, _)| {
            let p = prepare(img, s);
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
}

fn prepare(mut img: RgbImage, s: &Settings) -> Prepared {
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
            let a = analyse(&imageops::grayscale(&img), s.dpi);
            deskew::detect_skew(&a.ink, s.max_skew_deg)
        }
        None => None,
    };
    if let Some(a) = angle {
        if a.abs() > 0.02f32.to_radians() {
            img = transform::rotate_cw(&img, a, bg);
            skew_deg = a.to_degrees();
        }
    }

    // --- dewarp -----------------------------------------------------------
    if s.dewarp {
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
    Prepared {
        img,
        bg,
        skew_deg,
        content,
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
    binarize::global(&g, 127)
}

fn render(p: Prepared, split_index: Option<u8>, s: &Settings) -> Page {
    let Prepared {
        img,
        bg,
        skew_deg,
        content,
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
    let (lay, win) = match content {
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
        let mut window = transform::crop_pad(&img, win.0, win.1, win.2, win.3, bg);
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
                let zones = pictures::detect(&gray, out_dpi, s.picture_sensitivity);
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
    Page {
        image,
        dpi: out_dpi,
        skew_deg,
        blank,
        split_index,
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
                dropout: Dropout::ALL[rnd(4) as usize],
                despeckle_strength: rnd(500) as f32 / 100.0,
                show_through: rnd(2) == 0,
                descreen: rnd(2) == 0,
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
