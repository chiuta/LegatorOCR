//! `--max-memory MB`: limit how many pages are processed at the same time so that the
//! working set stays near a budget (default: one page per thread, no limit).
//!
//! The estimate is deliberately simple: a page in flight holds its decoded RGB image plus
//! about five working copies (grey, binary, deskewed, cropped, output), so
//! `bytes = width * height * 3 * FACTOR` for the largest sheet. It is a guide for batch size,
//! not a hard limit; at least one page is always processed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CAP_MB: AtomicU64 = AtomicU64::new(0);

/// Working copies held per page on top of the decoded image.
const FACTOR: u64 = 6;

pub fn set(mb: u64) {
    CAP_MB.store(mb, Ordering::Relaxed);
}

/// Estimated bytes of one page in flight.
pub fn page_bytes(w: u32, h: u32) -> u64 {
    w as u64 * h as u64 * 3 * FACTOR
}

/// How many pages may run at once given the cap, the thread count and the largest page.
pub fn batch_size(threads: usize, largest_page_bytes: u64) -> usize {
    let cap = CAP_MB.load(Ordering::Relaxed);
    if cap == 0 || largest_page_bytes == 0 {
        return threads;
    }
    ((cap * 1024 * 1024 / largest_page_bytes) as usize).clamp(1, threads)
}

/// Largest estimated page among the files (headers only; unreadable files count as 0).
pub fn largest(paths: &[PathBuf]) -> u64 {
    paths
        .iter()
        .filter_map(|p| image::image_dimensions(p).ok())
        .map(|(w, h)| page_bytes(w, h))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_follows_the_cap() {
        let a4_600dpi = page_bytes(4960, 7016);
        set(0);
        assert_eq!(batch_size(8, a4_600dpi), 8);
        set(1000);
        let b = batch_size(8, a4_600dpi);
        assert!((1..8).contains(&b), "{b}");
        set(1);
        assert_eq!(batch_size(8, a4_600dpi), 1);
        set(0);
    }
}
