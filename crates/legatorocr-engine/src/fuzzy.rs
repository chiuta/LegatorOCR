//! Diacritic-insensitive, fuzzy full-text matching with a DocFetcher-style query language
//! (ROADMAP 1.10 and 1.19).
//!
//! Query language (one query string, evaluated against one text, e.g. one line or one page):
//!
//! | syntax              | meaning                                                           |
//! |---------------------|-------------------------------------------------------------------|
//! | `word`              | whole word, case- and diacritic-insensitive (`Brasov` = `Brașov`) |
//! | `a b`, `a AND b`    | both must occur (implicit AND)                                    |
//! | `a OR b`            | either (OR binds weaker than AND)                                 |
//! | `NOT a`, `-a`       | `a` must not occur                                                |
//! | `+a`                | `a` must occur (same as AND; accepted for Lucene habits)          |
//! | `"two words"`       | exact phrase: consecutive words, any punctuation between them     |
//! | `fac*`, `?actura`   | wildcards inside a word: `*` = any run, `?` = one character      |
//! | `/regex/`           | regular expression over the folded text (see below)               |
//! | `word~1`, `"a b"~2` | fuzzy: Levenshtein distance up to N (0..=2) per word              |
//! | `( ... )`           | grouping                                                          |
//!
//! `AND`, `OR`, `NOT` are operators only in capitals; `si`, `or`, `not` are ordinary words.
//!
//! Matching runs on the *folded* text ([`fold`]): lower case, Romanian diacritics and Latin
//! accents removed. Hit spans are mapped back to **byte offsets of the original text**, also
//! when folding changes byte lengths (`ș` is 2 bytes, `s` is 1; `ß` becomes `ss`; decomposed
//! accents vanish).
//!
//! Regex notes: patterns are matched against the folded text, so write them without
//! diacritics. Non-ASCII literal characters in the pattern are folded for you (`/brașov/`
//! works) but character classes such as `[ăâ]` are not rewritten. Matching is
//! case-insensitive. The `regex` crate is automaton based (no backtracking), so catastrophic
//! patterns like `(a+)+$` run in linear time; on top of that the compiled size, nesting
//! depth and pattern length are capped, which is what protects against pathological patterns.

use legatorocr_core::text::fold as core_fold;
use regex::{Regex, RegexBuilder};

/// Highest fuzzy distance accepted by `~N`.
pub const MAX_FUZZY: usize = 2;
/// Longest accepted query, in bytes.
pub const MAX_QUERY_LEN: usize = 2000;
/// Longest accepted `/regex/` pattern, in bytes.
pub const MAX_REGEX_LEN: usize = 512;
/// Cap on hits returned by one [`matches`] call.
pub const MAX_HITS: usize = 100_000;
const MAX_DEPTH: usize = 16;
const MAX_TERMS: usize = 64;
const MAX_PHRASE_WORDS: usize = 32;
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A match: byte span `start..end` in the ORIGINAL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub start: usize,
    pub end: usize,
    /// Edit distance of a fuzzy word match (0 for exact, wildcard and regex hits).
    pub distance: usize,
}

/// Lower-case and strip Romanian diacritics and Latin accents (`Brașov` -> `brasov`).
pub fn fold(s: &str) -> String {
    // twice: lower-casing can itself produce a combining mark (`İ` -> `i` + U+0307)
    core_fold(&core_fold(s))
}

/// Levenshtein distance between `a` and `b` (in `char`s, no folding), or `None` when it
/// exceeds `max`. Stops early, so it is cheap even for long strings.
pub fn levenshtein_bounded(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
            row_min = row_min.min(cur[j]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[m] <= max).then_some(prev[m])
}

// ---------------------------------------------------------------------------------------
// Query AST
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Term {
    /// One or more consecutive folded words (each may contain `*` / `?` when `dist == 0`).
    Words {
        words: Vec<String>,
        dist: usize,
    },
    Regex(Regex),
}

#[derive(Debug, Clone)]
enum Node {
    Term(Term),
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
}

/// A parsed query. Build with [`parse_query`].
#[derive(Debug, Clone)]
pub struct Query {
    root: Node,
}

// ---------------------------------------------------------------------------------------
// Lexer / parser
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
enum Tok {
    LParen,
    RParen,
    Plus,
    Minus,
    And,
    Or,
    Not,
    Term(Term),
}

