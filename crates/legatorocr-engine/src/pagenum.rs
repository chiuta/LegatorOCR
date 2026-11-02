//! Printed page numbers found by OCR become the PDF's page labels (what a viewer shows in
//! its page box), so "page 17" in the book is "17" in the viewer even after an unnumbered cover.

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}
