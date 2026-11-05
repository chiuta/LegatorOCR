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
    /// Scan resolution of this sheet (`dpi = N` in the file). `None`: the project's global `dpi`.
    /// `--embed-project` fills it with the resolution the run actually used (read from the image
    /// header), because a reloaded project does not detect it again.
    pub dpi: Option<u32>,
    pub rotate_quarters: u8,
    pub split: Option<SplitMode>,
    pub skew_deg: Option<f32>,
    pub content_adjust_mm: [f32; 4],
    /// Hand-drawn erase rectangles (x, y, w, h in mm of the loaded scan).
    pub erase_mm: Vec<[f32; 4]>,
    /// Hand-drawn picture rectangles (x, y, w, h in mm of the loaded scan).
    pub picture_mm: Vec<[f32; 4]>,
    /// Hand-drawn rectangles where the automatic picture detector is overruled (mm of the loaded scan).
    pub no_picture_mm: Vec<[f32; 4]>,
    /// Output mode of this sheet only (colour, grey, black and white, mixed, hybrid); `None` follows the global one.
    pub mode: Option<OutputMode>,
    /// Hand-drawn dewarp guide lines: points as fractions (0–1) of the loaded scan.
    pub dewarp_guides: Vec<Vec<(f32, f32)>>,
    /// Working rectangle (x, y, w, h in mm of the loaded scan); outside it the sheet is blanked.
    pub work_mm: Option<[f32; 4]>,
    /// Cut line of a two-page sheet as a fraction (0-1) of the loaded width.
    pub split_frac: Option<f32>,
    /// Turned by hand: no automatic orientation for this sheet (`orient-locked = 1`).
    pub orient_locked: bool,
}

