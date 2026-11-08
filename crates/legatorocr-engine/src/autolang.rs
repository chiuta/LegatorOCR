//! `--lang-auto ron,eng,fra`: choose the OCR language of every page from a short list.
//!
//! Every candidate reads the page; the candidate with the most *confidently read characters*
//! (the sum over words of confidence/100 times word length, words of at least two letters) wins.
//! A language that does not fit the page produces few words or low confidences, so it loses even
//! when it reads many words. Ties go to the first candidate. The page is read once per candidate,
//! so the time grows with the length of the list; with `--ocr-cache` a repeated run costs nothing.
//! Pages that `--page-lang` names, and zones with their own language, are not touched.

use legatorocr_core::{PageImage, Word};
use legatorocr_ocr::{OcrError, OcrOptions};
use std::sync::Mutex;

type Config = (String, Vec<String>); // (the general --ocr-lang, candidates)
static CFG: Mutex<Option<Config>> = Mutex::new(None);

/// Canonical, valid and distinct candidates, at least two.
pub fn candidates(list: &[String]) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for l in list {
        let c = legatorocr_ocr::canonical_lang(l.trim());
        if !legatorocr_ocr::valid_lang(&c) {
            anyhow::bail!("--lang-auto: invalid language {l:?} (e.g. ron or ron+eng)");
        }
        if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.len() < 2 {
        anyhow::bail!("--lang-auto needs at least two different languages");
    }
    Ok(out)
}

pub fn set(base_lang: &str, candidates: Vec<String>) {
    if let Ok(mut c) = CFG.lock() {
        *c = (candidates.len() > 1).then(|| (base_lang.to_string(), candidates));
    }
}

fn config() -> Option<Config> {
    CFG.lock().ok().and_then(|c| c.clone())
}

pub fn active() -> bool {
    config().is_some()
}

/// Number of characters the engine was sure about.
pub fn confident_chars(words: &[Word]) -> f32 {
    words
        .iter()
        .filter(|w| w.conf >= 0.0)
        .map(|w| {
            let n = w.text.chars().filter(|c| c.is_alphanumeric()).count();
            if n >= 2 {
                n as f32 * w.conf / 100.0
            } else {
                0.0
            }
        })
        .sum()
}

pub fn recognize(
    page: &PageImage,
    dpi: u32,
    o: &OcrOptions,
    inner: crate::ocrzones::Inner,
) -> Result<Vec<Word>, OcrError> {
    let Some((base, cands)) = config() else {
        return inner(page, dpi, o);
    };
    if o.lang != base {
        return inner(page, dpi, o); // a language chosen by --page-lang
    }
    let mut best: Option<(f32, Vec<Word>)> = None;
    let mut last_err = None;
    for l in &cands {
        let opt = OcrOptions {
            lang: l.clone(),
            ..o.clone()
        };
        match inner(page, dpi, &opt) {
            Ok(w) => {
                let s = confident_chars(&w);
                if best.as_ref().is_none_or(|(b, _)| s > *b) {
                    best = Some((s, w));
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    match (best, last_err) {
        (Some((_, w)), _) => Ok(w),
        (None, Some(e)) => Err(e),
        (None, None) => inner(page, dpi, o),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as M;
    static LOCK: M<()> = M::new(());

    fn w(t: &str, conf: f32) -> Word {
        Word {
            text: t.into(),
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            conf,
            line: 1,
        }
    }

    fn page() -> PageImage {
        PageImage::Gray(image::GrayImage::new(10, 10))
    }

    #[test]
    fn confident_characters_beat_many_doubtful_words() {
        let sure = [w("bonjour", 95.0), w("monde", 90.0)];
        let doubtful = [w("bon", 30.0), w("jo", 20.0), w("ur", 25.0), w("m", 99.0)];
        assert!(confident_chars(&sure) > 10.0);
        assert!(confident_chars(&doubtful) < 2.0);
        assert_eq!(confident_chars(&[w("x", 100.0), w("ab", -1.0)]), 0.0);
    }

    #[test]
    fn the_best_language_wins_and_page_lang_is_left_alone() {
        let _g = LOCK.lock().unwrap();
        set("ron", vec!["ron".into(), "eng".into(), "fra".into()]);
        // each language "reads" the page with different confidence
        let inner = |_: &PageImage, _: u32, o: &OcrOptions| {
            Ok(match o.lang.as_str() {
                "ron" => vec![w("xx", 20.0)],
                "eng" => vec![w("hello", 60.0)],
                _ => vec![w("bonjour", 96.0), w("monde", 92.0)],
            })
        };
        let base = OcrOptions {
            lang: "ron".into(),
            ..OcrOptions::default()
        };
        let got = recognize(&page(), 300, &base, &inner).unwrap();
        assert_eq!(got[0].text, "bonjour");
        // a language that --page-lang chose is not second-guessed
        let chosen = OcrOptions {
            lang: "eng".into(),
            ..OcrOptions::default()
        };
        let got = recognize(&page(), 300, &chosen, &inner).unwrap();
        assert_eq!(got[0].text, "hello");
        // a single candidate is no list: inactive
        set("ron", vec!["ron".into()]);
        assert!(!active());
        set("ron", vec![]);
    }

    #[test]
    fn candidate_lists_are_checked() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            candidates(&v(&["ron", "eng", "ron"])).unwrap(),
            v(&["ron", "eng"])
        );
        assert!(candidates(&v(&["ron"])).is_err());
        assert!(candidates(&v(&["ron", "e n g"])).is_err());
    }

    #[test]
    fn all_failing_candidates_give_the_error() {
        let _g = LOCK.lock().unwrap();
        set("ron", vec!["ron".into(), "eng".into()]);
        let inner = |_: &PageImage, _: u32, _: &OcrOptions| Err(OcrError::Failed("boom".into()));
        let base = OcrOptions {
            lang: "ron".into(),
            ..OcrOptions::default()
        };
        assert!(recognize(&page(), 300, &base, &inner).is_err());
        set("ron", vec![]);
    }
}