fn make_words(raw: &str, dist: usize) -> Result<Term, String> {
    let f = fold(raw);
    let words: Vec<String> = f
        .split(|c: char| !(c.is_alphanumeric() || c == '*' || c == '?'))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return Err(format!("term without letters or digits: {raw:?}"));
    }
    if words.len() > MAX_PHRASE_WORDS {
        return Err(format!("phrase longer than {MAX_PHRASE_WORDS} words"));
    }
    if dist > 0 && words.iter().any(|w| w.contains(['*', '?'])) {
        return Err("fuzzy (~N) cannot be combined with wildcards".into());
    }
    Ok(Term::Words { words, dist })
}

fn make_regex(pat: &str) -> Result<Term, String> {
    if pat.is_empty() {
        return Err("empty regular expression //".into());
    }
    if pat.len() > MAX_REGEX_LEN {
        return Err(format!(
            "regular expression longer than {MAX_REGEX_LEN} bytes"
        ));
    }
    // fold non-ASCII literals; copy escapes (`\S`, `\B`...) verbatim
    let mut p = String::with_capacity(pat.len());
    let mut it = pat.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            p.push(c);
            if let Some(n) = it.next() {
                p.push(n);
            }
        } else if c.is_ascii() {
            p.push(c);
        } else {
            p.push_str(&fold(&c.to_string()));
        }
    }
    let re = RegexBuilder::new(&p)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT * 4)
        .nest_limit(32)
        .build()
        .map_err(|e| format!("bad regular expression: {e}"))?;
    Ok(Term::Regex(re))
}

/// After a term: optional `~N`, then a boundary (space, paren, end).
fn read_fuzzy(ch: &[char], i: &mut usize) -> Result<usize, String> {
    let mut dist = 0;
    if ch.get(*i) == Some(&'~') {
        *i += 1;
        let st = *i;
        while ch.get(*i).is_some_and(|c| c.is_ascii_digit()) {
            *i += 1;
        }
        if st == *i || *i - st > 2 {
            return Err("fuzzy suffix must be ~0, ~1 or ~2".into());
        }
        dist = ch[st..*i]
            .iter()
            .collect::<String>()
            .parse::<usize>()
            .unwrap_or(99);
        if dist > MAX_FUZZY {
            return Err(format!(
                "fuzzy distance {dist} is above the maximum {MAX_FUZZY}"
            ));
        }
    }
    match ch.get(*i) {
        None | Some('(') | Some(')') => Ok(dist),
        Some(c) if c.is_whitespace() => Ok(dist),
        Some(c) => Err(format!("unexpected {c:?} after a term")),
    }
}

fn lex(q: &str) -> Result<Vec<Tok>, String> {
    let ch: Vec<char> = q.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut terms = 0;
    while i < ch.len() {
        let c = ch[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '+' | '-' => {
                let nxt = ch.get(i + 1);
                if nxt.is_none_or(|n| n.is_whitespace() || *n == ')') {
                    return Err(format!("operator {c:?} without an operand"));
                }
                out.push(if c == '+' { Tok::Plus } else { Tok::Minus });
                i += 1;
            }
            '"' => {
                let st = i + 1;
                let end = ch[st..]
                    .iter()
                    .position(|&x| x == '"')
                    .ok_or("unterminated quote \"")?
                    + st;
                let body: String = ch[st..end].iter().collect();
                i = end + 1;
                let dist = read_fuzzy(&ch, &mut i)?;
                out.push(Tok::Term(make_words(&body, dist)?));
                terms += 1;
            }
            '/' => {
                let mut j = i + 1;
                let mut pat = String::new();
                loop {
                    match ch.get(j) {
                        None => return Err("unterminated regular expression /".into()),
                        Some('\\') if ch.get(j + 1) == Some(&'/') => {
                            pat.push('/');
                            j += 2;
                        }
                        Some('\\') => {
                            pat.push('\\');
                            if let Some(n) = ch.get(j + 1) {
                                pat.push(*n);
                            }
                            j += 2;
                        }
                        Some('/') => break,
                        Some(&x) => {
                            pat.push(x);
                            j += 1;
                        }
                    }
                }
                i = j + 1;
                if ch.get(i) == Some(&'~') {
                    return Err("fuzzy (~N) cannot be applied to a regular expression".into());
                }
                read_fuzzy(&ch, &mut i)?;
                out.push(Tok::Term(make_regex(&pat)?));
                terms += 1;
            }
            _ => {
                let st = i;
                while i < ch.len() && !ch[i].is_whitespace() && ch[i] != '(' && ch[i] != ')' {
                    i += 1;
                }
                let word: String = ch[st..i].iter().collect();
                match word.as_str() {
                    "AND" => out.push(Tok::And),
                    "OR" => out.push(Tok::Or),
                    "NOT" => out.push(Tok::Not),
                    _ => {
                        let (body, dist) = match word.rfind('~') {
                            Some(p) => {
                                let mut k = 0;
                                let tail: Vec<char> = word[p..].chars().collect();
                                let d = read_fuzzy(&tail, &mut k)?;
                                (&word[..p], d)
                            }
                            None => (word.as_str(), 0),
                        };
                        out.push(Tok::Term(make_words(body, dist)?));
                        terms += 1;
                    }
                }
            }
        }
        if terms > MAX_TERMS {
            return Err(format!("more than {MAX_TERMS} terms"));
        }
    }
    Ok(out)
}

