//! `--doubtful-words FILE.tsv` (roadmap 4.3, as a flag list and not an automatic correction):
//! words that the language's word list does not contain **and** that OCR read with low
//! confidence, one line each with page, position and confidence, for a person to check first.
//! Nothing in the text is changed. Measured on real pages in ROADMAP.md.
use legatorocr_core::Word;
use legatorocr_nocr::lexicon::{self, Lexicon};
use std::sync::Mutex;

struct State {
    max_conf: f32,
    rows: Vec<String>,
    words: usize,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Start collecting; words under `max_conf` that are not in the word list are flagged.
pub fn begin(max_conf: f32) {
    *STATE.lock().unwrap() = Some(State {
        max_conf,
        rows: Vec::new(),
        words: 0,
    });
}

pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

/// Flag the doubtful words of one page (`page` counts from 1; `lang` as for OCR, e.g. `ron+eng`).
pub fn record(page: usize, lang: &str, words: &[Word]) {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return };
    let lex: Vec<&Lexicon> = lexicon::codes(lang)
        .iter()
        .filter_map(|c| Lexicon::load(c))
        .filter(|l| !l.is_empty())
        .collect();
    if lex.is_empty() {
        return; // no word list for this language: nothing can be judged
    }
    for w in words {
        let t = w.text.trim();
        if t.chars().filter(|c| c.is_alphabetic()).count() < 3 {
            continue;
        }
        st.words += 1;
        if w.conf < st.max_conf && !lexicon::known(&lex, t) {
            st.rows.push(format!(
                "{page}\t{}\t{}\t{}\t{}\t{:.0}\t{}",
                w.x,
                w.y,
                w.w,
                w.h,
                w.conf,
                t.replace(['\t', '\n'], " ")
            ));
        }
    }
}

/// The list as TSV text (header line first) and (flagged, words judged).
pub fn finish() -> Option<(String, usize, usize)> {
    let st = STATE.lock().unwrap().take()?;
    let n = st.rows.len();
    let mut out = String::from("page\tx\ty\tw\th\tconf\ttext\n");
    for r in &st.rows {
        out.push_str(r);
        out.push('\n');
    }
    Some((out, n, st.words))
}

#[cfg(all(test, feature = "lexicon"))]
mod tests {
    use super::*;

    fn w(text: &str, conf: f32) -> Word {
        Word {
            text: text.to_string(),
            x: 1,
            y: 2,
            w: 30,
            h: 10,
            conf,
            line: 0,
        }
    }

    #[test]
    fn only_low_confidence_words_outside_the_word_list_are_flagged() {
        begin(85.0);
        record(
            3,
            "ron",
            &[
                w("casa", 50.0),
                w("xqzvkp", 95.0),
                w("xqzvkp,", 60.0),
                w("12", 10.0),
                w("și", 40.0),
            ],
        );
        let (tsv, n, judged) = finish().unwrap();
        assert_eq!(n, 1, "{tsv}");
        assert_eq!(judged, 3, "digits and 2-letter words are not judged");
        assert!(
            tsv.lines()
                .nth(1)
                .unwrap()
                .starts_with("3\t1\t2\t30\t10\t60\txqzvkp,"),
            "{tsv}"
        );
        // a language without a word list: nothing is judged
        begin(85.0);
        record(1, "ara", &[w("xqzvkp", 10.0)]);
        assert_eq!(finish().unwrap().1, 0);
        assert!(finish().is_none());
    }
}
