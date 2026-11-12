//! Search window (Ctrl+F): looks for a piece of text (a word, a name such as "Voyager") in the
//! text layer of the PDFs of the project, or of a folder, and lists file, page and line. A click
//! on a result goes to that page. It reuses `legatorocr_engine::search` (the same engine as the
//! command-line `--search`): words broken with a hyphen at the end of a line are found, case and
//! diacritics are ignored, and the old Romanian spelling (1953-1993) can be matched as well.
//! Scans without a text layer cannot be searched; the window says how many and that they need OCR.

use eframe::egui;
use legatorocr_engine::search::{self, Hit, SearchOptions, Summary};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Results kept; the search stops there (the window says so).
const MAX_HITS: usize = 500;

enum Msg {
    Hit(Hit),
    Done(Result<Summary, String>),
}

#[derive(Default)]
pub struct FindWindow {
    pub show: bool,
    query: String,
    old: bool,
    exact: bool,
    /// Search this folder instead of the files of the project.
    folder: Option<PathBuf>,
    hits: Vec<Hit>,
    summary: Option<Summary>,
    error: Option<String>,
    rx: Option<Receiver<Msg>>,
    focus: bool,
    /// A result the person clicked: the app goes to its page.
    pub goto: Option<Hit>,
}

impl FindWindow {
    pub fn open(&mut self) {
        self.show = true;
        self.focus = true;
    }

    fn start(&mut self, ctx: &egui::Context, project: Vec<PathBuf>) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        let roots: Vec<PathBuf> = match &self.folder {
            Some(d) => vec![d.clone()],
            None => project,
        };
        self.hits.clear();
        self.summary = None;
        self.error = None;
        let o = SearchOptions {
            exact: self.exact,
            old_orthography: self.old,
            max_hits: MAX_HITS,
            ..SearchOptions::default()
        };
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = search::search(&roots, &q, &o, &mut |h| {
                let _ = tx.send(Msg::Hit(h));
                ctx.request_repaint();
            });
            let _ = tx.send(Msg::Done(r.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        loop {
            match rx.try_recv() {
                Ok(Msg::Hit(h)) => self.hits.push(h),
                Ok(Msg::Done(r)) => {
                    match r {
                        Ok(s) => self.summary = Some(s),
                        Err(e) => self.error = Some(e),
                    }
                    self.rx = None;
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return;
                }
            }
        }
    }

    /// `project`: the files the project was made from (PDFs have their text layer searched).
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        tr: &dyn Fn(&'static str) -> &'static str,
        project: &dyn Fn() -> Vec<PathBuf>,
    ) {
        if !self.show {
            return;
        }
        self.poll();
        let mut open = true;
        let mut go = false;
        egui::Window::new(tr("find.title"))
            .open(&mut open)
            .default_size([620.0, 460.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(tr("find.hint"))
                            .desired_width(340.0),
                    );
                    if std::mem::take(&mut self.focus) {
                        r.request_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let busy = self.rx.is_some();
                    if (ui
                        .add_enabled(!busy, egui::Button::new(tr("find.go")))
                        .clicked()
                        || enter)
                        && !busy
                    {
                        go = true;
                    }
                    if busy {
                        ui.spinner();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.old, tr("find.old"))
                        .on_hover_text(tr("find.old_tip"));
                    ui.checkbox(&mut self.exact, tr("find.exact"))
                        .on_hover_text(tr("find.exact_tip"));
                });
                ui.horizontal_wrapped(|ui| {
                    let in_project = self.folder.is_none();
                    if ui
                        .selectable_label(in_project, tr("find.in_project"))
                        .clicked()
                    {
                        self.folder = None;
                    }
                    let label = match &self.folder {
                        Some(d) => format!("{} {}", tr("find.in_folder"), d.display()),
                        None => format!("{}…", tr("find.in_folder")),
                    };
                    if ui.selectable_label(!in_project, label).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.folder = Some(d);
                        }
                    }
                });
                ui.separator();
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
                if let Some(s) = &self.summary {
                    let mut line =
                        format!("{} · {} {}", self.hits.len(), s.files, tr("find.files"));
                    if s.truncated {
                        line.push_str(&format!(" · {} {MAX_HITS}", tr("find.truncated")));
                    }
                    ui.label(line);
                    if s.image_only_pdfs > 0 {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("{} {}", s.image_only_pdfs, tr("find.need_ocr")),
                        );
                    }
                    if self.hits.is_empty() && self.error.is_none() {
                        ui.label(tr("find.none"));
                    }
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for h in &self.hits {
                        let file = std::path::Path::new(&h.source)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| h.source.clone());
                        let place = match h.page {
                            Some(p) => format!("{file} · p.{p}:{}", h.line),
                            None => format!("{file}:{}", h.line),
                        };
                        let r = ui
                            .add(
                                egui::Button::new(egui::RichText::new(&h.text).monospace())
                                    .wrap()
                                    .frame(false),
                            )
                            .on_hover_text(&h.source);
                        ui.small(place);
                        if r.clicked() {
                            self.goto = Some(h.clone());
                        }
                        ui.add_space(4.0);
                    }
                });
            });
        if go {
            let files = project();
            self.start(ctx, files);
        }
        if !open {
            self.show = false;
        }
    }
}