struct Parser {
    toks: std::iter::Peekable<std::vec::IntoIter<Tok>>,
    depth: usize,
}

impl Parser {
    fn or(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.and()?];
        while matches!(self.toks.peek(), Some(Tok::Or)) {
            self.toks.next();
            alts.push(self.and()?);
        }
        Ok(if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            Node::Or(alts)
        })
    }

    fn and(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        let mut need = false;
        loop {
            match self.toks.peek() {
                None | Some(Tok::Or) | Some(Tok::RParen) => break,
                Some(Tok::And) => {
                    if items.is_empty() || need {
                        return Err("AND without a left operand".into());
                    }
                    self.toks.next();
                    need = true;
                }
                _ => {
                    items.push(self.unary()?);
                    need = false;
                }
            }
        }
        if need {
            return Err("AND without a right operand".into());
        }
        match items.len() {
            0 => Err("missing operand (empty query, or OR with nothing beside it)".into()),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(Node::And(items)),
        }
    }

    fn unary(&mut self) -> Result<Node, String> {
        match self.toks.next() {
            Some(Tok::Not) | Some(Tok::Minus) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err("query nested too deeply".into());
                }
                let n = self.unary()?;
                self.depth -= 1;
                Ok(Node::Not(Box::new(n)))
            }
            Some(Tok::Plus) => self.unary(),
            Some(Tok::LParen) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err("query nested too deeply".into());
                }
                let n = self.or()?;
                match self.toks.next() {
                    Some(Tok::RParen) => {}
                    _ => return Err("missing closing parenthesis".into()),
                }
                self.depth -= 1;
                Ok(n)
            }
            Some(Tok::Term(t)) => Ok(Node::Term(t)),
            Some(Tok::RParen) => Err("unexpected ')'".into()),
            Some(Tok::And) | Some(Tok::Or) => Err("operator in the wrong place".into()),
            None => Err("operator without an operand".into()),
        }
    }
}

/// Parse a query string (see the module docs). Errors are human-readable.
pub fn parse_query(q: &str) -> Result<Query, String> {
    if q.len() > MAX_QUERY_LEN {
        return Err(format!("query longer than {MAX_QUERY_LEN} bytes"));
    }
    let toks = lex(q)?;
    if toks.is_empty() {
        return Err("empty query".into());
    }
    let mut p = Parser {
        toks: toks.into_iter().peekable(),
        depth: 0,
    };
    let root = p.or()?;
    if p.toks.next().is_some() {
        return Err("unbalanced parenthesis".into());
    }
    Ok(Query { root })
}

// ---------------------------------------------------------------------------------------
// Folding with an offset map, and matching
// ---------------------------------------------------------------------------------------

/// Folded text plus, for every folded `char`, where it came from in the original.
struct Doc {
    f: String,
    /// `(folded byte start, original byte start, original byte end)`, ascending.
    map: Vec<(usize, usize, usize)>,
    /// Byte ranges of words (maximal alphanumeric runs) in `f`.
    toks: Vec<(usize, usize)>,
}

