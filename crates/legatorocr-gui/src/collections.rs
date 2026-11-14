//! The Collections window: newspapers, magazines, books … in one `.lgrcol` file. A filterable,
//! sortable table on the left, the details of the chosen item on the right, a search over the
//! recognised text of the whole collection, and batch processing with progress and cancel.
//! All the work is done by `legatorocr_engine::collection`; this file is the window.

use eframe::egui::{self, Color32, RichText};
use legatorocr_engine::collection::{
    self as col, Collection, Hit, Index, Item, Kind, RunConfig, Status,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

type Tr<'a> = &'a dyn Fn(&'static str) -> &'static str;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Title,
    Issue,
    Pages,
    Status,
}

/// What the status column shows: the four states plus "failed" (to do, last run went wrong).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    Todo,
    Processed,
    OcrDone,
    Checked,
    Failed,
}

pub fn chip_of(it: &Item) -> Chip {
    match it.status {
        Status::Todo if !it.error.is_empty() => Chip::Failed,
        Status::Todo => Chip::Todo,
        Status::Processed => Chip::Processed,
        Status::OcrDone => Chip::OcrDone,
        Status::Checked => Chip::Checked,
    }
}

impl Chip {
    fn key(self) -> &'static str {
        match self {
            Chip::Todo => "col.status_todo",
            Chip::Processed => "col.status_processed",
            Chip::OcrDone => "col.status_ocr_done",
            Chip::Checked => "col.status_checked",
            Chip::Failed => "col.status_failed",
        }
    }
    /// Background colour; the text is always written out too, colour is never alone.
    fn bg(self) -> Color32 {
        match self {
            Chip::Todo => Color32::from_rgb(95, 99, 104),
            Chip::Processed => Color32::from_rgb(25, 95, 170),
            Chip::OcrDone => Color32::from_rgb(20, 120, 70),
            Chip::Checked => Color32::from_rgb(90, 55, 150),
            Chip::Failed => Color32::from_rgb(180, 40, 40),
        }
    }
}

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Newspaper => "col.kind_newspaper",
        Kind::Magazine => "col.kind_magazine",
        Kind::Book => "col.kind_book",
        Kind::Other => "col.kind_other",
    }
}

fn status_key(s: Status) -> &'static str {
    match s {
        Status::Todo => "col.status_todo",
        Status::Processed => "col.status_processed",
        Status::OcrDone => "col.status_ocr_done",
        Status::Checked => "col.status_checked",
    }
}

