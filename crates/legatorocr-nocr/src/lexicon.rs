//! Per-language word lists (gzip, one lowercase word per line, most frequent first) used to
//! repair isolated recognition errors: a word that is not in the list is replaced by a one-letter
//! variant the network itself considered plausible, when that variant *is* in the list.

use crate::ctc::Glyph;
use flate2::read::GzDecoder;
use std::collections::HashSet;
use std::io::Read;
use std::sync::OnceLock;

macro_rules! lex {
    ($code:literal) => {
        ($code, include_bytes!(concat!("../lexicons/", $code, ".txt.gz")) as &[u8])
    };
}
const FILES: [(&str, &[u8]); 8] =
    [lex!("ro"), lex!("en"), lex!("fr"), lex!("it"), lex!("es"), lex!("pt"), lex!("de"), lex!("hu")];

pub struct Lexicon {
    words: HashSet<String>,
}

impl Lexicon {
    pub fn load(code: &str) -> Option<&'static Lexicon> {
        static CELLS: [OnceLock<Lexicon>; 8] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        let i = FILES.iter().position(|(c, _)| *c == code)?;
        Some(CELLS[i].get_or_init(|| {
            let mut s = String::new();
            let _ = GzDecoder::new(FILES[i].1).read_to_string(&mut s);
            Lexicon { words: s.lines().map(|l| l.to_string()).collect() }
        }))
    }

    pub fn contains(&self, w: &str) -> bool {
        self.words.contains(&w.to_lowercase())
    }

    /// Core of a token without surrounding punctuation (`„word,` → `word`) as a range.
    fn core(w: &str) -> Option<(usize, usize)> {
        let a = w.char_indices().find(|(_, c)| c.is_alphabetic())?.0;
        let (b, c) = w.char_indices().rev().find(|(_, c)| c.is_alphabetic())?;
        Some((a, b + c.len_utf8()))
    }

    /// Try to repair one word given its glyphs; returns the corrected text or `None`.
    pub fn repair(&self, text: &str, glyphs: &[&Glyph]) -> Option<String> {
        let (a, b) = Self::core(text)?;
        let core = &text[a..b];
        if core.chars().count() < 3 || core.chars().any(|c| c.is_ascii_digit()) || self.contains(core) {
            return None;
        }
        // glyph index of each char: words are built from exactly these glyphs
        let chars: Vec<char> = text.chars().collect();
        if chars.len() != glyphs.len() {
            return None;
        }
        let first = text[..a].chars().count();
        let mut best: Option<(f32, String)> = None;
        for (i, g) in glyphs.iter().enumerate() {
            if i < first || i >= first + core.chars().count() {
                continue;
            }
            let Some((alt, ap)) = g.alt else { continue };
            if ap < 0.02 || !alt.is_alphabetic() {
                continue;
            }
            let mut cand: Vec<char> = chars.clone();
            cand[i] = alt;
            let s: String = cand.iter().collect();
            let (ca, cb) = Self::core(&s)?;
            if self.contains(&s[ca..cb]) && best.as_ref().is_none_or(|(p, _)| ap > *p) {
                best = Some((ap, s));
            }
        }
        best.map(|(_, s)| s)
    }
}

/// Language codes (`ro`, `en` …) of a Tesseract-style list `ron+eng`.
pub fn codes(lang: &str) -> Vec<&'static str> {
    let mut v = Vec::new();
    for p in lang.split('+') {
        let c = match p.to_ascii_lowercase().as_str() {
            "ron" | "ro" => "ro",
            "eng" | "en" => "en",
            "fra" | "fr" => "fr",
            "ita" | "it" => "it",
            "spa" | "es" => "es",
            "por" | "pt" => "pt",
            "deu" | "de" => "de",
            "hun" | "hu" => "hu",
            _ => continue,
        };
        if !v.contains(&c) {
            v.push(c);
        }
    }
    v
}