impl Doc {
    fn new(text: &str) -> Doc {
        let mut f = String::with_capacity(text.len());
        let mut map: Vec<(usize, usize, usize)> = Vec::with_capacity(text.len());
        let mut buf = [0u8; 4];
        for (i, c) in text.char_indices() {
            let oend = i + c.len_utf8();
            let folded = fold(c.encode_utf8(&mut buf));
            if folded.is_empty() {
                // combining mark: belongs to the previous character
                if let Some(last) = map.last_mut() {
                    last.2 = oend;
                }
                continue;
            }
            for fc in folded.chars() {
                map.push((f.len(), i, oend));
                f.push(fc);
            }
        }
        let mut toks = Vec::new();
        let mut st: Option<usize> = None;
        for (i, c) in f.char_indices() {
            if c.is_alphanumeric() {
                st.get_or_insert(i);
            } else if let Some(s) = st.take() {
                toks.push((s, i));
            }
        }
        if let Some(s) = st {
            toks.push((s, f.len()));
        }
        Doc { f, map, toks }
    }

    /// Folded byte span (non-empty) -> original byte span.
    fn orig(&self, a: usize, b: usize) -> (usize, usize) {
        let i = self.map.partition_point(|e| e.0 <= a).saturating_sub(1);
        let j = self.map.partition_point(|e| e.0 < b).saturating_sub(1);
        (self.map[i].1, self.map[j.max(i)].2)
    }
}

