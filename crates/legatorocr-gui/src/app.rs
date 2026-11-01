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
}

struct PageTex {
    tex: TextureHandle,
    w: u32,
    h: u32,
    skew_deg: f32,
    blank: bool,
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
    /// Erase-zone history for Undo / Redo (snapshots of `adj.erase_mm`).
    zone_undo: Vec<Vec<[f32; 4]>>,
    zone_redo: Vec<Vec<[f32; 4]>>,
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
    settings_gen: u32,
    sheets: Vec<Sheet>,
    selected: Option<usize>,
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
    ocr_info: String,
    show_about: bool,
    draw_zones: bool,
    zone_drag: Option<egui::Pos2>,
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

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let (pv_tx, pv_rx) = mpsc::channel();
        let mut app = Self {
            tick: 0,
            lang: Lang::detect(),
            s: UiSettings::default(),
            settings_gen: 1,
            sheets: Vec::new(),
            selected: None,
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
            ocr_info: String::new(),
            show_about: false,
            draw_zones: false,
            zone_drag: None,
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
                    },
                )
            })
            .collect()
    }

    fn save_project(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("LegatorOCR", &["lgr"])
            .set_file_name("project.lgr")
            .save_file()
        else {
            return;
        };
        let r = self.s.to_project(self.project_sheets()).save(&path);
        match r {
            Ok(()) => self.say(
                format!("{}: {}", self.tr("status.project_saved"), path.display()),
                false,
            ),
            Err(e) => self.say(format!("{}: {e:#}", self.tr("status.error")), true),
        }
    }

    fn open_project(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("LegatorOCR", &["lgr"])
            .pick_file()
        else {
            return;
        };
        match legatorocr_engine::project::Project::load(&path) {
            Ok(p) => {
                self.s.apply_project(&p);
                self.sheets.clear();
                self.selected = None;
                let files: Vec<PathBuf> = p.sheets.iter().map(|(f, _)| f.clone()).collect();
                self.add_files(&files);
                for (sh, (_, a)) in self.sheets.iter_mut().zip(&p.sheets) {
                    sh.adj = SheetAdjust {
                        rotate_quarters: a.rotate_quarters,
                        split: a.split.map(Into::into),
                        skew_deg: a.skew_deg,
                        content_adjust_mm: a.content_adjust_mm,
                        erase_mm: a.erase_mm.clone(),
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
                        self.has_osd = langs.iter().any(|l| l == "osd");
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
                            let key = if d.kind == JobKind::Pdf {
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
                        Err(e) if e.contains("cancelled") => {
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

    fn request_preview(&mut self, ctx: &egui::Context) {
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
        let target = match kind {
            JobKind::Pdf => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name("LegatorOCR.pdf")
                .save_file(),
            JobKind::Images => rfd::FileDialog::new().pick_folder(),
        };
        let Some(target) = target else { return };
        self.launch_export(ctx, kind, target);
    }

    fn launch_export(&mut self, ctx: &egui::Context, kind: JobKind, target: PathBuf) {
        if self.job.is_some() || self.sheets.is_empty() {
            return;
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
                    if let Some(f) = rfd::FileDialog::new()
                        .add_filter("images, PDF, archives", &all_input_ext())
                        .pick_files()
                    {
                        self.add_files(&f);
                    }
                    ui.close();
                }
                if ui.button(self.tr("menu.add_folder")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
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
            if ui.button(self.tr("menu.about")).clicked() {
                self.show_about = true;
            }
        });
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
                if let Some(f) = rfd::FileDialog::new()
                    .add_filter("images, PDF, archives", &all_input_ext())
                    .pick_files()
                {
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
        if self.sheets.is_empty() {
            ui.label(self.tr("list.empty"));
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in 0..self.sheets.len() {
                    let key = self.key_of(&self.sheets[i]);
                    let s = &self.sheets[i];
                    let color = if s.failed.as_ref().map(|f| f.0) == Some(key) {
                        Color32::from_rgb(220, 70, 60)
                    } else if s.ready.as_ref().map(|r| r.key) == Some(key) {
                        Color32::from_rgb(70, 170, 90)
                    } else if s.pending.is_some() {
                        Color32::from_rgb(230, 180, 50)
                    } else {
                        Color32::GRAY
                    };
                    let label = format!("{:>3}  {}", i + 1, s.name);
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            self.selected = Some(i);
                        }
                    });
                }
            });
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let tr = |k: &'static str| t(lang, k);
        let mut ch = false;
        ui.heading(tr("set.title"));
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
                            ui.label(tr("set.split"));
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
                    });
                egui::CollapsingHeader::new(tr("set.cleanup"))
                    .default_open(false)
                    .show(ui, |ui| {
                        ch |= ui
                            .checkbox(&mut self.s.auto_invert, tr("set.invert"))
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
                            .checkbox(&mut self.s.punch_holes, tr("set.punch"))
                            .changed();
                        ch |= ui
                            .checkbox(&mut self.s.fingers, tr("set.fingers"))
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label(tr("set.dropout"));
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
                            .changed();
                        self.config_dirty |= ui
                            .checkbox(&mut self.s.split_on_barcode, tr("set.split_barcode"))
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
                                egui::ComboBox::from_id_salt("psm")
                                    .selected_text(self.s.psm.to_string())
                                    .show_ui(ui, |ui| {
                                        for p in [3u32, 4, 6, 11] {
                                            ui.selectable_value(&mut self.s.psm, p, p.to_string());
                                        }
                                    });
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
        if ch {
            self.settings_gen += 1;
            self.config_dirty = true;
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let tr = |k: &'static str| t(lang, k);
        let Some(i) = self.selected.filter(|&i| i < self.sheets.len()) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(tr("preview.empty")).size(18.0).weak());
            });
            return;
        };
        // --- toolbar: view + zoom
        ui.horizontal(|ui| {
            for (v, k) in [
                (View::Both, "view.both"),
                (View::Original, "view.original"),
                (View::Processed, "view.processed"),
            ] {
                ui.selectable_value(&mut self.view, v, tr(k));
            }
            ui.separator();
            ui.checkbox(&mut self.fit, tr("view.fit"));
            ui.separator();
            ui.checkbox(&mut self.draw_zones, tr("zones.draw"))
                .on_hover_text(tr("zones.hint"));
            ui.add_enabled(
                !self.fit,
                egui::Slider::new(&mut self.zoom, 0.2..=4.0).suffix("×"),
            );
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
        let mut reset_snap: Option<Vec<[f32; 4]>> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("adj.title")).strong());
            let adj = &mut self.sheets[i].adj;
            if ui.button(tr("adj.rot_left")).clicked() {
                adj.rotate_quarters = (adj.rotate_quarters + 3) % 4;
                adj_changed = true;
            }
            if ui.button(tr("adj.rot_right")).clicked() {
                adj.rotate_quarters = (adj.rotate_quarters + 1) % 4;
                adj_changed = true;
            }
            ui.separator();
            let label = match adj.split {
                None => tr("adj.split_default"),
                Some(SplitChoice::Auto) => tr("split.auto"),
                Some(SplitChoice::Middle) => tr("split.middle"),
                Some(SplitChoice::Off) => tr("split.off"),
            };
            egui::ComboBox::from_id_salt("adj_split")
                .selected_text(format!("{}: {label}", tr("set.split")))
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
            let mut manual = adj.skew_deg.is_some();
            if ui.checkbox(&mut manual, tr("adj.skew_manual")).changed() {
                adj.skew_deg = manual.then_some((detected * 100.0).round() / 100.0);
                adj_changed = true;
            }
            if let Some(v) = adj.skew_deg.as_mut() {
                adj_changed |= ui
                    .add(
                        egui::Slider::new(v, -10.0..=10.0)
                            .suffix("°")
                            .fixed_decimals(2),
                    )
                    .changed();
            }
            if ui.button(tr("zones.clear")).clicked() && !adj.erase_mm.is_empty() {
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
                if !adj.erase_mm.is_empty() {
                    reset_snap = Some(adj.erase_mm.clone());
                }
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
            if let Some(snap) = reset_snap {
                sh.zone_undo.push(snap);
                sh.zone_redo.clear();
            }
            if clear_zones && !sh.adj.erase_mm.is_empty() {
                sh.zone_undo.push(sh.adj.erase_mm.clone());
                sh.zone_redo.clear();
                sh.adj.erase_mm.clear();
                sh.adj_gen += 1;
            } else if undo_zone {
                if let Some(prev) = sh.zone_undo.pop() {
                    sh.zone_redo
                        .push(std::mem::replace(&mut sh.adj.erase_mm, prev));
                    sh.adj_gen += 1;
                }
            } else if redo_zone {
                if let Some(next) = sh.zone_redo.pop() {
                    sh.zone_undo
                        .push(std::mem::replace(&mut sh.adj.erase_mm, next));
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
        let mut cells: Vec<(&TextureHandle, String, bool)> = Vec::new();
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
        let mut new_zone: Option<[f32; 4]> = None;
        let mut remove_zone: Option<usize> = None;
        let drag = &mut self.zone_drag;
        let mut draw = |ui: &mut egui::Ui| {
            ui.horizontal_top(|ui| {
                for (tex, caption, is_orig) in &cells {
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
                            if draw_zones {
                                if resp.drag_started() {
                                    *drag = resp.interact_pointer_pos();
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
                                        remove_zone = zones.iter().rposition(|z| {
                                            mx >= z[0]
                                                && mx <= z[0] + z[2]
                                                && my >= z[1]
                                                && my <= z[1] + z[3]
                                        });
                                    }
                                }
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
        if let Some(z) = new_zone {
            let a = &mut self.sheets[i].adj;
            if a.erase_mm.len() < 64 {
                let before = a.erase_mm.clone();
                a.erase_mm.push(z);
                self.sheets[i].zone_undo.push(before);
                self.sheets[i].zone_redo.clear();
                self.sheets[i].adj_gen += 1;
            }
        }
        if let Some(k) = remove_zone {
            let before = self.sheets[i].adj.erase_mm.clone();
            self.sheets[i].adj.erase_mm.remove(k);
            self.sheets[i].zone_undo.push(before);
            self.sheets[i].zone_redo.clear();
            self.sheets[i].adj_gen += 1;
        }
    }
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

#[cfg(test)]
mod tests {
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
