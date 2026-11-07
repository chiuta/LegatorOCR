use crate::i18n::{t, Lang};
use crate::settings::{ModeChoice, PageSizeChoice, SheetAdjust, SplitChoice, UiSettings};
use crate::worker::{self, PreviewRequest, PreviewResult};
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions, Vec2};
use legatorocr_engine::{check_ocr, run_export, write_atomic, ExportJob, PdfExport, SheetInput};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use legatorocr_engine::files::{collect_images, IMAGE_EXT};

fn split_name(lang: Lang, c: SplitChoice) -> &'static str {
    t(
        lang,
        match c {
            SplitChoice::Auto => "split.auto",
            SplitChoice::Off => "split.off",
            SplitChoice::Middle => "split.middle",
        },
    )
}

fn size_name(lang: Lang, c: PageSizeChoice) -> &'static str {
    match c {
        PageSizeChoice::Free => t(lang, "size.free"),
        PageSizeChoice::A4 => "A4",
        PageSizeChoice::A5 => "A5",
        PageSizeChoice::Letter => "Letter",
        PageSizeChoice::Custom => t(lang, "size.custom"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Original,
    Processed,
    Both,
    /// One picture with a movable divider: the original on one side, the processed page on the
    /// other (each fitted to the same frame, so they do not overlap pixel for pixel: the processed
    /// page is straightened and cropped).
    Slider,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd {
    AddImages,
    AddFolder,
    OpenProject,
    SaveProject,
    SaveProjectFolder,
    ExportPdf,
    ExportImages,
    Modules,
    Proof,
    FirstPage,
    LastPage,
    NextAttention,
    OnlyAttention,
    ViewBoth,
    ViewOriginal,
    ViewProcessed,
    ViewSlider,
}

/// The command palette (Ctrl+K): every entry reuses the text of the menu or toolbar item.
const CMDS: &[(Cmd, &str)] = &[
    (Cmd::AddImages, "menu.add_images"),
    (Cmd::AddFolder, "menu.add_folder"),
    (Cmd::OpenProject, "menu.open_project"),
    (Cmd::SaveProject, "menu.save_project"),
    (Cmd::SaveProjectFolder, "menu.save_project_folder"),
    (Cmd::ExportPdf, "menu.export_pdf"),
    (Cmd::ExportImages, "menu.export_images"),
    (Cmd::Modules, "menu.modules"),
    (Cmd::Proof, "menu.proof"),
    (Cmd::FirstPage, "cmd.first_page"),
    (Cmd::LastPage, "cmd.last_page"),
    (Cmd::NextAttention, "cmd.next_attention"),
    (Cmd::OnlyAttention, "cmd.only_attention"),
    (Cmd::ViewBoth, "cmd.view_both"),
    (Cmd::ViewOriginal, "cmd.view_original"),
    (Cmd::ViewProcessed, "cmd.view_processed"),
    (Cmd::ViewSlider, "cmd.view_slider"),
];

struct PageTex {
    tex: TextureHandle,
    w: u32,
    h: u32,
    skew_deg: f32,
    blank: bool,
    content_box: Option<[u32; 4]>,
    dpi: u32,
}

/// Why a processed sheet deserves a look (None = nothing suspicious). A heuristic on the preview
/// only: blank result, skew at the limit, no content found, content touching the page edge.
fn attention(r: &Ready, max_skew: f32, manual_skew: bool) -> Option<&'static str> {
    let p = r.pages.first()?;
    if r.pages.iter().any(|p| p.blank) {
        return Some("attn.blank");
    }
    if !manual_skew && r.pages.iter().any(|p| p.skew_deg.abs() >= 0.9 * max_skew) {
        return Some("attn.skew");
    }
    if r.pages.iter().any(|p| p.content_box.is_none()) {
        return Some("attn.nocontent");
    }
    let touches = |p: &PageTex| {
        p.content_box.is_some_and(|[x, y, w, h]| {
            let (mx, my) = ((p.w / 200).max(2), (p.h / 200).max(2));
            x <= mx || y <= my || x + w + mx >= p.w || y + h + my >= p.h
        })
    };
    let _ = p;
    r.pages.iter().any(touches).then_some("attn.edge")
}

struct Ready {
    key: u64,
    original: TextureHandle,
    original_size: (u32, u32),
    pages: Vec<PageTex>,
}

struct Sheet {
    id: u64,
    path: PathBuf,
    name: String,
    adj: SheetAdjust,
    adj_gen: u32,
    /// History for Undo / Redo: snapshots of every per-sheet adjustment (turns, skew, split, content
    /// box, erase and picture zones, dewarp guides).
    zone_undo: Vec<ZoneSnap>,
    zone_redo: Vec<ZoneSnap>,
    /// When the last change to a control (slider, number box, button) was recorded: changes within
    /// a second of each other are one step, so a drag is one Undo.
    last_edit: Option<std::time::Instant>,
    ready: Option<Ready>,
    pending: Option<u64>,
    failed: Option<(u64, String)>,
    /// Last time (App::tick) this sheet was on screen; used to evict old previews.
    used: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Pdf,
    Images,
}

struct Done {
    kind: JobKind,
    target: PathBuf,
    pages: usize,
    words: usize,
    skipped: usize,
    /// With a split export: how many PDFs were written (the target name itself is not a file).
    docs: usize,
    /// Sheets that could not be processed: (file name, reason).
    failed: Vec<(String, String)>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: usize,
    rx: mpsc::Receiver<Result<Done, String>>,
}

/// Result of probing the OCR engine: (version line, installed languages) or an error text.
type OcrProbe = Result<(String, Vec<String>), String>;

struct Shot {
    path: PathBuf,
    ready_frames: u32,
    requested: bool,
    saved: Arc<AtomicBool>,
}

pub struct App {
    tick: u64,
    lang: Lang,
    s: UiSettings,
    /// Password for encrypted PDFs; kept for this session only, never written to the settings.
    pdf_password: String,
    settings_gen: u32,
    sheets: Vec<Sheet>,
    selected: Option<usize>,
    /// The selection was moved with the keyboard: scroll the list so the row is visible.
    scroll_to_selected: bool,
    palette_open: bool,
    palette_q: String,
    palette_sel: usize,
    /// The page list shows only the pages flagged for attention.
    only_attention: bool,
    /// Where the divider of the slider view is (0..1 of the frame width).
    slider_frac: f32,
    next_id: u64,
    pv_tx: mpsc::Sender<PreviewResult>,
    pv_rx: mpsc::Receiver<PreviewResult>,
    view: View,
    fit: bool,
    zoom: f32,
    job: Option<Job>,
    status: Option<(String, bool)>,
    langs_rx: Option<mpsc::Receiver<OcrProbe>>,
    installed: Vec<String>,
    /// Tesseract has the `osd` data (orientation detection).
    has_osd: bool,
    /// Undo / redo of the global settings (the sheet adjustments have their own stacks).
    settings_undo: Vec<crate::settings::UiSettings>,
    settings_redo: Vec<crate::settings::UiSettings>,
    settings_edit: Option<std::time::Instant>,
    /// When the running preview was started (watchdog for a hung decode).
    preview_started: Option<std::time::Instant>,
    ocr_info: String,
    show_about: bool,
    modules: ModulesWindow,
    proof: crate::proof::ProofWindow,
    allopts: crate::alloptions::AllOptions,
    draw_zones: bool,
    /// With `draw_zones`: the mouse draws dewarp guide lines (freehand along a bent text line).
    draw_guide: bool,
    guide_path: Vec<egui::Pos2>,
    /// A node of a dewarp guide being dragged: (guide, node, position as fractions of the scan).
    guide_live: Option<(usize, usize, (f32, f32))>,
    /// What the mouse draws when `draw_zones` is on: picture zones instead of erase zones.
    draw_pictures: bool,
    /// With `draw_zones`: the rectangles drawn overrule the automatic picture detector.
    draw_nopic: bool,
    /// With `draw_zones`: 1 = draw the working rectangle, 2 = drag the cut line of a two-page sheet.
    draw_extra: u8,
    /// Processed pages show the content box with draggable edges.
    edit_box: bool,
    /// Edge being dragged: (0 left, 1 top, 2 right, 3 bottom, screen position at the start).
    box_drag: Option<(usize, egui::Pos2)>,
    zone_drag: Option<egui::Pos2>,
    /// A corner of an existing erase/picture zone being dragged: (is picture zone, index, corner).
    zone_resize: Option<(u8, usize, u8)>,
    shot: Option<Shot>,
    /// Test hook: run an export without a file dialog (env LEGATOROCR_GUI_AUTOEXPORT_PDF).
    auto_export: Option<(PathBuf, bool)>,
    /// Test hook: after the first preview, apply per-page adjustments (env LEGATOROCR_GUI_TEST_ADJUST).
    adjust_test: Option<bool>,
    /// Settings or language changed and not yet written to the user's config file.
    config_dirty: bool,
    /// Name typed for "save as profile".
    profile_name: String,
    /// Last language written to config (so a menu change marks it dirty once).
    saved_lang: Lang,
}

/// One page cell of the preview: texture, caption, is-processed, content box + dpi + size.
type Cell<'a> = (
    &'a TextureHandle,
    String,
    bool,
    Option<([u32; 4], u32, (u32, u32))>,
);

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let (pv_tx, pv_rx) = mpsc::channel();
        let mut app = Self {
            tick: 0,
            lang: Lang::detect(),
            s: UiSettings::default(),
            pdf_password: String::new(),
            settings_gen: 1,
            sheets: Vec::new(),
            selected: None,
            scroll_to_selected: false,
            palette_open: false,
            palette_q: String::new(),
            palette_sel: 0,
            only_attention: false,
            slider_frac: 0.5,
            next_id: 1,
            pv_tx,
            pv_rx,
            view: View::Both,
            fit: true,
            zoom: 1.0,
            job: None,
            status: None,
            langs_rx: None,
            installed: Vec::new(),
            has_osd: false,
            settings_undo: Vec::new(),
            settings_redo: Vec::new(),
            settings_edit: None,
            preview_started: None,
            ocr_info: String::new(),
            show_about: false,
            modules: ModulesWindow::default(),
            proof: crate::proof::ProofWindow::default(),
            allopts: Default::default(),
            draw_zones: false,
            draw_guide: false,
            guide_path: Vec::new(),
            guide_live: None,
            draw_pictures: false,
            draw_nopic: false,
            draw_extra: 0,
            edit_box: false,
            box_drag: None,
            zone_drag: None,
            zone_resize: None,
            auto_export: std::env::var_os("LEGATOROCR_GUI_AUTOEXPORT_PDF")
                .map(|p| (PathBuf::from(p), false)),
            adjust_test: std::env::var_os("LEGATOROCR_GUI_TEST_ADJUST").map(|_| false),
            config_dirty: false,
            profile_name: String::new(),
            saved_lang: Lang::En,
            shot: std::env::var_os("LEGATOROCR_GUI_SCREENSHOT").map(|p| Shot {
                path: p.into(),
                ready_frames: 0,
                requested: false,
                saved: Arc::new(AtomicBool::new(false)),
            }),
        };
        app.load_config();
        app.saved_lang = app.lang;
        if let Ok(v) = std::env::var("LEGATOROCR_GUI_LANG") {
            app.lang = Lang::from_code(&v).unwrap_or(Lang::En);
        }
        if let Ok(v) = std::env::var("LEGATOROCR_GUI_MODE") {
            app.s.mode = match v.as_str() {
                "bw" => ModeChoice::Bw,
                "gray" => ModeChoice::Gray,
                "mixed" => ModeChoice::Mixed,
                "hybrid" => ModeChoice::Hybrid,
                _ => ModeChoice::Color,
            };
        }
        app.add_files(&collect_images(&files));
        app.refresh_langs(&cc.egui_ctx);
        app
    }

    /// Runs a file dialog. A dialog that comes back empty within a moment cannot have been
    /// answered by a person: the system has no dialog provider, so say that instead of doing
    /// nothing (dragging files into the window and the command line still work).
    fn pick<T>(&mut self, open: impl FnOnce() -> Option<T>) -> Option<T> {
        let started = std::time::Instant::now();
        let r = open();
        if r.is_none() && started.elapsed() < std::time::Duration::from_millis(400) {
            let m = self.tr("status.no_dialog").to_string();
            self.say(m, true);
        }
        r
    }

    fn tr(&self, key: &'static str) -> &'static str {
        t(self.lang, key)
    }

    /// Restore the previous session's settings and language (silently ignores a missing or bad file).
    fn load_config(&mut self) {
        let Some(path) = crate::settings::config_path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        if let Ok(p) = legatorocr_engine::project::Project::parse(&text, None) {
            self.s.apply_project(&p);
            self.s.apply_machine(&p);
            self.s.ocr = false; // OCR is slow: always opt in per session
            if let Some(l) = p.get("lang").and_then(Lang::from_code) {
                self.lang = l;
            }
        }
    }

    fn save_config(&mut self) {
        let Some(path) = crate::settings::config_path() else {
            return;
        };
        let mut p = self.s.to_project(Vec::new());
        p.extra.push(("lang".into(), self.lang.code().into()));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = write_atomic(&path, p.to_text(None).as_bytes()); // best effort
        self.config_dirty = false;
    }

    fn project_sheets(&self) -> Vec<(PathBuf, legatorocr_engine::project::SheetAdj)> {
        self.sheets
            .iter()
            .map(|s| {
                let a = &s.adj;
                (
                    std::fs::canonicalize(&s.path).unwrap_or_else(|_| s.path.clone()),
                    legatorocr_engine::project::SheetAdj {
                        rotate_quarters: a.rotate_quarters,
                        split: a.split.map(Into::into),
                        skew_deg: a.skew_deg,
                        content_adjust_mm: a.content_adjust_mm,
                        erase_mm: a.erase_mm.clone(),
                        picture_mm: a.picture_mm.clone(),
                        no_picture_mm: a.no_picture_mm.clone(),
                        mode: a.mode.map(ModeChoice::to_core),
                        dewarp_guides: a.dewarp_guides.clone(),
                        orient_locked: a.orient_locked,
                        work_mm: a.work_mm,
                        split_frac: a.split_frac,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    fn save_project(&mut self) {
        let Some(path) = self.pick(|| {
            rfd::FileDialog::new()
                .add_filter("LegatorOCR", &["lgr"])
                .set_file_name("project.lgr")
                .save_file()
        }) else {
            return;
        };
        let sheets = self.project_sheets();
        let work = std::fs::canonicalize(legatorocr_engine::archive::work_dir()).ok();
        let temp = work
            .as_ref()
            .is_some_and(|w| sheets.iter().any(|(p, _)| p.starts_with(w)));
        let r = self.s.to_project(sheets).save(&path);
        match r {
            Ok(()) if temp => {
                let m = self.tr("status.project_temp").to_string();
                self.say(format!("{m}: {}", path.display()), true)
            }
            Ok(()) => self.say(
                format!("{}: {}", self.tr("status.project_saved"), path.display()),
                false,
            ),
            Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
        }
    }

    /// Save the project into a folder of the user's choice together with copies of the scans
    /// (`project.lgr` + `images/`), so the whole folder can be moved or opened elsewhere.
    fn save_project_folder(&mut self) {
        let Some(folder) = self.pick(|| rfd::FileDialog::new().pick_folder()) else {
            return;
        };
        let sheets = self.project_sheets();
        match self.s.to_project(sheets).save_to_folder(&folder) {
            Ok((file, n)) => self.say(
                format!(
                    "{}: {} — {n} {}",
                    self.tr("status.project_folder_saved"),
                    file.display(),
                    self.tr("status.files_copied"),
                ),
                false,
            ),
            Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
        }
    }

    fn open_project(&mut self) {
        let Some(path) = self.pick(|| {
            rfd::FileDialog::new()
                .add_filter("LegatorOCR", &["lgr"])
                .pick_file()
        }) else {
            return;
        };
        match legatorocr_engine::project::Project::load(&path) {
            Ok(p) => {
                // The project's resolution decides what its millimetre values mean, and OCR stays
                // opt-in per session: neither may be changed by what happens while loading.
                let ocr = self.s.ocr;
                self.s.apply_project(&p);
                let (dpi, ocr_now) = (self.s.dpi, ocr);
                self.sheets.clear();
                self.selected = None;
                let files: Vec<PathBuf> = p.sheets.iter().map(|(f, _)| f.clone()).collect();
                self.add_files(&files);
                self.s.dpi = dpi;
                self.s.ocr = ocr_now;
                // adjustments follow their file, not the position: files that were dropped or
                // expanded while loading must not shift them onto other pages
                let mut by_path: std::collections::HashMap<&PathBuf, Vec<&_>> =
                    std::collections::HashMap::new();
                for (f, a) in &p.sheets {
                    by_path.entry(f).or_default().push(a);
                }
                let same_len = self.sheets.len() == p.sheets.len();
                let mut used: std::collections::HashMap<PathBuf, usize> =
                    std::collections::HashMap::new();
                for (n, sh) in self.sheets.iter_mut().enumerate() {
                    let k = used.entry(sh.path.clone()).or_insert(0);
                    let a = match by_path.get(&sh.path).and_then(|v| v.get(*k)) {
                        Some(a) => {
                            *k += 1;
                            *a
                        }
                        None if same_len => &p.sheets[n].1,
                        None => continue,
                    };
                    sh.adj = SheetAdjust {
                        rotate_quarters: a.rotate_quarters,
                        split: a.split.map(Into::into),
                        skew_deg: a.skew_deg,
                        content_adjust_mm: a.content_adjust_mm,
                        erase_mm: a.erase_mm.clone(),
                        picture_mm: a.picture_mm.clone(),
                        no_picture_mm: a.no_picture_mm.clone(),
                        mode: a.mode.map(ModeChoice::from_core),
                        dewarp_guides: a.dewarp_guides.clone(),
                        orient_locked: a.orient_locked,
                        work_mm: a.work_mm,
                        split_frac: a.split_frac,
                    };
                }
                self.settings_gen += 1;
                self.config_dirty = true;
                self.say(
                    format!("{}: {}", self.tr("status.project_loaded"), path.display()),
                    false,
                );
            }
            Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
        }
    }

    fn say(&mut self, text: String, error: bool) {
        self.status = Some((text, error));
    }

    fn add_files(&mut self, files: &[PathBuf]) {
        let files = &collect_images(files);
        // First files of a session: adopt the resolution stored in the scan, if any.
        if self.sheets.is_empty() {
            if let Some(d) = files
                .first()
                .and_then(|f| legatorocr_engine::files::detect_dpi(f))
            {
                self.s.dpi = d;
            }
        }
        for p in files {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.sheets.push(Sheet {
                id: self.next_id,
                path: p.clone(),
                name,
                adj: SheetAdjust::default(),
                adj_gen: 0,
                ready: None,
                pending: None,
                used: 0,
                zone_undo: Vec::new(),
                zone_redo: Vec::new(),
                last_edit: None,
                failed: None,
            });
            self.next_id += 1;
        }
        if self.selected.is_none() && !self.sheets.is_empty() {
            self.selected = Some(0);
        }
    }

    /// OCR options for orientation detection, when the user asked for it and it is available.
    fn osd_options(&self) -> Option<legatorocr_engine::ocr::OcrOptions> {
        (self.s.use_osd && self.has_osd).then(|| self.s.ocr_options())
    }

    fn key_of(&self, sheet: &Sheet) -> u64 {
        ((self.settings_gen as u64) << 32) | sheet.adj_gen as u64
    }

    fn refresh_langs(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let opts = self.s.ocr_options();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = legatorocr_engine::ocr::check_engine(&opts)
                .and_then(|v| legatorocr_engine::ocr::list_languages(&opts).map(|l| (v, l)))
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.langs_rx = Some(rx);
    }

    // ----------------------------------------------------------------- background plumbing
    /// Keep at most `KEEP` rendered previews (each is up to ~40 MB of textures).
    fn evict_old_previews(&mut self) {
        const KEEP: usize = 4;
        self.tick += 1;
        if let Some(sh) = self.selected.and_then(|i| self.sheets.get_mut(i)) {
            sh.used = self.tick;
        }
        let mut held: Vec<usize> = (0..self.sheets.len())
            .filter(|&i| self.sheets[i].ready.is_some())
            .collect();
        if held.len() > KEEP {
            held.sort_by_key(|&i| std::cmp::Reverse(self.sheets[i].used));
            for &i in &held[KEEP..] {
                self.sheets[i].ready = None;
            }
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        self.evict_old_previews();
        // dropped files
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            let files = collect_images(&dropped);
            let n = files.len();
            self.add_files(&files);
            let msg = format!("{} {}", n, self.tr("status.added"));
            self.say(msg, false);
        }
        // OCR engine info
        if let Some(rx) = &self.langs_rx {
            if let Ok(r) = rx.try_recv() {
                match r {
                    Ok((v, langs)) => {
                        self.ocr_info = v;
                        if !langs.iter().any(|l| self.s.ocr_langs.contains(l)) {
                            self.s.ocr_langs =
                                langs.iter().filter(|l| *l == "eng").cloned().collect();
                        }
                        self.has_osd = langs.iter().any(|l| l == "osd")
                            || legatorocr_engine::ocr::native_available();
                        self.installed = langs.into_iter().filter(|l| l != "osd").collect();
                    }
                    Err(e) => {
                        self.ocr_info = String::new();
                        self.installed.clear();
                        self.has_osd = false;
                        let _ = e;
                    }
                }
                self.langs_rx = None;
            }
        }
        // preview results
        while let Ok(res) = self.pv_rx.try_recv() {
            let cur = self.sheets.iter().position(|s| s.id == res.id);
            let Some(i) = cur else { continue };
            let key_now = self.key_of(&self.sheets[i]);
            let sheet = &mut self.sheets[i];
            if sheet.pending == Some(res.key) {
                sheet.pending = None;
            }
            if res.key != key_now {
                continue; // stale
            }
            match res.outcome {
                Ok(pv) => {
                    let up = |name: String, th: worker::Thumb| {
                        let img = egui::ColorImage::from_rgba_unmultiplied([th.w, th.h], &th.rgba);
                        ctx.load_texture(name, img, TextureOptions::LINEAR)
                    };
                    let original = up(format!("o{}-{}", res.id, res.key), pv.original);
                    let pages = pv
                        .pages
                        .into_iter()
                        .enumerate()
                        .map(|(n, (th, info))| PageTex {
                            tex: up(format!("p{}-{}-{n}", res.id, res.key), th),
                            w: info.w,
                            h: info.h,
                            skew_deg: info.skew_deg,
                            blank: info.blank,
                            content_box: info.content_box,
                            dpi: info.dpi,
                        })
                        .collect();
                    sheet.ready = Some(Ready {
                        key: res.key,
                        original,
                        original_size: pv.original_size,
                        pages,
                    });
                    sheet.failed = None;
                }
                Err(e) => sheet.failed = Some((res.key, e)),
            }
        }
        // export job
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(r) => {
                    self.job = None;
                    match r {
                        Ok(d) => {
                            let key = if d.docs > 0 {
                                "status.split_saved"
                            } else if d.kind == JobKind::Pdf {
                                "status.pdf_saved"
                            } else {
                                "status.images_saved"
                            };
                            let mut msg = format!(
                                "{}: {} — {} {}",
                                self.tr(key),
                                d.target.display(),
                                d.pages,
                                self.tr("status.pages")
                            );
                            if d.kind == JobKind::Pdf && self.s.ocr {
                                msg += &format!(", {} {}", d.words, self.tr("status.words"));
                            }
                            if d.skipped > 0 {
                                msg +=
                                    &format!(", {} {}", d.skipped, self.tr("status.blank_skipped"));
                            }
                            if let Some((name, why)) = d.failed.first() {
                                msg += &format!(
                                    " — {} {}: {name}: {why}",
                                    d.failed.len(),
                                    self.tr("status.failed")
                                );
                            }
                            self.say(msg, !d.failed.is_empty());
                        }
                        Err(e) if e == "cancelled" || e.ends_with(": cancelled") => {
                            let m = self.tr("status.cancelled").to_string();
                            self.say(m, false)
                        }
                        Err(e) => {
                            let m = format!("{}: {e}", self.tr("status.error"));
                            self.say(m, true)
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(150))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    let m = format!("{}: {}", self.tr("status.error"), self.tr("status.crashed"));
                    self.say(m, true);
                }
            }
        }
        // auto-export hook (automated GUI test): start once the OCR probe has finished
        if self.langs_rx.is_none() && !self.sheets.is_empty() && self.job.is_none() {
            if let Some((target, started)) = self.auto_export.clone() {
                if !started {
                    self.auto_export = Some((target.clone(), true));
                    self.s.ocr = std::env::var_os("LEGATOROCR_GUI_OCR").is_some()
                        && !self.installed.is_empty();
                    self.launch_export(ctx, JobKind::Pdf, target);
                }
            }
        }
        if self.adjust_test == Some(false) {
            if let Some(sh) = self
                .selected
                .and_then(|i| self.sheets.get_mut(i))
                .filter(|s| s.ready.is_some())
            {
                sh.adj = SheetAdjust {
                    rotate_quarters: 1,
                    split: Some(SplitChoice::Off),
                    skew_deg: Some(1.25),
                    ..Default::default()
                };
                sh.adj_gen += 1;
                self.adjust_test = Some(true);
            }
        }
        let export_settled = match &self.auto_export {
            Some((_, started)) => *started && self.job.is_none(),
            None => true,
        };
        // screenshot hook (automated GUI test)
        if let Some(shot) = &mut self.shot {
            let ready = self
                .selected
                .and_then(|i| self.sheets.get(i))
                .map(|s| {
                    s.ready.as_ref().map(|r| r.key)
                        == Some(((self.settings_gen as u64) << 32) | s.adj_gen as u64)
                })
                .unwrap_or(false)
                && self.adjust_test != Some(false);
            if shot.saved.load(Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if (ready || self.sheets.is_empty()) && export_settled {
                shot.ready_frames += 1;
                ctx.request_repaint();
                if shot.ready_frames >= 12 && !shot.requested {
                    shot.requested = true;
                    let (path, saved) = (shot.path.clone(), shot.saved.clone());
                    ctx.request_screenshot(move |img| {
                        let [w, h] = img.size;
                        let mut buf = Vec::with_capacity(w * h * 4);
                        for p in &img.pixels {
                            buf.extend_from_slice(&p.to_array());
                        }
                        if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                            let _ = rgba.save(&path);
                        }
                        saved.store(true, Ordering::Relaxed);
                    });
                }
            }
        }
    }

    /// A preview still running after two minutes is given up: its thread cannot be stopped, but
    /// a late answer is ignored (`pending` no longer matches), so one hung scan cannot freeze
    /// every later preview.
    fn preview_watchdog(&mut self) {
        if self
            .preview_started
            .is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(120))
        {
            for s in self.sheets.iter_mut() {
                if let Some(k) = s.pending.take() {
                    s.failed = Some((k, "preview timed out".to_string()));
                }
            }
            self.preview_started = None;
        }
    }

    fn request_preview(&mut self, ctx: &egui::Context) {
        self.preview_watchdog();
        let Some(i) = self.selected else { return };
        let Some(sheet) = self.sheets.get(i) else {
            return;
        };
        let key = self.key_of(sheet);
        if sheet.ready.as_ref().map(|r| r.key) == Some(key)
            || sheet.pending.is_some()
            || sheet.failed.as_ref().map(|f| f.0) == Some(key)
        {
            return;
        }
        // One preview at a time: scrolling quickly through big scans must not
        // decode dozens of full-size images concurrently.
        if self.sheets.iter().any(|s| s.pending.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let req = PreviewRequest {
            id: sheet.id,
            key,
            path: sheet.path.clone(),
            settings: self.s.to_core(&sheet.adj),
            osd: self.osd_options(),
        };
        self.sheets[i].pending = Some(key);
        self.preview_started = Some(std::time::Instant::now());
        let (tx, ctx) = (self.pv_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(|| worker::render(&req)).unwrap_or_else(|_| {
                Err(format!(
                    "{}: internal error while processing",
                    req.path.display()
                ))
            });
            let _ = tx.send(PreviewResult {
                id: req.id,
                key: req.key,
                outcome,
            });
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------------------------- export
    fn sheet_inputs(&self) -> Vec<SheetInput> {
        self.sheets
            .iter()
            .map(|s| SheetInput {
                path: s.path.clone(),
                settings: self.s.to_core(&s.adj),
            })
            .collect()
    }

    fn start_export(&mut self, ctx: &egui::Context, kind: JobKind) {
        if self.job.is_some() || self.sheets.is_empty() {
            return;
        }
        let target = self.pick(|| match kind {
            JobKind::Pdf => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name("LegatorOCR.pdf")
                .save_file(),
            JobKind::Images => rfd::FileDialog::new().pick_folder(),
        });
        let Some(target) = target else { return };
        self.launch_export(ctx, kind, target);
    }

    fn launch_export(&mut self, ctx: &egui::Context, kind: JobKind, target: PathBuf) {
        if self.job.is_some() || self.sheets.is_empty() {
            return;
        }
        // never replace a page that is being read: the export target may not be one of the inputs
        {
            let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            let t = canon(&target);
            let hit = self.sheets.iter().any(|s| {
                let src = s.path.to_string_lossy();
                let src = src.split('#').next().unwrap_or("");
                canon(Path::new(src)) == t
            });
            if hit {
                let m = self.tr("status.export_into_input").to_string();
                self.say(m, true);
                return;
            }
        }
        let sheets = self.sheet_inputs();
        let names: Vec<String> = self
            .sheets
            .iter()
            .map(|s| {
                Path::new(&s.name)
                    .file_stem()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        let pdf = (kind == JobKind::Pdf).then(|| PdfExport {
            quality: self.s.pdf_quality,
            ocr: self.s.ocr.then(|| self.s.ocr_options()),
            title: target.file_stem().map(|s| s.to_string_lossy().into_owned()),
            meta: legatorocr_engine::PdfMeta {
                created_unix: Some(legatorocr_engine::now_unix()),
                author: Some(self.s.pdf_author.clone()).filter(|a| !a.trim().is_empty()),
                ..Default::default()
            },
            bookmarks: self.s.bookmarks,
            front_matter: self.s.front_matter,
            pdfa: self.s.pdfa,
        });
        let (side_txt, side_hocr) = (
            self.s.sidecar_txt && self.s.ocr,
            self.s.sidecar_hocr && self.s.ocr,
        );
        let skip_blank = self.s.skip_blank;
        let (skip_duplicates, split_on_blank, target_mb) = (
            self.s.skip_duplicates,
            self.s.split_on_blank && kind == JobKind::Pdf,
            self.s.target_mb,
        );
        let split_on_barcode = self.s.split_on_barcode && kind == JobKind::Pdf;
        let uniform_size = self.s.uniform_size;
        let osd = self.osd_options();
        let (cancel, done) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let (tx, rx) = mpsc::channel();
        let total = sheets.len();
        let (c2, d2, ctx2, target2) = (cancel.clone(), done.clone(), ctx.clone(), target.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<Done, String> {
                    if kind == JobKind::Pdf {
                        legatorocr_engine::check_writable(&target2)
                            .map_err(|e| format!("{e:#}"))?;
                    } else {
                        std::fs::create_dir_all(&target2)
                            .map_err(|e| format!("{}: {e}", target2.display()))?;
                    }
                    if let Some(o) = pdf.as_ref().and_then(|p| p.ocr.as_ref()) {
                        check_ocr(o).map_err(|e| e.to_string())?;
                    }
                    let first_err: Mutex<Option<String>> = Mutex::new(None);
                    let mut pdf = pdf;
                    if let (Some(p), true) = (pdf.as_mut(), target_mb > 0) {
                        if p.quality > 0 {
                            p.quality = legatorocr_engine::quality_for_target(
                                &sheets,
                                osd.as_ref(),
                                target_mb as u64 * 1_000_000,
                                &c2,
                            )
                            .map_err(|e| format!("{e:#}"))?;
                        }
                    }
                    let job = ExportJob {
                        sheets: &sheets,
                        pdf,
                        skip_blank,
                        keep_going: true,
                        osd,
                        uniform_size,
                        text_ocr: None,
                        skip_duplicates,
                        split_on_blank,
                        split_on_barcode,
                    };
                    let res = run_export(
                        &job,
                        &c2,
                        &|i, pg| {
                            if kind == JobKind::Images {
                                let suffix = match pg.split_index {
                                    Some(0) => "_L",
                                    Some(_) => "_R",
                                    None => "",
                                };
                                let path = target2.join(format!(
                                    "{:04}_{}{}.png",
                                    i + 1,
                                    names[i],
                                    suffix
                                ));
                                if let Err(e) = legatorocr_engine::save_page_image(pg, &path) {
                                    first_err
                                        .lock()
                                        .unwrap()
                                        .get_or_insert(format!("{}: {e}", path.display()));
                                }
                            }
                        },
                        &|d, _| {
                            d2.store(d, Ordering::Relaxed);
                            ctx2.request_repaint();
                        },
                    )
                    .map_err(|e| format!("{e:#}"))?;
                    if let Some(e) = first_err.into_inner().unwrap() {
                        return Err(e);
                    }
                    let stem = target2
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    for (i, bytes) in res.docs.iter().enumerate() {
                        write_atomic(
                            &target2.with_file_name(legatorocr_engine::doc_file_name(
                                &stem,
                                i,
                                res.doc_names.get(i).and_then(|n| n.as_deref()),
                            )),
                            bytes,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    if let Some(bytes) = &res.pdf {
                        write_atomic(&target2, bytes).map_err(|e| e.to_string())?;
                    }
                    // the text sidecars belong to the export whether or not it was split
                    if kind == JobKind::Pdf {
                        if let (true, Some(t)) = (side_txt, &res.text) {
                            write_atomic(&target2.with_extension("txt"), t.as_bytes())
                                .map_err(|e| e.to_string())?;
                        }
                        if let (true, Some(t)) = (side_hocr, &res.hocr) {
                            write_atomic(&target2.with_extension("hocr"), t.as_bytes())
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    let failed = res
                        .failed
                        .iter()
                        .map(|(p, e)| {
                            (
                                p.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                e.clone(),
                            )
                        })
                        .collect();
                    Ok(Done {
                        kind,
                        target: target2.clone(),
                        pages: res.pages,
                        words: res.words,
                        skipped: res.skipped_blank,
                        docs: res.docs.len(),
                        failed,
                    })
                },
            ))
            .unwrap_or_else(|_| Err(String::from("internal error")));
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.job = Some(Job {
            cancel,
            done,
            total,
            rx,
        });
    }

    // ---------------------------------------------------------------------------------- UI
    fn menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(self.tr("menu.file"), |ui| {
                if ui.button(self.tr("menu.add_images")).clicked() {
                    if let Some(f) = self.pick(|| {
                        rfd::FileDialog::new()
                            .add_filter("images, PDF, archives", &all_input_ext())
                            .pick_files()
                    }) {
                        self.add_files(&f);
                    }
                    ui.close();
                }
                if ui.button(self.tr("menu.add_folder")).clicked() {
                    if let Some(d) = self.pick(|| rfd::FileDialog::new().pick_folder()) {
                        self.add_files(&collect_images(&[d]));
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button(self.tr("menu.open_project")).clicked() {
                    self.open_project();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.sheets.is_empty(),
                        egui::Button::new(self.tr("menu.save_project")),
                    )
                    .clicked()
                {
                    self.save_project();
                    ui.close();
                }
                ui.separator();
                let idle = self.job.is_none() && !self.sheets.is_empty();
                if ui
                    .add_enabled(idle, egui::Button::new(self.tr("menu.export_pdf")))
                    .clicked()
                {
                    self.start_export(&ctx, JobKind::Pdf);
                    ui.close();
                }
                if ui
                    .add_enabled(idle, egui::Button::new(self.tr("menu.export_images")))
                    .clicked()
                {
                    self.start_export(&ctx, JobKind::Images);
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(
                        !self.sheets.is_empty(),
                        egui::Button::new(self.tr("menu.save_project_folder")),
                    )
                    .on_hover_text(self.tr("menu.save_project_folder_tip"))
                    .clicked()
                {
                    self.save_project_folder();
                    ui.close();
                }
                if ui.button(self.tr("menu.modules")).clicked() {
                    self.modules.open();
                    ui.close();
                }
                if ui.button(self.tr("menu.proof")).clicked() {
                    self.proof.open();
                    ui.close();
                }
                ui.separator();
                if ui.button(self.tr("menu.quit")).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(self.tr("menu.language"), |ui| {
                for (l, _, name) in Lang::ALL {
                    if ui.selectable_label(self.lang == l, name).clicked() {
                        self.lang = l;
                        ui.close();
                    }
                }
            });
            if ui.button(self.tr("menu.all_options")).clicked() {
                self.allopts.open();
            }
            ui.menu_button(self.tr("menu.help"), |ui| {
                if ui.button(self.tr("menu.guide_online")).clicked() {
                    crate::alloptions::open_target(crate::alloptions::GUIDE_URL);
                    ui.close();
                }
                if let Some(p) = crate::alloptions::local_guide() {
                    if ui.button(self.tr("menu.guide_local")).clicked() {
                        crate::alloptions::open_target(&p.to_string_lossy());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(self.tr("menu.about")).clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
    }

    /// "All options": every command-line option, grouped and searchable, with the equivalent
    /// command and a Run button (see `alloptions.rs`).
    fn all_options_window(&mut self, ctx: &egui::Context) {
        use crate::alloptions as ao;
        if !self.allopts.show {
            // closed while a command runs: dropping the run stops it (nobody would see its output)
            self.allopts.run = None;
            return;
        }
        // output of a running command
        let mut finished = false;
        if let Some(run) = &mut self.allopts.run {
            while let Ok(l) = run.rx.try_recv() {
                if let Some(rest) = l.strip_prefix('\u{1}') {
                    run.done = true;
                    self.allopts.last = Some(rest.to_string());
                } else {
                    self.allopts.log.push(l);
                    if self.allopts.log.len() > 2000 {
                        self.allopts.log.drain(..500);
                    }
                }
            }
            finished = run.done;
            if !finished {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        if finished {
            self.allopts.run = None;
        }
        let tr_search = self.tr("all.search");
        let tr_only = self.tr("all.only_chosen");
        let tr_chosen = self.tr("all.chosen");
        let tr_reset = self.tr("all.reset");
        let tr_pages = self.tr("all.use_pages");
        let tr_extra = self.tr("all.extra");
        let tr_cmd = self.tr("all.command");
        let tr_copy = self.tr("all.copy");
        let tr_run = self.tr("all.run");
        let tr_stop = self.tr("all.stop");
        let tr_choose = self.tr("all.hint_choose");
        let tr_lines = self.tr("all.hint_lines");
        let cat_names: Vec<&'static str> = ao::Cat::ALL.iter().map(|c| self.tr(c.key())).collect();
        let n_pages = self.sheets.len();
        let mut open = true;
        let mut browse: Option<(String, ao::PathKind)> = None;
        let mut do_run = false;
        let mut do_stop = false;
        let mut do_copy: Option<String> = None;
        let defs = self.allopts.defs.clone();
        let running = self.allopts.run.is_some();
        let mut a = std::mem::take(&mut self.allopts);
        egui::Window::new(self.tr("all.title"))
            .open(&mut open)
            .default_size([860.0, 780.0])
            .min_size([640.0, 560.0])
            .resizable(true)
            .show(ctx, |ui| {
                match &defs {
                    None | Some(Err(_)) => {
                        ui.label(self.tr("all.no_cli"));
                        if let Some(Err(e)) = &defs {
                            ui.label(RichText::new(e).weak());
                        }
                        return;
                    }
                    Some(Ok(_)) => {}
                }
                let Some(Ok(list)) = &defs else { return };
                ui.label(RichText::new(self.tr("all.intro")).weak());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut a.filter)
                            .hint_text(tr_search)
                            .desired_width(300.0),
                    );
                    ui.checkbox(&mut a.only_chosen, tr_only);
                    ui.label(format!("{} {}", a.chosen(), tr_chosen));
                    if ui.button(tr_reset).clicked() {
                        a.reset();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("allopts_list")
                    .max_height((ui.available_height() - 300.0).max(180.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (ci, cat) in ao::Cat::ALL.iter().enumerate() {
                            let items: Vec<&ao::OptDef> = list
                                .iter()
                                .filter(|d| d.cat == *cat && ao::AllOptions::matches(d, &a.filter))
                                .filter(|d| {
                                    !a.only_chosen
                                        || a.vals
                                            .get(&d.long)
                                            .is_some_and(|v| v.on || !v.text.trim().is_empty())
                                })
                                .collect();
                            if items.is_empty() {
                                continue;
                            }
                            let mut h = egui::CollapsingHeader::new(
                                RichText::new(format!("{} ({})", cat_names[ci], items.len()))
                                    .strong(),
                            )
                            .id_salt(("allcat", ci));
                            if !a.filter.trim().is_empty() || a.only_chosen {
                                h = h.open(Some(true));
                            }
                            h.show(ui, |ui| {
                                for d in items {
                                    let v = a.vals.entry(d.long.clone()).or_default();
                                    ui.horizontal_wrapped(|ui| match d.kind {
                                        ao::Kind::Flag => {
                                            ui.checkbox(
                                                &mut v.on,
                                                RichText::new(format!("--{}", d.long)).monospace(),
                                            );
                                        }
                                        _ => {
                                            ui.label(
                                                RichText::new(format!("--{}", d.long)).monospace(),
                                            );
                                            if !d.values.is_empty() {
                                                let shown = if v.text.is_empty() {
                                                    "—".to_string()
                                                } else {
                                                    v.text.clone()
                                                };
                                                egui::ComboBox::from_id_salt(("allval", &d.long))
                                                    .selected_text(shown)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut v.text,
                                                            String::new(),
                                                            "—",
                                                        );
                                                        for x in &d.values {
                                                            ui.selectable_value(
                                                                &mut v.text,
                                                                x.clone(),
                                                                x,
                                                            );
                                                        }
                                                    });
                                            } else if d.kind == ao::Kind::Repeat {
                                                ui.add(
                                                    egui::TextEdit::multiline(&mut v.text)
                                                        .desired_rows(1)
                                                        .desired_width(280.0)
                                                        .hint_text(format!(
                                                            "{} ({})",
                                                            d.vname, tr_lines
                                                        )),
                                                );
                                            } else {
                                                let hint = if d.default.is_empty() {
                                                    d.vname.clone()
                                                } else {
                                                    format!("{} = {}", d.vname, d.default)
                                                };
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut v.text)
                                                        .desired_width(280.0)
                                                        .hint_text(hint),
                                                );
                                            }
                                            if let Some(pk) = d.path_kind() {
                                                if ui.button(tr_choose).clicked() {
                                                    browse = Some((d.long.clone(), pk));
                                                }
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(&d.help);
                                    ui.label(RichText::new(d.short_help()).small().weak());
                                    ui.add_space(2.0);
                                }
                            });
                        }
                    });
                ui.separator();
                ui.checkbox(&mut a.use_pages, format!("{tr_pages} ({n_pages})"));
                ui.horizontal(|ui| {
                    ui.label(tr_extra);
                    ui.add(
                        egui::TextEdit::singleline(&mut a.extra)
                            .desired_width(f32::INFINITY)
                            .hint_text("in1.pdf \"my file.png\""),
                    );
                });
                ui.label(RichText::new(tr_cmd).strong());
                let proj = (a.use_pages && n_pages > 0).then_some("project.lgr");
                let mut cmd = a.command_line(proj);
                ui.add(
                    egui::TextEdit::multiline(&mut cmd)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr_copy).clicked() {
                        do_copy = Some(cmd.clone());
                    }
                    if ui
                        .add_enabled(!running, egui::Button::new(tr_run))
                        .clicked()
                    {
                        do_run = true;
                    }
                    if ui
                        .add_enabled(running, egui::Button::new(tr_stop))
                        .clicked()
                    {
                        do_stop = true;
                    }
                    if running {
                        ui.spinner();
                        ui.label(self.tr("all.running"));
                    } else if let Some(l) = &a.last {
                        if l == "ok" {
                            ui.label(self.tr("all.done_ok"));
                        } else {
                            ui.label(
                                RichText::new(format!("{}: {l}", self.tr("all.done_fail")))
                                    .color(Color32::from_rgb(200, 60, 60)),
                            );
                        }
                    }
                });
                if !a.log.is_empty() {
                    egui::ScrollArea::vertical()
                        .id_salt("allopts_log")
                        .max_height(150.0)
                        .stick_to_bottom(true)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for l in &a.log {
                                ui.label(RichText::new(l).monospace().small());
                            }
                        });
                }
            });
        self.allopts = a;
        if let Some(t) = do_copy {
            ctx.copy_text(t);
        }
        if let Some((long, pk)) = browse {
            let picked = self.pick(|| match pk {
                ao::PathKind::Dir => rfd::FileDialog::new().pick_folder(),
                ao::PathKind::Open => rfd::FileDialog::new().pick_file(),
                ao::PathKind::Save => rfd::FileDialog::new().save_file(),
            });
            if let Some(p) = picked {
                let v = self.allopts.vals.entry(long).or_default();
                let s = p.to_string_lossy().to_string();
                if v.text.trim().is_empty() || !v.text.contains('\n') {
                    v.text = s;
                } else {
                    v.text.push('\n');
                    v.text.push_str(&s);
                }
            }
        }
        if do_stop {
            if let Some(r) = &self.allopts.run {
                r.stop();
            }
        }
        if do_run {
            let mut args: Vec<String> = Vec::new();
            if self.allopts.use_pages && !self.sheets.is_empty() {
                let path =
                    std::env::temp_dir().join(format!("legatorocr-gui-{}.lgr", std::process::id()));
                let sheets = self.project_sheets();
                match self.s.to_project(sheets).save(&path) {
                    Ok(()) => {
                        args.push("--project".into());
                        args.push(path.to_string_lossy().to_string());
                    }
                    Err(e) => {
                        let m = format!("{}: {e:#}", self.tr("status.error"));
                        self.say(m, true);
                        return;
                    }
                }
            }
            args.extend(self.allopts.args());
            self.allopts.log.clear();
            self.allopts.last = None;
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from);
            self.allopts.log.push(format!(
                "$ legatorocr {}",
                args.iter()
                    .map(|s| ao::quote(s))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
            match ao::spawn(&args, home) {
                Ok(r) => self.allopts.run = Some(r),
                Err(e) => self.allopts.log.push(e),
            }
        }
        if !open {
            self.allopts.show = false;
        }
    }

    /// "Optional modules" window: what is missing, the commands that install it, and a button
    /// that runs them when no administrator rights are needed.
    fn modules_window(&mut self, ctx: &egui::Context) {
        use legatorocr_engine::install as inst;
        if !self.modules.show {
            return;
        }
        if let Some(rx) = &self.modules.probe_rx {
            if let Ok(info) = rx.try_recv() {
                self.modules.info = Some(info);
                self.modules.probe_rx = None;
            }
        }
        if let Some(rx) = &self.modules.run_rx {
            if let Ok(out) = rx.try_recv() {
                let keep = out;
                self.modules.run_rx = None;
                self.modules.open(); // look again at what is installed now
                self.modules.outcomes = keep;
            }
        }
        if self.modules.probe_rx.is_some() || self.modules.run_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let mut open = true;
        let mut start_run = false;
        let mut copy: Option<String> = None;
        let sudo = inst::needs_sudo();
        let running = self.modules.run_rx.is_some();
        egui::Window::new(self.tr("modules.title"))
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(self.tr("modules.intro"));
                ui.separator();
                match &self.modules.info {
                    None => {
                        ui.label(self.tr(if running {
                            "modules.running"
                        } else {
                            "modules.checking"
                        }));
                    }
                    Some(info) if info.missing.is_empty() => {
                        ui.label(self.tr("modules.none_missing"));
                    }
                    Some(info) => {
                        let names: Vec<&str> = info.missing.iter().map(|m| m.name()).collect();
                        ui.label(format!(
                            "{} {}",
                            self.tr("modules.missing"),
                            names.join(", ")
                        ));
                        if info.manager.is_none() {
                            ui.label(self.tr("modules.no_manager"));
                        } else {
                            ui.label(self.tr("modules.commands"));
                            let text: Vec<String> =
                                info.steps.iter().map(|s| s.display(sudo)).collect();
                            let mut shown = text.join("\n");
                            ui.add(
                                egui::TextEdit::multiline(&mut shown)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                            let needs_terminal = info.steps.iter().any(|s| s.root) && sudo;
                            if needs_terminal {
                                ui.label(self.tr("modules.needs_terminal"));
                            }
                            ui.horizontal(|ui| {
                                if ui.button(self.tr("modules.copy")).clicked() {
                                    copy = Some(text.join("\n"));
                                }
                                if ui
                                    .add_enabled(
                                        !needs_terminal && !running,
                                        egui::Button::new(self.tr("modules.run")),
                                    )
                                    .clicked()
                                {
                                    start_run = true;
                                }
                            });
                        }
                    }
                }
                if running {
                    ui.label(self.tr("modules.running"));
                }
                if !self.modules.outcomes.is_empty() {
                    ui.separator();
                    if self.modules.outcomes.iter().all(|o| o.ok) {
                        ui.label(self.tr("modules.done"));
                    } else {
                        ui.label(self.tr("modules.failed"));
                        for o in self.modules.outcomes.iter().filter(|o| !o.ok) {
                            ui.label(
                                RichText::new(format!("{} — {}", o.command, o.detail))
                                    .monospace()
                                    .small(),
                            );
                        }
                    }
                }
            });
        if let Some(t) = copy {
            ctx.copy_text(t);
        }
        if start_run {
            if let Some(info) = &self.modules.info {
                let steps = info.steps.clone();
                let (tx, rx) = mpsc::channel();
                self.modules.run_rx = Some(rx);
                self.modules.outcomes.clear();
                std::thread::spawn(move || {
                    let _ = tx.send(inst::run_captured(&steps));
                });
            }
        }
        if !open {
            self.modules.show = false;
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(job) = &self.job {
                let d = job.done.load(Ordering::Relaxed);
                ui.add(
                    egui::ProgressBar::new(d as f32 / job.total.max(1) as f32)
                        .desired_width(260.0)
                        .text(format!("{d}/{}", job.total)),
                );
                if ui.button(self.tr("btn.cancel")).clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                ui.label(self.tr("status.working"));
            } else if let Some((m, err)) = &self.status {
                let c = if *err {
                    Color32::from_rgb(220, 70, 60)
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(m).color(c));
            } else {
                ui.label(self.tr("status.hint"));
            }
        });
    }

    fn page_list(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.tr("list.title"));
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.tr("btn.add")).clicked() {
                if let Some(f) = self.pick(|| {
                    rfd::FileDialog::new()
                        .add_filter("images, PDF, archives", &all_input_ext())
                        .pick_files()
                }) {
                    self.add_files(&f);
                }
            }
            if ui
                .add_enabled(
                    self.selected.is_some(),
                    egui::Button::new(self.tr("btn.remove")),
                )
                .clicked()
            {
                if let Some(i) = self.selected {
                    self.sheets.remove(i);
                    self.selected = if self.sheets.is_empty() {
                        None
                    } else {
                        Some(i.min(self.sheets.len() - 1))
                    };
                }
            }
            if ui
                .add_enabled(
                    !self.sheets.is_empty(),
                    egui::Button::new(self.tr("btn.clear")),
                )
                .clicked()
            {
                self.sheets.clear();
                self.selected = None;
            }
        });
        ui.horizontal(|ui| {
            let can_up = matches!(self.selected, Some(i) if i > 0);
            let can_down = matches!(self.selected, Some(i) if i + 1 < self.sheets.len());
            if ui
                .add_enabled(can_up, egui::Button::new(self.tr("btn.up")))
                .clicked()
            {
                let i = self.selected.unwrap();
                self.sheets.swap(i, i - 1);
                self.selected = Some(i - 1);
            }
            if ui
                .add_enabled(can_down, egui::Button::new(self.tr("btn.down")))
                .clicked()
            {
                let i = self.selected.unwrap();
                self.sheets.swap(i, i + 1);
                self.selected = Some(i + 1);
            }
        });
        ui.separator();
        let max_skew = self.s.max_skew;
        let flagged = self.flagged_sheets();
        if !flagged.is_empty() {
            ui.horizontal(|ui| {
                ui.colored_label(
                    Color32::from_rgb(235, 140, 30),
                    format!("{}: {}", self.tr("attn.count"), flagged.len()),
                );
                if ui.small_button(self.tr("attn.next")).clicked() {
                    let cur = self.selected.unwrap_or(usize::MAX);
                    let nxt = flagged
                        .iter()
                        .copied()
                        .find(|&i| cur == usize::MAX || i > cur)
                        .unwrap_or(flagged[0]);
                    self.selected = Some(nxt);
                }
                let lbl = self.tr("list.only_attention");
                ui.checkbox(&mut self.only_attention, lbl);
            });
        }
        if self.sheets.is_empty() {
            ui.label(self.tr("list.empty"));
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..self.sheets.len() {
                    if self.only_attention && !flagged.contains(&i) && self.selected != Some(i) {
                        continue;
                    }
                    let key = self.key_of(&self.sheets[i]);
                    let s = &self.sheets[i];
                    let warn = s
                        .ready
                        .as_ref()
                        .filter(|r| r.key == key)
                        .and_then(|r| attention(r, max_skew, s.adj.skew_deg.is_some()));
                    let color = if s.failed.as_ref().map(|f| f.0) == Some(key) {
                        Color32::from_rgb(220, 70, 60)
                    } else if warn.is_some() {
                        Color32::from_rgb(235, 140, 30)
                    } else if s.ready.as_ref().map(|r| r.key) == Some(key) {
                        Color32::from_rgb(70, 170, 90)
                    } else if s.pending.is_some() {
                        Color32::from_rgb(230, 180, 50)
                    } else {
                        Color32::GRAY
                    };
                    let edited = if s.adj != SheetAdjust::default() {
                        " ✎"
                    } else {
                        ""
                    };
                    let label = format!("{:>3}  {}{}", i + 1, s.name, edited);
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                        let mut resp = ui.selectable_label(self.selected == Some(i), label);
                        if let Some(w) = warn {
                            resp = resp.on_hover_text(self.tr(w));
                        }
                        if self.scroll_to_selected && self.selected == Some(i) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            self.selected = Some(i);
                            // a failed preview (timeout, transient read error) is tried again
                            // when the page is selected again
                            self.sheets[i].failed = None;
                        }
                    });
                }
            });
    }

    /// Indices of the sheets whose processed preview looks suspicious (see [`attention`]).
    fn flagged_sheets(&self) -> Vec<usize> {
        let max_skew = self.s.max_skew;
        (0..self.sheets.len())
            .filter(|&i| {
                let key = self.key_of(&self.sheets[i]);
                self.sheets[i].ready.as_ref().is_some_and(|r| {
                    r.key == key
                        && attention(r, max_skew, self.sheets[i].adj.skew_deg.is_some()).is_some()
                })
            })
            .collect()
    }

    /// Go to the next flagged sheet after the selected one (wrapping round).
    fn next_attention(&mut self) {
        let flagged = self.flagged_sheets();
        if flagged.is_empty() {
            return;
        }
        let cur = self.selected.unwrap_or(usize::MAX);
        let nxt = flagged
            .iter()
            .copied()
            .find(|&i| cur == usize::MAX || i > cur)
            .unwrap_or(flagged[0]);
        self.selected = Some(nxt);
        self.sheets[nxt].failed = None;
        self.scroll_to_selected = true;
    }

    fn run_cmd(&mut self, ctx: &egui::Context, c: Cmd) {
        let idle = self.job.is_none() && !self.sheets.is_empty();
        match c {
            Cmd::AddImages => {
                if let Some(f) = self.pick(|| {
                    rfd::FileDialog::new()
                        .add_filter("images, PDF, archives", &all_input_ext())
                        .pick_files()
                }) {
                    self.add_files(&f);
                }
            }
            Cmd::AddFolder => {
                if let Some(d) = self.pick(|| rfd::FileDialog::new().pick_folder()) {
                    self.add_files(&collect_images(&[d]));
                }
            }
            Cmd::OpenProject => self.open_project(),
            Cmd::SaveProject if !self.sheets.is_empty() => self.save_project(),
            Cmd::SaveProjectFolder if !self.sheets.is_empty() => self.save_project_folder(),
            Cmd::ExportPdf if idle => self.start_export(ctx, JobKind::Pdf),
            Cmd::ExportImages if idle => self.start_export(ctx, JobKind::Images),
            Cmd::Modules => self.modules.open(),
            Cmd::Proof => self.proof.open(),
            Cmd::FirstPage if !self.sheets.is_empty() => {
                self.selected = Some(0);
                self.scroll_to_selected = true;
            }
            Cmd::LastPage if !self.sheets.is_empty() => {
                self.selected = Some(self.sheets.len() - 1);
                self.scroll_to_selected = true;
            }
            Cmd::NextAttention => self.next_attention(),
            Cmd::OnlyAttention => self.only_attention = !self.only_attention,
            Cmd::ViewBoth => self.view = View::Both,
            Cmd::ViewOriginal => self.view = View::Original,
            Cmd::ViewProcessed => self.view = View::Processed,
            Cmd::ViewSlider => self.view = View::Slider,
            _ => {}
        }
    }

    /// Ctrl+K: type a few letters of a command and press Enter.
    fn palette(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.palette_open = !self.palette_open;
            self.palette_q.clear();
            self.palette_sel = 0;
        }
        if !self.palette_open {
            return;
        }
        let q = self.palette_q.to_lowercase();
        let hits: Vec<(Cmd, &'static str)> = CMDS
            .iter()
            .map(|&(c, k)| (c, self.tr(k)))
            .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
            .collect();
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if hits.is_empty() {
            self.palette_sel = 0;
        } else {
            if down {
                self.palette_sel = (self.palette_sel + 1) % hits.len();
            }
            if up {
                self.palette_sel = (self.palette_sel + hits.len() - 1) % hits.len();
            }
            self.palette_sel = self.palette_sel.min(hits.len() - 1);
        }
        let mut chosen: Option<Cmd> = None;
        if enter && !hits.is_empty() {
            chosen = Some(hits[self.palette_sel].0);
        }
        let title = self.tr("palette.title");
        let none = self.tr("palette.none");
        let hint = self.tr("palette.hint");
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_q)
                        .hint_text(hint)
                        .desired_width(360.0),
                );
                r.request_focus();
                if r.changed() {
                    self.palette_sel = 0;
                }
                if hits.is_empty() {
                    ui.label(none);
                }
                for (n, (c, label)) in hits.iter().enumerate() {
                    if ui.selectable_label(n == self.palette_sel, *label).clicked() {
                        chosen = Some(*c);
                    }
                }
            });
        if esc {
            self.palette_open = false;
        }
        if let Some(c) = chosen {
            self.palette_open = false;
            self.run_cmd(ctx, c);
        }
    }

    /// The slider view: the original left of the divider, the processed first page right of it.
    fn slider_view(ui: &mut egui::Ui, ready: &Ready, frac: &mut f32, before: &str, after: &str) {
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(avail.x.max(50.0), (avail.y - 24.0).max(50.0)),
            egui::Sense::click_and_drag(),
        );
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.clicked() {
                *frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            }
        }
        let fit = |w: f32, h: f32| {
            let k = (rect.width() / w).min(rect.height() / h);
            egui::Rect::from_center_size(rect.center(), Vec2::new(w * k, h * k))
        };
        let x = rect.left() + rect.width() * *frac;
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let (ow, oh) = (ready.original_size.0 as f32, ready.original_size.1 as f32);
        let left = egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.max.y));
        ui.painter_at(left)
            .image(ready.original.id(), fit(ow, oh), uv, Color32::WHITE);
        if let Some(p) = ready.pages.first() {
            let right = egui::Rect::from_min_max(egui::pos2(x, rect.min.y), rect.max);
            ui.painter_at(right)
                .image(p.tex.id(), fit(p.w as f32, p.h as f32), uv, Color32::WHITE);
        }
        let pt = ui.painter_at(rect);
        pt.line_segment(
            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
            egui::Stroke::new(2.0, Color32::from_rgb(40, 120, 220)),
        );
        pt.circle_filled(
            egui::pos2(x, rect.center().y),
            9.0,
            Color32::from_rgb(40, 120, 220),
        );
        pt.text(
            egui::pos2(rect.min.x + 6.0, rect.min.y + 4.0),
            egui::Align2::LEFT_TOP,
            before,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(200, 60, 40),
        );
        pt.text(
            egui::pos2(rect.max.x - 6.0, rect.min.y + 4.0),
            egui::Align2::RIGHT_TOP,
            after,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(30, 130, 60),
        );
    }

    /// Keyboard: Up / Down (also PageUp / PageDown) go to the previous / next page, Home / End to
    /// the first / last. Not while a text box has the focus, and not with Ctrl / Alt / Shift held.
    fn page_keys(&mut self, ctx: &egui::Context) {
        self.scroll_to_selected = false;
        if self.sheets.is_empty() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let last = self.sheets.len() - 1;
        let cur = self.selected.unwrap_or(0).min(last);
        let mut want_next = false;
        let target = ctx.input_mut(|inp| {
            if !inp.modifiers.is_none() {
                return None;
            }
            use egui::Key;
            let mut t = None;
            if inp.consume_key(egui::Modifiers::NONE, Key::ArrowDown)
                || inp.consume_key(egui::Modifiers::NONE, Key::PageDown)
            {
                t = Some((cur + 1).min(last));
            } else if inp.consume_key(egui::Modifiers::NONE, Key::ArrowUp)
                || inp.consume_key(egui::Modifiers::NONE, Key::PageUp)
            {
                t = Some(cur.saturating_sub(1));
            } else if inp.consume_key(egui::Modifiers::NONE, Key::Home) {
                t = Some(0);
            } else if inp.consume_key(egui::Modifiers::NONE, Key::End) {
                t = Some(last);
            } else if inp.consume_key(egui::Modifiers::NONE, Key::N) {
                want_next = true;
            }
            t
        });
        if want_next {
            self.next_attention();
            return;
        }
        if let Some(t) = target {
            if self.selected != Some(t) {
                self.selected = Some(t);
                // a failed preview is tried again when the page is selected again
                self.sheets[t].failed = None;
            }
            self.scroll_to_selected = true;
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let tr = |k: &'static str| t(lang, k);
        let mut ch = false;
        let settings_before = self.s.clone();
        let mut swap: Option<bool> = None; // Some(true) = undo, Some(false) = redo
        ui.horizontal(|ui| {
            ui.heading(tr("set.title"));
            if ui
                .add_enabled(
                    !self.settings_undo.is_empty(),
                    egui::Button::new(tr("zones.undo")),
                )
                .on_hover_text(tr("set.undo_tip"))
                .clicked()
            {
                swap = Some(true);
            }
            if ui
                .add_enabled(
                    !self.settings_redo.is_empty(),
                    egui::Button::new(tr("zones.redo")),
                )
                .on_hover_text(tr("set.redo_tip"))
                .clicked()
            {
                swap = Some(false);
            }
        });
        let mut chosen: Option<String> = None;
        let mut to_save: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(tr("set.profile"));
            egui::ComboBox::from_id_salt("profile")
                .selected_text(tr("set.profile_pick"))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (n, d) in legatorocr_engine::profiles::list() {
                        if ui.selectable_label(false, &n).on_hover_text(d).clicked() {
                            chosen = Some(n);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.profile_name)
                    .desired_width(110.0)
                    .hint_text(tr("set.profile_name")),
            );
            if ui
                .add_enabled(
                    legatorocr_engine::profiles::valid_name(&self.profile_name),
                    egui::Button::new(tr("btn.save_profile")),
                )
                .clicked()
            {
                to_save = Some(self.profile_name.clone());
            }
        });
        if let Some(n) = chosen {
            match legatorocr_engine::profiles::load(&n) {
                Ok(p) => {
                    let ocr = self.s.ocr;
                    self.s.apply_project(&p);
                    self.s.ocr = ocr;
                    ch = true;
                    self.say(format!("{}: {n}", self.tr("status.profile_applied")), false);
                }
                Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
            }
        }
        if let Some(n) = to_save {
            let p = self.s.to_project(Vec::new());
            match legatorocr_engine::profiles::save_user(&n, &p) {
                Ok(_) => self.say(format!("{}: {n}", self.tr("status.profile_saved")), false),
                Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
            }
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::CollapsingHeader::new(tr("set.input"))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr("set.dpi"));
                            ch |= ui
                                .add(egui::DragValue::new(&mut self.s.dpi).range(72..=1200))
                                .changed();
                        });
                        ch |= ui
                            .checkbox(&mut self.s.auto_orient, tr("set.auto_orient"))
                            .changed();
                        let osd_ok = self.has_osd;
                        ch |= ui
                            .add_enabled(
                                osd_ok,
                                egui::Checkbox::new(&mut self.s.use_osd, tr("set.osd")),
                            )
                            .on_hover_text(tr(if osd_ok {
                                "set.osd_tip"
                            } else {
                                "set.osd_missing"
                            }))
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.split")).on_hover_text(tr("set.split_tip"));
                            egui::ComboBox::from_id_salt("split")
                                .selected_text(split_name(lang, self.s.split))
                                .show_ui(ui, |ui| {
                                    for c in
                                        [SplitChoice::Auto, SplitChoice::Middle, SplitChoice::Off]
                                    {
                                        ch |= ui
                                            .selectable_value(
                                                &mut self.s.split,
                                                c,
                                                split_name(lang, c),
                                            )
                                            .changed();
                                    }
                                });
                        });
                    });
                egui::CollapsingHeader::new(tr("set.geometry"))
                    .default_open(true)
                    .show(ui, |ui| {
                        ch |= ui.checkbox(&mut self.s.deskew, tr("set.deskew")).changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.max_skew"));
                            ch |= ui
                                .add(
                                    egui::Slider::new(&mut self.s.max_skew, 1.0..=15.0).suffix("°"),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.margin"));
                            ch |= ui
                                .add(
                                    egui::Slider::new(&mut self.s.margin_mm, 0.0..=30.0)
                                        .suffix(" mm"),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.page_size"));
                            egui::ComboBox::from_id_salt("psize")
                                .selected_text(size_name(lang, self.s.page_size))
                                .show_ui(ui, |ui| {
                                    for c in [
                                        PageSizeChoice::Free,
                                        PageSizeChoice::A4,
                                        PageSizeChoice::A5,
                                        PageSizeChoice::Letter,
                                        PageSizeChoice::Custom,
                                    ] {
                                        ch |= ui
                                            .selectable_value(
                                                &mut self.s.page_size,
                                                c,
                                                size_name(lang, c),
                                            )
                                            .changed();
                                    }
                                });
                        });
                        if self.s.page_size != PageSizeChoice::Free {
                            ch |= ui
                                .checkbox(&mut self.s.fit_width, tr("set.fit_width"))
                                .on_hover_text(tr("set.fit_tip"))
                                .changed();
                            ch |= ui
                                .checkbox(&mut self.s.fit_height, tr("set.fit_height"))
                                .on_hover_text(tr("set.fit_tip"))
                                .changed();
                        }
                        if self.s.page_size != PageSizeChoice::Free || self.s.uniform_size {
                            ui.horizontal(|ui| {
                                ui.label(tr("set.align"));
                                use legatorocr_core::{HAlign, VAlign};
                                let hn = |h: HAlign| {
                                    tr(match h {
                                        HAlign::Left => "align.left",
                                        HAlign::Center => "align.center",
                                        HAlign::Right => "align.right",
                                    })
                                };
                                let vn = |v: VAlign| {
                                    tr(match v {
                                        VAlign::Top => "align.top",
                                        VAlign::Center => "align.center",
                                        VAlign::Bottom => "align.bottom",
                                    })
                                };
                                egui::ComboBox::from_id_salt("alignh")
                                    .selected_text(hn(self.s.align.0))
                                    .width(80.0)
                                    .show_ui(ui, |ui| {
                                        for h in [HAlign::Left, HAlign::Center, HAlign::Right] {
                                            ch |= ui
                                                .selectable_value(&mut self.s.align.0, h, hn(h))
                                                .changed();
                                        }
                                    });
                                egui::ComboBox::from_id_salt("alignv")
                                    .selected_text(vn(self.s.align.1))
                                    .width(80.0)
                                    .show_ui(ui, |ui| {
                                        for v in [VAlign::Top, VAlign::Center, VAlign::Bottom] {
                                            ch |= ui
                                                .selectable_value(&mut self.s.align.1, v, vn(v))
                                                .changed();
                                        }
                                    });
                            });
                        }
                        if self.s.page_size == PageSizeChoice::Custom {
                            ui.horizontal(|ui| {
                                ch |= ui
                                    .add(
                                        egui::DragValue::new(&mut self.s.custom_w)
                                            .range(10.0..=1000.0)
                                            .suffix(" mm"),
                                    )
                                    .changed();
                                ui.label("×");
                                ch |= ui
                                    .add(
                                        egui::DragValue::new(&mut self.s.custom_h)
                                            .range(10.0..=1000.0)
                                            .suffix(" mm"),
                                    )
                                    .changed();
                            });
                        }
                    });
                egui::CollapsingHeader::new(tr("set.output"))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (m, k) in [
                                (ModeChoice::Color, "mode.color"),
                                (ModeChoice::Gray, "mode.gray"),
                                (ModeChoice::Bw, "mode.bw"),
                                (ModeChoice::Mixed, "mode.mixed"),
                                (ModeChoice::Hybrid, "mode.hybrid"),
                            ] {
                                ch |= ui.selectable_value(&mut self.s.mode, m, tr(k)).changed();
                            }
                        });
                        if self.s.mode != ModeChoice::Color && self.s.mode != ModeChoice::Gray {
                            ui.horizontal(|ui| {
                                ui.label(tr("set.threshold"));
                                egui::ComboBox::from_id_salt("thr")
                                    .selected_text(self.s.threshold.name())
                                    .show_ui(ui, |ui| {
                                        for m in legatorocr_core::Threshold::ALL {
                                            ch |= ui
                                                .selectable_value(
                                                    &mut self.s.threshold,
                                                    m,
                                                    m.name(),
                                                )
                                                .changed();
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                ui.label(tr("set.sauvola"));
                                ch |= ui
                                    .add(egui::Slider::new(&mut self.s.sauvola_k, 0.05..=0.6))
                                    .changed();
                            });
                            ch |= ui
                                .checkbox(&mut self.s.despeckle, tr("set.despeckle"))
                                .changed();
                        }
                        if matches!(self.s.mode, ModeChoice::Mixed | ModeChoice::Hybrid) {
                            ui.horizontal(|ui| {
                                ui.label(tr("set.picture_sens"));
                                ch |= ui
                                    .add(egui::Slider::new(
                                        &mut self.s.picture_sensitivity,
                                        0.0..=1.0,
                                    ))
                                    .changed();
                            });
                        }
                        if self.s.mode == ModeChoice::Hybrid {
                            ui.horizontal(|ui| {
                                ui.label(tr("set.hybrid_dpi"));
                                ch |= ui
                                    .add(egui::Slider::new(&mut self.s.hybrid_dpi, 36..=300))
                                    .changed();
                            });
                        }
                        if self.s.mode != ModeChoice::Bw {
                            ch |= ui
                                .checkbox(&mut self.s.normalize_bg, tr("set.normalize"))
                                .changed();
                        }
                        ch |= ui.checkbox(&mut self.s.dewarp, tr("set.dewarp")).changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.dewarp_guides"));
                            let bad = legatorocr_core::dewarp::parse_guides(&self.s.dewarp_guides)
                                .is_none();
                            let mut te = egui::TextEdit::singleline(&mut self.s.dewarp_guides)
                                .desired_width(150.0)
                                .hint_text("0,0.5 0.5,0.52 1,0.5;…");
                            if bad {
                                te = te.text_color(egui::Color32::LIGHT_RED);
                            }
                            ch |= ui.add(te).changed();
                        });
                    });
                egui::CollapsingHeader::new(tr("set.cleanup"))
                    .default_open(false)
                    .show(ui, |ui| {
                        ch |= ui
                            .checkbox(&mut self.s.auto_invert, tr("set.invert"))
                            .on_hover_text(tr("set.invert_tip"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.auto_levels, tr("set.levels"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.show_through, tr("set.show_through"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.descreen, tr("set.descreen"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.perspective, tr("set.perspective"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.trim_background, tr("set.trim_background"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.punch_holes, tr("set.punch"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.fingers, tr("set.fingers"))
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.dropout"))
                                .on_hover_text(tr("set.dropout_tip"));
                            let dn = |d: legatorocr_core::Dropout| {
                                tr(match d {
                                    legatorocr_core::Dropout::Off => "dropout.off",
                                    legatorocr_core::Dropout::Red => "dropout.red",
                                    legatorocr_core::Dropout::Green => "dropout.green",
                                    legatorocr_core::Dropout::Blue => "dropout.blue",
                                })
                            };
                            egui::ComboBox::from_id_salt("dropout")
                                .selected_text(dn(self.s.dropout))
                                .show_ui(ui, |ui| {
                                    for d in legatorocr_core::Dropout::ALL {
                                        ch |= ui
                                            .selectable_value(&mut self.s.dropout, d, dn(d))
                                            .changed();
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.erase"));
                            let bad =
                                legatorocr_core::cleanup::parse_zones(&self.s.erase).is_none();
                            let mut te = egui::TextEdit::singleline(&mut self.s.erase)
                                .desired_width(120.0)
                                .hint_text("x,y,w,h;…");
                            if bad {
                                te = te.text_color(egui::Color32::LIGHT_RED);
                            }
                            ch |= ui.add(te).changed();
                        });
                        if matches!(self.s.mode, ModeChoice::Mixed | ModeChoice::Hybrid) {
                            ch |= ui
                                .checkbox(&mut self.s.auto_pictures, tr("set.auto_pictures"))
                                .on_hover_text(tr("set.auto_pictures_hint"))
                                .changed();
                            ui.horizontal(|ui| {
                                ui.label(tr("set.no_picture_zones"));
                                let bad =
                                    legatorocr_core::cleanup::parse_zones(&self.s.no_picture_zones)
                                        .is_none();
                                let mut te =
                                    egui::TextEdit::singleline(&mut self.s.no_picture_zones)
                                        .desired_width(120.0)
                                        .hint_text("x,y,w,h;…");
                                if bad {
                                    te = te.text_color(egui::Color32::LIGHT_RED);
                                }
                                ch |= ui.add(te).changed();
                            });
                            ui.horizontal(|ui| {
                                ui.label(tr("set.picture_zones"));
                                let bad =
                                    legatorocr_core::cleanup::parse_zones(&self.s.picture_zones)
                                        .is_none();
                                let mut te = egui::TextEdit::singleline(&mut self.s.picture_zones)
                                    .desired_width(120.0)
                                    .hint_text("x,y,w,h;…");
                                if bad {
                                    te = te.text_color(egui::Color32::LIGHT_RED);
                                }
                                ch |= ui.add(te).changed();
                            });
                        }
                        if self.s.mode != ModeChoice::Color && self.s.mode != ModeChoice::Gray {
                            ch |= ui
                                .checkbox(&mut self.s.remove_lines, tr("set.lines"))
                                .changed();
                            ui.horizontal(|ui| {
                                ui.label(tr("set.despeckle_strength"));
                                ch |= ui
                                    .add(egui::Slider::new(
                                        &mut self.s.despeckle_strength,
                                        0.25..=4.0,
                                    ))
                                    .changed();
                            });
                        }
                        if self.s.mode != ModeChoice::Bw {
                            ui.horizontal(|ui| {
                                ui.label(tr("set.sharpen"));
                                ch |= ui
                                    .add(egui::Slider::new(&mut self.s.sharpen, 0.0..=2.0))
                                    .changed();
                            });
                        }
                    });
                egui::CollapsingHeader::new(tr("set.pdf"))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr("set.quality"));
                            self.config_dirty |= ui
                                .add(egui::Slider::new(&mut self.s.pdf_quality, 0..=100))
                                .changed();
                        });
                        ui.label(RichText::new(tr("set.quality_hint")).small().weak());
                        ui.horizontal(|ui| {
                            ui.label(tr("set.output_dpi"));
                            ch |= ui
                                .add(
                                    egui::DragValue::new(&mut self.s.output_dpi)
                                        .range(0..=2400)
                                        .custom_formatter(|v, _| {
                                            if v < 36.0 {
                                                "—".into()
                                            } else {
                                                format!("{v:.0}")
                                            }
                                        }),
                                )
                                .changed();
                        });
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.skip_blank, tr("set.skip_blank"))
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.uniform_size, tr("set.uniform"))
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.skip_duplicates, tr("set.skip_dup"))
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.split_on_blank, tr("set.split_blank"))
                            .on_hover_text(tr("set.split_blank_tip"))
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.split_on_barcode, tr("set.split_barcode"))
                            .on_hover_text(tr("set.split_barcode_tip"))
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.bookmarks, tr("set.bookmarks"))
                            .changed();
                        self.config_dirty |=
                            ui.checkbox(&mut self.s.pdfa, tr("set.pdfa")).changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.front_matter"));
                            self.config_dirty |= ui
                                .add(egui::DragValue::new(&mut self.s.front_matter).range(0..=999))
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.target_mb"));
                            self.config_dirty |= ui
                                .add(
                                    egui::DragValue::new(&mut self.s.target_mb)
                                        .range(0..=100_000)
                                        .custom_formatter(|v, _| {
                                            if v < 1.0 {
                                                "—".into()
                                            } else {
                                                format!("{v:.0} MB")
                                            }
                                        }),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.author"));
                            self.config_dirty |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.s.pdf_author)
                                        .desired_width(140.0),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("set.pdf_password"));
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.pdf_password)
                                        .password(true)
                                        .desired_width(120.0),
                                )
                                .changed()
                            {
                                legatorocr_engine::set_pdf_password(&self.pdf_password);
                                ch = true;
                            }
                        });
                        ui.label(RichText::new(tr("set.pdf_password_hint")).small().weak());
                        if self.s.ocr {
                            self.config_dirty |= ui
                                .checkbox(&mut self.s.sidecar_txt, tr("set.sidecar_txt"))
                                .changed();
                            self.config_dirty |= ui
                                .checkbox(&mut self.s.sidecar_hocr, tr("set.sidecar_hocr"))
                                .changed();
                        }
                    });
                egui::CollapsingHeader::new(tr("set.ocr"))
                    .default_open(true)
                    .show(ui, |ui| {
                        let can_ocr = !self.installed.is_empty();
                        ui.add_enabled(
                            can_ocr,
                            egui::Checkbox::new(&mut self.s.ocr, tr("set.ocr_enable")),
                        );
                        if !can_ocr {
                            self.s.ocr = false;
                            ui.label(
                                RichText::new(tr("set.ocr_missing"))
                                    .color(Color32::from_rgb(220, 150, 60)),
                            );
                        } else {
                            ui.label(RichText::new(&self.ocr_info).small().weak());
                            ui.label(tr("set.ocr_langs"));
                            let missing: Vec<&str> = legatorocr_engine::ocr::UI_LANGUAGES
                                .iter()
                                .filter(|(_, t, _)| !self.installed.iter().any(|i| i == t))
                                .map(|(_, t, _)| *t)
                                .collect();
                            if !missing.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "{}: {}",
                                        tr("set.ocr_missing_langs"),
                                        missing.join(", ")
                                    ))
                                    .small()
                                    .color(Color32::from_rgb(220, 150, 60)),
                                )
                                .on_hover_text(legatorocr_engine::ocr::install_hint(missing[0]));
                            }
                            ui.horizontal_wrapped(|ui| {
                                for l in self.installed.clone() {
                                    let mut on = self.s.ocr_langs.contains(&l);
                                    if ui.checkbox(&mut on, &l).changed() {
                                        // saved with the settings; the preview only depends on
                                        // them when orientation is read by OCR
                                        self.config_dirty = true;
                                        ch |= self.s.use_osd;
                                        if on {
                                            self.s.ocr_langs.push(l.clone());
                                        } else {
                                            self.s.ocr_langs.retain(|x| x != &l);
                                        }
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(tr("set.psm"));
                                let psm0 = self.s.psm;
                                egui::ComboBox::from_id_salt("psm")
                                    .selected_text(self.s.psm.to_string())
                                    .show_ui(ui, |ui| {
                                        for p in [3u32, 4, 6, 11] {
                                            ui.selectable_value(&mut self.s.psm, p, p.to_string());
                                        }
                                    });
                                if self.s.psm != psm0 {
                                    self.config_dirty = true;
                                    ch |= self.s.use_osd;
                                }
                            });
                        }
                    });
                ui.add_space(8.0);
                let idle = self.job.is_none() && !self.sheets.is_empty();
                let ctx = ui.ctx().clone();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            idle,
                            egui::Button::new(RichText::new(tr("menu.export_pdf")).strong()),
                        )
                        .clicked()
                    {
                        self.start_export(&ctx, JobKind::Pdf);
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new(tr("menu.export_images")))
                        .clicked()
                    {
                        self.start_export(&ctx, JobKind::Images);
                    }
                });
            });
        if let Some(undo) = swap {
            let (from, to) = if undo {
                (&mut self.settings_undo, &mut self.settings_redo)
            } else {
                (&mut self.settings_redo, &mut self.settings_undo)
            };
            if let Some(prev) = from.pop() {
                to.push(std::mem::replace(&mut self.s, prev));
                self.settings_edit = None;
                ch = true;
            }
        } else if ch {
            let fresh = self
                .settings_edit
                .is_none_or(|t| t.elapsed().as_secs_f32() > 1.0);
            if fresh {
                self.settings_undo.push(settings_before);
                self.settings_undo
                    .drain(..self.settings_undo.len().saturating_sub(100));
                self.settings_redo.clear();
            }
            self.settings_edit = Some(std::time::Instant::now());
        }
        if ch {
            self.settings_gen += 1;
            self.config_dirty = true;
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let tr = |k: &'static str| t(lang, k);
        let Some(i) = self.selected.filter(|&i| i < self.sheets.len()) else {
            let mut picked: Vec<PathBuf> = Vec::new();
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new(tr("welcome.title")).size(24.0).strong());
                ui.add_space(14.0);
                for k in ["welcome.step1", "welcome.step2", "welcome.step3"] {
                    ui.label(RichText::new(tr(k)).size(16.0));
                    ui.add_space(4.0);
                }
                ui.add_space(14.0);
                if ui
                    .add(egui::Button::new(
                        RichText::new(tr("welcome.add")).size(18.0),
                    ))
                    .clicked()
                {
                    if let Some(f) = self.pick(|| {
                        rfd::FileDialog::new()
                            .add_filter("images, PDF, archives", &all_input_ext())
                            .pick_files()
                    }) {
                        picked = f;
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new(tr("preview.empty")).weak());
            });
            if !picked.is_empty() {
                self.add_files(&picked);
            }
            return;
        };
        let mut make_grid = false;
        // --- toolbar: view + zoom (wraps: the zone kinds must stay reachable on narrow windows)
        ui.horizontal_wrapped(|ui| {
            for (v, k) in [
                (View::Both, "view.both"),
                (View::Original, "view.original"),
                (View::Processed, "view.processed"),
                (View::Slider, "view.slider"),
            ] {
                ui.selectable_value(&mut self.view, v, tr(k));
            }
            ui.separator();
            ui.checkbox(&mut self.fit, tr("view.fit"));
            ui.separator();
            ui.checkbox(&mut self.draw_zones, tr("zones.draw"))
                .on_hover_text(tr("zones.hint"));
            ui.checkbox(&mut self.edit_box, tr("box.edit"))
                .on_hover_text(tr("box.hint"));
            if self.draw_zones {
                let plain = !self.draw_guide && !self.draw_pictures && !self.draw_nopic;
                if ui
                    .selectable_label(plain && self.draw_extra == 0, tr("zones.kind_erase"))
                    .clicked()
                {
                    (self.draw_guide, self.draw_pictures, self.draw_nopic) = (false, false, false);
                    self.draw_extra = 0;
                }
                if ui
                    .selectable_label(
                        !self.draw_guide && self.draw_pictures && self.draw_extra == 0,
                        tr("zones.kind_picture"),
                    )
                    .clicked()
                {
                    (self.draw_guide, self.draw_pictures, self.draw_nopic) = (false, true, false);
                    self.draw_extra = 0;
                }
                if ui
                    .selectable_label(
                        !self.draw_guide && self.draw_nopic && self.draw_extra == 0,
                        tr("zones.kind_nopic"),
                    )
                    .on_hover_text(tr("zones.nopic_hint"))
                    .clicked()
                {
                    (self.draw_guide, self.draw_pictures, self.draw_nopic) = (false, false, true);
                    self.draw_extra = 0;
                }
                if ui
                    .selectable_value(&mut self.draw_guide, true, tr("zones.kind_guide"))
                    .on_hover_text(tr("zones.guide_hint"))
                    .clicked()
                {
                    self.draw_extra = 0;
                }
                for (v, k, h) in [
                    (1u8, "zones.kind_work", "zones.work_hint"),
                    (2u8, "zones.kind_cut", "zones.cut_hint"),
                ] {
                    if ui
                        .selectable_label(self.draw_extra == v, tr(k))
                        .on_hover_text(tr(h))
                        .clicked()
                    {
                        self.draw_extra = v;
                        (self.draw_guide, self.draw_pictures, self.draw_nopic) =
                            (false, false, false);
                    }
                }
                if self.draw_guide
                    && ui
                        .button(tr("zones.guide_grid"))
                        .on_hover_text(tr("zones.guide_grid_hint"))
                        .clicked()
                {
                    make_grid = true;
                }
            }
            // the zoom slider only means something when "fit" is off; shown greyed it looked like a stuck control
            if !self.fit {
                ui.add(egui::Slider::new(&mut self.zoom, 0.2..=4.0).suffix("×"));
            }
        });
        // --- per-sheet adjustments
        let mut adj_changed = false;
        let detected = self.sheets[i]
            .ready
            .as_ref()
            .and_then(|r| r.pages.first())
            .map(|p| p.skew_deg)
            .unwrap_or(0.0);
        let (can_undo, can_redo) = (
            !self.sheets[i].zone_undo.is_empty(),
            !self.sheets[i].zone_redo.is_empty(),
        );
        let (mut clear_zones, mut undo_zone, mut redo_zone) = (false, false, false);
        let adj_before = ZoneSnap::of(&self.sheets[i].adj);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("adj.title")).strong());
            let adj = &mut self.sheets[i].adj;
            if ui.button(tr("adj.rot_left")).clicked() {
                adj.rotate_quarters = (adj.rotate_quarters + 3) % 4;
                adj.orient_locked = true;
                adj_changed = true;
            }
            if ui.button(tr("adj.rot_right")).clicked() {
                adj.rotate_quarters = (adj.rotate_quarters + 1) % 4;
                adj.orient_locked = true;
                adj_changed = true;
            }
            // 45° steps: quarter turns plus the manual skew together make the total angle
            for (label, step) in [("-45°", -45.0f32), ("+45°", 45.0)] {
                if ui.button(label).clicked() {
                    adj.turn(step);
                    adj.orient_locked = true;
                    adj_changed = true;
                }
            }
            ui.separator();
            let label = match adj.split {
                None => tr("adj.split_default"),
                Some(SplitChoice::Auto) => tr("split.auto"),
                Some(SplitChoice::Middle) => tr("split.middle"),
                Some(SplitChoice::Off) => tr("split.off"),
            };
            egui::ComboBox::from_id_salt("adj_split")
                .selected_text(format!("{} - {label}", tr("adj.split_page")))
                .show_ui(ui, |ui| {
                    adj_changed |= ui
                        .selectable_value(&mut adj.split, None, tr("adj.split_default"))
                        .changed();
                    for c in [SplitChoice::Auto, SplitChoice::Middle, SplitChoice::Off] {
                        let n = match c {
                            SplitChoice::Auto => "split.auto",
                            SplitChoice::Off => "split.off",
                            SplitChoice::Middle => "split.middle",
                        };
                        adj_changed |= ui
                            .selectable_value(&mut adj.split, Some(c), tr(n))
                            .changed();
                    }
                });
            ui.separator();
            // output mode of this page only (one page in colour, the next in grey, ...)
            let mode_label = match adj.mode {
                None => tr("adj.split_default"),
                Some(ModeChoice::Color) => tr("mode.color"),
                Some(ModeChoice::Gray) => tr("mode.gray"),
                Some(ModeChoice::Bw) => tr("mode.bw"),
                Some(ModeChoice::Mixed) => tr("mode.mixed"),
                Some(ModeChoice::Hybrid) => tr("mode.hybrid"),
            };
            egui::ComboBox::from_id_salt("adj_mode")
                .selected_text(format!("{} - {mode_label}", tr("adj.mode_page")))
                .show_ui(ui, |ui| {
                    adj_changed |= ui
                        .selectable_value(&mut adj.mode, None, tr("adj.split_default"))
                        .changed();
                    for (c, n) in [
                        (ModeChoice::Color, "mode.color"),
                        (ModeChoice::Gray, "mode.gray"),
                        (ModeChoice::Bw, "mode.bw"),
                        (ModeChoice::Mixed, "mode.mixed"),
                        (ModeChoice::Hybrid, "mode.hybrid"),
                    ] {
                        adj_changed |= ui.selectable_value(&mut adj.mode, Some(c), tr(n)).changed();
                    }
                });
            ui.separator();
            let mut manual = adj.skew_deg.is_some();
            if ui.checkbox(&mut manual, tr("adj.skew_manual")).changed() {
                adj.skew_deg = manual.then_some((detected * 100.0).round() / 100.0);
                adj_changed = true;
            }
            if let Some(v) = adj.skew_deg.as_mut() {
                // The slider is for fine work (±10°); a larger value (left by a 45° step) must not be
                // clamped back every frame, so the slider only clamps what the user edits, and a
                // number box takes any angle up to ±45°.
                adj_changed |= ui
                    .add(
                        egui::Slider::new(v, -10.0..=10.0)
                            .clamping(egui::SliderClamping::Edits)
                            .suffix("°")
                            .fixed_decimals(2),
                    )
                    .changed();
                adj_changed |= ui
                    .add(
                        egui::DragValue::new(v)
                            .range(-45.0..=45.0)
                            .speed(0.1)
                            .suffix("°")
                            .fixed_decimals(2),
                    )
                    .changed();
            }
            if ui.button(tr("zones.clear")).clicked()
                && !(adj.erase_mm.is_empty()
                    && adj.picture_mm.is_empty()
                    && adj.no_picture_mm.is_empty()
                    && adj.dewarp_guides.is_empty())
            {
                clear_zones = true;
            }
            if ui
                .add_enabled(can_undo, egui::Button::new(tr("zones.undo")))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                undo_zone = true;
            }
            if ui
                .add_enabled(can_redo, egui::Button::new(tr("zones.redo")))
                .on_hover_text("Ctrl+Y")
                .clicked()
            {
                redo_zone = true;
            }
            if ui.button(tr("adj.reset")).clicked() {
                *adj = SheetAdjust::default();
                adj_changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("adj.content"))
                .on_hover_text(tr("adj.content_hint"));
            let a = &mut self.sheets[i].adj.content_adjust_mm;
            for (v, k) in a
                .iter_mut()
                .zip(["adj.left", "adj.top", "adj.right", "adj.bottom"])
            {
                ui.label(tr(k));
                adj_changed |= ui
                    .add(
                        egui::DragValue::new(v)
                            .range(-100.0..=100.0)
                            .speed(0.2)
                            .fixed_decimals(1),
                    )
                    .changed();
            }
        });
        if adj_changed {
            self.sheets[i].adj_gen += 1;
        }
        if make_grid {
            // a regular grid of draggable points: 5 lines of 7 points over the page. Lines that
            // are already there are not added again (a second click used to double the grid).
            let adj = &mut self.sheets[i].adj;
            let grid: Vec<Vec<(f32, f32)>> = (0..5)
                .map(|r| {
                    let y = 0.15 + 0.175 * r as f32;
                    (0..7).map(|c| (0.06 + 0.88 * c as f32 / 6.0, y)).collect()
                })
                .collect();
            let missing: Vec<_> = grid
                .into_iter()
                .filter(|g| !adj.dewarp_guides.contains(g))
                .collect();
            if !missing.is_empty() && adj.dewarp_guides.len() + missing.len() <= 64 {
                adj.dewarp_guides.extend(missing);
                self.sheets[i].adj_gen += 1;
            }
        }
        // Undo for every control: record the state from before this frame's change, unless it
        // continues a change made less than a second ago (a slider drag is one step).
        if self.sheets[i].adj != adj_before.0 {
            let sh = &mut self.sheets[i];
            let fresh = sh.last_edit.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0);
            if fresh {
                sh.zone_undo.push(adj_before);
                sh.zone_redo.clear();
            }
            sh.last_edit = Some(std::time::Instant::now());
        }
        // Keyboard: Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo (not while typing in a text box).
        if !ui.ctx().egui_wants_keyboard_input() {
            ui.input_mut(|inp| {
                let z = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
                let zs = egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                );
                let y = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
                if inp.consume_shortcut(&zs) || inp.consume_shortcut(&y) {
                    redo_zone = true;
                } else if inp.consume_shortcut(&z) {
                    undo_zone = true;
                }
            });
        }
        {
            let sh = &mut self.sheets[i];
            if clear_zones
                && !(sh.adj.erase_mm.is_empty()
                    && sh.adj.picture_mm.is_empty()
                    && sh.adj.no_picture_mm.is_empty()
                    && sh.adj.dewarp_guides.is_empty())
            {
                sh.zone_undo.push(ZoneSnap::of(&sh.adj));
                sh.zone_redo.clear();
                sh.adj.erase_mm.clear();
                sh.adj.picture_mm.clear();
                sh.adj.no_picture_mm.clear();
                sh.adj.dewarp_guides.clear();
                sh.adj_gen += 1;
            } else if undo_zone {
                sh.last_edit = None;
                if let Some(z) = sh.zone_undo.pop() {
                    sh.zone_redo.push(z.swap_into(&mut sh.adj));
                    sh.adj_gen += 1;
                }
            } else if redo_zone {
                sh.last_edit = None;
                if let Some(z) = sh.zone_redo.pop() {
                    sh.zone_undo.push(z.swap_into(&mut sh.adj));
                    sh.adj_gen += 1;
                }
            }
        }
        ui.separator();

        // --- image area
        let key = self.key_of(&self.sheets[i]);
        let sheet = &self.sheets[i];
        if let Some((k, e)) = &sheet.failed {
            if *k == key {
                ui.colored_label(Color32::from_rgb(220, 70, 60), e);
                return;
            }
        }
        let Some(ready) = &sheet.ready else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        };
        if self.view == View::Slider {
            Self::slider_view(
                ui,
                ready,
                &mut self.slider_frac,
                tr("view.original"),
                tr("view.processed"),
            );
            return;
        }
        let mut cells: Vec<Cell> = Vec::new();
        if self.view != View::Processed {
            cells.push((
                &ready.original,
                format!(
                    "{} — {}×{} px",
                    tr("view.original"),
                    ready.original_size.0,
                    ready.original_size.1
                ),
                true,
                None,
            ));
        }
        if self.view != View::Original {
            for (n, p) in ready.pages.iter().enumerate() {
                let blank = if p.blank {
                    format!(" — {}", tr("preview.blank"))
                } else {
                    String::new()
                };
                cells.push((
                    &p.tex,
                    format!(
                        "{} {} — {}×{} px, {} {:+.2}°{}",
                        tr("view.processed"),
                        n + 1,
                        p.w,
                        p.h,
                        tr("preview.skew"),
                        p.skew_deg,
                        blank
                    ),
                    false,
                    p.content_box.map(|b| (b, p.dpi, (p.w, p.h))),
                ));
            }
        }
        let avail = ui.available_size();
        let n = cells.len().max(1) as f32;
        let cell_w = (avail.x / n - ui.spacing().item_spacing.x).max(50.0);
        let cell_h = (avail.y - 24.0).max(50.0);
        let (fit, zoom) = (self.fit, self.zoom);
        let draw_zones = self.draw_zones;
        let osz = ready.original_size;
        let mm_per_px = 25.4 / self.s.dpi.max(36) as f32;
        let zones: Vec<[f32; 4]> = sheet.adj.erase_mm.clone();
        let pic_zones: Vec<[f32; 4]> = sheet.adj.picture_mm.clone();
        let nopic_zones: Vec<[f32; 4]> = sheet.adj.no_picture_mm.clone();
        let draw_pictures = self.draw_pictures;
        let draw_nopic = self.draw_nopic;
        let draw_guide = self.draw_guide;
        let draw_work = self.draw_extra == 1;
        let draw_cut = self.draw_extra == 2;
        let work_now: Option<[f32; 4]> = sheet.adj.work_mm;
        let cut_now: Option<f32> = sheet.adj.split_frac;
        let mut new_work: Option<Option<[f32; 4]>> = None;
        let mut new_cut: Option<Option<f32>> = None;
        let guide_list: Vec<Vec<(f32, f32)>> = sheet.adj.dewarp_guides.clone();
        let mut new_guide: Option<Vec<(f32, f32)>> = None;
        let mut remove_guide: Option<usize> = None;
        let gpath = &mut self.guide_path;
        let glive = &mut self.guide_live;
        let mut node_move: Option<(usize, usize, (f32, f32))> = None;
        let mut node_remove: Option<(usize, usize)> = None;
        let mut node_insert: Option<(usize, usize, (f32, f32))> = None;
        let edit_box = self.edit_box;
        let mut box_delta: Option<(usize, f32)> = None;
        let bdrag = &mut self.box_drag;
        let mut new_zone: Option<[f32; 4]> = None;
        let mut remove_zone: Option<(u8, usize)> = None;
        let drag = &mut self.zone_drag;
        let resize = &mut self.zone_resize;
        let mut edit_zone: Option<(u8, usize, [f32; 4])> = None;
        let mut draw = |ui: &mut egui::Ui| {
            ui.horizontal_top(|ui| {
                for (tex, caption, is_orig, boxinfo) in &cells {
                    ui.vertical(|ui| {
                        let ts = tex.size_vec2();
                        let scale = if fit {
                            (cell_w / ts.x).min(cell_h / ts.y).min(4.0)
                        } else {
                            zoom
                        };
                        let mut img = egui::Image::from_texture(*tex).fit_to_exact_size(ts * scale);
                        if *is_orig && draw_zones {
                            img = img.sense(egui::Sense::click_and_drag());
                        }
                        if !*is_orig && edit_box && boxinfo.is_some() {
                            img = img.sense(egui::Sense::click_and_drag());
                        }
                        let resp = ui.add(img);
                        if *is_orig {
                            let rect = resp.rect;
                            // image px -> screen and back
                            let k = Vec2::new(
                                rect.width() / osz.0.max(1) as f32,
                                rect.height() / osz.1.max(1) as f32,
                            );
                            let to_screen = |x_mm: f32, y_mm: f32| {
                                egui::pos2(
                                    rect.min.x + x_mm / mm_per_px * k.x,
                                    rect.min.y + y_mm / mm_per_px * k.y,
                                )
                            };
                            let to_mm = |p: egui::Pos2| {
                                (
                                    (p.x - rect.min.x) / k.x * mm_per_px,
                                    (p.y - rect.min.y) / k.y * mm_per_px,
                                )
                            };
                            let painter = ui.painter_at(rect);
                            for z in &zones {
                                let r = egui::Rect::from_two_pos(
                                    to_screen(z[0], z[1]),
                                    to_screen(z[0] + z[2], z[1] + z[3]),
                                );
                                painter.rect_filled(
                                    r,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(220, 50, 50, 70),
                                );
                                painter.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(1.5, Color32::from_rgb(220, 50, 50)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            for z in &pic_zones {
                                let r = egui::Rect::from_two_pos(
                                    to_screen(z[0], z[1]),
                                    to_screen(z[0] + z[2], z[1] + z[3]),
                                );
                                painter.rect_filled(
                                    r,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(50, 110, 230, 60),
                                );
                                painter.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(1.5, Color32::from_rgb(50, 110, 230)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            // "no automatic picture here": violet with a cross
                            for z in &nopic_zones {
                                let r = egui::Rect::from_two_pos(
                                    to_screen(z[0], z[1]),
                                    to_screen(z[0] + z[2], z[1] + z[3]),
                                );
                                let c = Color32::from_rgb(170, 70, 220);
                                painter.rect_filled(
                                    r,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(170, 70, 220, 50),
                                );
                                painter.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(1.5, c),
                                    egui::StrokeKind::Inside,
                                );
                                painter.line_segment(
                                    [r.left_top(), r.right_bottom()],
                                    egui::Stroke::new(1.0, c),
                                );
                                painter.line_segment(
                                    [r.right_top(), r.left_bottom()],
                                    egui::Stroke::new(1.0, c),
                                );
                            }
                            // working rectangle: everything outside is blanked; shown by dimming it
                            if let Some(w) = work_now {
                                let r = egui::Rect::from_two_pos(
                                    to_screen(w[0], w[1]),
                                    to_screen(w[0] + w[2], w[1] + w[3]),
                                )
                                .intersect(rect);
                                let dim = Color32::from_rgba_unmultiplied(0, 0, 0, 80);
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        rect.min,
                                        egui::pos2(rect.max.x, r.min.y),
                                    ),
                                    0.0,
                                    dim,
                                );
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(rect.min.x, r.max.y),
                                        rect.max,
                                    ),
                                    0.0,
                                    dim,
                                );
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(rect.min.x, r.min.y),
                                        egui::pos2(r.min.x, r.max.y),
                                    ),
                                    0.0,
                                    dim,
                                );
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(r.max.x, r.min.y),
                                        egui::pos2(rect.max.x, r.max.y),
                                    ),
                                    0.0,
                                    dim,
                                );
                                painter.rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(2.0, Color32::from_rgb(40, 170, 80)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            // cut line of a two-page sheet
                            if let Some(f) = cut_now {
                                let x = rect.min.x + f * rect.width();
                                painter.line_segment(
                                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                                    egui::Stroke::new(2.0, Color32::from_rgb(255, 140, 0)),
                                );
                            }
                            if draw_zones && draw_work {
                                if resp.drag_started() {
                                    *drag = ui
                                        .input(|i| i.pointer.press_origin())
                                        .or_else(|| resp.interact_pointer_pos());
                                }
                                if !ui.input(|i| i.pointer.any_down()) && !resp.drag_stopped() {
                                    *drag = None;
                                }
                                if let (Some(a), Some(b)) =
                                    (*drag, ui.input(|i| i.pointer.latest_pos()))
                                {
                                    let r = egui::Rect::from_two_pos(a, b).intersect(rect);
                                    painter.rect_stroke(
                                        r,
                                        0.0,
                                        egui::Stroke::new(1.5, Color32::from_rgb(40, 170, 80)),
                                        egui::StrokeKind::Inside,
                                    );
                                    if resp.drag_stopped() {
                                        let (x0, y0) = to_mm(r.min);
                                        let (x1, y1) = to_mm(r.max);
                                        if x1 - x0 > 5.0 && y1 - y0 > 5.0 {
                                            new_work = Some(Some([x0, y0, x1 - x0, y1 - y0]));
                                        }
                                        *drag = None;
                                    }
                                }
                                if resp.secondary_clicked() {
                                    new_work = Some(None);
                                }
                            } else if draw_zones && draw_cut {
                                if resp.clicked() || resp.dragged() {
                                    if let Some(p) = resp.interact_pointer_pos() {
                                        let f = ((p.x - rect.min.x) / rect.width().max(1.0))
                                            .clamp(0.02, 0.98);
                                        if resp.dragged() {
                                            let x = rect.min.x + f * rect.width();
                                            painter.line_segment(
                                                [
                                                    egui::pos2(x, rect.min.y),
                                                    egui::pos2(x, rect.max.y),
                                                ],
                                                egui::Stroke::new(
                                                    2.0,
                                                    Color32::from_rgb(255, 140, 0),
                                                ),
                                            );
                                        }
                                        if resp.drag_stopped() || resp.clicked() {
                                            new_cut = Some(Some(f));
                                        }
                                    }
                                }
                                if resp.drag_stopped() {
                                    if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                                        let f = ((p.x - rect.min.x) / rect.width().max(1.0))
                                            .clamp(0.02, 0.98);
                                        new_cut = Some(Some(f));
                                    }
                                }
                                if resp.secondary_clicked() {
                                    new_cut = Some(None);
                                }
                            }
                            for (gi, g) in guide_list.iter().enumerate() {
                                let pts: Vec<egui::Pos2> = g
                                    .iter()
                                    .enumerate()
                                    .map(|(ki, p)| {
                                        let p = match *glive {
                                            Some((lg, lk, lp)) if lg == gi && lk == ki => lp,
                                            _ => *p,
                                        };
                                        egui::pos2(
                                            rect.min.x + p.0 * rect.width(),
                                            rect.min.y + p.1 * rect.height(),
                                        )
                                    })
                                    .collect();
                                painter.add(egui::Shape::line(
                                    pts.clone(),
                                    egui::Stroke::new(2.0, Color32::from_rgb(40, 170, 80)),
                                ));
                                if draw_zones && draw_guide {
                                    for q in &pts {
                                        painter.circle(
                                            *q,
                                            4.5,
                                            Color32::WHITE,
                                            egui::Stroke::new(1.5, Color32::from_rgb(40, 170, 80)),
                                        );
                                    }
                                }
                            }
                            if draw_zones && draw_guide {
                                let to_scr = |q: (f32, f32)| {
                                    egui::pos2(
                                        rect.min.x + q.0 * rect.width(),
                                        rect.min.y + q.1 * rect.height(),
                                    )
                                };
                                let from_scr = |p: egui::Pos2| {
                                    (
                                        ((p.x - rect.min.x) / rect.width().max(1.0))
                                            .clamp(0.0, 1.0),
                                        ((p.y - rect.min.y) / rect.height().max(1.0))
                                            .clamp(0.0, 1.0),
                                    )
                                };
                                // the node under the pointer (within 10 screen pixels)
                                let nearest_node = |p: egui::Pos2| {
                                    let mut best: Option<(usize, usize, f32)> = None;
                                    for (gi, g) in guide_list.iter().enumerate() {
                                        for (ki, q) in g.iter().enumerate() {
                                            let d = to_scr(*q).distance(p);
                                            if d <= 10.0 && best.is_none_or(|b| d < b.2) {
                                                best = Some((gi, ki, d));
                                            }
                                        }
                                    }
                                    best.map(|b| (b.0, b.1))
                                };
                                if resp.drag_started() {
                                    if let Some(o) = ui
                                        .input(|i| i.pointer.press_origin())
                                        .or_else(|| resp.interact_pointer_pos())
                                    {
                                        if let Some((g, k)) = nearest_node(o) {
                                            *glive = Some((g, k, guide_list[g][k]));
                                        }
                                    }
                                }
                                if let Some((g, k, _)) = *glive {
                                    if resp.dragged() {
                                        if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                                            *glive = Some((g, k, from_scr(p)));
                                        }
                                    }
                                    if resp.drag_stopped() {
                                        if let Some((_, _, pos)) = *glive {
                                            node_move = Some((g, k, pos));
                                        }
                                        *glive = None;
                                    }
                                }
                                if resp.double_clicked() {
                                    if let Some(p) = resp.interact_pointer_pos() {
                                        if let Some((g, k)) = nearest_node(p) {
                                            node_remove = Some((g, k));
                                        } else {
                                            // a new node on the nearest segment
                                            let mut best: Option<(usize, usize, f32, egui::Pos2)> =
                                                None;
                                            for (gi, g) in guide_list.iter().enumerate() {
                                                for (ki, w) in g.windows(2).enumerate() {
                                                    let (a, b) = (to_scr(w[0]), to_scr(w[1]));
                                                    let ab = b - a;
                                                    let l2 = ab.length_sq();
                                                    let t = if l2 <= f32::EPSILON {
                                                        0.0
                                                    } else {
                                                        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
                                                    };
                                                    let q = a + ab * t;
                                                    let d = p.distance(q);
                                                    if d <= 10.0 && best.is_none_or(|bb| d < bb.2) {
                                                        best = Some((gi, ki + 1, d, q));
                                                    }
                                                }
                                            }
                                            if let Some((g, k, _, q)) = best {
                                                node_insert = Some((g, k, from_scr(q)));
                                            }
                                        }
                                    }
                                }
                                if glive.is_none() && resp.drag_started() {
                                    gpath.clear();
                                    if let Some(p) = ui
                                        .input(|i| i.pointer.press_origin())
                                        .or_else(|| resp.interact_pointer_pos())
                                    {
                                        gpath.push(p);
                                    }
                                }
                                if glive.is_none() && resp.dragged() {
                                    if let Some(p) = ui.input(|i| i.pointer.latest_pos()) {
                                        let p = p.clamp(rect.min, rect.max);
                                        if gpath.last().is_none_or(|l| l.distance(p) >= 4.0)
                                            && gpath.len() < 4000
                                        {
                                            gpath.push(p);
                                        }
                                    }
                                }
                                if gpath.len() >= 2 {
                                    painter.add(egui::Shape::line(
                                        gpath.clone(),
                                        egui::Stroke::new(2.0, Color32::from_rgb(255, 160, 0)),
                                    ));
                                }
                                if glive.is_none() && resp.drag_stopped() {
                                    // thin the path to at most 24 points, fractions of the image
                                    let n = gpath.len();
                                    if n >= 2
                                        && (gpath[0].x - gpath[n - 1].x).abs() >= 0.3 * rect.width()
                                    {
                                        let step = n.div_ceil(24).max(1);
                                        let mut v: Vec<(f32, f32)> = gpath
                                            .iter()
                                            .step_by(step)
                                            .map(|p| {
                                                (
                                                    (p.x - rect.min.x) / rect.width().max(1.0),
                                                    (p.y - rect.min.y) / rect.height().max(1.0),
                                                )
                                            })
                                            .collect();
                                        let l = gpath[n - 1];
                                        v.push((
                                            (l.x - rect.min.x) / rect.width().max(1.0),
                                            (l.y - rect.min.y) / rect.height().max(1.0),
                                        ));
                                        new_guide = Some(v);
                                    }
                                    gpath.clear();
                                }
                                if resp.secondary_clicked() {
                                    if let Some(p) = resp.interact_pointer_pos() {
                                        // nearest guide (distance to its segments, in screen
                                        // pixels), within 12 px
                                        let to_screen = |q: &(f32, f32)| {
                                            egui::pos2(
                                                rect.min.x + q.0 * rect.width(),
                                                rect.min.y + q.1 * rect.height(),
                                            )
                                        };
                                        let best = guide_list
                                            .iter()
                                            .enumerate()
                                            .map(|(k, g)| {
                                                let pts: Vec<egui::Pos2> =
                                                    g.iter().map(to_screen).collect();
                                                let d = pts
                                                    .windows(2)
                                                    .map(|w| dist_to_segment(p, w[0], w[1]))
                                                    .fold(f32::MAX, f32::min);
                                                (k, d)
                                            })
                                            .min_by(|a, b| a.1.total_cmp(&b.1));
                                        if let Some((k, d)) = best {
                                            if d < 12.0 {
                                                remove_guide = Some(k);
                                            }
                                        }
                                    }
                                }
                            } else if draw_zones && !draw_work && !draw_cut {
                                if resp.drag_started() {
                                    // egui reports the start only after the pointer has moved a
                                    // little: the rectangle begins where the button went down
                                    *drag = ui
                                        .input(|i| i.pointer.press_origin())
                                        .or_else(|| resp.interact_pointer_pos());
                                    // pressing on a corner of an existing zone grabs that corner
                                    // instead of starting a new rectangle
                                    *resize = None;
                                    if let Some(o) = *drag {
                                        let (ox, oy) = to_mm(o);
                                        let tol = 8.0 * mm_per_px / k.x.max(0.01);
                                        let find = |list: &Vec<[f32; 4]>, kind: u8| {
                                            list.iter().enumerate().rev().find_map(|(k, z)| {
                                                corner_hit(z, (ox, oy), tol).map(|c| (kind, k, c))
                                            })
                                        };
                                        *resize = find(&pic_zones, 1)
                                            .or_else(|| find(&nopic_zones, 2))
                                            .or_else(|| find(&zones, 0));
                                        if resize.is_some() {
                                            *drag = None;
                                        }
                                    }
                                }
                                if let Some((kind, k, c)) = *resize {
                                    let list = match kind {
                                        1 => &pic_zones,
                                        2 => &nopic_zones,
                                        _ => &zones,
                                    };
                                    if let (Some(z), Some(lp)) =
                                        (list.get(k), ui.input(|i| i.pointer.latest_pos()))
                                    {
                                        let (mx, my) = to_mm(lp.clamp(rect.min, rect.max));
                                        let nz = move_corner(z, c, (mx, my));
                                        painter.rect_stroke(
                                            egui::Rect::from_two_pos(
                                                to_screen(nz[0], nz[1]),
                                                to_screen(nz[0] + nz[2], nz[1] + nz[3]),
                                            ),
                                            0.0,
                                            egui::Stroke::new(1.5, Color32::from_rgb(255, 160, 0)),
                                            egui::StrokeKind::Inside,
                                        );
                                        if resp.drag_stopped() {
                                            edit_zone = Some((kind, k, nz));
                                            *resize = None;
                                        }
                                    } else {
                                        *resize = None;
                                    }
                                }
                                if !ui.input(|i| i.pointer.any_down()) && !resp.drag_stopped() {
                                    *drag = None;
                                    *resize = None;
                                }
                                if let (Some(a), Some(b)) =
                                    (*drag, ui.input(|i| i.pointer.latest_pos()))
                                {
                                    let r = egui::Rect::from_two_pos(a, b).intersect(rect);
                                    painter.rect_stroke(
                                        r,
                                        0.0,
                                        egui::Stroke::new(1.5, Color32::from_rgb(255, 160, 0)),
                                        egui::StrokeKind::Inside,
                                    );
                                    if resp.drag_stopped() {
                                        let (x0, y0) = to_mm(r.min);
                                        let (x1, y1) = to_mm(r.max);
                                        if x1 - x0 > 0.5 && y1 - y0 > 0.5 {
                                            new_zone = Some([x0, y0, x1 - x0, y1 - y0]);
                                        }
                                        *drag = None;
                                    }
                                } else if resp.drag_stopped() {
                                    *drag = None;
                                }
                                if resp.secondary_clicked() {
                                    if let Some(p) = resp.interact_pointer_pos() {
                                        let (mx, my) = to_mm(p);
                                        let hit = |z: &[f32; 4]| {
                                            mx >= z[0]
                                                && mx <= z[0] + z[2]
                                                && my >= z[1]
                                                && my <= z[1] + z[3]
                                        };
                                        remove_zone = pic_zones
                                            .iter()
                                            .rposition(hit)
                                            .map(|k| (1u8, k))
                                            .or_else(|| {
                                                nopic_zones.iter().rposition(hit).map(|k| (2u8, k))
                                            })
                                            .or_else(|| {
                                                zones.iter().rposition(hit).map(|k| (0u8, k))
                                            });
                                    }
                                }
                            }
                        }
                        if let (false, true, Some((b, dpi, (pw, ph)))) =
                            (*is_orig, edit_box, boxinfo)
                        {
                            // content box of this page: green frame, edges draggable
                            let rect = resp.rect;
                            let (kx, ky) = (
                                rect.width() / (*pw).max(1) as f32,
                                rect.height() / (*ph).max(1) as f32,
                            );
                            let br = egui::Rect::from_min_max(
                                egui::pos2(
                                    rect.min.x + b[0] as f32 * kx,
                                    rect.min.y + b[1] as f32 * ky,
                                ),
                                egui::pos2(
                                    rect.min.x + (b[0] + b[2]) as f32 * kx,
                                    rect.min.y + (b[1] + b[3]) as f32 * ky,
                                ),
                            );
                            let painter = ui.painter_at(rect.expand(2.0));
                            let green = Color32::from_rgb(40, 170, 70);
                            painter.rect_stroke(
                                br,
                                0.0,
                                egui::Stroke::new(1.5, green),
                                egui::StrokeKind::Middle,
                            );
                            // which edge is the pointer on (within 8 px)?
                            let edge_at = |p: egui::Pos2| -> Option<usize> {
                                let near = |a: f32, b: f32| (a - b).abs() <= 8.0;
                                let in_x = p.x >= br.min.x - 8.0 && p.x <= br.max.x + 8.0;
                                let in_y = p.y >= br.min.y - 8.0 && p.y <= br.max.y + 8.0;
                                let c = [
                                    (near(p.x, br.min.x) && in_y, (p.x - br.min.x).abs()),
                                    (near(p.y, br.min.y) && in_x, (p.y - br.min.y).abs()),
                                    (near(p.x, br.max.x) && in_y, (p.x - br.max.x).abs()),
                                    (near(p.y, br.max.y) && in_x, (p.y - br.max.y).abs()),
                                ];
                                c.iter()
                                    .enumerate()
                                    .filter(|(_, (ok, _))| *ok)
                                    .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
                                    .map(|(i, _)| i)
                            };
                            if let Some(p) = resp.hover_pos() {
                                if let Some(e) = edge_at(p) {
                                    ui.ctx().set_cursor_icon(if e % 2 == 0 {
                                        egui::CursorIcon::ResizeHorizontal
                                    } else {
                                        egui::CursorIcon::ResizeVertical
                                    });
                                }
                            }
                            if resp.drag_started() {
                                // egui reports a drag only after the pointer has moved a few
                                // pixels: the edge is the one under the press, not under the pointer
                                *bdrag = ui
                                    .input(|i| i.pointer.press_origin())
                                    .and_then(|p| edge_at(p).map(|e| (e, p)));
                            }
                            if let (Some((e, start)), Some(now)) =
                                (*bdrag, ui.input(|i| i.pointer.latest_pos()))
                            {
                                // the edge follows the pointer while dragging (drawn inside the
                                // image; the value may go beyond it: the page grows with the box)
                                let shown = now.clamp(rect.min, rect.max);
                                let (a, c) = match e {
                                    0 | 2 => (
                                        egui::pos2(shown.x, br.min.y),
                                        egui::pos2(shown.x, br.max.y),
                                    ),
                                    _ => (
                                        egui::pos2(br.min.x, shown.y),
                                        egui::pos2(br.max.x, shown.y),
                                    ),
                                };
                                painter.line_segment(
                                    [a, c],
                                    egui::Stroke::new(2.0, Color32::from_rgb(255, 160, 0)),
                                );
                                if resp.drag_stopped() {
                                    let mm_per_px = 25.4 / (*dpi).max(1) as f32;
                                    // outward = grow: left/top grow when moved left/up
                                    let grow = match e {
                                        0 => -(now.x - start.x) / kx,
                                        1 => -(now.y - start.y) / ky,
                                        2 => (now.x - start.x) / kx,
                                        _ => (now.y - start.y) / ky,
                                    } * mm_per_px;
                                    if grow.abs() > 0.05 {
                                        box_delta = Some((e, grow));
                                    }
                                    *bdrag = None;
                                }
                            } else if resp.drag_stopped() {
                                *bdrag = None;
                            }
                        }
                        ui.label(RichText::new(caption).small());
                    });
                }
            });
        };
        if fit {
            draw(ui);
        } else {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, draw);
        }
        if let Some((e, mm)) = box_delta {
            let before = ZoneSnap::of(&self.sheets[i].adj);
            let a = &mut self.sheets[i].adj.content_adjust_mm[e];
            *a = (*a + mm).clamp(-100.0, 100.0);
            self.sheets[i].zone_undo.push(before);
            self.sheets[i].zone_redo.clear();
            self.sheets[i].last_edit = None;
            self.sheets[i].adj_gen += 1;
        }
        if let Some(g) = new_guide {
            let a = &mut self.sheets[i].adj;
            if a.dewarp_guides.len() < 64 {
                let before = ZoneSnap::of(a);
                a.dewarp_guides.push(g);
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some((g, k, pos)) = node_move {
            let a = &mut self.sheets[i].adj;
            if g < a.dewarp_guides.len() && k < a.dewarp_guides[g].len() {
                let before = ZoneSnap::of(a);
                a.dewarp_guides[g][k] = pos;
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].last_edit = None;
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some((g, k)) = node_remove {
            let a = &mut self.sheets[i].adj;
            if g < a.dewarp_guides.len()
                && a.dewarp_guides[g].len() > 2
                && k < a.dewarp_guides[g].len()
            {
                let before = ZoneSnap::of(a);
                a.dewarp_guides[g].remove(k);
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].last_edit = None;
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some((g, k, pos)) = node_insert {
            let a = &mut self.sheets[i].adj;
            if g < a.dewarp_guides.len()
                && k <= a.dewarp_guides[g].len()
                && a.dewarp_guides[g].len() < 256
            {
                let before = ZoneSnap::of(a);
                a.dewarp_guides[g].insert(k, pos);
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].last_edit = None;
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some(k) = remove_guide {
            let a = &mut self.sheets[i].adj;
            if k < a.dewarp_guides.len() {
                let before = ZoneSnap::of(a);
                a.dewarp_guides.remove(k);
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some(z) = new_zone {
            let a = &mut self.sheets[i].adj;
            let list = if draw_nopic {
                &a.no_picture_mm
            } else if draw_pictures {
                &a.picture_mm
            } else {
                &a.erase_mm
            };
            if list.len() < 64 {
                let before = ZoneSnap::of(a);
                if draw_nopic {
                    a.no_picture_mm.push(z);
                } else if draw_pictures {
                    a.picture_mm.push(z);
                } else {
                    a.erase_mm.push(z);
                }
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some(w) = new_work {
            let before = ZoneSnap::of(&self.sheets[i].adj);
            self.sheets[i].adj.work_mm = w;
            self.sheets[i].zone_undo.push(before);
            self.sheets[i].zone_redo.clear();
            self.sheets[i].adj_gen += 1;
        }
        if let Some(c) = new_cut {
            let before = ZoneSnap::of(&self.sheets[i].adj);
            let adj = &mut self.sheets[i].adj;
            adj.split_frac = c;
            // a cut line does nothing while splitting is off: turn it on for this sheet
            if c.is_some() && matches!(adj.split, Some(SplitChoice::Off)) {
                adj.split = Some(SplitChoice::Middle);
            }
            self.sheets[i].zone_undo.push(before);
            self.sheets[i].zone_redo.clear();
            self.sheets[i].adj_gen += 1;
        }
        if let Some((kind, k, z)) = edit_zone {
            let a = &mut self.sheets[i].adj;
            let before = ZoneSnap::of(a);
            let list = match kind {
                1 => &mut a.picture_mm,
                2 => &mut a.no_picture_mm,
                _ => &mut a.erase_mm,
            };
            if k < list.len() {
                list[k] = z;
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some((kind, k)) = remove_zone {
            let a = &mut self.sheets[i].adj;
            let before = ZoneSnap::of(a);
            match kind {
                1 => drop(a.picture_mm.remove(k)),
                2 => drop(a.no_picture_mm.remove(k)),
                _ => drop(a.erase_mm.remove(k)),
            }
            self.sheets[i].zone_undo.push(before);
            self.sheets[i].zone_redo.clear();
            self.sheets[i].adj_gen += 1;
        }
    }
}

/// All per-sheet adjustments at one moment, for Undo / Redo.
#[derive(Clone, Default)]
struct ZoneSnap(SheetAdjust);

impl ZoneSnap {
    fn of(a: &SheetAdjust) -> Self {
        Self(a.clone())
    }
    /// Put this state into `a`, returning what was there.
    fn swap_into(self, a: &mut SheetAdjust) -> Self {
        Self(std::mem::replace(a, self.0))
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn dist_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    let t = if l2 <= f32::EPSILON {
        0.0
    } else {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    p.distance(a + ab * t)
}

/// Image, PDF and archive extensions for the "add" dialogs.
fn all_input_ext() -> Vec<&'static str> {
    IMAGE_EXT
        .iter()
        .chain(legatorocr_engine::archive::EXT.iter())
        .copied()
        .collect()
}

impl Drop for App {
    fn drop(&mut self) {
        legatorocr_engine::archive::cleanup();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        self.palette(&ctx);
        self.page_keys(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("pages")
            .default_size(230.0)
            .min_size(160.0)
            .show(ui, |ui| self.page_list(ui));
        egui::Panel::right("settings")
            .default_size(310.0)
            .min_size(250.0)
            .show(ui, |ui| self.settings_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.request_preview(&ctx);
        if self.lang != self.saved_lang {
            self.saved_lang = self.lang;
            self.config_dirty = true;
        }
        // Write the config once a slider drag has ended, not on every frame of it.
        if self.config_dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.save_config();
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::unique("drop_hint"))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(&ctx, |ui| {
                    ui.label(RichText::new(self.tr("drop.hint")).size(28.0).strong());
                });
        }
        self.modules_window(&ctx);
        {
            let lang = self.lang;
            self.proof.ui(&ctx, &|k| t(lang, k));
        }
        self.all_options_window(&ctx);
        if self.show_about {
            let mut open = true;
            egui::Window::new(self.tr("menu.about"))
                .open(&mut open)
                .collapsible(false)
                .show(&ctx, |ui| {
                    ui.heading("LegatorOCR");
                    ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                    ui.label(self.tr("about.text"));
                    ui.label("MIT — © 2026 Alexio <alexio@trom.tf>");
                    ui.horizontal_wrapped(|ui| {
                        ui.label(self.tr("about.coauthor"));
                        ui.hyperlink_to(
                            "Alexandru Dumbravă",
                            "https://competentedigitalizare.blogspot.com/",
                        );
                        ui.label("·");
                        ui.hyperlink_to("portfolio", "https://dumbravaalex.github.io/Portofoliu/");
                    });
                });
            self.show_about = open;
        }
    }
}

/// What the "Optional modules" window knows: which modules the doctor reports missing and
/// the commands for this system's package manager.
struct ModulesInfo {
    manager: Option<legatorocr_engine::install::Manager>,
    missing: Vec<legatorocr_engine::install::Module>,
    steps: Vec<legatorocr_engine::install::Step>,
}

#[derive(Default)]
struct ModulesWindow {
    show: bool,
    info: Option<ModulesInfo>,
    probe_rx: Option<mpsc::Receiver<ModulesInfo>>,
    run_rx: Option<mpsc::Receiver<Vec<legatorocr_engine::install::Outcome>>>,
    outcomes: Vec<legatorocr_engine::install::Outcome>,
}

impl ModulesWindow {
    /// Show the window and look (in a thread: the doctor starts a few programs) at what is missing.
    fn open(&mut self) {
        use legatorocr_engine::install as inst;
        self.show = true;
        self.info = None;
        let (tx, rx) = mpsc::channel();
        self.probe_rx = Some(rx);
        std::thread::spawn(move || {
            let checks = legatorocr_engine::doctor::run(std::path::Path::new("tesseract"));
            let missing = inst::missing_modules(&checks);
            let manager = inst::detect();
            let steps = manager.map(|m| inst::plan(&missing, m)).unwrap_or_default();
            let _ = tx.send(ModulesInfo {
                manager,
                missing,
                steps,
            });
        });
    }
}

/// Which corner of zone `z` (x, y, w, h in mm) lies within `tol` mm of `p`: 0 top-left, 1 top-right,
/// 2 bottom-left, 3 bottom-right.
fn corner_hit(z: &[f32; 4], p: (f32, f32), tol: f32) -> Option<u8> {
    let cs = [
        (z[0], z[1]),
        (z[0] + z[2], z[1]),
        (z[0], z[1] + z[3]),
        (z[0] + z[2], z[1] + z[3]),
    ];
    cs.iter()
        .enumerate()
        .map(|(c, q)| (c as u8, ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt()))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
}

/// The zone after dragging `corner` to `p`; the opposite corner stays where it is, and the zone
/// keeps at least 0.5 mm in each direction (it flips instead of turning inside out).
fn move_corner(z: &[f32; 4], corner: u8, p: (f32, f32)) -> [f32; 4] {
    let (fx, fy) = (
        if corner & 1 == 0 { z[0] + z[2] } else { z[0] },
        if corner & 2 == 0 { z[1] + z[3] } else { z[1] },
    );
    let (x0, x1) = (p.0.min(fx), p.0.max(fx));
    let (y0, y1) = (p.1.min(fy), p.1.max(fy));
    [
        x0.max(0.0),
        y0.max(0.0),
        (x1 - x0).max(0.5),
        (y1 - y0).max(0.5),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn zone_corners_are_found_and_moved() {
        let z = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(corner_hit(&z, (10.5, 20.2), 2.0), Some(0));
        assert_eq!(corner_hit(&z, (40.0, 60.0), 2.0), Some(3));
        assert_eq!(corner_hit(&z, (25.0, 40.0), 2.0), None);
        // dragging the bottom-right corner keeps the top-left corner
        assert_eq!(move_corner(&z, 3, (50.0, 70.0)), [10.0, 20.0, 40.0, 50.0]);
        // dragging the top-left corner keeps the bottom-right corner (40, 60)
        assert_eq!(move_corner(&z, 0, (5.0, 15.0)), [5.0, 15.0, 35.0, 45.0]);
        // dragged past the opposite corner: the zone flips
        assert_eq!(move_corner(&z, 0, (50.0, 80.0)), [40.0, 60.0, 10.0, 20.0]);
    }

    use super::*;
    use legatorocr_engine::files::{is_image, natural_cmp};
    use std::cmp::Ordering as Ord_;

    #[test]
    fn natural_order() {
        let mut v = vec!["p10.png", "p2.png", "P1.png", "p02b.png", "p100.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        // digit runs compare numerically, letters case-insensitively; p2 < p02b because '.' < 'b'
        assert_eq!(v, ["P1.png", "p2.png", "p02b.png", "p10.png", "p100.png"]);
        assert_eq!(natural_cmp("a", "a"), Ord_::Equal);
        assert_eq!(natural_cmp("a", "ab"), Ord_::Less);
    }

    #[test]
    fn image_detection_and_folder_collection() {
        assert!(is_image(Path::new("x/Scan.JPG")) && is_image(Path::new("a.tiff")));
        assert!(!is_image(Path::new("notes.txt")) && !is_image(Path::new("noext")));
        let d = std::env::temp_dir().join(format!("lgr-gui-col-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for n in ["p10.png", "p2.png", "readme.txt", "p1.jpg"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let single = d.join("p2.png");
        let got: Vec<String> = collect_images(&[d.clone(), single.clone(), d.join("readme.txt")])
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // folder expanded in natural order, explicit image kept, non-image dropped
        assert_eq!(got, ["p1.jpg", "p2.png", "p10.png", "p2.png"]);
        let _ = std::fs::remove_dir_all(d);
    }
}
