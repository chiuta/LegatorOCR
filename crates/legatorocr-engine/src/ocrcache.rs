//! `--ocr-cache DIR`: remember the words OCR found for a page, keyed by the page's pixels and
//! the OCR settings, so a re-run that leaves the page image unchanged (other PDF quality, other
//! output name, a crash half-way through a book) does not pay for OCR again.
//!
//! The key also holds the engine version and this program's version, so an upgrade invalidates
//! old entries by itself. Entries are small text files; a damaged one is simply ignored.

use legatorocr_core::{PageImage, Word};
use legatorocr_ocr::OcrOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static DIR: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);
static HITS: AtomicUsize = AtomicUsize::new(0);
static MISSES: AtomicUsize = AtomicUsize::new(0);

/// Turn the cache on. `salt` must change whenever the engine does (its version string).
pub fn set(dir: PathBuf, salt: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(&dir)?;
    if let Ok(mut d) = DIR.lock() {
        *d = Some((dir, format!("{salt}|{}", env!("CARGO_PKG_VERSION"))));
    }
    HITS.store(0, Ordering::Relaxed);
    MISSES.store(0, Ordering::Relaxed);
    Ok(())
}

/// `(hits, misses)` since [`set`].
pub fn stats() -> (usize, usize) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}

fn key(page: &PageImage, dpi: u32, o: &OcrOptions, salt: &str) -> String {
    let img = page.to_dynamic();
    let mut buf = Vec::with_capacity(img.as_bytes().len() + 128);
    buf.extend_from_slice(
        format!(
            "{salt}|{}x{}|{dpi}|{:?}|{}|{}|{}|{:?}|",
            img.width(),
            img.height(),
            o.engine,
            o.lang,
            o.psm,
            o.min_conf,
            o.tessdata
        )
        .as_bytes(),
    );
    buf.extend_from_slice(img.as_bytes());
    crate::checksums::sha256_hex(&buf)
}

fn encode(words: &[Word]) -> String {
    let mut s = String::from("lgr-ocr-cache 1\n");
    for w in words {
        s.push_str(&format!(
            "{} {} {} {} {} {}\t{}\n",
            w.x,
            w.y,
            w.w,
            w.h,
            w.conf,
            w.line,
            w.text.replace(['\n', '\t'], " ")
        ));
    }
    s
}

fn decode(s: &str) -> Option<Vec<Word>> {
    let mut lines = s.lines();
    if lines.next()? != "lgr-ocr-cache 1" {
        return None;
    }
    let mut out = Vec::new();
    for l in lines {
        let (nums, text) = l.split_once('\t')?;
        let n: Vec<&str> = nums.split(' ').collect();
        if n.len() != 6 {
            return None;
        }
        out.push(Word {
            text: text.to_string(),
            x: n[0].parse().ok()?,
            y: n[1].parse().ok()?,
            w: n[2].parse().ok()?,
            h: n[3].parse().ok()?,
            conf: n[4].parse().ok()?,
            line: n[5].parse().ok()?,
        });
    }
    Some(out)
}

/// Recognise through the cache (or directly when it is off).
pub fn recognize(
    page: &PageImage,
    dpi: u32,
    o: &OcrOptions,
) -> Result<Vec<Word>, legatorocr_ocr::OcrError> {
    let cfg = DIR.lock().ok().and_then(|d| d.clone());
    let Some((dir, salt)) = cfg else {
        return legatorocr_ocr::recognize(page, dpi, o);
    };
    let k = key(page, dpi, o, &salt);
    let file = dir.join(format!("{k}.txt"));
    if let Some(w) = std::fs::read_to_string(&file).ok().and_then(|s| decode(&s)) {
        HITS.fetch_add(1, Ordering::Relaxed);
        return Ok(w);
    }
    MISSES.fetch_add(1, Ordering::Relaxed);
    let words = legatorocr_ocr::recognize(page, dpi, o)?;
    // write-then-rename so a parallel run never reads half a file
    let tmp = dir.join(format!("{k}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, encode(&words)).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_survive_the_round_trip() {
        let w = vec![
            Word { text: "școala".into(), x: 1, y: 2, w: 30, h: 12, conf: 93.5, line: 0 },
            Word { text: "a\tb".into(), x: 40, y: 2, w: 8, h: 12, conf: -1.0, line: 1 },
        ];
        let back = decode(&encode(&w)).unwrap();
        assert_eq!(back[0], w[0]);
        assert_eq!(back[1].text, "a b");
        assert!(decode("garbage").is_none());
        assert!(decode("lgr-ocr-cache 1\n1 2 3\tx\n").is_none());
    }
}