/// Indices of the items to show: those matching the text filter (title, issue, date, tags,
/// notes; case and diacritics ignored), the kind and the status, in the chosen order.
pub fn visible_rows(
    c: &Collection,
    filter: &str,
    kind: Option<Kind>,
    status: Option<Status>,
    sort: SortKey,
    ascending: bool,
) -> Vec<usize> {
    let norm = |s: &str| legatorocr_core::text::fold(&legatorocr_core::text::old_ro_to_modern(s));
    let words: Vec<String> = filter.split_whitespace().map(norm).collect();
    let mut v: Vec<usize> = (0..c.items.len())
        .filter(|&i| {
            let it = &c.items[i];
            if kind.is_some_and(|k| it.kind != k) || status.is_some_and(|s| it.status != s) {
                return false;
            }
            let hay = norm(&format!(
                "{} {} {} {} {}",
                it.title,
                it.issue,
                it.date,
                it.tags.join(" "),
                it.notes
            ));
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect();
    v.sort_by(|&a, &b| {
        let (x, y) = (&c.items[a], &c.items[b]);
        let o = match sort {
            SortKey::Title => {
                legatorocr_core::text::fold(&x.title).cmp(&legatorocr_core::text::fold(&y.title))
            }
            SortKey::Issue => x
                .date
                .cmp(&y.date)
                .then_with(|| legatorocr_engine::files::natural_cmp(&x.issue, &y.issue)),
            SortKey::Pages => x.pages.cmp(&y.pages),
            SortKey::Status => x.status.cmp(&y.status),
        };
        let o = o.then_with(|| a.cmp(&b));
        if ascending {
            o
        } else {
            o.reverse()
        }
    });
    v
}

fn split_list(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

enum Msg {
    Progress(usize, usize, String),
    Done(Box<(Collection, Index, Result<col::BatchReport, String>)>),
}

struct Job {
    rx: mpsc::Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
    title: String,
}

pub struct CollectionsWindow {
    pub show: bool,
    file: Option<PathBuf>,
    col: Option<Collection>,
    index: Index,
    selected: Option<String>,
    marked: HashSet<String>,
    filter: String,
    kind_filter: Option<Kind>,
    status_filter: Option<Status>,
    sort: SortKey,
    ascending: bool,
    query: String,
    hits: Vec<Hit>,
    hit_sel: usize,
    message: Option<(String, bool)>,
    job: Option<Job>,
    dirty: bool,
    confirm_remove: bool,
    profiles: Vec<String>,
    scroll_to: Option<String>,
    hovered: bool,
    focus_search: bool,
    tried_last: bool,
}

impl Default for CollectionsWindow {
    fn default() -> Self {
        CollectionsWindow {
            show: false,
            file: None,
            col: None,
            index: Index::default(),
            selected: None,
            marked: HashSet::new(),
            filter: String::new(),
            kind_filter: None,
            status_filter: None,
            sort: SortKey::Title,
            ascending: true,
            query: String::new(),
            hits: Vec::new(),
            hit_sel: 0,
            message: None,
            job: None,
            dirty: false,
            confirm_remove: false,
            profiles: Vec::new(),
            scroll_to: None,
            hovered: false,
            focus_search: false,
            tried_last: false,
        }
    }
}

fn last_path_file() -> Option<PathBuf> {
    crate::settings::config_path().map(|p| p.with_file_name("collection_last.txt"))
}

impl CollectionsWindow {
    pub fn open(&mut self) {
        self.show = true;
        self.profiles = legatorocr_engine::profiles::list()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        if !self.tried_last && self.col.is_none() {
            self.tried_last = true;
            if let Some(p) = last_path_file()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .map(|s| PathBuf::from(s.trim()))
                .filter(|p| p.is_file())
            {
                self.load(&p);
                self.message = None;
            }
        }
    }

    /// While the window is under the pointer the arrow keys belong to its table, not to the
    /// page list of the main window.
    pub fn captures_keys(&self) -> bool {
        self.show && self.hovered
    }

    fn err(&mut self, e: impl std::fmt::Display) {
        self.message = Some((format!("{e}"), true));
    }

    fn load(&mut self, path: &Path) {
        match Collection::load(path)
            .and_then(|c| Index::load(&col::index_path(path)).map(|i| (c, i)))
        {
            Ok((c, i)) => {
                self.col = Some(c);
                self.index = i;
                self.file = Some(path.to_path_buf());
                self.selected = None;
                self.marked.clear();
                self.hits.clear();
                self.query.clear();
                if let Some(f) = last_path_file() {
                    let _ = std::fs::write(f, path.to_string_lossy().as_bytes());
                }
                self.message = None;
            }
            Err(e) => self.err(format!("{e:#}")),
        }
    }

    fn create(&mut self, path: PathBuf) {
        let path = if path.extension().is_none() {
            path.with_extension(col::EXT)
        } else {
            path
        };
        if path.exists() {
            self.err(format!("{}: exists", path.display()));
            return;
        }
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match Collection::new(&name).save(&path) {
            Ok(()) => self.load(&path),
            Err(e) => self.err(format!("{e:#}")),
        }
    }

    fn save(&mut self) {
        if let (Some(c), Some(f)) = (&self.col, &self.file) {
            if let Err(e) = c.save(f) {
                let m = format!("{e:#}");
                self.err(m);
            }
        }
        self.dirty = false;
    }

    fn run_search(&mut self) {
        self.hits = if self.query.trim().is_empty() {
            Vec::new()
        } else {
            self.index.search(&self.query, 300)
        };
        self.hit_sel = 0;
    }

    fn start(&mut self, ctx: &egui::Context, ids: Vec<String>) {
        let (Some(c), Some(f)) = (self.col.clone(), self.file.clone()) else {
            return;
        };
        if ids.is_empty() || self.job.is_some() {
            return;
        }
        self.save();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let total = ids.len();
        let (c2, ctx2, cancel2) = (cancel.clone(), ctx.clone(), cancel.clone());
        let mut ix = self.index.clone();
        std::thread::spawn(move || {
            let mut col = c;
            let tx2 = tx.clone();
            let ctx3 = ctx2.clone();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                col::run_batch(
                    &f,
                    &mut col,
                    &mut ix,
                    &ids,
                    &RunConfig::default(),
                    &cancel2,
                    &col::run_item_with_engine,
                    &mut |n, t, title| {
                        let _ = tx2.send(Msg::Progress(n, t, title.to_string()));
                        ctx3.request_repaint();
                    },
                )
                .map_err(|e| format!("{e:#}"))
            }))
            .unwrap_or_else(|_| Err("internal error".to_string()));
            let _ = tx.send(Msg::Done(Box::new((col, ix, r))));
            ctx2.request_repaint();
        });
        self.job = Some(Job {
            rx,
            cancel: c2,
            done: 0,
            total,
            title: String::new(),
        });
    }

    fn pump(&mut self, tr: Tr) {
        let Some(job) = &mut self.job else { return };
        let mut finished = None;
        while let Ok(m) = job.rx.try_recv() {
            match m {
                Msg::Progress(n, t, title) => {
                    job.done = n;
                    job.total = t;
                    job.title = title;
                }
                Msg::Done(b) => finished = Some(*b),
            }
        }
        if let Some((c, ix, r)) = finished {
            self.job = None;
            self.col = Some(c);
            self.index = ix;
            self.run_search();
            self.message = Some(match r {
                Ok(rep) => {
                    let mut s = format!(
                        "{}: {} · {}: {}",
                        tr("col.done"),
                        rep.ok.len(),
                        tr("col.failed"),
                        rep.failed.len()
                    );
                    if rep.cancelled {
                        s.push_str(&format!(" · {}", tr("col.cancelled")));
                    }
                    (s, !rep.failed.is_empty())
                }
                Err(e) => (e, true),
            });
        }
    }

    fn selected_item(&self) -> Option<&Item> {
        let id = self.selected.as_ref()?;
        self.col.as_ref()?.item(id)
    }

    fn target_ids(&self) -> Vec<String> {
        let Some(c) = &self.col else {
            return Vec::new();
        };
        let mut ids: Vec<String> = c
            .items
            .iter()
            .filter(|i| self.marked.contains(&i.id) || self.selected.as_ref() == Some(&i.id))
            .map(|i| i.id.clone())
            .collect();
        ids.dedup();
        ids
    }

    fn pdf_of(&self, it: &Item) -> Option<PathBuf> {
        let f = self.file.as_ref()?;
        if it.output.is_empty() {
            return None;
        }
        Some(col::from_stored(
            f.parent().unwrap_or(Path::new(".")),
            &it.output,
        ))
    }

    fn open_pdf(&mut self, tr: Tr) {
        let p = self.selected_item().and_then(|i| self.pdf_of(i));
        match p {
            Some(p) if p.is_file() => crate::alloptions::open_target(&p.to_string_lossy()),
            _ => self.message = Some((tr("col.no_pdf").to_string(), true)),
        }
    }

    fn show_folder(&mut self, tr: Tr) {
        let dir = self
            .selected_item()
            .and_then(|i| self.pdf_of(i))
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.file
                    .as_ref()
                    .and_then(|f| f.parent().map(Path::to_path_buf))
            });
        match dir {
            Some(d) if d.is_dir() || d.as_os_str().is_empty() => {
                let d = if d.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    d
                };
                crate::alloptions::open_target(&d.to_string_lossy())
            }
            _ => self.message = Some((tr("col.no_pdf").to_string(), true)),
        }
    }

    fn add_report(&mut self, tr: Tr, added: usize, dup: usize, ignored: usize) {
        let mut s = format!("{added} {}", tr("col.added"));
        if dup > 0 {
            s.push_str(&format!(" · {dup} {}", tr("col.dups")));
        }
        if ignored > 0 {
            s.push_str(&format!(" · {ignored} {}", tr("col.ignored")));
        }
        self.message = Some((s, false));
    }

    fn add_files(&mut self, tr: Tr) {
        let Some(f) = self.file.clone() else { return };
        let picked = rfd::FileDialog::new()
            .add_filter(
                "PDF, images, .lgr",
                &["pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "lgr"],
            )
            .pick_files();
        let Some(files) = picked else { return };
        let Some(c) = self.col.as_mut() else { return };
        // PDFs and projects are one item each; loose images are the pages of one item
        let (docs, imgs): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().partition(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf") || e.eq_ignore_ascii_case("lgr"))
        });
        let mut added = Vec::new();
        let mut errs = Vec::new();
        for d in docs {
            match c.add_item(&f, &[d]) {
                Ok(id) => added.push(id),
                Err(e) => errs.push(format!("{e:#}")),
            }
        }
        if !imgs.is_empty() {
            let mut imgs = imgs;
            imgs.sort_by(|a, b| {
                legatorocr_engine::files::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())
            });
            match c.add_item(&f, &imgs) {
                Ok(id) => added.push(id),
                Err(e) => errs.push(format!("{e:#}")),
            }
        }
        if let Some(id) = added.last() {
            self.selected = Some(id.clone());
            self.scroll_to = Some(id.clone());
        }
        let n = added.len();
        self.save();
        if let Some(e) = errs.first() {
            self.err(e.clone());
        } else {
            self.add_report(tr, n, 0, 0);
        }
    }

    fn add_folder(&mut self, tr: Tr) {
        let Some(f) = self.file.clone() else { return };
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let Some(c) = self.col.as_mut() else { return };
        match c.add_folder(&f, &dir) {
            Ok(r) => {
                if let Some(id) = r.added.last() {
                    self.selected = Some(id.clone());
                    self.scroll_to = Some(id.clone());
                }
                self.save();
                self.add_report(tr, r.added.len(), r.duplicates, r.ignored);
            }
            Err(e) => self.err(format!("{e:#}")),
        }
    }

    fn remove_selected(&mut self) {
        let ids = self.target_ids();
        if let Some(c) = &mut self.col {
            for id in &ids {
                c.remove(id);
                self.index.remove(id);
            }
        }
        if let Some(f) = &self.file {
            let _ = self.index.save(&col::index_path(f));
        }
        self.selected = None;
        self.marked.clear();
        self.confirm_remove = false;
        self.run_search();
        self.save();
    }

    pub fn ui(&mut self, ctx: &egui::Context, tr: Tr) {
        if !self.show {
            return;
        }
        self.pump(tr);
        let mut open = true;
        let resp = egui::Window::new(tr("col.title"))
            .open(&mut open)
            .default_size([1100.0, 700.0])
            .min_size([640.0, 420.0])
            .show(ctx, |ui| self.contents(ui, ctx, tr));
        self.hovered = resp.is_some_and(|r| {
            ctx.input(|i| i.pointer.hover_pos())
                .is_some_and(|p| r.response.rect.contains(p))
        });
        if !open {
            if self.dirty {
                self.save();
            }
            if let Some(j) = &self.job {
                j.cancel.store(true, Ordering::Relaxed);
            }
            self.show = false;
            self.hovered = false;
        }
        if self.dirty && !ctx.egui_wants_keyboard_input() {
            self.save();
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, tr: Tr) {
        // ---- top bar
        ui.horizontal_wrapped(|ui| {
            let idle = self.job.is_none();
            if ui
                .add_enabled(idle, egui::Button::new(tr("col.new")))
                .on_hover_text(tr("col.new_tip"))
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("LegatorOCR collection", &[col::EXT])
                    .set_file_name(format!("collection.{}", col::EXT))
                    .save_file()
                {
                    self.create(p);
                }
            }
            if ui
                .add_enabled(idle, egui::Button::new(tr("col.open")))
                .on_hover_text(tr("col.open_tip"))
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("LegatorOCR collection", &[col::EXT])
                    .pick_file()
                {
                    self.load(&p);
                }
            }
            if let (Some(c), Some(f)) = (&self.col, &self.file) {
                ui.label(RichText::new(&c.name).strong())
                    .on_hover_text(f.display().to_string());
            }
        });
        if let Some((m, bad)) = self.message.clone() {
            ui.horizontal_wrapped(|ui| {
                let t = RichText::new(&m);
                ui.label(if bad {
                    t.color(Color32::from_rgb(220, 90, 80))
                } else {
                    t
                });
                if ui
                    .small_button("×")
                    .on_hover_text(tr("col.dismiss"))
                    .clicked()
                {
                    self.message = None;
                }
            });
        }
        ui.separator();

        let Some(items_len) = self.col.as_ref().map(|c| c.items.len()) else {
            self.empty_no_collection(ui, tr);
            return;
        };
        if items_len == 0 && self.job.is_none() {
            self.empty_no_items(ui, ctx, tr);
            return;
        }

        // ---- search and batch bar
        let busy = self.job.is_some();
        ui.horizontal_wrapped(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text(tr("col.search_hint"))
                    .desired_width(320.0),
            );
            if std::mem::take(&mut self.focus_search) {
                r.request_focus();
            }
            r.clone().on_hover_text(tr("col.search_tip"));
            if r.changed() {
                self.run_search();
            }
            if r.has_focus() {
                let (down, up, enter) = ctx.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    )
                });
                if !self.hits.is_empty() {
                    if down {
                        self.hit_sel = (self.hit_sel + 1).min(self.hits.len() - 1);
                    }
                    if up {
                        self.hit_sel = self.hit_sel.saturating_sub(1);
                    }
                    if enter {
                        let id = self.hits[self.hit_sel].item_id.clone();
                        self.selected = Some(id.clone());
                        self.scroll_to = Some(id);
                    }
                }
            }
            if ui
                .add_enabled(!self.query.is_empty(), egui::Button::new("×"))
                .on_hover_text(tr("col.search_clear"))
                .clicked()
            {
                self.query.clear();
                self.run_search();
            }
            ui.separator();
            if ui
                .add_enabled(!busy, egui::Button::new(tr("col.add_files")))
                .on_hover_text(tr("col.add_files_tip"))
                .clicked()
            {
                self.add_files(tr);
            }
            if ui
                .add_enabled(!busy, egui::Button::new(tr("col.add_folder")))
                .on_hover_text(tr("col.add_folder_tip"))
                .clicked()
            {
                self.add_folder(tr);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let has_sel = self.selected.is_some() || !self.marked.is_empty();
            let pending = self
                .col
                .as_ref()
                .map(|c| c.pending_ids(false).len())
                .unwrap_or(0);
            if ui
                .add_enabled(has_sel && !busy, egui::Button::new(tr("col.process_sel")))
                .on_hover_text(tr("col.process_sel_tip"))
                .clicked()
            {
                let ids = self.target_ids();
                self.start(ctx, ids);
            }
            if ui
                .add_enabled(
                    pending > 0 && !busy,
                    egui::Button::new(format!("{} ({pending})", tr("col.process_all"))),
                )
                .on_hover_text(tr("col.process_all_tip"))
                .clicked()
            {
                let ids = self
                    .col
                    .as_ref()
                    .map(|c| c.pending_ids(false))
                    .unwrap_or_default();
                self.start(ctx, ids);
            }
            let has_pdf = self
                .selected_item()
                .and_then(|i| self.pdf_of(i))
                .is_some_and(|p| p.is_file());
            if ui
                .add_enabled(has_pdf, egui::Button::new(tr("col.open_pdf")))
                .on_hover_text(tr("col.open_pdf_tip"))
                .clicked()
            {
                self.open_pdf(tr);
            }
            if ui
                .add_enabled(
                    self.selected.is_some(),
                    egui::Button::new(tr("col.show_folder")),
                )
                .on_hover_text(tr("col.show_folder_tip"))
                .clicked()
            {
                self.show_folder(tr);
            }
            if self.confirm_remove {
                if ui
                    .button(
                        RichText::new(tr("col.remove_confirm"))
                            .color(Color32::from_rgb(220, 90, 80)),
                    )
                    .on_hover_text(tr("col.remove_tip"))
                    .clicked()
                {
                    self.remove_selected();
                }
                if ui.button(tr("col.cancel_remove")).clicked() {
                    self.confirm_remove = false;
                }
            } else if ui
                .add_enabled(has_sel && !busy, egui::Button::new(tr("col.remove")))
                .on_hover_text(tr("col.remove_tip"))
                .clicked()
            {
                self.confirm_remove = true;
            }
        });
        if let Some(j) = &self.job {
            let frac = if j.total == 0 {
                0.0
            } else {
                j.done as f32 / j.total as f32
            };
            let label = format!(
                "{} {}/{} {}",
                tr("col.processing"),
                (j.done + 1).min(j.total),
                j.total,
                j.title
            );
            let cancel_now = ui
                .horizontal(|ui| {
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(320.0)
                            .text(label),
                    );
                    ui.button(tr("col.cancel"))
                        .on_hover_text(tr("col.cancel_tip"))
                        .clicked()
                })
                .inner;
            if cancel_now {
                j.cancel.store(true, Ordering::Relaxed);
            }
            ui.label(RichText::new(tr("col.busy")).weak());
        }
        ui.add_space(2.0);

        // ---- panes
        let h = ui.available_height().max(200.0);
        let total_w = ui.available_width();
        let left_w = (total_w * 0.58).max(360.0).min(total_w - 260.0).max(200.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left_w, h),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| self.left_pane(ui, ctx, tr),
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2((ui.available_width()).max(240.0), h),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| self.detail_pane(ui, tr),
            );
        });
    }

    fn empty_no_collection(&mut self, ui: &mut egui::Ui, tr: Tr) {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(tr("col.empty_none")).size(16.0));
            ui.add_space(12.0);
            if ui
                .add_sized(
                    [260.0, 48.0],
                    egui::Button::new(RichText::new(tr("col.new")).size(18.0)),
                )
                .on_hover_text(tr("col.new_tip"))
                .clicked()
            {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("LegatorOCR collection", &[col::EXT])
                    .set_file_name(format!("collection.{}", col::EXT))
                    .save_file()
                {
                    self.create(p);
                }
            }
        });
    }

    fn empty_no_items(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, tr: Tr) {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(tr("col.empty_items")).size(16.0));
            ui.add_space(12.0);
            if ui
                .add_sized(
                    [260.0, 48.0],
                    egui::Button::new(RichText::new(tr("col.add_folder")).size(18.0)),
                )
                .on_hover_text(tr("col.add_folder_tip"))
                .clicked()
            {
                self.add_folder(tr);
            }
            ui.add_space(8.0);
            if ui
                .button(tr("col.add_files"))
                .on_hover_text(tr("col.add_files_tip"))
                .clicked()
            {
                self.add_files(tr);
            }
        });
    }

    fn left_pane(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, tr: Tr) {
        // filters
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(tr("col.filter_hint"))
                    .desired_width(160.0),
            )
            .on_hover_text(tr("col.filter_tip"));
            let kt = self.kind_filter.map_or(tr("col.all"), |k| tr(kind_key(k)));
            egui::ComboBox::from_id_salt("col_kind_filter")
                .selected_text(kt)
                .width(110.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.kind_filter, None, tr("col.all"));
                    for k in Kind::ALL {
                        ui.selectable_value(&mut self.kind_filter, Some(k), tr(kind_key(k)));
                    }
                });
            let st = self
                .status_filter
                .map_or(tr("col.all"), |s| tr(status_key(s)));
            egui::ComboBox::from_id_salt("col_status_filter")
                .selected_text(st)
                .width(110.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.status_filter, None, tr("col.all"));
                    for s in Status::ALL {
                        ui.selectable_value(&mut self.status_filter, Some(s), tr(status_key(s)));
                    }
                });
        });
        let Some(c) = &self.col else { return };
        let rows = visible_rows(
            c,
            &self.filter,
            self.kind_filter,
            self.status_filter,
            self.sort,
            self.ascending,
        );
        let shown = rows.len();
        ui.label(
            RichText::new(format!("{shown} / {}", c.items.len()))
                .weak()
                .small(),
        );

        // keyboard: the table owns the arrows while the pointer is over this window
        if !ctx.egui_wants_keyboard_input() && self.hovered && self.job.is_none() {
            let (down, up, enter, find) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    i.consume_key(egui::Modifiers::COMMAND, egui::Key::J),
                )
            });
            if find {
                self.focus_search = true;
            }
            if !rows.is_empty() && (down || up) {
                let cur = self
                    .selected
                    .as_ref()
                    .and_then(|id| rows.iter().position(|&r| &c.items[r].id == id));
                let n = match (cur, down) {
                    (None, _) => 0,
                    (Some(i), true) => (i + 1).min(rows.len() - 1),
                    (Some(i), false) => i.saturating_sub(1),
                };
                let id = c.items[rows[n]].id.clone();
                self.selected = Some(id.clone());
                self.scroll_to = Some(id);
                self.confirm_remove = false;
            }
            if enter && self.selected.is_some() {
                self.open_pdf(tr);
            }
        }
        let Some(c) = &self.col else { return };

        // header
        let mut new_sort = None;
        egui::Grid::new("col_head")
            .num_columns(5)
            .min_col_width(24.0)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                ui.label("").on_hover_text(tr("col.h_kind_tip"));
                for (k, key, w) in [
                    (SortKey::Title, "col.h_title", 190.0),
                    (SortKey::Issue, "col.h_issue", 110.0),
                    (SortKey::Pages, "col.h_pages", 50.0),
                    (SortKey::Status, "col.h_status", 90.0),
                ] {
                    let arrow = if self.sort == k {
                        if self.ascending {
                            " ↑"
                        } else {
                            " ↓"
                        }
                    } else {
                        ""
                    };
                    if ui
                        .add_sized(
                            [w, 20.0],
                            egui::Button::new(format!("{}{arrow}", tr(key))).frame(false),
                        )
                        .on_hover_text(tr("col.sort_tip"))
                        .clicked()
                    {
                        new_sort = Some(k);
                    }
                }
                ui.end_row();
            });
        if let Some(k) = new_sort {
            if self.sort == k {
                self.ascending = !self.ascending;
            } else {
                self.sort = k;
                self.ascending = true;
            }
        }

        // search results take the lower part when there is a query
        let results_h = if self.query.trim().is_empty() {
            0.0
        } else {
            (ui.available_height() * 0.4).clamp(90.0, 260.0)
        };
        let table_h = (ui.available_height() - results_h - 6.0).max(80.0);
        let mut clicked: Option<(String, bool)> = None;
        let mut dbl: Option<String> = None;
        let scroll_to = self.scroll_to.take();
        egui::ScrollArea::vertical()
            .id_salt("col_table")
            .max_height(table_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if rows.is_empty() {
                    ui.label(RichText::new(tr("col.no_rows")).weak());
                }
                egui::Grid::new("col_rows")
                    .num_columns(5)
                    .min_col_width(24.0)
                    .spacing([8.0, 3.0])
                    .show(ui, |ui| {
                        for &i in &rows {
                            let it = &c.items[i];
                            let sel = self.selected.as_ref() == Some(&it.id);
                            let mark = self.marked.contains(&it.id);
                            let on = sel || mark;
                            let kname = tr(kind_key(it.kind));
                            let letter: String =
                                kname.chars().take(1).flat_map(char::to_uppercase).collect();
                            let mut resp = Vec::new();
                            resp.push(
                                ui.add_sized(
                                    [24.0, 20.0],
                                    egui::Button::selectable(on, RichText::new(letter).strong()),
                                )
                                .on_hover_text(kname),
                            );
                            let title = if it.title.is_empty() {
                                "—"
                            } else {
                                it.title.as_str()
                            };
                            resp.push(
                                ui.add_sized([190.0, 20.0], egui::Button::selectable(on, title))
                                    .on_hover_text(&it.title),
                            );
                            resp.push(ui.add_sized(
                                [110.0, 20.0],
                                egui::Button::selectable(on, it.issue_line()),
                            ));
                            let pages = it.pages.map_or("–".to_string(), |p| p.to_string());
                            resp.push(
                                ui.add_sized([50.0, 20.0], egui::Button::selectable(on, pages)),
                            );
                            let chip = chip_of(it);
                            let r = ui.add_sized(
                                [90.0, 20.0],
                                egui::Button::selectable(
                                    on,
                                    RichText::new(format!(" {} ", tr(chip.key())))
                                        .color(Color32::WHITE)
                                        .background_color(chip.bg()),
                                ),
                            );
                            resp.push(if it.error.is_empty() {
                                r
                            } else {
                                r.on_hover_text(&it.error)
                            });
                            for r in &resp {
                                if r.clicked() {
                                    clicked =
                                        Some((it.id.clone(), ui.input(|i| i.modifiers.command)));
                                }
                                if r.double_clicked() {
                                    dbl = Some(it.id.clone());
                                }
                            }
                            if scroll_to.as_ref() == Some(&it.id) {
                                resp[1].scroll_to_me(Some(egui::Align::Center));
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some((id, ctrl)) = clicked {
            self.confirm_remove = false;
            if ctrl {
                if !self.marked.remove(&id) {
                    self.marked.insert(id.clone());
                }
            } else {
                self.marked.clear();
            }
            self.selected = Some(id);
        }
        if let Some(id) = dbl {
            self.selected = Some(id);
            self.open_pdf(tr);
        }
        ui.label(RichText::new(tr("col.keys")).weak().small());

        let Some(c) = &self.col else { return };
        // results
        if results_h > 0.0 {
            ui.separator();
            let n = self.hits.len();
            let head = if self.index.page_count() == 0 {
                tr("col.no_index").to_string()
            } else {
                format!(
                    "{}: {n}{}",
                    tr("col.results"),
                    if n >= 300 { "+" } else { "" }
                )
            };
            ui.label(RichText::new(head).strong());
            let mut pick: Option<(usize, bool)> = None;
            egui::ScrollArea::vertical()
                .id_salt("col_hits")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if n == 0 && self.index.page_count() > 0 {
                        ui.label(RichText::new(tr("col.no_results")).weak());
                    }
                    for (k, h) in self.hits.iter().enumerate() {
                        let Some(it) = c.item(&h.item_id) else {
                            continue;
                        };
                        let line = format!(
                            "{} {} — {} {}{}: {}",
                            it.title,
                            if it.issue_line().is_empty() {
                                String::new()
                            } else {
                                format!("[{}]", it.issue_line())
                            },
                            tr("col.page"),
                            h.page,
                            if h.loose {
                                format!(" {}", tr("col.loose"))
                            } else {
                                String::new()
                            },
                            h.snippet
                        );
                        let r = ui
                            .add(egui::Button::selectable(k == self.hit_sel, line).wrap())
                            .on_hover_text(tr("col.hit_tip"));
                        if r.clicked() {
                            pick = Some((k, false));
                        }
                        if r.double_clicked() {
                            pick = Some((k, true));
                        }
                    }
                });
            if let Some((k, open_pdf)) = pick {
                self.hit_sel = k;
                let id = self.hits[k].item_id.clone();
                self.selected = Some(id.clone());
                self.scroll_to = Some(id);
                self.marked.clear();
                if open_pdf {
                    self.open_pdf(tr);
                }
            }
        }
    }

    fn detail_pane(&mut self, ui: &mut egui::Ui, tr: Tr) {
        let busy = self.job.is_some();
        egui::ScrollArea::vertical()
            .id_salt("col_detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // collection settings first, collapsed
                if let Some(c) = &mut self.col {
                    let mut ch = false;
                    egui::CollapsingHeader::new(tr("col.settings"))
                        .id_salt("col_settings")
                        .show(ui, |ui| {
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.label(tr("col.f_name"));
                                ch |= ui.text_edit_singleline(&mut c.name).changed();
                                ui.label(tr("col.f_description"));
                                ch |= ui
                                    .add(
                                        egui::TextEdit::multiline(&mut c.description)
                                            .desired_rows(2)
                                            .desired_width(f32::INFINITY),
                                    )
                                    .changed();
                                ui.label(tr("col.f_profile"));
                                let cur = if c.default_profile.is_empty() {
                                    tr("col.profile_default_none").to_string()
                                } else {
                                    c.default_profile.clone()
                                };
                                egui::ComboBox::from_id_salt("col_def_profile")
                                    .selected_text(cur)
                                    .show_ui(ui, |ui| {
                                        ch |= ui
                                            .selectable_value(
                                                &mut c.default_profile,
                                                String::new(),
                                                tr("col.profile_default_none"),
                                            )
                                            .changed();
                                        for p in &self.profiles {
                                            ch |= ui
                                                .selectable_value(
                                                    &mut c.default_profile,
                                                    p.clone(),
                                                    p,
                                                )
                                                .changed();
                                        }
                                    });
                                ch |= ui
                                    .checkbox(&mut c.ocr, tr("col.s_ocr"))
                                    .on_hover_text(tr("col.s_ocr_tip"))
                                    .changed();
                                ui.label(tr("col.s_outdir"));
                                ui.horizontal(|ui| {
                                    ui.label(if c.output_dir.is_empty() {
                                        tr("col.s_outdir_next")
                                    } else {
                                        c.output_dir.as_str()
                                    });
                                    if ui.button(tr("col.choose")).clicked() {
                                        if let (Some(d), Some(f)) =
                                            (rfd::FileDialog::new().pick_folder(), &self.file)
                                        {
                                            c.output_dir = col::to_stored(
                                                f.parent().unwrap_or(Path::new(".")),
                                                &d,
                                            );
                                            ch = true;
                                        }
                                    }
                                    if !c.output_dir.is_empty()
                                        && ui
                                            .button("×")
                                            .on_hover_text(tr("col.s_outdir_next"))
                                            .clicked()
                                    {
                                        c.output_dir.clear();
                                        ch = true;
                                    }
                                });
                            });
                        });
                    if ch {
                        self.dirty = true;
                    }
                    ui.separator();
                }
                let Some(id) = self.selected.clone() else {
                    ui.label(RichText::new(tr("col.none_selected")).weak());
                    return;
                };
                let profiles = self.profiles.clone();
                let file = self.file.clone();
                let Some(it) = self.col.as_mut().and_then(|c| c.item_mut(&id)) else {
                    ui.label(RichText::new(tr("col.none_selected")).weak());
                    return;
                };
                let mut ch = false;
                ui.add_enabled_ui(!busy, |ui| {
                    egui::Grid::new("col_detail_grid")
                        .num_columns(2)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(tr("col.f_title"));
                            ch |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut it.title)
                                        .desired_width(f32::INFINITY),
                                )
                                .changed();
                            ui.end_row();
                            ui.label(tr("col.f_kind"));
                            egui::ComboBox::from_id_salt("col_kind")
                                .selected_text(tr(kind_key(it.kind)))
                                .show_ui(ui, |ui| {
                                    for k in Kind::ALL {
                                        ch |= ui
                                            .selectable_value(&mut it.kind, k, tr(kind_key(k)))
                                            .changed();
                                    }
                                });
                            ui.end_row();
                            ui.label(tr("col.f_issue"));
                            ch |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut it.issue)
                                        .desired_width(f32::INFINITY),
                                )
                                .changed();
                            ui.end_row();
                            ui.label(tr("col.f_date"));
                            ch |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut it.date)
                                        .hint_text("1977-09")
                                        .desired_width(f32::INFINITY),
                                )
                                .on_hover_text(tr("col.f_date_tip"))
                                .changed();
                            ui.end_row();
                            ui.label(tr("col.f_langs"));
                            let mut langs = it.languages.join(", ");
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut langs)
                                        .hint_text("ro, fr")
                                        .desired_width(f32::INFINITY),
                                )
                                .on_hover_text(tr("col.f_langs_tip"))
                                .changed()
                            {
                                it.languages = split_list(&langs);
                                ch = true;
                            }
                            ui.end_row();
                            ui.label(tr("col.f_tags"));
                            let mut tags = it.tags.join(", ");
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut tags)
                                        .desired_width(f32::INFINITY),
                                )
                                .changed()
                            {
                                it.tags = split_list(&tags);
                                ch = true;
                            }
                            ui.end_row();
                            ui.label(tr("col.f_profile"));
                            let cur = if it.profile.is_empty() {
                                tr("col.profile_default").to_string()
                            } else {
                                it.profile.clone()
                            };
                            egui::ComboBox::from_id_salt("col_profile")
                                .selected_text(cur)
                                .show_ui(ui, |ui| {
                                    ch |= ui
                                        .selectable_value(
                                            &mut it.profile,
                                            String::new(),
                                            tr("col.profile_default"),
                                        )
                                        .changed();
                                    for p in &profiles {
                                        ch |= ui
                                            .selectable_value(&mut it.profile, p.clone(), p)
                                            .changed();
                                    }
                                });
                            ui.end_row();
                            ui.label(tr("col.f_status"));
                            egui::ComboBox::from_id_salt("col_status")
                                .selected_text(tr(status_key(it.status)))
                                .show_ui(ui, |ui| {
                                    for s in Status::ALL {
                                        if ui
                                            .selectable_value(&mut it.status, s, tr(status_key(s)))
                                            .changed()
                                        {
                                            ch = true;
                                            if s != Status::Todo {
                                                it.error.clear();
                                            }
                                        }
                                    }
                                });
                            ui.end_row();
                        });
                    ui.label(tr("col.f_notes"));
                    ch |= ui
                        .add(
                            egui::TextEdit::multiline(&mut it.notes)
                                .desired_rows(4)
                                .desired_width(f32::INFINITY),
                        )
                        .changed();
                });
                ui.separator();
                ui.label(RichText::new(tr("col.f_sources")).strong());
                let base = file
                    .as_ref()
                    .and_then(|f| f.parent().map(Path::to_path_buf))
                    .unwrap_or_default();
                for s in &it.sources {
                    let exists = col::from_stored(&base, s).exists();
                    let t = RichText::new(s);
                    ui.label(if exists {
                        t
                    } else {
                        t.color(Color32::from_rgb(220, 90, 80))
                    })
                    .on_hover_text(if exists {
                        ""
                    } else {
                        tr("col.source_missing")
                    });
                    if !exists {
                        ui.label(
                            RichText::new(tr("col.source_missing"))
                                .small()
                                .color(Color32::from_rgb(220, 90, 80)),
                        );
                    }
                }
                ui.label(RichText::new(tr("col.f_output")).strong());
                ui.label(if it.output.is_empty() {
                    "–"
                } else {
                    it.output.as_str()
                });
                if !it.error.is_empty() {
                    ui.label(
                        RichText::new(tr("col.f_error"))
                            .strong()
                            .color(Color32::from_rgb(220, 90, 80)),
                    );
                    ui.label(RichText::new(&it.error).color(Color32::from_rgb(220, 90, 80)));
                }
                if ch {
                    self.dirty = true;
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Collection {
        let mut c = Collection::new("t");
        for (t, k, d, i, p, s) in [
            (
                "Scânteia",
                Kind::Newspaper,
                "1977-09",
                "8801",
                Some(8),
                Status::OcrDone,
            ),
            (
                "Viața Românească",
                Kind::Magazine,
                "1985",
                "14",
                Some(120),
                Status::Todo,
            ),
            ("Amintiri", Kind::Book, "1892", "", None, Status::Checked),
            (
                "Scânteia tineretului",
                Kind::Newspaper,
                "1977-10",
                "8810",
                Some(4),
                Status::Todo,
            ),
        ] {
            c.push(Item {
                title: t.into(),
                kind: k,
                date: d.into(),
                issue: i.into(),
                pages: p,
                status: s,
                tags: vec!["arhiva".into()],
                ..Default::default()
            });
        }
        c
    }

    fn titles(c: &Collection, v: &[usize]) -> Vec<String> {
        v.iter().map(|&i| c.items[i].title.clone()).collect()
    }

    #[test]
    fn filter_ignores_case_and_diacritics_and_combines() {
        let c = sample();
        for q in ["scanteia", "Scînteia", "SCÂNTEIA"] {
            let r = visible_rows(&c, q, None, None, SortKey::Title, true);
            assert_eq!(
                titles(&c, &r),
                vec!["Scânteia", "Scânteia tineretului"],
                "{q}"
            );
        }
        let r = visible_rows(&c, "scanteia", None, None, SortKey::Title, true);
        assert_eq!(titles(&c, &r), vec!["Scânteia", "Scânteia tineretului"]);
        let r = visible_rows(
            &c,
            "",
            Some(Kind::Newspaper),
            Some(Status::Todo),
            SortKey::Title,
            true,
        );
        assert_eq!(titles(&c, &r), vec!["Scânteia tineretului"]);
        let r = visible_rows(&c, "viata romaneasca 14", None, None, SortKey::Title, true);
        assert_eq!(r.len(), 1);
        assert!(visible_rows(&c, "zzz", None, None, SortKey::Title, true).is_empty());
    }

    #[test]
    fn sorting_by_each_column_both_ways() {
        let c = sample();
        let by = |k, asc| titles(&c, &visible_rows(&c, "", None, None, k, asc));
        assert_eq!(by(SortKey::Title, true)[0], "Amintiri");
        assert_eq!(by(SortKey::Title, false)[0], "Viața Românească");
        assert_eq!(by(SortKey::Issue, true)[0], "Amintiri");
        assert_eq!(
            by(SortKey::Pages, true)[0],
            "Amintiri",
            "no page count sorts first"
        );
        assert_eq!(by(SortKey::Pages, false)[0], "Viața Românească");
        assert_eq!(by(SortKey::Status, true)[0], "Viața Românească");
    }

    #[test]
    fn failed_items_show_the_failed_chip_with_text() {
        let mut it = Item::default();
        assert_eq!(chip_of(&it), Chip::Todo);
        it.error = "boom".into();
        assert_eq!(chip_of(&it), Chip::Failed);
        it.status = Status::Checked;
        assert_eq!(chip_of(&it), Chip::Checked);
        for c in [
            Chip::Todo,
            Chip::Processed,
            Chip::OcrDone,
            Chip::Checked,
            Chip::Failed,
        ] {
            assert!(c.key().contains("status_"));
        }
        assert_eq!(split_list(" ro, fr;; en "), vec!["ro", "fr", "en"]);
    }
}
