//! GUI-side settings and their mapping onto the core pipeline settings.

use legatorocr_core::{HAlign, OutputMode, Settings, SplitMode, Threshold, VAlign};
use legatorocr_engine::ocr::OcrOptions;
use legatorocr_engine::project::{Project, SheetAdj};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitChoice {
    Auto,
    Off,
    Middle,
}

impl From<SplitChoice> for SplitMode {
    fn from(c: SplitChoice) -> Self {
        match c {
            SplitChoice::Auto => SplitMode::Auto,
            SplitChoice::Off => SplitMode::Off,
            SplitChoice::Middle => SplitMode::Middle,
        }
    }
}

impl From<SplitMode> for SplitChoice {
    fn from(m: SplitMode) -> Self {
        match m {
            SplitMode::Auto => SplitChoice::Auto,
            SplitMode::Off => SplitChoice::Off,
            SplitMode::Middle => SplitChoice::Middle,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageSizeChoice {
    Free,
    A4,
    A5,
    Letter,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModeChoice {
    Color,
    Gray,
    Bw,
    Mixed,
    Hybrid,
}

#[derive(Clone, Debug)]
pub struct UiSettings {
    pub dpi: u32,
    pub auto_orient: bool,
    /// Turn pages upright with Tesseract's orientation detection (needs `osd` data).
    pub use_osd: bool,
    pub split: SplitChoice,
    pub deskew: bool,
    pub max_skew: f32,
    pub margin_mm: f32,
    pub page_size: PageSizeChoice,
    pub custom_w: f32,
    pub custom_h: f32,
    pub mode: ModeChoice,
    pub sauvola_k: f32,
    pub despeckle: bool,
    pub threshold: Threshold,
    pub normalize_bg: bool,
    pub dewarp: bool,
    /// 0 = keep the scan resolution.
    pub output_dpi: u32,
    pub picture_sensitivity: f32,
    pub hybrid_dpi: u32,
    pub auto_invert: bool,
    pub auto_levels: bool,
    pub sharpen: f32,
    pub punch_holes: bool,
    pub fingers: bool,
    pub remove_lines: bool,
    /// `x,y,w,h;…` in mm (see `legatorocr_core::cleanup::parse_zones`).
    pub erase: String,
    pub picture_zones: String,
    pub dropout: legatorocr_core::Dropout,
    pub despeckle_strength: f32,
    pub show_through: bool,
    pub descreen: bool,
    pub perspective: bool,
    pub align: (HAlign, VAlign),
    pub pdf_quality: u8,
    pub skip_blank: bool,
    pub uniform_size: bool,
    /// Write `<pdf>.txt` / `<pdf>.hocr` next to the PDF when OCR is on.
    pub skip_duplicates: bool,
    pub split_on_blank: bool,
    pub split_on_barcode: bool,
    pub bookmarks: bool,
    pub pdfa: bool,
    pub front_matter: u32,
    /// 0 = off; otherwise the PDF should stay within this many MB.
    pub target_mb: u32,
    pub sidecar_txt: bool,
    pub sidecar_hocr: bool,
    pub pdf_author: String,
    pub ocr: bool,
    pub ocr_langs: Vec<String>,
    pub psm: u32,
    pub min_conf: f32,
    pub tesseract: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            dpi: 300,
            auto_orient: true,
            use_osd: false,
            split: SplitChoice::Auto,
            deskew: true,
            max_skew: 7.0,
            margin_mm: 5.0,
            page_size: PageSizeChoice::Free,
            custom_w: 148.0,
            custom_h: 210.0,
            mode: ModeChoice::Color,
            sauvola_k: 0.3,
            despeckle: true,
            threshold: Threshold::Sauvola,
            normalize_bg: false,
            dewarp: false,
            output_dpi: 0,
            picture_sensitivity: 0.5,
            hybrid_dpi: 100,
            auto_invert: false,
            auto_levels: false,
            sharpen: 0.0,
            punch_holes: false,
            fingers: false,
            remove_lines: false,
            erase: String::new(),
            picture_zones: String::new(),
            dropout: legatorocr_core::Dropout::Off,
            despeckle_strength: 1.0,
            show_through: false,
            descreen: false,
            perspective: false,
            align: (HAlign::Center, VAlign::Center),
            pdf_quality: 80,
            skip_blank: false,
            uniform_size: false,
            skip_duplicates: false,
            split_on_blank: false,
            split_on_barcode: false,
            bookmarks: false,
            pdfa: false,
            front_matter: 0,
            target_mb: 0,
            sidecar_txt: false,
            sidecar_hocr: false,
            pdf_author: String::new(),
            ocr: false,
            ocr_langs: vec!["eng".into()],
            psm: 3,
            min_conf: 0.0,
            tesseract: "tesseract".into(),
        }
    }
}

/// Per-sheet overrides (everything else is global).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetAdjust {
    pub rotate_quarters: u8,
    pub split: Option<SplitChoice>,
    /// Manual skew correction in degrees, clockwise positive.
    pub skew_deg: Option<f32>,
    /// Move the detected content box outwards by this many mm per side (l, t, r, b).
    pub content_adjust_mm: [f32; 4],
    /// Erase rectangles drawn on the original (x, y, w, h in mm).
    pub erase_mm: Vec<[f32; 4]>,
    pub picture_mm: Vec<[f32; 4]>,
}

impl UiSettings {
    pub fn page_size_mm(&self) -> Option<(f32, f32)> {
        match self.page_size {
            PageSizeChoice::Free => None,
            PageSizeChoice::A4 => Some((210.0, 297.0)),
            PageSizeChoice::A5 => Some((148.0, 210.0)),
            PageSizeChoice::Letter => Some((215.9, 279.4)),
            PageSizeChoice::Custom => Some((self.custom_w.max(10.0), self.custom_h.max(10.0))),
        }
    }

