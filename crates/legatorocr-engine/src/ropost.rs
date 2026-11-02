//! Romanian-aware post-processing of raw OCR text.
//!
//! Four independent, individually switchable clean-ups:
//!
//! 1. cedilla letters (`ş ţ Ş Ţ`) become the correct comma-below letters
//!    (`ș ț Ș Ț`);
//! 2. words split by a hyphen at the end of a line are joined, but only when
//!    the hyphen follows a letter and the next line starts with a lowercase
//!    letter (so `Nord-\nEst` or `anul 1990-\n1991` are kept);
//! 3. straight quotes become Romanian typographic quotes `„ ”`; apostrophes
//!    inside words (`l'aut`) are left alone;
//! 4. punctuation spacing: no space before `, . ; : ! ? )` or after `(`, one
//!    space after `, ; :` when a letter follows, runs of spaces collapsed.
//!    Numbers (`3,14`, `1.000`), times (`12:30`), URLs and e-mail addresses
//!    are never modified.
//!
//! The text is never reordered and the functions never panic. The result is
//! iterated to a fixed point, so `postprocess` is idempotent.

/// Which clean-ups to apply. `Default` enables everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostOptions {
    pub comma_below: bool,
    pub dehyphenate: bool,
    pub quotes: bool,
    pub punct_spacing: bool,
}

impl Default for PostOptions {
    fn default() -> Self {
        PostOptions {
            comma_below: true,
            dehyphenate: true,
            quotes: true,
            punct_spacing: true,
        }
    }
}

/// Post-process `text` according to `o`.
pub fn postprocess(text: &str, o: &PostOptions) -> String {
    let mut cur = once(text, o);
    // Iterate to a fixed point: later steps can change the context an earlier
    // step looks at, so extra rounds guarantee idempotence in practice.
    for _ in 0..8 {
        let next = once(&cur, o);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn once(text: &str, o: &PostOptions) -> String {
    let mut s = text.to_string();
    if o.comma_below {
        s = comma_below(&s);
    }
    if o.dehyphenate {
        s = dehyphenate(&s);
    }
    if o.quotes {
        s = quotes(&s);
    }
    if o.punct_spacing {
        s = punct_spacing(&s);
    }
    s
}

/// Number of changed characters between `before` and `after` (substitutions
/// count once; otherwise the larger of the insertions/deletions; 0 when
/// equal). Exact for moderate edit distances, a coarse upper bound beyond.
pub fn stats(before: &str, after: &str) -> usize {
    if before == after {
        return 0;
    }
    let a: Vec<char> = before.chars().collect();
    let b: Vec<char> = after.chars().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[pre..], &b[pre..]);
    let suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[..a.len() - suf], &b[..b.len() - suf]);
    match edit_distance(a, b, 6000) {
        Some(d) => {
            let diff = a.len() as isize - b.len() as isize;
            let del = (d as isize + diff) / 2;
            let ins = (d as isize - diff) / 2;
            del.max(ins).max(0) as usize
        }
        None => a.len().max(b.len()),
    }
}

/// Myers O(ND) insert+delete distance, `None` if it exceeds `cap`.
fn edit_distance(a: &[char], b: &[char], cap: usize) -> Option<usize> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    let max = ((n + m) as usize).min(cap) as isize;
    let off = max + 1;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    for d in 0..=max {
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                return Some(d as usize);
            }
            k += 2;
        }
    }
    None
}

// ---------------------------------------------------------------- cedilla

fn comma_below(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{015E}' => '\u{0218}',
            '\u{015F}' => '\u{0219}',
            '\u{0162}' => '\u{021A}',
            '\u{0163}' => '\u{021B}',
            c => c,
        })
        .collect()
}

// ------------------------------------------------------------ dehyphenate

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{00AD}' | '\u{2010}')
}

