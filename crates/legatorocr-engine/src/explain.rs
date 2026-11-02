//! `--explain`: for every output page, what the pipeline decided and the setting behind it.
//! Only decisions that are visible from the settings and the finished page are reported;
//! the numbers are the measured ones (skew angle, content box), not guesses.

use legatorocr_core::{Page, Settings, SplitMode};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
static LINES: Mutex<BTreeMap<(usize, usize), String>> = Mutex::new(BTreeMap::new());

pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if let Ok(mut l) = LINES.lock() {
        l.clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Describe one output page of sheet `index`. `quarters_before` is the rotation requested
/// before OSD ran; `kept` says whether the page went on to the output.
pub fn record(
    index: usize,
    seq: usize,
    path: &Path,
    pg: &Page,
    s: &Settings,
    quarters_before: u8,
    kept: bool,
) {
    if !enabled() {
        return;
    }
    let mut parts: Vec<String> = Vec::new();
    if s.rotate_quarters != quarters_before {
        parts.push(format!(
            "rotated {}° (OCR orientation detection)",
            (s.rotate_quarters as i32 - quarters_before as i32).rem_euclid(4) * 90
        ));
    } else if s.rotate_quarters != 0 {
        parts.push(format!("rotated {}° (as requested)", s.rotate_quarters as u32 * 90));
    } else if s.auto_orient {
        parts.push("orientation checked, left upright".into());
    }
    match (pg.split_index, &s.split) {
        (Some(0), _) => parts.push("left half of a split spread".into()),
        (Some(_), _) => parts.push("right half of a split spread".into()),
        (None, SplitMode::Off) => {}
        (None, _) => parts.push("single page (no spread found)".into()),
    }
    if let Some(m) = s.manual_skew_deg {
        parts.push(format!("skew set by hand: {m:+.2}°"));
    } else if s.deskew {
        if pg.skew_deg.abs() < 0.05 {
            parts.push("deskew: already straight".into());
        } else {
            parts.push(format!("deskew: corrected {:+.2}°", pg.skew_deg));
        }
    }
    if pg.blank {
        parts.push(if kept {
            "no content found: blank page, kept".into()
        } else {
            "no content found: blank page, dropped (--skip-blank)".into()
        });
    } else if let Some([x, y, w, h]) = pg.content_box {
        let (iw, ih) = (pg.image.width(), pg.image.height());
        parts.push(format!(
            "content {w}×{h} px at ({x},{y}) on a {iw}×{ih} page ({:.0}% of the area)",
            100.0 * (w as f64 * h as f64) / (iw as f64 * ih as f64).max(1.0)
        ));
    }
    parts.push(format!("{} dpi", pg.dpi));
    let line = format!("{} [{}]: {}", path.display(), seq + 1, parts.join("; "));
    if let Ok(mut l) = LINES.lock() {
        l.insert((index, seq), line);
    }
}

/// All lines, in batch order.
pub fn report() -> Vec<String> {
    LINES.lock().map(|l| l.values().cloned().collect()).unwrap_or_default()
}
