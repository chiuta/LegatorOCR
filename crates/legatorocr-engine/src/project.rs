//! Project files (`.lgr`): a plain `key = value` text format, so no extra dependency
//! and easy to diff or edit by hand.
//!
//! ```text
//! legatorocr-project 1
//! dpi = 300
//! mode = hybrid
//! [sheet]
//! path = scans/001.png
//! rotate = 1
//! skew = 1.25
//! adjust = 0,0,2.5,0
//! ```
//! Relative sheet paths are relative to the project file. Unknown keys are ignored
//! (newer files open in older versions); bad values are reported with the line number.

use anyhow::{bail, Context, Result};
use legatorocr_core::{HAlign, OutputMode, Settings, SplitMode, Threshold, VAlign};
use std::path::{Path, PathBuf};

pub const HEADER: &str = "legatorocr-project 1";

/// Per-sheet overrides on top of the global settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SheetAdj {
    pub rotate_quarters: u8,
    pub split: Option<SplitMode>,
    pub skew_deg: Option<f32>,
    pub content_adjust_mm: [f32; 4],
    /// Hand-drawn erase rectangles (x, y, w, h in mm of the loaded scan).
    pub erase_mm: Vec<[f32; 4]>,
    /// Hand-drawn picture rectangles (x, y, w, h in mm of the loaded scan).
    pub picture_mm: Vec<[f32; 4]>,
    /// Hand-drawn dewarp guide lines: points as fractions (0–1) of the loaded scan.
    pub dewarp_guides: Vec<Vec<(f32, f32)>>,
}

