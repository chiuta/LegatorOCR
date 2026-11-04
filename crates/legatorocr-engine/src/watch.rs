//! Hot folder: report each new scan once it has stopped growing (a scanner may still be
//! writing the file when it first appears).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const EXTS: [&str; 7] = ["png", "jpg", "jpeg", "tif", "tiff", "bmp", "pdf"];

pub fn is_scan(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

pub struct Watcher {
    dir: PathBuf,
    seen: HashSet<PathBuf>,
    sizes: HashMap<PathBuf, u64>,
}

impl Watcher {
    /// Files already in the folder are handled too, unless `skip_existing` is set.
    pub fn new(dir: &Path, skip_existing: bool) -> Watcher {
        let mut w = Watcher {
            dir: dir.to_path_buf(),
            seen: HashSet::new(),
            sizes: HashMap::new(),
        };
        if skip_existing {
            w.seen = w.listing().into_iter().map(|(p, _)| p).collect();
        }
        w
    }

    fn listing(&self) -> Vec<(PathBuf, u64)> {
        let mut v: Vec<(PathBuf, u64)> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                let p = e.path();
                (m.is_file() && is_scan(&p)).then_some((p, m.len()))
            })
            .collect();
        v.sort();
        v
    }

    /// New, non-empty files whose size is the same as at the previous call, in name order.
    pub fn poll(&mut self) -> Vec<PathBuf> {
        let mut ready = Vec::new();
        let mut now = HashMap::new();
        for (p, len) in self.listing() {
            if self.seen.contains(&p) {
                continue;
            }
            if len > 0 && self.sizes.get(&p) == Some(&len) {
                self.seen.insert(p.clone());
                ready.push(p);
            } else {
                now.insert(p, len);
            }
        }
        self.sizes = now;
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_reported_once_it_stops_growing() {
        let d = std::env::temp_dir().join(format!("lgr_watch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("old.png"), b"x").unwrap();
        let mut w = Watcher::new(&d, true);
        assert!(w.poll().is_empty());
        std::fs::write(d.join("a.png"), b"12").unwrap();
        std::fs::write(d.join("notes.txt"), b"ignored").unwrap();
        assert!(w.poll().is_empty(), "first sight: size not yet confirmed");
        std::fs::write(d.join("a.png"), b"123").unwrap();
        assert!(w.poll().is_empty(), "still growing");
        assert_eq!(w.poll(), vec![d.join("a.png")]);
        assert!(w.poll().is_empty(), "reported only once");
        std::fs::write(d.join("empty.jpg"), b"").unwrap();
        assert!(
            w.poll().is_empty() && w.poll().is_empty(),
            "empty files wait"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn existing_files_are_taken_when_not_skipped() {
        let d = std::env::temp_dir().join(format!("lgr_watch2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("b.PDF"), b"x").unwrap();
        let mut w = Watcher::new(&d, false);
        assert!(w.poll().is_empty());
        assert_eq!(w.poll(), vec![d.join("b.PDF")]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