    pub fn to_core(&self, adj: &SheetAdjust) -> Settings {
        Settings {
            dpi: self.dpi.max(36),
            rotate_quarters: adj.rotate_quarters % 4,
            auto_orient: self.auto_orient,
            split: adj.split.unwrap_or(self.split).into(),
            deskew: self.deskew,
            manual_skew_deg: adj.skew_deg,
            max_skew_deg: self.max_skew,
            margins_mm: [self.margin_mm; 4],
            page_size_mm: self.page_size_mm(),
            mode: match self.mode {
                ModeChoice::Color => OutputMode::Color,
                ModeChoice::Gray => OutputMode::Gray,
                ModeChoice::Bw => OutputMode::BlackWhite,
                ModeChoice::Mixed => OutputMode::Mixed,
                ModeChoice::Hybrid => OutputMode::Hybrid,
            },
            sauvola_k: self.sauvola_k,
            despeckle: self.despeckle,
            dewarp: self.dewarp,
            content_adjust_mm: adj.content_adjust_mm,
            sheet_erase_zones: adj.erase_mm.clone(),
            sheet_picture_zones: adj.picture_mm.clone(),
            align: self.align,
            threshold: self.threshold,
            normalize_bg: self.normalize_bg,
            output_dpi: (self.output_dpi >= 36).then_some(self.output_dpi),
            picture_sensitivity: self.picture_sensitivity,
            hybrid_bg_dpi: self.hybrid_dpi,
            auto_invert: self.auto_invert,
            auto_levels: self.auto_levels,
            sharpen: self.sharpen,
            punch_holes: self.punch_holes,
            fingers: self.fingers,
            remove_lines: self.remove_lines,
            erase_zones: legatorocr_core::cleanup::parse_zones(&self.erase).unwrap_or_default(),
            picture_zones: legatorocr_core::cleanup::parse_zones(&self.picture_zones)
                .unwrap_or_default(),
            dropout: self.dropout,
            despeckle_strength: self.despeckle_strength,
            show_through: self.show_through,
            descreen: self.descreen,
            perspective: self.perspective,
        }
    }

    /// Everything as a project (global settings + extras); `sheets` as given.
    pub fn to_project(&self, sheets: Vec<(PathBuf, SheetAdj)>) -> Project {
        let mut p = Project {
            settings: self.to_core(&SheetAdjust::default()),
            extra: Vec::new(),
            sheets,
        };
        let e = &mut p.extra;
        e.push(("pdf_quality".into(), self.pdf_quality.to_string()));
        e.push(("skip_blank".into(), self.skip_blank.to_string()));
        e.push(("uniform_size".into(), self.uniform_size.to_string()));
        e.push(("skip_duplicates".into(), self.skip_duplicates.to_string()));
        e.push(("split_on_blank".into(), self.split_on_blank.to_string()));
        e.push(("split_on_barcode".into(), self.split_on_barcode.to_string()));
        e.push(("bookmarks".into(), self.bookmarks.to_string()));
        e.push(("pdfa".into(), self.pdfa.to_string()));
        e.push(("front_matter".into(), self.front_matter.to_string()));
        e.push(("target_mb".into(), self.target_mb.to_string()));
        e.push(("sidecar_txt".into(), self.sidecar_txt.to_string()));
        e.push(("sidecar_hocr".into(), self.sidecar_hocr.to_string()));
        e.push(("pdf_author".into(), self.pdf_author.clone()));
        e.push(("use_osd".into(), self.use_osd.to_string()));
        e.push(("ocr".into(), self.ocr.to_string()));
        e.push(("ocr_langs".into(), self.ocr_langs.join("+")));
        e.push(("psm".into(), self.psm.to_string()));
        e.push(("min_conf".into(), self.min_conf.to_string()));
        e.push(("tesseract".into(), self.tesseract.clone()));
        p
    }

