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
    pub align: (HAlign, VAlign),
    pub pdf_quality: u8,
    pub skip_blank: bool,
    pub uniform_size: bool,
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
            align: (HAlign::Center, VAlign::Center),
            pdf_quality: 80,
            skip_blank: false,
            uniform_size: false,
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
            align: self.align,
            threshold: self.threshold,
            normalize_bg: self.normalize_bg,
            output_dpi: (self.output_dpi >= 36).then_some(self.output_dpi),
            picture_sensitivity: self.picture_sensitivity,
            hybrid_bg_dpi: self.hybrid_dpi,
        }
    }

    /// Everything as a project (global settings + extras); `sheets` as given.
    pub fn to_project(&self, sheets: Vec<(PathBuf, SheetAdj)>) -> Project {
        let mut p = Project { settings: self.to_core(&SheetAdjust::default()), extra: Vec::new(), sheets };
        let e = &mut p.extra;
        e.push(("pdf_quality".into(), self.pdf_quality.to_string()));
        e.push(("skip_blank".into(), self.skip_blank.to_string()));
        e.push(("uniform_size".into(), self.uniform_size.to_string()));
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
        if let Some(v) = p.get("pdf_quality").and_then(|v| v.parse::<u8>().ok()) {
            self.pdf_quality = v.min(100);
        }
        let flag = |k: &str| p.get(k).map(|v| matches!(v, "true" | "yes" | "on" | "1"));
        self.skip_blank = flag("skip_blank").unwrap_or(self.skip_blank);
        self.uniform_size = flag("uniform_size").unwrap_or(self.uniform_size);
        self.use_osd = flag("use_osd").unwrap_or(self.use_osd);
        self.ocr = flag("ocr").unwrap_or(self.ocr);
        if let Some(l) = p.get("ocr_langs") {
            self.ocr_langs = l.split('+').filter(|x| legatorocr_engine::ocr::valid_lang(x)).map(str::to_string).collect();
        }
        if let Some(v) = p.get("psm").and_then(|v| v.parse::<u32>().ok()).filter(|v| (1..=13).contains(v)) {
            self.psm = v;
        }
        if let Some(v) = p.get("min_conf").and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()) {
            self.min_conf = v.clamp(0.0, 100.0);
        }
        if let Some(v) = p.get("tesseract").filter(|v| !v.is_empty()) {
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
            ..Default::default()
        };
        let text = s.to_project(vec![]).to_text(None);
        let mut back = UiSettings::default();
        back.apply_project(&Project::parse(&text, None).unwrap());
        assert_eq!(back.to_core(&SheetAdjust::default()).dpi, 450);
        assert_eq!(back.mode, ModeChoice::Hybrid);
        assert_eq!(back.threshold, legatorocr_core::Threshold::Nick);
        assert_eq!((back.page_size, back.custom_w, back.custom_h), (PageSizeChoice::Custom, 123.0, 234.0));
        assert_eq!(back.align, (HAlign::Left, VAlign::Bottom));
        assert_eq!((back.output_dpi, back.dewarp, back.normalize_bg), (200, true, true));
        assert_eq!((back.ocr, back.ocr_langs.clone(), back.psm), (true, vec!["ron".into(), "eng".into()], 6));
        assert_eq!((back.pdf_quality, back.uniform_size), (55, true));
    }

    #[test]
    fn config_file_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("lgr-cfg-{}", std::process::id()));
        let path = dir.join("nested").join("settings.lgr");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let s = UiSettings { mode: ModeChoice::Mixed, margin_mm: 7.5, ..Default::default() };
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
        let s = UiSettings { split: SplitChoice::Auto, deskew: true, ..Default::default() };
        let c = s.to_core(&SheetAdjust { rotate_quarters: 5, split: Some(SplitChoice::Off), skew_deg: Some(-1.5), ..Default::default() });
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
        assert_eq!(s.to_core(&SheetAdjust::default()).mode, OutputMode::BlackWhite);
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