fn dehyphenate(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let len = c.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < len {
        let ch = c[i];
        if is_hyphen(ch) && out.chars().next_back().is_some_and(char::is_alphabetic) {
            let mut j = i + 1;
            while j < len && (c[j] == ' ' || c[j] == '\t') {
                j += 1;
            }
            if j < len && c[j] == '\r' {
                j += 1;
            }
            if j < len && c[j] == '\n' {
                let mut k = j + 1;
                while k < len && (c[k] == ' ' || c[k] == '\t') {
                    k += 1;
                }
                if k < len && c[k].is_lowercase() {
                    i = k;
                    continue;
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

// ----------------------------------------------------------------- quotes

fn quotes(text: &str) -> String {
    if !text.contains(['"', '\'']) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut para = String::new();
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            out.push_str(&fix_quotes(&para));
            para.clear();
            out.push_str(line);
        } else {
            para.push_str(line);
        }
    }
    out.push_str(&fix_quotes(&para));
    out
}

fn is_open_ctx(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(p) => {
            p.is_whitespace() || matches!(p, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '-')
        }
    }
}

fn fix_quotes(para: &str) -> String {
    let mut v: Vec<char> = para.chars().collect();
    if !v.iter().any(|&c| c == '"' || c == '\'') {
        return para.to_string();
    }
    // Double quotes: alternate when balanced, otherwise decide by context.
    let positions: Vec<usize> = (0..v.len()).filter(|&i| v[i] == '"').collect();
    let balanced = positions.len().is_multiple_of(2);
    for (n, &i) in positions.iter().enumerate() {
        let prev = if i > 0 { Some(v[i - 1]) } else { None };
        let open = if balanced {
            n.is_multiple_of(2)
        } else {
            is_open_ctx(prev)
        };
        v[i] = if open { '\u{201E}' } else { '\u{201D}' };
    }
    // Single quotes: only genuine pairs, never apostrophes inside words.
    let mut pending: Option<usize> = None;
    for i in 0..v.len() {
        if v[i] != '\'' {
            continue;
        }
        let prev = if i > 0 { Some(v[i - 1]) } else { None };
        let next = v.get(i + 1).copied();
        if prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric) {
            continue;
        }
        let open = is_open_ctx(prev)
            && next.is_some_and(|n| n.is_alphanumeric() || matches!(n, '\u{201E}' | '(' | '['));
        if open {
            pending = Some(i);
            continue;
        }
        let close = prev.is_some_and(|p| !p.is_whitespace() && !matches!(p, '(' | '[' | '{'))
            && !next.is_some_and(char::is_alphanumeric);
        if close {
            if let Some(p) = pending.take() {
                v[p] = '\u{201E}';
                v[i] = '\u{201D}';
            }
        }
    }
    v.into_iter().collect()
}

// ---------------------------------------------------------- punct spacing

fn punct_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        punct_line(line, &mut out);
    }
    out
}

/// Per-char flag: true when the char sits inside a URL / e-mail like token.
fn guard_flags(c: &[char]) -> Vec<bool> {
    let mut g = vec![false; c.len()];
    let mut i = 0;
    while i < c.len() {
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < c.len() && !c[i].is_whitespace() {
            i += 1;
        }
        let tok: String = c[start..i].iter().collect();
        if tok.contains('@') || tok.contains("://") || tok.contains("www.") {
            g[start..i].iter_mut().for_each(|x| *x = true);
        }
    }
    g
}

fn punct_line(line: &str, out: &mut String) {
    let c: Vec<char> = line.chars().collect();
    let g = guard_flags(&c);
    let mut buf: Vec<char> = Vec::with_capacity(c.len());
    let mut seen = false; // a non-whitespace char has been emitted
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == ' ' {
            if seen && buf.last() == Some(&' ') {
                i += 1;
                continue;
            }
            buf.push(' ');
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            buf.push(ch);
            i += 1;
            continue;
        }
        let next = c.get(i + 1).copied();
        if !g[i] && matches!(ch, ',' | '.' | ';' | ':' | '!' | '?' | ')') {
            let n = buf.len();
            if seen && n >= 2 && buf[n - 1] == ' ' && !buf[n - 2].is_whitespace() {
                let before = buf[n - 2];
                let numeric = matches!(ch, ',' | ':' | ';' | '.')
                    && before.is_ascii_digit()
                    && next.is_some_and(|x| x.is_ascii_digit());
                let dotted = ch == '.' && next.is_some_and(char::is_alphanumeric);
                if !numeric && !dotted {
                    buf.pop();
                }
            }
        }
        buf.push(ch);
        seen = true;
        if !g[i] {
            if ch == '(' {
                while i + 1 < c.len() && c[i + 1] == ' ' {
                    i += 1;
                }
            } else if matches!(ch, ',' | ';' | ':') && next.is_some_and(char::is_alphabetic) {
                buf.push(' ');
            }
        }
        i += 1;
    }
    out.extend(buf);
}

