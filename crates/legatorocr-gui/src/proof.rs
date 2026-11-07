//! Proofreading window: the picture of an output page next to the editable text of that page.
//! The text is the plain-text output of a run with `--ocr --txt` (pages separated by a form
//! feed); the corrected file is saved the same way and goes back through `--apply-text`.
//! The window never touches the OCR words itself, it only edits and saves the text file.

use eframe::egui::{self, TextureHandle, TextureOptions};
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
                                Ok(n) => (format!("{n} · {}", f.display()), false),
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
                    }
                    ui.label(format!("{} / {}", self.index + 1, n));
                    if ui
                        .add_enabled(self.index + 1 < n, egui::Button::new(">"))
                        .clicked()
                    {
                        self.index += 1;
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
                let tex = self.texture(ctx).map(|t| (t.id(), t.size_vec2()));
                ui.columns(2, |cols| {
                    egui::ScrollArea::both().id_salt("proof_img").show(
                        &mut cols[0],
                        |ui| match tex {
                            Some((id, size)) => {
                                let w = ui.available_width().max(200.0);
                                let k = (w / size.x).min(1.0);
                                ui.image(egui::load::SizedTexture::new(id, size * k));
                            }
                            None => {
                                ui.label(tr("proof.no_image"));
                            }
                        },
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("proof_text")
                        .show(&mut cols[1], |ui| {
                            let r = ui.add(
                                egui::TextEdit::multiline(&mut self.pages[index])
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(32),
                            );
                            if r.changed() {
                                self.dirty = true;
                            }
                        });
                });
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
}
