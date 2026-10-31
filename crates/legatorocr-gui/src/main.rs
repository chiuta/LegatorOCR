// No console window on Windows release builds.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod app;
mod i18n;
mod settings;
mod worker;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result {
    // Optional command-line files/folders to preload: `legatorocr-gui scan1.png scans/`
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LegatorOCR")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([980.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LegatorOCR",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, files)))),
    )
}
