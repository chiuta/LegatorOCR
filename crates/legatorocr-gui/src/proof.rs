//! Proofreading window: the picture of an output page next to the editable text of that page.
//! The text is the plain-text output of a run with `--ocr --txt` (pages separated by a form
//! feed); the corrected file is saved the same way and goes back through `--apply-text`.
//! The window never touches the OCR words itself, it only edits and saves the text file.

use eframe::egui::{self, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Longest side of the picture kept in memory (the window never shows it larger).
const MAX_SIDE: u32 = 1800;

/// Split a text file into pages at form feeds; a form feed after the last page is not a page.
/// Same rule as `--apply-text`.
pub fn split_pages(text: &str) -> Vec<String> {
    let mut pages: Vec<String> = text.split('\x0c').map(str::to_string).collect();
    if pages.len() > 1 && pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    pages
}

/// The inverse: pages joined by form feeds (no trailing form feed).
pub fn join_pages(pages: &[String]) -> String {
    pages.join("\x0c")
}

/// A recognised word with its box (page pixels) and confidence (0-100), from an hOCR file.
#[derive(Debug, Clone, PartialEq)]
pub struct HWord {
    pub text: String,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub conf: f32,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';').filter(|&j| j <= 8) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..j];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Words of every page of an hOCR file (one `ocr_page` per page), in file order, with the page's
