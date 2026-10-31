//! GUI-side settings and their mapping onto the core pipeline settings.

use legatorocr_core::{OutputMode, Settings, SplitMode};
use legatorocr_engine::ocr::OcrOptions;

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
    pub pdf_quality: u8,
    pub skip_blank: bool,
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
            pdf_quality: 80,
            skip_blank: false,
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
            },
            sauvola_k: self.sauvola_k,
            despeckle: self.despeckle,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_sheet_overrides_win() {
        let s = UiSettings { split: SplitChoice::Auto, deskew: true, ..Default::default() };
        let c = s.to_core(&SheetAdjust { rotate_quarters: 5, split: Some(SplitChoice::Off), skew_deg: Some(-1.5) });
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