impl SheetAdj {
    pub fn apply(&self, s: &Settings) -> Settings {
        let mut s = s.clone();
        s.rotate_quarters = self.rotate_quarters % 4;
        if let Some(sp) = self.split {
            s.split = sp;
        }
        s.manual_skew_deg = self.skew_deg;
        s.content_adjust_mm = self.content_adjust_mm;
        s.sheet_erase_zones = self.erase_mm.clone();
        s.sheet_picture_zones = self.picture_mm.clone();
        s.sheet_dewarp_guides = self.dewarp_guides.clone();
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct Project {
    pub settings: Settings,
    /// Everything that is not a pipeline setting: `pdf_quality`, `ocr`, `lang`, … in file order.
    pub extra: Vec<(String, String)>,
    pub sheets: Vec<(PathBuf, SheetAdj)>,
}

fn split_name(m: SplitMode) -> &'static str {
    match m {
        SplitMode::Off => "off",
        SplitMode::Auto => "auto",
        SplitMode::Middle => "middle",
    }
}
fn parse_split(v: &str) -> Option<SplitMode> {
    Some(match v {
        "off" => SplitMode::Off,
        "auto" => SplitMode::Auto,
        "middle" => SplitMode::Middle,
        _ => return None,
    })
}
fn mode_name(m: OutputMode) -> &'static str {
    match m {
        OutputMode::Color => "color",
        OutputMode::Gray => "gray",
        OutputMode::BlackWhite => "bw",
        OutputMode::Mixed => "mixed",
        OutputMode::Hybrid => "hybrid",
    }
}
fn parse_mode(v: &str) -> Option<OutputMode> {
    Some(match v {
        "color" => OutputMode::Color,
        "gray" => OutputMode::Gray,
        "bw" => OutputMode::BlackWhite,
        "mixed" => OutputMode::Mixed,
        "hybrid" => OutputMode::Hybrid,
        _ => return None,
    })
}
fn align_name(a: (HAlign, VAlign)) -> String {
    let h = match a.0 {
        HAlign::Left => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
    };
    let v = match a.1 {
        VAlign::Top => "top",
        VAlign::Center => "center",
        VAlign::Bottom => "bottom",
    };
    format!("{h},{v}")
}
fn parse_align(v: &str) -> Option<(HAlign, VAlign)> {
    let (h, w) = v.split_once(',')?;
    Some((
        match h.trim() {
            "left" => HAlign::Left,
            "center" => HAlign::Center,
            "right" => HAlign::Right,
            _ => return None,
        },
        match w.trim() {
            "top" => VAlign::Top,
            "center" => VAlign::Center,
            "bottom" => VAlign::Bottom,
            _ => return None,
        },
    ))
}
fn four(v: &str) -> Option<[f32; 4]> {
    let p: Vec<f32> = v
        .split(',')
        .map(|x| x.trim().parse::<f32>().ok().filter(|f| f.is_finite()))
        .collect::<Option<_>>()?;
    <[f32; 4]>::try_from(p).ok()
}
fn b(v: &str) -> Option<bool> {
    match v {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Settings as `(key, value)` pairs.
pub fn settings_to_pairs(s: &Settings) -> Vec<(&'static str, String)> {
    let [a, b, c, d] = s.margins_mm;
    let [e, f, g, h] = s.content_adjust_mm;
    vec![
        ("dpi", s.dpi.to_string()),
        ("auto_orient", s.auto_orient.to_string()),
        ("split", split_name(s.split).into()),
        ("deskew", s.deskew.to_string()),
        ("max_skew", s.max_skew_deg.to_string()),
        ("dewarp", s.dewarp.to_string()),
        ("margins", format!("{a},{b},{c},{d}")),
        ("content_adjust", format!("{e},{f},{g},{h}")),
        (
            "page_size",
            s.page_size_mm
                .map_or("free".into(), |(w, h)| format!("{w},{h}")),
        ),
        ("align", align_name(s.align)),
        ("mode", mode_name(s.mode).into()),
        ("threshold", s.threshold.name().to_ascii_lowercase()),
        ("sensitivity", s.sauvola_k.to_string()),
        ("despeckle", s.despeckle.to_string()),
        ("normalize", s.normalize_bg.to_string()),
        (
            "output_dpi",
            s.output_dpi.map_or("0".into(), |v| v.to_string()),
        ),
        ("picture_sensitivity", s.picture_sensitivity.to_string()),
        ("hybrid_dpi", s.hybrid_bg_dpi.to_string()),
        ("auto_invert", s.auto_invert.to_string()),
        ("auto_levels", s.auto_levels.to_string()),
        ("sharpen", s.sharpen.to_string()),
        ("punch_holes", s.punch_holes.to_string()),
        ("fingers", s.fingers.to_string()),
        ("remove_lines", s.remove_lines.to_string()),
        (
            "erase",
            legatorocr_core::cleanup::zones_to_string(&s.erase_zones),
        ),
        (
            "dewarp_guides",
            legatorocr_core::dewarp::guides_to_string(&s.dewarp_guides),
        ),
        (
            "picture_zones",
            legatorocr_core::cleanup::zones_to_string(&s.picture_zones),
        ),
        ("dropout", s.dropout.name().into()),
        ("despeckle_strength", s.despeckle_strength.to_string()),
        ("show_through", s.show_through.to_string()),
        ("descreen", s.descreen.to_string()),
        ("perspective", s.perspective.to_string()),
    ]
}

/// Apply one setting. `Ok(false)` = not a pipeline setting (caller may keep it as extra).
pub fn apply_pair(s: &mut Settings, k: &str, v: &str) -> Result<bool> {
    let bad = || anyhow::anyhow!("invalid value '{v}' for '{k}'");
    match k {
        "dpi" => s.dpi = v.parse().map_err(|_| bad())?,
        "auto_orient" => s.auto_orient = b(v).ok_or_else(bad)?,
        "split" => s.split = parse_split(v).ok_or_else(bad)?,
        "deskew" => s.deskew = b(v).ok_or_else(bad)?,
        "max_skew" => s.max_skew_deg = v.parse().map_err(|_| bad())?,
        "dewarp" => s.dewarp = b(v).ok_or_else(bad)?,
        "margins" => s.margins_mm = four(v).ok_or_else(bad)?,
        "content_adjust" => s.content_adjust_mm = four(v).ok_or_else(bad)?,
        "page_size" => {
            s.page_size_mm = if v == "free" {
                None
            } else {
                let (w, h) = v.split_once(',').ok_or_else(bad)?;
                Some((
                    w.trim().parse().map_err(|_| bad())?,
                    h.trim().parse().map_err(|_| bad())?,
                ))
            }
        }
        "align" => s.align = parse_align(v).ok_or_else(bad)?,
        "mode" => s.mode = parse_mode(v).ok_or_else(bad)?,
        "threshold" => s.threshold = Threshold::from_name(v).ok_or_else(bad)?,
        "sensitivity" => s.sauvola_k = v.parse().map_err(|_| bad())?,
        "despeckle" => s.despeckle = b(v).ok_or_else(bad)?,
        "normalize" => s.normalize_bg = b(v).ok_or_else(bad)?,
        "output_dpi" => {
            let o: u32 = v.parse().map_err(|_| bad())?;
            s.output_dpi = (o >= 36).then_some(o);
        }
        "picture_sensitivity" => s.picture_sensitivity = v.parse().map_err(|_| bad())?,
        "hybrid_dpi" => s.hybrid_bg_dpi = v.parse().map_err(|_| bad())?,
        "auto_invert" => s.auto_invert = b(v).ok_or_else(bad)?,
        "auto_levels" => s.auto_levels = b(v).ok_or_else(bad)?,
        "sharpen" => s.sharpen = v.parse().map_err(|_| bad())?,
        "punch_holes" => s.punch_holes = b(v).ok_or_else(bad)?,
        "fingers" => s.fingers = b(v).ok_or_else(bad)?,
        "remove_lines" => s.remove_lines = b(v).ok_or_else(bad)?,
        "dewarp_guides" => {
            s.dewarp_guides = legatorocr_core::dewarp::parse_guides(v).ok_or_else(bad)?
        }
        "erase" => s.erase_zones = legatorocr_core::cleanup::parse_zones(v).ok_or_else(bad)?,
        "picture_zones" => {
            s.picture_zones = legatorocr_core::cleanup::parse_zones(v).ok_or_else(bad)?
        }
        "dropout" => s.dropout = legatorocr_core::Dropout::from_name(v).ok_or_else(bad)?,
        "despeckle_strength" => s.despeckle_strength = v.parse().map_err(|_| bad())?,
        "show_through" => s.show_through = b(v).ok_or_else(bad)?,
        "descreen" => s.descreen = b(v).ok_or_else(bad)?,
        "perspective" => s.perspective = b(v).ok_or_else(bad)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// A path as one safe line of a project file: line breaks, tabs, `%` and edge spaces (which the
/// parser trims) become `%XX`, so a file name can never inject a `[sheet]` or a `key = value` line.
fn escape_path(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let n = s.chars().count();
    for (i, c) in s.chars().enumerate() {
        match c {
            '%' => o.push_str("%25"),
            '\n' => o.push_str("%0A"),
            '\r' => o.push_str("%0D"),
            '\t' => o.push_str("%09"),
            ' ' if i == 0 || i + 1 == n => o.push_str("%20"),
            c => o.push(c),
        }
    }
    o
}

fn unescape_path(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        o.push_str(&rest[..i]);
        let tail = &rest[i..];
        let code = tail.get(1..3);
        match code {
            Some("25") => o.push('%'),
            Some("0A") => o.push('\n'),
            Some("0D") => o.push('\r'),
            Some("09") => o.push('\t'),
            Some("20") => o.push(' '),
            _ => {
                o.push('%');
                rest = &tail[1..];
                continue;
            }
        }
        rest = &tail[3..];
    }
    o.push_str(rest);
    o
}

impl Project {
    pub fn to_text(&self, base: Option<&Path>) -> String {
        let mut out = format!("{HEADER}\n");
        for (k, v) in settings_to_pairs(&self.settings) {
            out.push_str(&format!("{k} = {v}\n"));
        }
        for (k, v) in &self.extra {
            out.push_str(&format!("{k} = {}\n", v.replace('\n', " ")));
        }
        for (p, a) in &self.sheets {
            let shown = base.and_then(|b| p.strip_prefix(b).ok()).unwrap_or(p);
            out.push_str(&format!(
                "\n[sheet]\npath = {}\n",
                escape_path(&shown.to_string_lossy())
            ));
            if a.rotate_quarters % 4 != 0 {
                out.push_str(&format!("rotate = {}\n", a.rotate_quarters % 4));
            }
            if let Some(s) = a.split {
                out.push_str(&format!("split = {}\n", split_name(s)));
            }
            if let Some(k) = a.skew_deg {
                out.push_str(&format!("skew = {k}\n"));
            }
            if a.content_adjust_mm != [0.0; 4] {
                let [w, x, y, z] = a.content_adjust_mm;
                out.push_str(&format!("adjust = {w},{x},{y},{z}\n"));
            }
            if !a.picture_mm.is_empty() {
                out.push_str(&format!(
                    "picture = {}\n",
                    legatorocr_core::cleanup::zones_to_string(&a.picture_mm)
                ));
            }
            if !a.dewarp_guides.is_empty() {
                out.push_str(&format!(
                    "guides = {}\n",
                    legatorocr_core::dewarp::guides_to_string(&a.dewarp_guides)
                ));
            }
            if !a.erase_mm.is_empty() {
                out.push_str(&format!(
                    "erase = {}\n",
                    legatorocr_core::cleanup::zones_to_string(&a.erase_mm)
                ));
            }
        }
        out
    }

    pub fn parse(text: &str, base: Option<&Path>) -> Result<Project> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, l)) if l.trim_start_matches('\u{feff}').trim() == HEADER => {}
            Some((_, l)) if l.trim().starts_with("legatorocr-project") => {
                bail!("unsupported project version: {}", l.trim())
            }
            _ => bail!("not a LegatorOCR project (missing '{HEADER}' line)"),
        }
        let mut p = Project::default();
        let mut cur: Option<(PathBuf, SheetAdj)> = None;
        for (n, raw) in lines {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: anyhow::Error| e.context(format!("line {}", n + 1));
            if line == "[sheet]" {
                if let Some(s) = cur.take() {
                    p.sheets.push(s);
                }
                cur = Some((PathBuf::new(), SheetAdj::default()));
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(at(anyhow::anyhow!("expected 'key = value', got '{line}'")));
            };
            let (k, v) = (k.trim(), v.trim());
            match cur.as_mut() {
                Some((path, adj)) => match k {
                    "path" => {
                        let pb = PathBuf::from(unescape_path(v));
                        *path = if pb.is_absolute() {
                            pb
                        } else {
                            base.map_or(pb.clone(), |b| b.join(&pb))
                        };
                    }
                    "rotate" => {
                        adj.rotate_quarters = v.parse::<u8>().map_err(|e| at(e.into()))? % 4
                    }
                    "split" => {
                        adj.split = Some(
                            parse_split(v)
                                .ok_or_else(|| at(anyhow::anyhow!("invalid split '{v}'")))?,
                        )
                    }
                    "skew" => {
                        adj.skew_deg = Some(
                            v.parse()
                                .map_err(|e: std::num::ParseFloatError| at(e.into()))?,
                        )
                    }
                    "picture" => {
                        adj.picture_mm = legatorocr_core::cleanup::parse_zones(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid picture zones '{v}'")))?
                    }
                    "guides" => {
                        adj.dewarp_guides = legatorocr_core::dewarp::parse_guides(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid dewarp guides '{v}'")))?
                    }
                    "erase" => {
                        adj.erase_mm = legatorocr_core::cleanup::parse_zones(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid erase zones '{v}'")))?
                    }
                    "adjust" => {
                        adj.content_adjust_mm =
                            four(v).ok_or_else(|| at(anyhow::anyhow!("invalid adjust '{v}'")))?
                    }
                    _ => {}
                },
                None => {
                    if !apply_pair(&mut p.settings, k, v).map_err(at)? {
                        p.extra.push((k.to_string(), v.to_string()));
                    }
                }
            }
        }
        if let Some(s) = cur.take() {
            p.sheets.push(s);
        }
        if let Some((i, _)) = p
            .sheets
            .iter()
            .enumerate()
            .find(|(_, s)| s.0.as_os_str().is_empty())
        {
            bail!("sheet #{} has no path", i + 1);
        }
        p.settings = p.settings.sanitized();
        Ok(p)
    }

    pub fn load(path: &Path) -> Result<Project> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Project::parse(&text, path.parent()).with_context(|| path.display().to_string())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::write_atomic(path, self.to_text(path.parent()).as_bytes())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.extra
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_everything() {
        let mut p = Project::default();
        p.settings.mode = OutputMode::Hybrid;
        p.settings.threshold = Threshold::Wolf;
        p.settings.page_size_mm = Some((148.0, 210.5));
        p.settings.align = (HAlign::Right, VAlign::Top);
        p.settings.output_dpi = Some(200);
        p.settings.dewarp = true;
        p.settings.margins_mm = [1.0, 2.0, 3.0, 4.5];
        p.extra.push(("pdf_quality".into(), "70".into()));
        p.sheets.push((
            PathBuf::from("/scans/a b.png"),
            SheetAdj {
                rotate_quarters: 3,
                split: Some(SplitMode::Off),
                skew_deg: Some(-1.25),
                content_adjust_mm: [0.0, 1.0, 2.0, -3.0],
                erase_mm: vec![[10.0, 20.5, 30.0, 40.0], [0.0, 0.0, 5.0, 5.0]],
                picture_mm: vec![[50.0, 60.0, 70.5, 80.0]],
                dewarp_guides: vec![vec![(0.0, 0.25), (0.5, 0.3), (1.0, 0.25)]],
            },
        ));
        p.sheets
            .push((PathBuf::from("/scans/c.png"), SheetAdj::default()));
        let q = Project::parse(&p.to_text(None), None).unwrap();
        assert_eq!(q.sheets, p.sheets);
        assert_eq!(q.settings.mode, OutputMode::Hybrid);
        assert_eq!(q.settings.threshold, Threshold::Wolf);
        assert_eq!(q.settings.page_size_mm, Some((148.0, 210.5)));
        assert_eq!(q.settings.align, (HAlign::Right, VAlign::Top));
        assert_eq!(q.settings.output_dpi, Some(200));
        assert!(q.settings.dewarp);
        assert_eq!(q.settings.margins_mm, [1.0, 2.0, 3.0, 4.5]);
        assert_eq!(q.get("pdf_quality"), Some("70"));
    }

    #[test]
    fn hostile_file_names_cannot_inject_lines_and_round_trip() {
        for name in [
            "x\n[sheet]\npath = /etc/passwd",
            " lead.png",
            "trail.png ",
            "50%25.png",
            "tab\there.png",
            "a%b.png",
            "100%",
        ] {
            let mut p = Project::default();
            p.sheets
                .push((PathBuf::from(format!("/scans/{name}")), SheetAdj::default()));
            let text = p.to_text(None);
            assert_eq!(
                text.lines().filter(|l| l.trim() == "[sheet]").count(),
                1,
                "injected a sheet: {name:?}"
            );
            let q = Project::parse(&text, None).unwrap();
            assert_eq!(q.sheets.len(), 1);
            assert_eq!(
                q.sheets[0].0,
                PathBuf::from(format!("/scans/{name}")),
                "{name:?}"
            );
        }
    }

    #[test]
    fn relative_paths_and_errors() {
        let t = format!("{HEADER}\n[sheet]\npath = x/1.png\n[sheet]\npath = /abs/2.png\n");
        let q = Project::parse(&t, Some(Path::new("/proj"))).unwrap();
        assert_eq!(q.sheets[0].0, Path::new("/proj/x/1.png"));
        assert_eq!(q.sheets[1].0, Path::new("/abs/2.png"));
        assert!(Project::parse("hello", None).is_err());
        assert!(Project::parse("legatorocr-project 9\n", None)
            .unwrap_err()
            .to_string()
            .contains("version"));
        let e = Project::parse(&format!("{HEADER}\nmode = purple\n"), None).unwrap_err();
        assert!(format!("{e:#}").contains("line 2"), "{e:#}");
        assert!(Project::parse(&format!("{HEADER}\n[sheet]\nrotate = 1\n"), None).is_err());
        // unknown keys and comments are tolerated
        assert!(Project::parse(&format!("{HEADER}\n# note\nfuture_option = 1\n"), None).is_ok());
    }

    #[test]
    fn hostile_values_are_sanitised() {
        let q = Project::parse(
            &format!("{HEADER}\ndpi = 999999\nmargins = nan,1,1,1\n"),
            None,
        );
        assert!(q.is_err());
        let q =
            Project::parse(&format!("{HEADER}\ndpi = 999999\nsensitivity = 9\n"), None).unwrap();
        assert_eq!(q.settings.dpi, 2400);
        assert!(q.settings.sauvola_k <= 1.0);
    }
}