/// pixel size. Reads what `--hocr` of this program writes; other hOCR files may differ.
pub fn parse_hocr(text: &str) -> Vec<(u32, u32, Vec<HWord>)> {
    let mut pages = Vec::new();
    for chunk in text.split("class='ocr_page'").skip(1) {
        let (mut pw, mut ph) = (0, 0);
        if let Some(t) = chunk.split("title='").nth(1) {
            let n: Vec<u32> = t
                .split(['\'', ';'])
                .next()
                .unwrap_or("")
                .split_whitespace()
                .skip(1)
                .take(4)
                .filter_map(|v| v.parse().ok())
                .collect();
            if n.len() == 4 {
                (pw, ph) = (n[2], n[3]);
            }
        }
        let mut words = Vec::new();
        for w in chunk.split("class='ocrx_word'").skip(1) {
            let Some(t0) = w.find("title='") else {
                continue;
            };
            let rest = &w[t0 + 7..];
            let Some(t1) = rest.find('\'') else { continue };
            let title = &rest[..t1];
            let Some(gt) = rest[t1..].find('>') else {
                continue;
            };
            let body = &rest[t1 + gt + 1..];
            let Some(lt) = body.find('<') else { continue };
            let nums = |key: &str, n: usize| -> Vec<f32> {
                title
                    .split(';')
                    .find_map(|p| p.trim().strip_prefix(key))
                    .map(|v| {
                        v.split_whitespace()
                            .take(n)
                            .filter_map(|x| x.parse().ok())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let b = nums("bbox", 4);
            let c = nums("x_wconf", 1);
            if b.len() == 4 {
                words.push(HWord {
                    text: unescape(&body[..lt]),
                    x0: b[0] as u32,
                    y0: b[1] as u32,
                    x1: b[2] as u32,
                    y1: b[3] as u32,
                    conf: c.first().copied().unwrap_or(100.0),
                });
            }
        }
        pages.push((pw, ph, words));
    }
    pages
}

/// Replace the first whole token `old` of `page` (tokens are separated by white space) by `new`.
/// Returns false when no token equals `old`.
pub fn replace_token(page: &mut String, old: &str, new: &str) -> bool {
    if old.is_empty() {
        return false;
    }
    let mut start = None;
    let mut pos = 0;
    for tok in page.split_inclusive(char::is_whitespace) {
        let word = tok.trim_end_matches(char::is_whitespace);
        if word == old {
            start = Some(pos);
            break;
        }
        pos += tok.len();
    }
    match start {
        Some(i) => {
            page.replace_range(i..i + old.len(), new);
            true
        }
        None => false,
    }
}

/// Box of a word (corners in page pixels) and the page size.
type Mark = ((u32, u32), (u32, u32), f32, f32);

/// One line of the doubtful-word list.
struct DoubtRow {
    k: usize,
    text: String,
    conf: f32,
    tex: Option<(egui::TextureId, egui::Vec2)>,
}

#[derive(Default)]
pub struct ProofWindow {
    pub show: bool,
    text_path: Option<PathBuf>,
    pages: Vec<String>,
    images: Vec<PathBuf>,
    index: usize,
    dirty: bool,
    message: Option<(String, bool)>,
    tex: Option<(usize, TextureHandle)>,
    hocr: Vec<(u32, u32, Vec<HWord>)>,
    limit: f32,
    fixes: HashMap<(usize, usize), String>,
    done: HashSet<(usize, usize)>,
    selected: Option<(usize, usize)>,
    pending_scroll: bool,
    crops: Option<(usize, Vec<(usize, TextureHandle)>)>,
}

impl ProofWindow {
    pub fn open(&mut self) {
        self.show = true;
    }

    /// Load the text file; returns how many pages it holds.
    pub fn load_text(&mut self, path: &Path) -> Result<usize, String> {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        self.pages = split_pages(&text);
        self.text_path = Some(path.to_path_buf());
        self.index = 0;
        self.dirty = false;
        self.tex = None;
        Ok(self.pages.len())
    }

    pub fn set_images(&mut self, mut files: Vec<PathBuf>) {
        files.sort();
        self.images = files;
        self.tex = None;
    }

    /// Load an hOCR file written by the same run; returns how many pages it holds.
    pub fn load_hocr(&mut self, path: &Path) -> Result<usize, String> {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.hocr = parse_hocr(&String::from_utf8_lossy(&raw));
        self.crops = None;
        Ok(self.hocr.len())
    }

    /// Doubtful words of page `index` (confidence below the limit), least sure first, at most 40.
    pub fn doubtful(&self, index: usize) -> Vec<(usize, &HWord)> {
        let limit = if self.limit > 0.0 { self.limit } else { 60.0 };
        let Some((_, _, words)) = self.hocr.get(index) else {
            return Vec::new();
        };
        let mut v: Vec<(usize, &HWord)> = words
            .iter()
            .enumerate()
            .filter(|(k, w)| {
                w.conf < limit && !w.text.trim().is_empty() && !self.done.contains(&(index, *k))
            })
            .collect();
        v.sort_by(|a, b| a.1.conf.total_cmp(&b.1.conf));
        v.truncate(40);
        v
    }

    fn word_crops(&mut self, ctx: &egui::Context) {
        let index = self.index;
        if self.crops.as_ref().map(|(i, _)| *i) == Some(index) {
            return;
        }
        self.crops = None;
        let Some(path) = self.images.get(index) else {
            return;
        };
        let Ok(img) = image::open(path) else { return };
        let Some((pw, ph, _)) = self.hocr.get(index) else {
            return;
        };
        if *pw == 0 || *ph == 0 {
            return;
        }
        let (sx, sy) = (
            img.width() as f32 / *pw as f32,
            img.height() as f32 / *ph as f32,
        );
        let mut list = Vec::new();
        for (k, w) in self.doubtful(index) {
            let pad = 6.0;
            let x0 = ((w.x0 as f32 - pad) * sx).max(0.0) as u32;
            let y0 = ((w.y0 as f32 - pad) * sy).max(0.0) as u32;
            let x1 = (((w.x1 as f32 + pad) * sx) as u32).min(img.width());
            let y1 = (((w.y1 as f32 + pad) * sy) as u32).min(img.height());
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let c = img.crop_imm(x0, y0, x1 - x0, y1 - y0).to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [c.width() as usize, c.height() as usize],
                c.as_raw(),
            );
            list.push((
                k,
                ctx.load_texture(format!("crop{index}_{k}"), ci, TextureOptions::LINEAR),
            ));
        }
        self.crops = Some((index, list));
    }

    /// Write the pages back (to `path`, or over the file they came from).
    pub fn save(&mut self, path: Option<&Path>) -> Result<PathBuf, String> {
        let target = path
            .map(Path::to_path_buf)
            .or_else(|| self.text_path.clone())
            .ok_or_else(|| "no file".to_string())?;
        std::fs::write(&target, join_pages(&self.pages))
            .map_err(|e| format!("{}: {e}", target.display()))?;
        self.text_path = Some(target.clone());
        self.dirty = false;
        Ok(target)
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
        let want = self.index;
        if self.tex.as_ref().map(|(i, _)| *i) != Some(want) {
            self.tex = None;
            let path = self.images.get(want)?;
            let img = image::open(path).ok()?;
            let img = if img.width().max(img.height()) > MAX_SIDE {
                img.thumbnail(MAX_SIDE, MAX_SIDE)
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            self.tex = Some((
                want,
                ctx.load_texture(format!("proof{want}"), ci, TextureOptions::LINEAR),
            ));
        }
        self.tex.as_ref().map(|(_, t)| t)
    }

    /// Draw the window; `tr` gives the translated text of a key.
    pub fn ui(&mut self, ctx: &egui::Context, tr: &dyn Fn(&'static str) -> &'static str) {
        if !self.show {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("proof.title"))
            .open(&mut open)
            .default_size([1000.0, 700.0])
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("proof.open_text")).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("text", &["txt"])
                            .pick_file()
                        {
                            self.message = Some(match self.load_text(&f) {
                                Ok(n) => {
                                    // the hOCR of the same run sits beside the text: take it too
                                    let h = f.with_extension("hocr");
                                    let extra = if h.is_file() && self.load_hocr(&h).is_ok() {
                                        " + hOCR"
                                    } else {
                                        ""
                                    };
                                    (format!("{n}{extra} · {}", f.display()), false)
                                }
                                Err(e) => (e, true),
                            });
                        }
                    }
                    if ui.button(tr("proof.open_images")).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("images", &["png", "jpg", "jpeg", "tif", "tiff", "bmp"])
                            .pick_files()
                        {
                            self.set_images(f);
                        }
                    }
                    if ui.button(tr("proof.open_hocr")).clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("hOCR", &["hocr", "html", "xml"])
                            .pick_file()
                        {
                            self.message = Some(match self.load_hocr(&f) {
                                Ok(n) => (format!("hOCR: {n}"), false),
                                Err(e) => (e, true),
                            });
                        }
                    }
                    let can_save = self.text_path.is_some() && !self.pages.is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new(tr("proof.save")))
                        .clicked()
                    {
                        self.message = Some(match self.save(None) {
                            Ok(p) => (p.display().to_string(), false),
                            Err(e) => (e, true),
                        });
                    }
                    if ui
                        .add_enabled(can_save, egui::Button::new(tr("proof.save_as")))
                        .clicked()
                    {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("text", &["txt"])
                            .save_file()
                        {
                            self.message = Some(match self.save(Some(&f)) {
                                Ok(p) => (p.display().to_string(), false),
                                Err(e) => (e, true),
                            });
                        }
                    }
                });
                ui.label(tr("proof.hint"));
                if let Some((m, err)) = &self.message {
                    let c = if *err {
                        egui::Color32::from_rgb(200, 60, 60)
                    } else {
                        egui::Color32::from_rgb(70, 170, 90)
                    };
                    ui.colored_label(c, m);
                }
                if self.pages.is_empty() {
                    ui.separator();
                    ui.label(tr("proof.empty"));
                    return;
                }
                ui.separator();
                let n = self.pages.len();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.index > 0, egui::Button::new("<"))
                        .clicked()
                    {
                        self.index -= 1;
                        self.selected = None;
                    }
                    ui.label(format!("{} / {}", self.index + 1, n));
                    if ui
                        .add_enabled(self.index + 1 < n, egui::Button::new(">"))
                        .clicked()
                    {
                        self.index += 1;
                        self.selected = None;
                    }
                    if self.dirty {
                        ui.label("●");
                    }
                    if !self.images.is_empty() && self.images.len() != n {
                        ui.colored_label(
                            egui::Color32::from_rgb(235, 140, 30),
                            format!("{}: {} ≠ {}", tr("proof.count"), self.images.len(), n),
                        );
                    }
                });
                let index = self.index;
                let hocr_on = !self.hocr.is_empty();
                let tex = self.texture(ctx).map(|t| (t.id(), t.size_vec2()));
                // box of the selected doubtful word, in page pixels, with the page size
                let mark: Option<Mark> =
                    self.selected
                        .filter(|(p, _)| *p == index)
                        .and_then(|(_, k)| {
                            let (pw, ph, words) = self.hocr.get(index)?;
                            let w = words.get(k)?;
                            (*pw > 0 && *ph > 0).then_some((
                                (w.x0, w.y0),
                                (w.x1, w.y1),
                                *pw as f32,
                                *ph as f32,
                            ))
                        });
                ui.columns(2, |cols| {
                    egui::ScrollArea::both()
                        .id_salt("proof_img")
                        .max_height(if hocr_on { 330.0 } else { f32::INFINITY })
                        .show(&mut cols[0], |ui| match tex {
                            Some((id, size)) => {
                                let w = ui.available_width().max(200.0);
                                let k = (w / size.x).min(1.0);
                                let r = ui.image(egui::load::SizedTexture::new(id, size * k));
                                if let Some(b) = mark {
                                    let sx = r.rect.width() / b.2;
                                    let sy = r.rect.height() / b.3;
                                    let rect = egui::Rect::from_min_max(
                                        r.rect.min
                                            + egui::vec2(b.0 .0 as f32 * sx, b.0 .1 as f32 * sy),
                                        r.rect.min
                                            + egui::vec2(b.1 .0 as f32 * sx, b.1 .1 as f32 * sy),
                                    )
                                    .expand(3.0);
                                    ui.painter().rect_stroke(
                                        rect,
                                        2.0,
                                        egui::Stroke::new(
                                            2.0,
                                            egui::Color32::from_rgb(230, 40, 40),
                                        ),
                                        egui::StrokeKind::Outside,
                                    );
                                    if self.pending_scroll {
                                        ui.scroll_to_rect(
                                            rect.expand(80.0),
                                            Some(egui::Align::Center),
                                        );
                                    }
                                }
                            }
                            None => {
                                ui.label(tr("proof.no_image"));
                            }
                        });
                    egui::ScrollArea::vertical()
                        .id_salt("proof_text")
                        .max_height(if hocr_on { 330.0 } else { f32::INFINITY })
                        .show(&mut cols[1], |ui| {
                            let r = ui.add(
                                egui::TextEdit::multiline(&mut self.pages[index])
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(if hocr_on { 14 } else { 32 }),
                            );
                            if r.changed() {
                                self.dirty = true;
                            }
                        });
                });
                self.pending_scroll = false;
                if !self.hocr.is_empty() {
                    ui.separator();
                    if self.limit <= 0.0 {
                        self.limit = 60.0;
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("proof.doubtful"));
                        if ui
                            .add(egui::Slider::new(&mut self.limit, 10.0..=100.0).integer())
                            .changed()
                        {
                            self.crops = None;
                        }
                    });
                    if self.hocr.len() != n {
                        ui.colored_label(
                            egui::Color32::from_rgb(235, 140, 30),
                            format!("hOCR {} ≠ {}", self.hocr.len(), n),
                        );
                    }
                    self.word_crops(ctx);
                    let rows: Vec<DoubtRow> = {
                        let crops = self
                            .crops
                            .as_ref()
                            .map(|(_, c)| c.as_slice())
                            .unwrap_or(&[]);
                        self.doubtful(index)
                            .into_iter()
                            .map(|(k, w)| {
                                let t = crops
                                    .iter()
                                    .find(|(i, _)| *i == k)
                                    .map(|(_, t)| (t.id(), t.size_vec2()));
                                DoubtRow {
                                    k,
                                    text: w.text.clone(),
                                    conf: w.conf,
                                    tex: t,
                                }
                            })
                            .collect()
                    };
                    if rows.is_empty() {
                        ui.label(tr("proof.none_doubtful"));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("proof_doubt")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for DoubtRow { k, text, conf, tex } in rows {
                                ui.horizontal(|ui| {
                                    if let Some((id, size)) = tex {
                                        let f = (40.0 / size.y).min(1.0).min(300.0 / size.x);
                                        ui.image(egui::load::SizedTexture::new(id, size * f));
                                    }
                                    let picked = self.selected == Some((index, k));
                                    let lab =
                                        ui.selectable_label(picked, format!("{text}  ({conf:.0})"));
                                    if lab.clicked() {
                                        self.selected = Some((index, k));
                                        self.pending_scroll = true;
                                    }
                                    let buf = self
                                        .fixes
                                        .entry((index, k))
                                        .or_insert_with(|| text.clone());
                                    ui.add(egui::TextEdit::singleline(buf).desired_width(160.0));
                                    let new = buf.clone();
                                    if ui.button(tr("proof.replace")).clicked() {
                                        if replace_token(&mut self.pages[index], &text, &new) {
                                            self.dirty = true;
                                            self.done.insert((index, k));
                                            self.crops = None;
                                        } else {
                                            self.message = Some((
                                                format!("{text}: {}", tr("proof.not_found")),
                                                true,
                                            ));
                                        }
                                    }
                                    if ui.button(tr("proof.ok")).clicked() {
                                        self.done.insert((index, k));
                                        self.crops = None;
                                    }
                                });
                            }
                        });
                }
            });
        self.show = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_split_and_join_like_apply_text() {
        assert_eq!(split_pages("a b\x0cc d\x0c"), ["a b", "c d"]);
        assert_eq!(split_pages("one"), ["one"]);
        assert_eq!(
            split_pages("a\x0c\x0cb"),
            ["a", "", "b"],
            "an empty middle page stays"
        );
        let p = vec!["x\ny".to_string(), "z".to_string()];
        assert_eq!(split_pages(&join_pages(&p)), p);
    }

    #[test]
    fn load_edit_save_round_trip() {
        let dir = std::env::temp_dir().join(format!("legator-proof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.txt");
        std::fs::write(&f, "cartea esle buna\x0cpagina doi\x0c").unwrap();
        let mut w = ProofWindow::default();
        assert_eq!(w.load_text(&f), Ok(2));
        w.pages[0] = "cartea este buna".into();
        w.dirty = true;
        let out = dir.join("c.txt");
        assert_eq!(w.save(Some(&out)).unwrap(), out);
        assert!(!w.dirty);
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "cartea este buna\x0cpagina doi"
        );
        assert!(w.load_text(&dir.join("missing.txt")).is_err());
        let mut none = ProofWindow::default();
        assert!(none.save(None).is_err(), "nothing loaded, nothing to save");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_are_kept_in_name_order() {
        let mut w = ProofWindow::default();
        w.set_images(vec!["b.png".into(), "a.png".into()]);
        assert_eq!(w.images, [PathBuf::from("a.png"), PathBuf::from("b.png")]);
    }

    #[test]
    fn hocr_words_are_read_with_box_and_confidence() {
        let h = "<div class='ocr_page' id='page_1' title='bbox 0 0 200 100; ppageno 0'>\n\
<span class='ocr_line' title='bbox 1 2 3 4'><span class='ocrx_word' id='w1' title='bbox 10 20 50 40; x_wconf 45'>a&amp;b&quot;</span> \
<span class='ocrx_word' id='w2' title='bbox 60 20 90 40; x_wconf 97'>c&#259;</span></span></div>\n\
<div class='ocr_page' id='page_2' title='bbox 0 0 300 150; ppageno 1'></div>";
        let p = parse_hocr(h);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].0, p[0].1), (200, 100));
        assert_eq!(p[0].2.len(), 2);
        assert_eq!(p[0].2[0].text, "a&b\"");
        assert_eq!((p[0].2[0].x0, p[0].2[0].y1, p[0].2[0].conf), (10, 40, 45.0));
        assert_eq!(p[0].2[1].text, "c\u{103}");
        assert_eq!((p[1].0, p[1].2.len()), (300, 0));
    }

    #[test]
    fn doubtful_words_come_least_sure_first_and_replace_by_token() {
        let mut w = ProofWindow::default();
        let hw = |t: &str, c: f32| HWord {
            text: t.into(),
            x0: 0,
            y0: 0,
            x1: 5,
            y1: 5,
            conf: c,
        };
        w.hocr = vec![(
            10,
            10,
            vec![hw("ok", 99.0), hw("esle", 30.0), hw("b", 50.0)],
        )];
        let d: Vec<&str> = w.doubtful(0).iter().map(|(_, x)| x.text.as_str()).collect();
        assert_eq!(d, ["esle", "b"]);
        w.done.insert((0, 1));
        assert_eq!(
            w.doubtful(0).len(),
            1,
            "a word already dealt with leaves the list"
        );
        let mut page = "cartea esle bunaesle\nesle".to_string();
        assert!(replace_token(&mut page, "esle", "este"));
        assert_eq!(page, "cartea este bunaesle\nesle", "first whole token only");
        assert!(replace_token(&mut page, "esle", "este"));
        assert_eq!(page, "cartea este bunaesle\neste");
        assert!(!replace_token(&mut page, "esle", "x"));
        assert!(!replace_token(&mut page, "", "x"));
    }
}