    /// Adopt a project's settings. Values that do not parse keep their current setting.
    pub fn apply_project(&mut self, p: &Project) {
        let c = &p.settings;
        self.dpi = c.dpi;
        self.auto_orient = c.auto_orient;
        self.split = match c.split {
            SplitMode::Auto => SplitChoice::Auto,
            SplitMode::Off => SplitChoice::Off,
            SplitMode::Middle => SplitChoice::Middle,
        };
        self.deskew = c.deskew;
        self.max_skew = c.max_skew_deg;
        self.margin_mm = c.margins_mm[0];
        self.page_size = match c.page_size_mm {
            None => PageSizeChoice::Free,
            Some((w, h)) => {
                let is = |a: f32, b: f32| (w - a).abs() < 0.2 && (h - b).abs() < 0.2;
                if is(210.0, 297.0) {
                    PageSizeChoice::A4
                } else if is(148.0, 210.0) {
                    PageSizeChoice::A5
                } else if is(215.9, 279.4) {
                    PageSizeChoice::Letter
                } else {
                    self.custom_w = w;
                    self.custom_h = h;
                    PageSizeChoice::Custom
                }
            }
        };
        self.mode = match c.mode {
            OutputMode::Color => ModeChoice::Color,
            OutputMode::Gray => ModeChoice::Gray,
            OutputMode::BlackWhite => ModeChoice::Bw,
            OutputMode::Mixed => ModeChoice::Mixed,
            OutputMode::Hybrid => ModeChoice::Hybrid,
        };
        self.sauvola_k = c.sauvola_k;
        self.despeckle = c.despeckle;
        self.dewarp = c.dewarp;
        self.align = c.align;
        self.threshold = c.threshold;
        self.normalize_bg = c.normalize_bg;
        self.output_dpi = c.output_dpi.unwrap_or(0);
        self.picture_sensitivity = c.picture_sensitivity;
        self.hybrid_dpi = c.hybrid_bg_dpi;
        self.auto_invert = c.auto_invert;
        self.auto_levels = c.auto_levels;
        self.sharpen = c.sharpen;
        self.punch_holes = c.punch_holes;
        self.fingers = c.fingers;
        self.remove_lines = c.remove_lines;
        self.erase = legatorocr_core::cleanup::zones_to_string(&c.erase_zones);
        self.picture_zones = legatorocr_core::cleanup::zones_to_string(&c.picture_zones);
        self.dropout = c.dropout;
        self.despeckle_strength = c.despeckle_strength;
        self.show_through = c.show_through;
        self.descreen = c.descreen;
        self.perspective = c.perspective;
        if let Some(v) = p.get("pdf_quality").and_then(|v| v.parse::<u8>().ok()) {
            self.pdf_quality = v.min(100);
        }
        let flag = |k: &str| p.get(k).map(|v| matches!(v, "true" | "yes" | "on" | "1"));
        self.skip_blank = flag("skip_blank").unwrap_or(self.skip_blank);
        self.uniform_size = flag("uniform_size").unwrap_or(self.uniform_size);
        self.skip_duplicates = flag("skip_duplicates").unwrap_or(self.skip_duplicates);
        self.split_on_blank = flag("split_on_blank").unwrap_or(self.split_on_blank);
        self.split_on_barcode = flag("split_on_barcode").unwrap_or(self.split_on_barcode);
        self.bookmarks = flag("bookmarks").unwrap_or(self.bookmarks);
        self.pdfa = flag("pdfa").unwrap_or(self.pdfa);
        if let Some(v) = p.get("front_matter").and_then(|v| v.parse::<u32>().ok()) {
            self.front_matter = v.min(999);
        }
        if let Some(v) = p.get("target_mb").and_then(|v| v.parse::<u32>().ok()) {
            self.target_mb = v.min(100_000);
        }
        self.sidecar_txt = flag("sidecar_txt").unwrap_or(self.sidecar_txt);
        self.sidecar_hocr = flag("sidecar_hocr").unwrap_or(self.sidecar_hocr);
        if let Some(a) = p.get("pdf_author") {
            self.pdf_author = a.chars().take(200).collect();
        }
        self.use_osd = flag("use_osd").unwrap_or(self.use_osd);
        self.ocr = flag("ocr").unwrap_or(self.ocr);
        if let Some(l) = p.get("ocr_langs") {
            self.ocr_langs = l
                .split('+')
                .filter(|x| legatorocr_engine::ocr::valid_lang(x))
                .map(str::to_string)
                .collect();
        }
        if let Some(v) = p
            .get("psm")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| (1..=13).contains(v))
        {
            self.psm = v;
        }
        if let Some(v) = p
            .get("min_conf")
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite())
        {
            self.min_conf = v.clamp(0.0, 100.0);
        }
        // the OCR program is a per-machine choice: never taken from a project file (it would be
        // executed at the next export), only from the user's own config file
    }

    /// Per-machine choices that only the user's own config file may set (never a shared project
    /// or profile): the Tesseract program, which would be executed at the next OCR run.
    pub fn apply_machine(&mut self, p: &Project) {
        if let Some(v) = p
            .get("tesseract")
            .filter(|v| !v.is_empty() && !v.chars().any(char::is_control))
        {
            self.tesseract = v.to_string();
        }
    }

    /// Tesseract language string (`ron+eng`); falls back to `eng`.
    pub fn lang_string(&self) -> String {
        if self.ocr_langs.is_empty() {
            "eng".into()
        } else {
            self.ocr_langs.join("+")
        }
    }

    pub fn ocr_options(&self) -> OcrOptions {
        OcrOptions {
            engine: legatorocr_engine::ocr::Engine::Auto,
            lang: self.lang_string(),
            psm: self.psm,
            min_conf: self.min_conf,
            tesseract: self.tesseract.clone().into(),
            tessdata: None,
        }
    }
}