/// `*` / `?` glob over chars (iterative, O(n*m) worst case, never recursive).
fn glob(pat: &[char], s: &[char]) -> bool {
    let (mut p, mut i) = (0, 0);
    let (mut star, mut mark) = (usize::MAX, 0);
    while i < s.len() {
        if p < pat.len() && pat[p] == '*' {
            star = p;
            mark = i;
            p += 1;
        } else if p < pat.len() && (pat[p] == '?' || pat[p] == s[i]) {
            p += 1;
            i += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == '*' {
        p += 1;
    }
    p == pat.len()
}

/// Does the folded query word match the folded document word? Returns the distance.
fn word_matches(pat: &str, dist: usize, tok: &str) -> Option<usize> {
    if pat.contains(['*', '?']) {
        let p: Vec<char> = pat.chars().collect();
        let t: Vec<char> = tok.chars().collect();
        glob(&p, &t).then_some(0)
    } else if dist == 0 {
        (pat == tok).then_some(0)
    } else {
        levenshtein_bounded(pat, tok, dist)
    }
}

fn eval_term(t: &Term, d: &Doc, out: &mut Vec<Hit>) {
    match t {
        Term::Words { words, dist } => {
            let n = words.len();
            if n > d.toks.len() {
                return;
            }
            for s in 0..=d.toks.len() - n {
                let mut worst = 0;
                let mut ok = true;
                for (k, w) in words.iter().enumerate() {
                    let (a, b) = d.toks[s + k];
                    match word_matches(w, *dist, &d.f[a..b]) {
                        Some(x) => worst = worst.max(x),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let (os, oe) = d.orig(d.toks[s].0, d.toks[s + n - 1].1);
                    out.push(Hit {
                        start: os,
                        end: oe,
                        distance: worst,
                    });
                    if out.len() >= MAX_HITS {
                        return;
                    }
                }
            }
        }
        Term::Regex(re) => {
            for m in re.find_iter(&d.f) {
                if m.is_empty() {
                    continue;
                }
                let (os, oe) = d.orig(m.start(), m.end());
                out.push(Hit {
                    start: os,
                    end: oe,
                    distance: 0,
                });
                if out.len() >= MAX_HITS {
                    return;
                }
            }
        }
    }
}

/// `Some(hits)` when the node holds for the document (hits may be empty for NOT-only nodes).
fn eval(n: &Node, d: &Doc) -> Option<Vec<Hit>> {
    match n {
        Node::Term(t) => {
            let mut v = Vec::new();
            eval_term(t, d, &mut v);
            (!v.is_empty()).then_some(v)
        }
        Node::Not(inner) => match eval(inner, d) {
            Some(_) => None,
            None => Some(Vec::new()),
        },
        Node::And(items) => {
            let mut all = Vec::new();
            for it in items {
                all.extend(eval(it, d)?);
            }
            Some(all)
        }
        Node::Or(items) => {
            let mut found = false;
            let mut all = Vec::new();
            for it in items {
                if let Some(h) = eval(it, d) {
                    found = true;
                    all.extend(h);
                }
            }
            found.then_some(all)
        }
    }
}

fn run(q: &Query, text: &str) -> Option<Vec<Hit>> {
    let d = Doc::new(text);
    let mut h = eval(&q.root, &d)?;
    h.sort();
    h.dedup();
    h.truncate(MAX_HITS);
    Some(h)
}

/// Does the whole text satisfy the query? (True for NOT-only queries on text without the
/// negated terms, where [`matches`] has no spans to return.)
pub fn is_match(q: &Query, text: &str) -> bool {
    run(q, text).is_some()
}

/// Spans (byte offsets into the original `text`, sorted, de-duplicated) of every positive
/// term occurrence, provided the text satisfies the whole boolean query; otherwise empty.
/// Terms under NOT never produce hits. Use [`is_match`] to tell "no match" from
/// "matches but only by absence".
pub fn matches(q: &Query, text: &str) -> Vec<Hit> {
    run(q, text).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// Romanian CUI / invoice series
// ---------------------------------------------------------------------------------------

/// Official CUI control-digit check: digits of the body (all but the last) are multiplied,
/// right-aligned, with the key 7 5 3 2 1 7 5 3 2; the sum times 10 modulo 11 (10 -> 1) must
/// equal the last digit. Accepts an optional `RO` prefix (any case) and spaces after it;
/// 2 to 10 digits.
pub fn valid_cui(s: &str) -> bool {
    const KEY: [u32; 9] = [7, 5, 3, 2, 1, 7, 5, 3, 2];
    let t = s.trim();
    let t = match t.get(..2) {
        Some(p) if p.eq_ignore_ascii_case("ro") => t[2..].trim_start(),
        _ => t,
    };
    if !(2..=10).contains(&t.len()) || !t.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let d: Vec<u32> = t.bytes().map(|b| u32::from(b - b'0')).collect();
    let (body, control) = d.split_at(d.len() - 1);
    let sum: u32 = body
        .iter()
        .rev()
        .zip(KEY.iter().rev())
        .map(|(x, k)| x * k)
        .sum();
    let mut c = sum * 10 % 11;
    if c == 10 {
        c = 1;
    }
    c == control[0]
}

/// Query string (a `/regex/` term for [`parse_query`]) for CUI / CIF candidates: optional
/// `RO`, 2-10 digits. It cannot check the control digit; use [`find_cui`] for that.
pub fn pattern_cui() -> &'static str {
    r"/\b(?:ro\s?)?[0-9]{2,10}\b/"
}

/// Query string for invoice series and numbers such as `FCT 0123`, `FA-2026/00045`,
/// `SRV123456`: 2-5 letters, optional space or dash, optional `YYYY/`, 3-8 digits.
/// Expect false positives on other letters+digits codes.
pub fn pattern_invoice_series() -> &'static str {
    r"/\b[a-z]{2,5}[ \-]?(?:[0-9]{4}\/)?[0-9]{3,8}\b/"
}

/// CUI candidates found by [`pattern_cui`] whose control digit is valid (original spans).
pub fn find_cui(text: &str) -> Vec<Hit> {
    let q = parse_query(pattern_cui()).expect("built-in pattern");
    matches(&q, text)
        .into_iter()
        .filter(|h| valid_cui(&text[h.start..h.end]))
        .collect()
}

/// Invoice series/number candidates found by [`pattern_invoice_series`].
pub fn find_invoice_series(text: &str) -> Vec<Hit> {
    let q = parse_query(pattern_invoice_series()).expect("built-in pattern");
    matches(&q, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans<'a>(q: &str, t: &'a str) -> Vec<&'a str> {
        let q = parse_query(q).unwrap();
        matches(&q, t).iter().map(|h| &t[h.start..h.end]).collect()
    }

    #[test]
    fn fold_basics() {
        assert_eq!(
            fold("Brașov Brașov Țară ţară ĂÂÎ"),
            "brasov brasov tara tara aai"
        );
        assert_eq!(fold("Brasov"), fold("BRAȘOV"));
        assert_eq!(fold("café Müller"), "cafe muller");
    }

    #[test]
    fn levenshtein() {
        assert_eq!(levenshtein_bounded("abc", "abc", 0), Some(0));
        assert_eq!(levenshtein_bounded("abc", "abd", 1), Some(1));
        assert_eq!(levenshtein_bounded("abc", "abd", 0), None);
        assert_eq!(levenshtein_bounded("kitten", "sitting", 3), Some(3));
        assert_eq!(levenshtein_bounded("kitten", "sitting", 2), None);
        assert_eq!(levenshtein_bounded("", "ab", 2), Some(2));
        assert_eq!(levenshtein_bounded("", "abc", 2), None);
        assert_eq!(levenshtein_bounded("țară", "tara", 0), None); // no folding inside
    }

    #[test]
    fn offsets_map_back_through_length_changes() {
        // ș, ț, ă are 2 bytes in the original and 1 in the folded text
        let t = "Orașul Brașov și Țara Românească, ăștia";
        assert_eq!(spans("brasov", t), vec!["Brașov"]);
        assert_eq!(spans("BRASOV", t), vec!["Brașov"]);
        assert_eq!(spans("si", t), vec!["și"]);
        assert_eq!(spans("tara", t), vec!["Țara"]);
        assert_eq!(spans("romaneasca", t), vec!["Românească"]);
        assert_eq!(spans("astia", t), vec!["ăștia"]);
        assert_eq!(spans("\"brasov si tara\"", t), vec!["Brașov și Țara"]);
        // cedilla variants and a word after many multibyte chars
        let t2 = "ăăăăă ţară ŞTEFAN şi";
        assert_eq!(spans("tara", t2), vec!["ţară"]);
        assert_eq!(spans("stefan", t2), vec!["ŞTEFAN"]);
        assert_eq!(spans("si", t2), vec!["şi"]);
    }

    #[test]
    fn offsets_with_expanding_and_combining() {
        let t = "Die Straße ist lang";
        assert_eq!(spans("strasse", t), vec!["Straße"]);
        let t = "Bras\u{0327}ov"; // s + combining cedilla
        assert_eq!(spans("brasov", t), vec![t]);
        let t = "x İstanbul y";
        assert_eq!(spans("istanbul", t), vec!["İstanbul"]);
        // regex spans map back too
        let t = "în Brașov, în Țară";
        assert_eq!(spans("/bra[a-z]+/", t), vec!["Brașov"]);
        assert_eq!(spans("/t[a-z]{3}/", t), vec!["Țară"]);
    }

    #[test]
    fn boolean_operators() {
        let t = "factura emisa pentru Brașov";
        assert!(is_match(&parse_query("factura brasov").unwrap(), t));
        assert!(is_match(&parse_query("factura AND brasov").unwrap(), t));
        assert!(!is_match(&parse_query("factura AND cluj").unwrap(), t));
        assert!(is_match(&parse_query("cluj OR brasov").unwrap(), t));
        assert!(!is_match(&parse_query("cluj OR iasi").unwrap(), t));
        assert!(!is_match(&parse_query("factura NOT brasov").unwrap(), t));
        assert!(is_match(&parse_query("factura NOT cluj").unwrap(), t));
        assert!(is_match(&parse_query("factura -cluj").unwrap(), t));
        assert!(!is_match(&parse_query("factura -brasov").unwrap(), t));
        assert!(is_match(&parse_query("+factura +brasov").unwrap(), t));
        // precedence: AND binds tighter than OR
        assert!(is_match(&parse_query("cluj factura OR brasov").unwrap(), t));
        assert!(!is_match(
            &parse_query("cluj (factura OR brasov)").unwrap(),
            t
        ));
        assert!(is_match(
            &parse_query("(cluj OR brasov) factura").unwrap(),
            t
        ));
        // hits only from positive terms
        assert_eq!(spans("factura -cluj", t), vec!["factura"]);
        // NOT-only: matches but has no spans
        let q = parse_query("NOT cluj").unwrap();
        assert!(is_match(&q, t));
        assert!(matches(&q, t).is_empty());
        // lower-case operator words are ordinary words
        assert!(is_match(&parse_query("emisa pentru").unwrap(), t));
        assert!(!is_match(&parse_query("emisa or cluj").unwrap(), t));
    }

    #[test]
    fn whole_words_and_phrases() {
        let t = "Facturare facturi factura. Total factura-proforma";
        assert_eq!(spans("factura", t), vec!["factura", "factura"]);
        assert_eq!(spans("\"factura proforma\"", t), vec!["factura-proforma"]);
        assert!(!is_match(&parse_query("\"proforma factura\"").unwrap(), t));
        assert_eq!(spans("\"total factura\"", t), vec!["Total factura"]);
    }

    #[test]
    fn wildcards() {
        let t = "factura facturi fact facturare afactura";
        assert_eq!(
            spans("fact*", t),
            vec!["factura", "facturi", "fact", "facturare"]
        );
        assert_eq!(spans("factur?", t), vec!["factura", "facturi"]);
        assert_eq!(spans("*actura", t), vec!["factura", "afactura"]);
        assert_eq!(spans("f*r?", t), vec!["factura", "facturi", "facturare"]);
        assert_eq!(spans("f*re", t), vec!["facturare"]);
        assert_eq!(spans("fact*", "Facturăm Țară"), vec!["Facturăm"]);
        assert_eq!(spans("?ara", "Țară"), vec!["Țară"]);
        assert!(glob(&['*'], &[]));
        assert!(!glob(&['?'], &[]));
    }

    #[test]
    fn fuzzy_words() {
        let t = "Brasov Brașov Brasow Brsv";
        assert_eq!(spans("brasov", t).len(), 2);
        assert_eq!(spans("brasov~1", t), vec!["Brasov", "Brașov", "Brasow"]);
        assert_eq!(spans("brasov~2", t).len(), 4);
        let q = parse_query("brasov~1").unwrap();
        let h = matches(&q, "Brasow");
        assert_eq!(h[0].distance, 1);
        assert_eq!(spans("\"mare ora\"~1", "Marea oră"), vec!["Marea oră"]);
        assert!(parse_query("brasov~3").is_err());
        assert!(parse_query("brasov~").is_err());
        assert!(parse_query("bra*~1").is_err());
        assert!(parse_query("/abc/~1").is_err());
    }

    #[test]
    fn regex_terms() {
        let t = "Cod RO 12345 si cod 678";
        assert_eq!(spans(r"/\d{3,}/", t), vec!["12345", "678"]);
        assert_eq!(spans(r"/ro\s\d+/", t), vec!["RO 12345"]);
        assert_eq!(spans("/a\\/b/", "x a/b y"), vec!["a/b"]);
        assert_eq!(spans("/brașov/", "Orașul Brasov"), vec!["Brasov"]);
        assert!(parse_query("/(/").is_err());
        assert!(parse_query("/abc").is_err());
        assert!(parse_query("//").is_err());
        assert!(parse_query("/a/b").is_err());
        // empty matches are ignored
        assert!(matches(&parse_query("/x*/").unwrap(), "abc").is_empty());
    }

    #[test]
    fn regex_limits_not_redos() {
        let hay = format!("{}b", "a".repeat(200_000));
        let t0 = std::time::Instant::now();
        let q = parse_query("/(a+)+$/").unwrap();
        assert!(matches(&q, &hay).is_empty());
        let q = parse_query("/(x+x+)+y/").unwrap();
        assert!(matches(&q, &hay).is_empty());
        assert!(t0.elapsed().as_secs() < 10, "regex search must stay linear");
        // compiled-size and length caps
        assert!(parse_query("/((a{100}){100}){100}/").is_err());
        assert!(parse_query(&format!("/{}/", "a".repeat(MAX_REGEX_LEN + 1))).is_err());
        assert!(parse_query(&format!("/{}{}/", "(".repeat(100), ")".repeat(100))).is_err());
        assert!(parse_query(&"a ".repeat(MAX_QUERY_LEN)).is_err());
    }

    #[test]
    fn parse_errors() {
        for bad in [
            "",
            "   ",
            "AND",
            "OR a",
            "a OR",
            "a AND",
            "NOT",
            "(",
            ")",
            "(a",
            "a)",
            "\"abc",
            "-",
            "+ a",
            "a - b",
            "...",
            "()",
            "a AND AND b",
            "\"\"",
        ] {
            assert!(parse_query(bad).is_err(), "should fail: {bad:?}");
        }
        let deep = format!("{}a{}", "(".repeat(40), ")".repeat(40));
        assert!(parse_query(&deep).is_err());
        assert!(parse_query(&"NOT ".repeat(40)).is_err());
    }

    #[test]
    fn cui_checksum() {
        // 14399840: body 1439984 against key tail 3 2 1 7 5 3 2:
        // 3+8+3+63+45+24+8 = 154; 154*10 mod 11 = 0 = control digit. Verified by hand.
        assert!(valid_cui("14399840"));
        assert!(valid_cui("RO14399840"));
        assert!(valid_cui("ro 14399840"));
        assert!(valid_cui(" 14399840 "));
        for last in 1..=9 {
            assert!(!valid_cui(&format!("1439984{last}")), "{last}");
        }
        assert!(!valid_cui("1"));
        assert!(!valid_cui("12345678901"));
        assert!(!valid_cui("RO"));
        assert!(!valid_cui("12a45"));
        assert!(!valid_cui(""));
        // control value 10 maps to 1: body "5" -> 5*2=10 -> 100 mod 11 = 1 -> "51"
        assert!(valid_cui("51"));
        assert!(!valid_cui("50"));
        // for the 10-digit-or-shorter bodies below, exactly one control digit is valid
        for body in ["7", "99", "123456", "123456789"] {
            let n = (0..10).filter(|c| valid_cui(&format!("{body}{c}"))).count();
            assert!(n <= 1, "{body}");
        }
    }

    #[test]
    fn cui_and_invoice_patterns() {
        let t = "Furnizor: SC Test SRL, CUI RO14399840, alt cod 14399841, tel 0722";
        let found: Vec<&str> = find_cui(t).iter().map(|h| &t[h.start..h.end]).collect();
        assert_eq!(found, vec!["RO14399840"]);
        let all = spans(pattern_cui(), t);
        assert!(all.contains(&"RO14399840") && all.contains(&"14399841"));

        let t = "Factura FCT 0123 din data, seria FA-2026/00045; ref SRV123456.";
        let got: Vec<&str> = find_invoice_series(t)
            .iter()
            .map(|h| &t[h.start..h.end])
            .collect();
        assert!(got.contains(&"FCT 0123"), "{got:?}");
        assert!(got.contains(&"FA-2026/00045"), "{got:?}");
        assert!(got.contains(&"SRV123456"), "{got:?}");
        // usable inside a larger query
        let q = parse_query(&format!("factura {}", pattern_invoice_series())).unwrap();
        assert!(is_match(&q, t));
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn random_input_never_panics() {
        let alphabet: Vec<char> =
            "ab c\"/()+-*?~012\\[]{}|^$.ANDORT ăâîșşțţßİé\u{0327}\u{0301}\n日😀"
                .chars()
                .collect();
        let words = [
            "AND", "OR", "NOT", "brasov", "fact*", "~1", "\"", "/x+/", "(", ")",
        ];
        let mut r = Rng(0x9E3779B97F4A7C15);
        let mut texts = vec![String::new(), "Brașov ţară ßß".to_string()];
        for _ in 0..200 {
            let n = (r.next() % 40) as usize;
            texts.push(
                (0..n)
                    .map(|_| alphabet[(r.next() % alphabet.len() as u64) as usize])
                    .collect(),
            );
        }
        for _ in 0..3000 {
            let n = (r.next() % 12) as usize;
            let mut q = String::new();
            for _ in 0..n {
                if r.next().is_multiple_of(3) {
                    q.push_str(words[(r.next() % words.len() as u64) as usize]);
                } else {
                    q.push(alphabet[(r.next() % alphabet.len() as u64) as usize]);
                }
                if r.next().is_multiple_of(2) {
                    q.push(' ');
                }
            }
            if let Ok(parsed) = parse_query(&q) {
                for t in &texts {
                    for h in matches(&parsed, t) {
                        assert!(h.start < h.end && h.end <= t.len());
                        assert!(t.is_char_boundary(h.start) && t.is_char_boundary(h.end));
                    }
                }
            }
            let _ = fold(&q);
            let _ = levenshtein_bounded(&q, &texts[(r.next() % 200) as usize], 2);
            let _ = valid_cui(&q);
        }
    }
}