static CLEAN_TEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Switch the Romanian clean-up of the plain-text and Word outputs on or off (`--clean-text`).
pub fn set_enabled(on: bool) {
    CLEAN_TEXT.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    CLEAN_TEXT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Apply [`postprocess`] page by page to text whose pages are separated by form feeds, so
/// nothing is joined across a page break.
pub fn postprocess_pages(text: &str, o: &PostOptions) -> String {
    text.split('\x0c')
        .map(|p| postprocess(p, o))
        .collect::<Vec<_>>()
        .join("\x0c")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pp(s: &str) -> String {
        postprocess(s, &PostOptions::default())
    }

    #[test]
    fn default_all_true() {
        let o = PostOptions::default();
        assert!(o.comma_below && o.dehyphenate && o.quotes && o.punct_spacing);
    }

    #[test]
    fn cedilla_to_comma_below() {
        assert_eq!(pp("Şcoală ţară şi Ţepeş"), "Școală țară și Țepeș");
        assert_eq!(
            pp("\u{015E}\u{015F}\u{0162}\u{0163}"),
            "\u{0218}\u{0219}\u{021A}\u{021B}"
        );
    }

    #[test]
    fn diacritics_intact() {
        let s = "ăâîșț ĂÂÎȘȚ";
        assert_eq!(pp(s), s);
    }

    #[test]
    fn option_switches() {
        let off = PostOptions {
            comma_below: false,
            dehyphenate: false,
            quotes: false,
            punct_spacing: false,
        };
        let s = "ş exem-\nplu \"x\" a ,b";
        assert_eq!(postprocess(s, &off), s);
        let only_cedilla = PostOptions {
            comma_below: true,
            ..off
        };
        assert_eq!(postprocess(s, &only_cedilla), "ș exem-\nplu \"x\" a ,b");
        let only_hyph = PostOptions {
            dehyphenate: true,
            ..off
        };
        assert_eq!(postprocess(s, &only_hyph), "ş exemplu \"x\" a ,b");
    }

    #[test]
    fn joins_split_word() {
        assert_eq!(pp("un exem-\nplu bun"), "un exemplu bun");
        assert_eq!(pp("exem-\r\nplu"), "exemplu");
        assert_eq!(pp("exem-  \n   plu"), "exemplu");
        assert_eq!(pp("a-\nb-\nc"), "abc");
    }

    #[test]
    fn keeps_compounds_and_structure() {
        assert_eq!(pp("Nord-\nEst"), "Nord-\nEst");
        assert_eq!(pp("anii 1990-\n1991"), "anii 1990-\n1991");
        assert_eq!(pp("foo -\nbar"), "foo -\nbar");
        assert_eq!(pp("sfârșit-\n\nnou"), "sfârșit-\n\nnou");
        assert_eq!(
            pp("rând unu\nrând doi\n\nalt paragraf"),
            "rând unu\nrând doi\n\nalt paragraf"
        );
        assert_eq!(pp("text-"), "text-");
        assert_eq!(pp("-\nabc"), "-\nabc");
    }

    #[test]
    fn double_quotes() {
        assert_eq!(pp("a spus \"da\" și a plecat"), "a spus „da” și a plecat");
        assert_eq!(pp("\"Da\", a spus."), "„Da”, a spus.");
        assert_eq!(pp("\"unu\" și \"doi\""), "„unu” și „doi”");
    }

    #[test]
    fn unbalanced_quotes_by_context() {
        assert_eq!(pp("a spus \"da"), "a spus „da");
        assert_eq!(pp("da\" a spus"), "da” a spus");
    }

    #[test]
    fn single_quotes_and_apostrophes() {
        assert_eq!(pp("l'aut este"), "l'aut este");
        assert_eq!(pp("a zis 'bine' acum"), "a zis „bine” acum");
        assert_eq!(pp("rock'n'roll"), "rock'n'roll");
        assert_eq!(pp("it\u{2019}s"), "it\u{2019}s");
        assert_eq!(pp("'singur"), "'singur");
    }

    #[test]
    fn quotes_reset_per_paragraph() {
        assert_eq!(pp("a \"b\n\nc \"d\" e"), "a „b\n\nc „d” e");
    }

    #[test]
    fn spaces_before_punct() {
        assert_eq!(pp("Ion , Maria ; Ana : toți !"), "Ion, Maria; Ana: toți!");
        assert_eq!(pp("Ce faci ?"), "Ce faci?");
        assert_eq!(pp("( text ) final ."), "(text) final.");
        assert_eq!(pp("a  b   c"), "a b c");
    }

    #[test]
    fn space_after_punct() {
        assert_eq!(pp("unu,doi;trei:patru"), "unu, doi; trei: patru");
        assert_eq!(pp("Ion ,Maria"), "Ion, Maria");
        assert_eq!(pp("a,\nb"), "a,\nb");
    }

    #[test]
    fn numbers_urls_times_untouched() {
        for s in [
            "3,14",
            "1.000",
            "1.000.000,50 lei",
            "ora 12:30",
            "http://exemplu.ro/a,b:c",
            "https://x.ro/a?b=c",
            "scrie la ion@exemplu.ro,mâine",
            "www.exemplu.ro/ro,fr",
            "mailto:ion@x.ro",
            "3 ,14",
            "fișier .txt",
        ] {
            assert_eq!(pp(s), s, "{s}");
        }
    }

    #[test]
    fn indentation_preserved() {
        assert_eq!(pp("    indent  text"), "    indent text");
        assert_eq!(pp("  ( x )"), "  (x)");
    }

    #[test]
    fn crlf_and_tabs() {
        assert_eq!(pp("a ,b\r\nc ."), "a, b\r\nc.");
        assert_eq!(pp("a\t,b"), "a\t, b");
    }

    #[test]
    fn empty_and_unicode() {
        assert_eq!(pp(""), "");
        assert_eq!(pp("日本語 , 😀 ."), "日本語, 😀.");
        assert_eq!(pp("\n\n"), "\n\n");
    }

    #[test]
    fn idempotent_examples() {
        for s in [
            "Şcoală ,\"ţară\" exem-\nplu ( a ) 'x' ;b",
            "a ' b ' c",
            "'a \"b' c\"",
            "\"\"\"",
            "(  ,  )",
            "a - \n b",
            "x ,,, y ...z",
        ] {
            let once = pp(s);
            assert_eq!(pp(&once), once, "input {s:?}");
        }
    }

    #[test]
    fn stats_counts() {
        assert_eq!(stats("abc", "abc"), 0);
        assert_eq!(stats("ş", "ș"), 1);
        assert_eq!(stats("exem-\nplu", "exemplu"), 2);
        assert_eq!(stats("a ,b", "a, b"), 1);
        assert_eq!(stats("", "abc"), 3);
        assert_eq!(stats("abc", ""), 3);
        assert_eq!(stats("Şcoală şi ţara", "Școală și țara"), 3);
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    fn alnum_seq(s: &str) -> String {
        comma_below(s)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    }

    #[test]
    fn property_loop() {
        let alphabet: Vec<char> = "aăâîșțşţAZb09 ,.;:!?()\"'-\n\r\t@/…„”\u{00AD}wé😀"
            .chars()
            .collect();
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        let mut changed = 0;
        for _ in 0..20000 {
            let len = (rng.next() % 40) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let o = PostOptions {
                comma_below: !rng.next().is_multiple_of(4),
                dehyphenate: !rng.next().is_multiple_of(4),
                quotes: !rng.next().is_multiple_of(4),
                punct_spacing: !rng.next().is_multiple_of(4),
            };
            let a = postprocess(&s, &o);
            let b = postprocess(&a, &o);
            assert_eq!(a, b, "not idempotent for {s:?} {o:?}");
            assert_eq!(alnum_seq(&s), alnum_seq(&a), "reordered {s:?}");
            let _ = stats(&s, &a);
            if a != s {
                changed += 1;
            }
        }
        assert!(changed > 1000);
    }

    #[test]
    fn property_long_text_no_panic() {
        let mut rng = Lcg(42);
        let alphabet: Vec<char> = "ab ăș,.\n\"'-".chars().collect();
        let s: String = (0..5000)
            .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
            .collect();
        let a = pp(&s);
        assert_eq!(pp(&a), a);
        assert!(stats(&s, &a) <= s.chars().count());
    }
}