/// Where the user's settings live: `…/legatorocr/settings.lgr` in the platform config dir.
pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("legatorocr").join("settings.lgr"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_project_round_trip() {
        let s = UiSettings {
            dpi: 450,
            mode: ModeChoice::Hybrid,
            threshold: legatorocr_core::Threshold::Nick,
            page_size: PageSizeChoice::Custom,
            custom_w: 123.0,
            custom_h: 234.0,
            align: (HAlign::Left, VAlign::Bottom),
            output_dpi: 200,
            dewarp: true,
            normalize_bg: true,
            ocr: true,
            ocr_langs: vec!["ron".into(), "eng".into()],
            psm: 6,
            pdf_quality: 55,
            uniform_size: true,
            sidecar_hocr: true,
            skip_duplicates: true,
            target_mb: 25,
            split_on_barcode: true,
            bookmarks: true,
            pdfa: true,
            front_matter: 4,
            pdf_author: "Ana".into(),
            auto_invert: true,
            sharpen: 0.8,
            punch_holes: true,
            remove_lines: true,
            erase: "10,20,30,15;0,0,5,5".into(),
            picture_zones: "5,5,40,30".into(),
            dropout: legatorocr_core::Dropout::Blue,
            despeckle_strength: 2.5,
            show_through: true,
            perspective: true,
            ..Default::default()
        };
        let text = s.to_project(vec![]).to_text(None);
        let mut back = UiSettings::default();
        back.apply_project(&Project::parse(&text, None).unwrap());
        assert_eq!(back.to_core(&SheetAdjust::default()).dpi, 450);
        assert_eq!(back.mode, ModeChoice::Hybrid);
        assert_eq!(back.threshold, legatorocr_core::Threshold::Nick);
        assert_eq!(
            (back.page_size, back.custom_w, back.custom_h),
            (PageSizeChoice::Custom, 123.0, 234.0)
        );
        assert_eq!(back.align, (HAlign::Left, VAlign::Bottom));
        assert_eq!(
            (back.output_dpi, back.dewarp, back.normalize_bg),
            (200, true, true)
        );
        assert_eq!(
            (back.ocr, back.ocr_langs.clone(), back.psm),
            (true, vec!["ron".into(), "eng".into()], 6)
        );
        assert_eq!((back.pdf_quality, back.uniform_size), (55, true));
        assert_eq!(back.erase, "10,20,30,15;0,0,5,5");
        assert_eq!(back.picture_zones, "5,5,40,30");
        assert_eq!(
            (back.split_on_barcode, back.bookmarks, back.front_matter),
            (true, true, 4)
        );
        assert!(back.pdfa);
        assert_eq!(
            (back.skip_duplicates, back.split_on_blank, back.target_mb),
            (true, false, 25)
        );
        assert_eq!(
            (back.show_through, back.descreen, back.perspective),
            (true, false, true)
        );
        assert_eq!(
            (
                back.sidecar_txt,
                back.sidecar_hocr,
                back.pdf_author.as_str()
            ),
            (false, true, "Ana")
        );
        assert_eq!(
            (
                back.auto_invert,
                back.punch_holes,
                back.remove_lines,
                back.fingers
            ),
            (true, true, true, false)
        );
        assert_eq!(
            (back.sharpen, back.despeckle_strength, back.dropout),
            (0.8, 2.5, legatorocr_core::Dropout::Blue)
        );
    }

    #[test]
    fn config_file_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("lgr-cfg-{}", std::process::id()));
        let path = dir.join("nested").join("settings.lgr");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let s = UiSettings {
            mode: ModeChoice::Mixed,
            margin_mm: 7.5,
            ..Default::default()
        };
        let mut p = s.to_project(Vec::new());
        p.extra.push(("lang".into(), "hu".into()));
        p.save(&path).unwrap();
        let back = Project::load(&path).unwrap();
        let mut t = UiSettings::default();
        t.apply_project(&back);
        assert_eq!((t.mode, t.margin_mm), (ModeChoice::Mixed, 7.5));
        assert_eq!(back.get("lang"), Some("hu"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn project_files_cannot_pick_the_ocr_program() {
        let text = "legatorocr-project 1\ntesseract = /tmp/evil\nocr = true\n";
        let p = Project::parse(text, None).unwrap();
        let mut s = UiSettings::default();
        s.apply_project(&p);
        assert_eq!(s.tesseract, "tesseract");
        s.apply_machine(&p); // the user's own config may name the program
        assert_eq!(s.tesseract, "/tmp/evil");
        let bad = Project::parse("legatorocr-project 1\ntesseract = a\u{7}b\n", None).unwrap();
        let mut t = UiSettings::default();
        t.apply_machine(&bad);
        assert_eq!(t.tesseract, "tesseract");
    }

    #[test]
    fn hostile_config_values_are_ignored() {
        let text = "legatorocr-project 1\npsm = 99\nocr_langs = a;b+--x\npdf_quality = 900\nmin_conf = nan\n";
        let mut s = UiSettings::default();
        s.apply_project(&Project::parse(text, None).unwrap());
        assert_eq!(s.psm, 3);
        assert!(s.ocr_langs.is_empty());
        assert_eq!(s.pdf_quality, 80); // 900 does not parse as u8
        assert_eq!(s.min_conf, 0.0);
    }

    #[test]
    fn per_sheet_overrides_win() {
        let s = UiSettings {
            split: SplitChoice::Auto,
            deskew: true,
            ..Default::default()
        };
        let c = s.to_core(&SheetAdjust {
            rotate_quarters: 5,
            split: Some(SplitChoice::Off),
            skew_deg: Some(-1.5),
            ..Default::default()
        });
        assert_eq!(c.split, SplitMode::Off);
        assert_eq!(c.rotate_quarters, 1); // 5 mod 4
        assert_eq!(c.manual_skew_deg, Some(-1.5));
        let d = s.to_core(&SheetAdjust::default());
        assert_eq!(d.split, SplitMode::Auto);
        assert_eq!(d.manual_skew_deg, None);
    }

    #[test]
    fn page_sizes_and_modes() {
        let mut s = UiSettings::default();
        assert_eq!(s.page_size_mm(), None);
        s.page_size = PageSizeChoice::A5;
        assert_eq!(s.page_size_mm(), Some((148.0, 210.0)));
        s.page_size = PageSizeChoice::Custom;
        s.custom_w = 3.0; // clamped to a sane minimum
        assert_eq!(s.page_size_mm().unwrap().0, 10.0);
        s.mode = ModeChoice::Bw;
        assert_eq!(
            s.to_core(&SheetAdjust::default()).mode,
            OutputMode::BlackWhite
        );
    }

    #[test]
    fn language_string() {
        let mut s = UiSettings::default();
        assert_eq!(s.lang_string(), "eng");
        s.ocr_langs = vec!["ron".into(), "eng".into()];
        assert_eq!(s.ocr_options().lang, "ron+eng");
        s.ocr_langs.clear();
        assert_eq!(s.lang_string(), "eng");
    }
}
