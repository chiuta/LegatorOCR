//! Per-language word lists (gzip, one lowercase word per line, most frequent first) used to
//! repair isolated recognition errors: a word that is in none of the requested lists is
//! replaced by a one-letter variant the network itself considered plausible, when that variant
//! *is* in a list. Deliberately conservative: names, capitalised and all-caps words are left alone.

use crate::ctc::Glyph;
use std::collections::HashSet;
use std::sync::OnceLock;

#[cfg(feature = "lexicon")]
macro_rules! lex {
    ($code:literal) => {
        (
            $code,
            include_bytes!(concat!("../lexicons/", $code, ".txt.gz")) as &[u8],
        )
    };
}
/// The word lists, compiled in (CC BY-SA 4.0 data, see `lexicons/README.md`). Built without the
/// `lexicon` feature the lists are absent: no list is ever loaded and no error repair happens.
#[cfg(feature = "lexicon")]
const FILES: [(&str, &[u8]); 8] = [
    lex!("ro"),
    lex!("en"),
    lex!("fr"),
    lex!("it"),
    lex!("es"),
    lex!("pt"),
    lex!("de"),
    lex!("hu"),
];
#[cfg(not(feature = "lexicon"))]
const FILES: [(&str, &[u8]); 0] = [];

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
            #[allow(unused_mut)]
            let mut s = String::new();
            #[cfg(feature = "lexicon")]
            {
                use std::io::Read;
                let _ = flate2::read::GzDecoder::new(FILES[i].1).read_to_string(&mut s);
            }
            Lexicon {
                words: s
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect(),
            }
        }))
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn contains(&self, w: &str) -> bool {
        self.words.contains(&w.to_lowercase())
    }

    /// Range of a token without surrounding punctuation (`„word,` → `word`).
    fn core(w: &str) -> Option<(usize, usize)> {
        let a = w.char_indices().find(|(_, c)| c.is_alphabetic())?.0;
        let (b, c) = w.char_indices().rev().find(|(_, c)| c.is_alphabetic())?;
        Some((a, b + c.len_utf8()))
    }
}

/// `true` if the token's letters form a word of any of the lists.
pub fn known(lexicons: &[&Lexicon], token: &str) -> bool {
    match Lexicon::core(token) {
        Some((a, b)) => lexicons.iter().any(|l| l.contains(&token[a..b])),
        None => true, // digits and punctuation are not words
    }
}

/// Try to repair one token given its glyphs; `first_in_line` allows a leading capital.
/// Returns the corrected text, or `None` when the token is fine or nothing safe is found.
pub fn repair(
    lexicons: &[&Lexicon],
    text: &str,
    glyphs: &[&Glyph],
    first_in_line: bool,
) -> Option<String> {
    let (a, b) = Lexicon::core(text)?;
    let core = &text[a..b];
    let n = core.chars().count();
    if n < 3 || core.chars().any(|c| c.is_ascii_digit()) || known(lexicons, text) {
        return None;
    }
    // names, acronyms and camel-case are out of vocabulary by nature
    let upper = core.chars().filter(|c| c.is_uppercase()).count();
    let first_upper = core.chars().next().is_some_and(|c| c.is_uppercase());
    if upper > 1 || (first_upper && !first_in_line) {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() != glyphs.len() {
        return None;
    }
    let first = text[..a].chars().count();
    let mut best: Option<(f32, String)> = None;
    for (i, g) in glyphs.iter().enumerate() {
        if i < first || i >= first + n {
            continue;
        }
        let Some((alt, ap)) = g.alt else { continue };
        // only where the network was itself unsure, and the alternative is a real contender
        if g.p > 0.9 || ap < 0.08 || !alt.is_alphabetic() {
            continue;
        }
        // same case class as the letter it replaces
        if alt.is_uppercase() != chars[i].is_uppercase() && chars[i].is_alphabetic() {
            continue;
        }
        let mut cand = chars.clone();
        cand[i] = alt;
        let s: String = cand.iter().collect();
        let Some((ca, cb)) = Lexicon::core(&s) else {
            continue;
        };
        if known(lexicons, &s[ca..cb]) && best.as_ref().is_none_or(|(p, _)| ap > *p) {
            best = Some((ap, s));
        }
    }
    best.map(|(_, s)| s)
}

/// Language codes (`ro`, `en` …) of a Tesseract-style list `ron+eng`; unknown names are dropped.
pub fn codes(lang: &str) -> Vec<&'static str> {
    let mut v = Vec::new();
    for p in lang.split('+') {
        if let Some(c) = code_of(p) {
            if !v.contains(&c) {
                v.push(c);
            }
        }
    }
    v
}

fn code_of(p: &str) -> Option<&'static str> {
    Some(match p.to_ascii_lowercase().as_str() {
        "ron" | "ro" => "ro",
        "eng" | "en" => "en",
        "fra" | "fr" => "fr",
        "ita" | "it" => "it",
        "spa" | "es" => "es",
        "por" | "pt" => "pt",
        "deu" | "de" => "de",
        "hun" | "hu" => "hu",
        _ => return None,
    })
}

/// `true` when every name in the list is one of the eight languages the native engine reads.
pub fn all_supported(lang: &str) -> bool {
    !lang.is_empty() && lang.split('+').all(|p| code_of(p).is_some())
}