impl SheetAdj {
    pub fn apply(&self, s: &Settings) -> Settings {
        let mut s = s.clone();
        // the sheet's turn comes on top of a global one (`--rotate`); its manual skew replaces a global
        // `--skew` only when the sheet has one (a default sheet used to wipe both options out)
        if let Some(d) = self.dpi {
            s.dpi = d;
        }
        s.rotate_quarters = (s.rotate_quarters % 4 + self.rotate_quarters % 4) % 4;
        if let Some(sp) = self.split {
            s.split = sp;
        }
        s.manual_skew_deg = self.skew_deg.or(s.manual_skew_deg);
        s.content_adjust_mm = self.content_adjust_mm;
        s.sheet_erase_zones = self.erase_mm.clone();
        s.sheet_picture_zones = self.picture_mm.clone();
        s.sheet_no_picture_zones = self.no_picture_mm.clone();
        if let Some(m) = self.mode {
            s.mode = m;
        }
        s.sheet_dewarp_guides = self.dewarp_guides.clone();
        s.sheet_orient_locked = self.orient_locked;
        s.sheet_work_mm = self.work_mm.or(s.sheet_work_mm);
        s.sheet_split_frac = self.split_frac.or(s.sheet_split_frac);
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
        ("fit_width", s.fit_width.to_string()),
        ("fit_height", s.fit_height.to_string()),
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
        (
            "no_picture_zones",
            legatorocr_core::cleanup::zones_to_string(&s.no_picture_zones),
        ),
        ("auto_pictures", s.auto_pictures.to_string()),
        (
            "work_area",
            s.sheet_work_mm
                .map_or(String::new(), |[x, y, w, h]| format!("{x},{y},{w},{h}")),
        ),
        (
            "cut_at",
            s.sheet_split_frac.map_or(String::new(), |f| f.to_string()),
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
        "fit_width" => s.fit_width = b(v).ok_or_else(bad)?,
        "fit_height" => s.fit_height = b(v).ok_or_else(bad)?,
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
        "no_picture_zones" => {
            s.no_picture_zones = legatorocr_core::cleanup::parse_zones(v).ok_or_else(bad)?
        }
        "auto_pictures" => s.auto_pictures = b(v).ok_or_else(bad)?,
        "work_area" => {
            s.sheet_work_mm = if v.is_empty() {
                None
            } else {
                let n: Vec<f32> = v
                    .split(',')
                    .map(|t| t.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| bad())?;
                match n[..] {
                    [x, y, w, h] if w > 0.0 && h > 0.0 => Some([x, y, w, h]),
                    _ => return Err(bad()),
                }
            }
        }
        "cut_at" => {
            s.sheet_split_frac = if v.is_empty() {
                None
            } else {
                let f: f32 = v.parse().map_err(|_| bad())?;
                if f > 0.0 && f < 1.0 {
                    Some(f)
                } else {
                    return Err(bad());
                }
            }
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
            if let Some(d) = a.dpi {
                out.push_str(&format!("dpi = {d}\n"));
            }
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
            if let Some(m) = a.mode {
                out.push_str(&format!("mode = {}\n", mode_name(m)));
            }
            if !a.no_picture_mm.is_empty() {
                out.push_str(&format!(
                    "no-picture = {}\n",
                    legatorocr_core::cleanup::zones_to_string(&a.no_picture_mm)
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
            if let Some(w) = a.work_mm {
                out.push_str(&format!(
                    "work = {}\n",
                    legatorocr_core::cleanup::zones_to_string(&[w])
                ));
            }
            if a.orient_locked {
                out.push_str("orient-locked = 1\n");
            }
            if let Some(f) = a.split_frac {
                out.push_str(&format!("split-at = {f}\n"));
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
                    "dpi" => {
                        adj.dpi = Some(v.parse::<u32>().map_err(|e| at(e.into()))?.clamp(36, 2400))
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
                    "mode" => {
                        adj.mode = Some(
                            parse_mode(v)
                                .ok_or_else(|| at(anyhow::anyhow!("invalid mode '{v}'")))?,
                        )
                    }
                    "no-picture" => {
                        adj.no_picture_mm = legatorocr_core::cleanup::parse_zones(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid no-picture zones '{v}'")))?
                    }
                    "guides" => {
                        adj.dewarp_guides = legatorocr_core::dewarp::parse_guides(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid dewarp guides '{v}'")))?
                    }
                    "erase" => {
                        adj.erase_mm = legatorocr_core::cleanup::parse_zones(v)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid erase zones '{v}'")))?
                    }
                    "work" => {
                        let z = legatorocr_core::cleanup::parse_zones(v)
                            .filter(|z| z.len() == 1)
                            .ok_or_else(|| at(anyhow::anyhow!("invalid work area '{v}'")))?;
                        adj.work_mm = Some(z[0]);
                    }
                    "orient-locked" => adj.orient_locked = matches!(v, "1" | "true" | "yes"),
                    "split-at" => {
                        let f: f32 = v
                            .parse()
                            .ok()
                            .filter(|f: &f32| f.is_finite() && *f > 0.0 && *f < 1.0)
                            .ok_or_else(|| {
                                at(anyhow::anyhow!(
                                    "invalid split-at '{v}' (a fraction between 0 and 1)"
                                ))
                            })?;
                        adj.split_frac = Some(f);
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
        if crate::scantailor::looks_like_scantailor(&text) {
            return crate::scantailor::parse(&text, path.parent())
                .with_context(|| path.display().to_string());
        }
        Project::parse(&text, path.parent()).with_context(|| path.display().to_string())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::write_atomic(path, self.to_text(path.parent()).as_bytes())
    }

    /// Save into a folder of the user's choice as a self-contained project: the source files are
    /// copied into `FOLDER/images/` (a PDF once, however many of its pages are used) and the
    /// project `FOLDER/project.lgr` points at the copies with relative paths, so the whole
    /// folder can be moved, zipped or opened on another computer. Nothing outside the folder is
    /// changed. Returns the project file and how many files were copied.
    pub fn save_to_folder(&self, folder: &Path) -> Result<(PathBuf, usize)> {
        let img_dir = folder.join("images");
        std::fs::create_dir_all(&img_dir)
            .with_context(|| format!("creating {}", img_dir.display()))?;
        let mut copied: std::collections::HashMap<PathBuf, PathBuf> =
            std::collections::HashMap::new();
        let mut moved = self.clone();
        let mut n = 0usize;
        for (path, _) in moved.sheets.iter_mut() {
            // a page of a PDF is `book.pdf#9`: the file is the part before `#`
            let text = path.to_string_lossy().into_owned();
            let (file, suffix) = match text.rsplit_once('#') {
                Some((f, n)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
                    (PathBuf::from(f), format!("#{n}"))
                }
                _ => (path.clone(), String::new()),
            };
            let dest = match copied.get(&file) {
                Some(d) => d.clone(),
                None => {
                    let name = file
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "scan".into());
                    let dest = img_dir.join(format!("{:04}-{name}", copied.len() + 1));
                    let same = std::fs::metadata(&dest).ok().map(|m| m.len())
                        == std::fs::metadata(&file).ok().map(|m| m.len());
                    if !(dest.exists() && same) {
                        std::fs::copy(&file, &dest).with_context(|| {
                            format!("copying {} to {}", file.display(), dest.display())
                        })?;
                        n += 1;
                    }
                    copied.insert(file.clone(), dest.clone());
                    dest
                }
            };
            *path = PathBuf::from(format!("{}{suffix}", dest.display()));
        }
        let project = folder.join("project.lgr");
        moved.save(&project)?;
        Ok((project, n))
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
    fn per_sheet_dpi_round_trips_and_overrides_the_global_one() {
        let mut p = Project::default();
        p.settings.dpi = 300;
        p.sheets.push((
            PathBuf::from("a.png"),
            SheetAdj {
                dpi: Some(150),
                ..Default::default()
            },
        ));
        p.sheets.push((PathBuf::from("b.png"), SheetAdj::default()));
        let q = Project::parse(&p.to_text(None), None).unwrap();
        assert_eq!(q.sheets[0].1.dpi, Some(150));
        assert_eq!(q.sheets[1].1.dpi, None);
        assert_eq!(q.sheets[0].1.apply(&q.settings).dpi, 150);
        assert_eq!(q.sheets[1].1.apply(&q.settings).dpi, 300);
    }

    #[test]
    fn a_default_sheet_keeps_global_rotate_and_skew() {
        let g = Settings {
            rotate_quarters: 1,
            manual_skew_deg: Some(2.5),
            ..Settings::default()
        };
        let a = SheetAdj::default().apply(&g);
        assert_eq!((a.rotate_quarters, a.manual_skew_deg), (1, Some(2.5)));
        let b = SheetAdj {
            rotate_quarters: 2,
            skew_deg: Some(-1.0),
            ..SheetAdj::default()
        }
        .apply(&g);
        assert_eq!((b.rotate_quarters, b.manual_skew_deg), (3, Some(-1.0)));
    }

    #[test]
    fn round_trip_keeps_everything() {
        let mut p = Project::default();
        p.settings.mode = OutputMode::Hybrid;
        p.settings.threshold = Threshold::Wolf;
        p.settings.page_size_mm = Some((148.0, 210.5));
        p.settings.fit_width = true;
        p.settings.align = (HAlign::Right, VAlign::Top);
        p.settings.output_dpi = Some(200);
        p.settings.dewarp = true;
        p.settings.margins_mm = [1.0, 2.0, 3.0, 4.5];
        p.extra.push(("pdf_quality".into(), "70".into()));
        p.sheets.push((
            PathBuf::from("/scans/a b.png"),
            SheetAdj {
                dpi: Some(200),
                rotate_quarters: 3,
                split: Some(SplitMode::Off),
                skew_deg: Some(-1.25),
                content_adjust_mm: [0.0, 1.0, 2.0, -3.0],
                erase_mm: vec![[10.0, 20.5, 30.0, 40.0], [0.0, 0.0, 5.0, 5.0]],
                work_mm: Some([5.0, 6.0, 100.0, 120.5]),
                split_frac: Some(0.4375),
                orient_locked: true,
                picture_mm: vec![[50.0, 60.0, 70.5, 80.0]],
                no_picture_mm: vec![[1.0, 2.0, 30.0, 40.5]],
                mode: Some(OutputMode::Gray),
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
        assert!(q.settings.fit_width && !q.settings.fit_height);
        assert_eq!(q.settings.align, (HAlign::Right, VAlign::Top));
        assert_eq!(q.settings.output_dpi, Some(200));
        assert!(q.settings.dewarp);
        assert_eq!(q.settings.margins_mm, [1.0, 2.0, 3.0, 4.5]);
        assert_eq!(q.get("pdf_quality"), Some("70"));
    }

    #[test]
    fn global_work_area_and_cut_line_survive_save_and_load() {
        let mut p = Project::default();
        p.settings.sheet_work_mm = Some([5.0, 6.0, 80.5, 60.0]);
        p.settings.sheet_split_frac = Some(0.4);
        let q = Project::parse(&p.to_text(None), None).unwrap();
        assert_eq!(q.settings.sheet_work_mm, Some([5.0, 6.0, 80.5, 60.0]));
        assert_eq!(q.settings.sheet_split_frac, Some(0.4));
        // Empty values mean "not set" (what the GUI writes, since it keeps these per sheet).
        let r = Project::parse(&Project::default().to_text(None), None).unwrap();
        assert_eq!(r.settings.sheet_work_mm, None);
        assert_eq!(r.settings.sheet_split_frac, None);
        // Nonsense is refused rather than silently ignored.
        let bad = p.to_text(None).replace("cut_at = 0.4", "cut_at = 7");
        assert!(Project::parse(&bad, None).is_err());
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

    #[test]
    fn a_project_saved_to_a_folder_is_self_contained() {
        let d = std::env::temp_dir().join(format!("lgr-proj-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.png"), b"AAAA").unwrap();
        std::fs::write(src.join("book.pdf"), b"%PDF-fake").unwrap();
        let mut p = Project::default();
        p.settings.mode = OutputMode::Mixed;
        p.sheets.push((
            src.join("a.png"),
            SheetAdj {
                mode: Some(OutputMode::Gray),
                ..SheetAdj::default()
            },
        ));
        p.sheets.push((
            PathBuf::from(format!("{}#1", src.join("book.pdf").display())),
            SheetAdj::default(),
        ));
        p.sheets.push((
            PathBuf::from(format!("{}#2", src.join("book.pdf").display())),
            SheetAdj::default(),
        ));
        let out = d.join("my project");
        let (file, n) = p.save_to_folder(&out).unwrap();
        assert_eq!(n, 2, "the PDF is copied once");
        assert_eq!(file, out.join("project.lgr"));
        let q = Project::load(&file).unwrap();
        assert_eq!(q.settings.mode, OutputMode::Mixed);
        assert_eq!(q.sheets.len(), 3);
        assert_eq!(q.sheets[0].1.mode, Some(OutputMode::Gray));
        assert_eq!(q.sheets[1].1.mode, None);
        assert!(q.sheets[0].0.starts_with(out.join("images")));
        assert_eq!(std::fs::read(&q.sheets[0].0).unwrap(), b"AAAA");
        let pdf1 = q.sheets[1].0.to_string_lossy().into_owned();
        assert!(pdf1.ends_with("book.pdf#1"), "{pdf1}");
        assert!(q.sheets[2].0.to_string_lossy().ends_with("book.pdf#2"));
        // the originals stay where they were; saving again copies nothing new
        assert!(src.join("a.png").exists());
        assert_eq!(p.save_to_folder(&out).unwrap().1, 0);
        // a missing source is an error, not a silent hole in the project
        p.sheets.push((d.join("gone.png"), SheetAdj::default()));
        assert!(p.save_to_folder(&d.join("other")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
