//! Machine-readable zone reading (roadmap 7.5, part): find ICAO 9303 MRZ lines (TD1 3×30,
//! TD2 2×36, TD3 2×44) in OCR text, repair the usual OCR confusions in the numeric fields and
//! check every check digit. A document whose digits all agree is very unlikely to have been
//! misread; one that fails is reported with which field failed. This reads and checks the
//! printed zone only: it does not prove the document is genuine (no chip, seal or security
//! feature is examined).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mrz {
    pub format: &'static str,
    pub doc_type: String,
    pub country: String,
    pub surname: String,
    pub given: String,
    pub number: String,
    pub nationality: String,
    pub birth: String,
    pub sex: String,
    pub expiry: String,
    /// Names of the check digits that do not match.
    pub failed: Vec<&'static str>,
    /// Content warnings (characters outside the field's alphabet, impossible dates, document
    /// codes that the standard does not use, ...). Each names the field.
    pub warnings: Vec<String>,
    /// Every OCR-confusion repair that was applied (`O` read as `0` and so on).
    pub repairs: Vec<String>,
    /// The name field ends in a letter: the name was cut to fit (or just fills the field).
    pub name_truncated: bool,
}

/// What a run collects besides the fields themselves.
struct Notes {
    repair: bool,
    warnings: Vec<String>,
    repairs: Vec<String>,
}

impl Notes {
    fn digits(&mut self, label: &str, raw: &str, fill_ok: bool) -> String {
        let fixed = if self.repair {
            to_digits(raw)
        } else {
            raw.to_string()
        };
        if fixed != raw {
            self.repairs.push(format!("{label}: {raw} read as {fixed}"));
        }
        if fixed
            .chars()
            .any(|c| !(c.is_ascii_digit() || (fill_ok && c == '<')))
        {
            self.warnings.push(format!(
                "{label}: character not allowed in a numeric field ({fixed})"
            ));
        }
        fixed
    }

    fn check_char(&mut self, label: &str, c: u8) -> u8 {
        let raw = (c as char).to_string();
        let fixed = self.digits(&format!("{label} check digit"), &raw, false);
        fixed.bytes().next().unwrap_or(c)
    }

    /// Letters-only field; fill characters `<` are kept in the result.
    fn letters(&mut self, label: &str, raw: &str) -> String {
        let fixed = if self.repair {
            to_letters_keep(raw)
        } else {
            raw.to_string()
        };
        if fixed != raw {
            self.repairs.push(format!("{label}: {raw} read as {fixed}"));
        }
        if fixed.chars().any(|c| !(c.is_ascii_uppercase() || c == '<')) {
            self.warnings.push(format!(
                "{label}: character not allowed in a letter field ({fixed})"
            ));
        }
        fixed
    }

    fn date(&mut self, label: &str, d: &str, unknown_ok: bool) {
        let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
        let (mm, dd) = (&d[2..4], &d[4..6]);
        let month = if digits(mm) {
            mm.parse::<u32>().ok()
        } else {
            None
        };
        let day = if digits(dd) {
            dd.parse::<u32>().ok()
        } else {
            None
        };
        let max = match month {
            Some(2) => 29,
            Some(4 | 6 | 9 | 11) => 30,
            _ => 31,
        };
        let bad_month = month.is_some_and(|m| !(1..=12).contains(&m));
        let bad_day = day.is_some_and(|x| x < 1 || x > max);
        let unknown = d.contains('<');
        if bad_month || bad_day || (unknown && !unknown_ok) {
            self.warnings.push(format!("{label}: impossible date {d}"));
        }
    }

    fn sex(&mut self, c: &str) {
        if !matches!(c, "F" | "M" | "<") {
            self.warnings.push(format!("sex: {c:?} is not F, M or <"));
        }
    }
}

fn to_letters_keep(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0' => 'O',
            '1' => 'I',
            '5' => 'S',
            '8' => 'B',
            '2' => 'Z',
            c => c,
        })
        .collect()
}

fn val(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'A'..=b'Z' => Some((c - b'A') as u32 + 10),
        b'<' => Some(0),
        _ => None,
    }
}

/// ICAO 9303 check digit (weights 7, 3, 1) of `s`, or `None` for characters outside the alphabet.
pub fn check_digit(s: &str) -> Option<u8> {
    let mut sum = 0;
    for (i, b) in s.bytes().enumerate() {
        sum += val(b)? * [7, 3, 1][i % 3];
    }
    Some((sum % 10) as u8)
}

fn digit_ok(field: &str, check: u8) -> bool {
    check.is_ascii_digit() && check_digit(field) == Some(check - b'0')
}

/// Letters that OCR returns instead of digits.
fn to_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'O' | 'Q' | 'D' => '0',
            'I' | 'L' => '1',
            'Z' => '2',
            'S' => '5',
            'B' => '8',
            c => c,
        })
        .collect()
}

fn clean(line: &str) -> String {
    line.trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .map(|c| if c == '«' { '<' } else { c })
        .collect()
}

fn name_parts(field: &str) -> (String, String) {
    let f = field.trim_end_matches('<');
    let (s, g) = f.split_once("<<").unwrap_or((f, ""));
    let sp = |x: &str| x.replace('<', " ").trim().to_string();
    (sp(s), sp(g))
}

fn plausible(l: &str, len: usize) -> bool {
    l.len() == len && l.bytes().all(|b| val(b).is_some()) && l.matches('<').count() >= 1
}

/// Document code: letters or fill; `first` lists the letters allowed first.
fn doc_code(n: &mut Notes, code: &str, first: &str) {
    let mut c = code.chars();
    let (f, g) = (c.next().unwrap_or('<'), c.next().unwrap_or('<'));
    if !first.contains(f) {
        n.warnings.push(format!(
            "document code {code:?}: first letter must be one of {first}"
        ));
    }
    if !(g.is_ascii_uppercase() || g == '<') {
        n.warnings.push(format!(
            "document code {code:?}: second character must be a letter or <"
        ));
    }
    if g == 'C' && (f == 'A' || f == 'I') {
        n.warnings.push(format!(
            "document code {code:?}: C does not follow A or I (crew member certificates excepted)"
        ));
    }
}

fn td3(a: &str, b: &str, n: &mut Notes) -> Mrz {
    let by = b.as_bytes();
    doc_code(n, &a[0..2], "P");
    let country = n.letters("issuing state", &a[2..5]).replace('<', "");
    let names = n.letters("name", &a[5..44]);
    let nat = n.letters("nationality", &b[10..13]).replace('<', "");
    let (surname, given) = name_parts(&names);
    let num = &b[0..9];
    let birth = n.digits("date of birth", &b[13..19], true);
    let expiry = n.digits("expiry date", &b[21..27], false);
    n.date("date of birth", &birth, true);
    n.date("expiry date", &expiry, false);
    n.sex(&b[20..21]);
    let (c_num, c_birth, c_exp) = (
        n.check_char("document number", by[9]),
        n.check_char("date of birth", by[19]),
        n.check_char("expiry date", by[27]),
    );
    let mut failed = Vec::new();
    if !digit_ok(num, c_num) {
        failed.push("document number");
    }
    if !digit_ok(&birth, c_birth) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, c_exp) {
        failed.push("expiry date");
    }
    // personal number: when the field is unused the check position may hold '<' or '0'
    let pn = &b[28..42];
    if pn.bytes().all(|c| c == b'<') {
        if !matches!(by[42], b'<' | b'0') {
            n.warnings.push(format!(
                "personal number: unused field, check position holds {:?} instead of < or 0",
                by[42] as char
            ));
        }
    } else {
        let c = n.check_char("personal number", by[42]);
        if !digit_ok(pn, c) {
            failed.push("personal number");
        }
    }
    let c_comp = n.check_char("composite", by[43]);
    let comp = format!(
        "{}{}{birth}{}{expiry}{}{}{}",
        num, c_num as char, c_birth as char, c_exp as char, pn, by[42] as char
    );
    if !digit_ok(&comp, c_comp) {
        failed.push("composite");
    }
    Mrz {
        format: "TD3",
        doc_type: a[0..2].replace('<', ""),
        country,
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: nat,
        birth,
        sex: b[20..21].replace('<', ""),
        expiry,
        failed,
        warnings: std::mem::take(&mut n.warnings),
        repairs: std::mem::take(&mut n.repairs),
        name_truncated: names.ends_with(|c: char| c.is_ascii_uppercase()),
    }
}

fn td2(a: &str, b: &str, n: &mut Notes) -> Mrz {
    let by = b.as_bytes();
    let country = n.letters("issuing state", &a[2..5]).replace('<', "");
    let names = n.letters("name", &a[5..36]);
    let nat = n.letters("nationality", &b[10..13]).replace('<', "");
    let (surname, given) = name_parts(&names);
    let num = &b[0..9];
    let birth = n.digits("date of birth", &b[13..19], true);
    let expiry = n.digits("expiry date", &b[21..27], false);
    let (c_num, c_birth, c_exp) = (
        n.check_char("document number", by[9]),
        n.check_char("date of birth", by[19]),
        n.check_char("expiry date", by[27]),
    );
    let mut failed = Vec::new();
    if !digit_ok(num, c_num) {
        failed.push("document number");
    }
    if !digit_ok(&birth, c_birth) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, c_exp) {
        failed.push("expiry date");
    }
    let c_comp = n.check_char("composite", by[35]);
    let comp = format!(
        "{}{}{birth}{}{expiry}{}{}",
        num,
        c_num as char,
        c_birth as char,
        c_exp as char,
        &b[28..35]
    );
    if !digit_ok(&comp, c_comp) {
        failed.push("composite");
    }
    Mrz {
        format: "TD2",
        doc_type: a[0..2].replace('<', ""),
        country,
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: nat,
        birth,
        sex: b[20..21].replace('<', ""),
        expiry,
        failed,
        warnings: std::mem::take(&mut n.warnings),
        repairs: std::mem::take(&mut n.repairs),
        name_truncated: false,
    }
}

fn td1(a: &str, b: &str, c: &str, n: &mut Notes) -> Mrz {
    let (ay, by) = (a.as_bytes(), b.as_bytes());
    doc_code(n, &a[0..2], "ACI");
    let country = n.letters("issuing state", &a[2..5]).replace('<', "");
    let nat = n.letters("nationality", &b[15..18]).replace('<', "");
    let names = n.letters("name", c);
    let (surname, given) = name_parts(&names);
    // A document number longer than nine characters: the first nine stay in 6-14, position 15
    // holds '<', and the rest sits at the start of the optional field, followed by its check
    // digit and '<'.
    let (num, num_check) = match (ay[14], a[15..30].find('<')) {
        (b'<', Some(q)) if q >= 2 => (
            format!("{}{}", &a[5..14], &a[15..14 + q]),
            n.check_char("document number", ay[14 + q]),
        ),
        _ => (
            a[5..14].to_string(),
            n.check_char("document number", ay[14]),
        ),
    };
    let birth = n.digits("date of birth", &b[0..6], true);
    let expiry = n.digits("expiry date", &b[8..14], false);
    n.date("date of birth", &birth, true);
    n.date("expiry date", &expiry, false);
    n.sex(&b[7..8]);
    let (c_birth, c_exp) = (
        n.check_char("date of birth", by[6]),
        n.check_char("expiry date", by[14]),
    );
    let mut failed = Vec::new();
    if !digit_ok(&num, num_check) {
        failed.push("document number");
    }
    if !digit_ok(&birth, c_birth) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, c_exp) {
        failed.push("expiry date");
    }
    let c_comp = n.check_char("composite", by[29]);
    let comp = format!(
        "{}{}{}{birth}{}{expiry}{}{}",
        &a[5..14],
        if ay[14] == b'<' {
            '<'
        } else {
            num_check as char
        },
        &a[15..30],
        c_birth as char,
        c_exp as char,
        &b[18..29]
    );
    if !digit_ok(&comp, c_comp) {
        failed.push("composite");
    }
    Mrz {
        format: "TD1",
        doc_type: a[0..2].replace('<', ""),
        country,
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: nat,
        birth,
        sex: b[7..8].replace('<', ""),
        expiry,
        failed,
        warnings: std::mem::take(&mut n.warnings),
        repairs: std::mem::take(&mut n.repairs),
        name_truncated: names.ends_with(|ch: char| ch.is_ascii_uppercase()),
    }
}

/// Every MRZ found in `text` (lines are matched after removing blanks), with OCR-confusion
/// repair on.
pub fn find(text: &str) -> Vec<Mrz> {
    find_with(text, true)
}

/// Like [`find`]; with `repair` off, a letter in a numeric field (or the reverse) is only
/// reported as a warning and used as it stands.
pub fn find_with(text: &str, repair: bool) -> Vec<Mrz> {
    let lines: Vec<String> = text.lines().map(clean).collect();
    let mut out = Vec::new();
    let mut notes = Notes {
        repair,
        warnings: Vec::new(),
        repairs: Vec::new(),
    };
    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len()
            && plausible(&lines[i], 30)
            && plausible(&lines[i + 1], 30)
            && plausible(&lines[i + 2], 30)
            && lines[i].starts_with(['I', 'A', 'C'])
        {
            out.push(td1(&lines[i], &lines[i + 1], &lines[i + 2], &mut notes));
            i += 3;
        } else if i + 1 < lines.len()
            && plausible(&lines[i], 44)
            && plausible(&lines[i + 1], 44)
            && lines[i].starts_with('P')
        {
            out.push(td3(&lines[i], &lines[i + 1], &mut notes));
            i += 2;
        } else if i + 1 < lines.len()
            && plausible(&lines[i], 36)
            && plausible(&lines[i + 1], 36)
            && lines[i].starts_with(['I', 'A', 'C', 'P'])
        {
            out.push(td2(&lines[i], &lines[i + 1], &mut notes));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// A name field built for writing: `text` is exactly the field width, `truncated` says the
/// name had to be cut.
#[derive(Debug, PartialEq, Eq)]
pub struct NameField {
    pub text: String,
    pub truncated: bool,
}

/// One identifier in MRZ form: A-Z only; apostrophes vanish, hyphens, commas and blanks become
/// one `<`, other ASCII punctuation vanishes. Letters outside A-Z are refused (how to
/// transliterate them is not settled in the spec).
fn identifier(s: &str) -> std::result::Result<String, String> {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' => out.push(c.to_ascii_uppercase()),
            '\'' | '\u{2019}' => {}
            '-' | ',' | ' ' | '\t' => {
                if !out.ends_with('<') {
                    out.push('<');
                }
            }
            c if c.is_ascii_punctuation() => {}
            c => return Err(format!("{c:?} cannot be written in a name field")),
        }
    }
    Ok(out.trim_matches('<').to_string())
}

/// Name field of `width` positions: primary, `<<`, secondary, filled with `<`. When too long and
/// a secondary identifier exists, the primary is cut so that `<<` and at least the first letter
/// of the secondary fit; the last position then holds a letter.
pub fn encode_name(
    primary: &str,
    secondary: &str,
    width: usize,
) -> std::result::Result<NameField, String> {
    let p = identifier(primary)?;
    let s = identifier(secondary)?;
    if p.is_empty() {
        return Err("the primary identifier is empty".into());
    }
    let full = if s.is_empty() {
        p.clone()
    } else {
        format!("{p}<<{s}")
    };
    if full.len() <= width {
        let truncated = full.len() == width && full.ends_with(|c: char| c.is_ascii_uppercase());
        return Ok(NameField {
            text: format!("{full:<<width$}"),
            truncated,
        });
    }
    let cut = |t: &str, n: usize| t[..n.min(t.len())].trim_end_matches('<').to_string();
    let text = if s.is_empty() {
        cut(&p, width)
    } else {
        let p_keep = p.len().min(width.saturating_sub(3).max(1));
        let pp = cut(&p, p_keep);
        let room = width.saturating_sub(pp.len() + 2).max(1);
        format!("{pp}<<{}", cut(&s, room))
    };
    Ok(NameField {
        text: format!("{text:<<width$}"),
        truncated: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ICAO 9303 specimen passport
    const A: &str = "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<";
    const B: &str = "L898902C36UTO7408122F1204159ZE184226B<<<<<10";

    #[test]
    fn check_digit_known_values() {
        assert_eq!(check_digit("L898902C3"), Some(6));
        assert_eq!(check_digit("740812"), Some(2));
        assert_eq!(check_digit("120415"), Some(9));
        assert_eq!(check_digit("ZE184226B<<<<<"), Some(1));
    }

    #[test]
    fn specimen_passport_is_read_and_passes() {
        let text = format!("some page text\n{A}\n{B}\nmore");
        let m = find(&text);
        assert_eq!(m.len(), 1);
        let m = &m[0];
        assert_eq!(
            (m.format, m.surname.as_str(), m.given.as_str()),
            ("TD3", "ERIKSSON", "ANNA MARIA")
        );
        assert_eq!(
            (
                m.number.as_str(),
                m.birth.as_str(),
                m.expiry.as_str(),
                m.country.as_str()
            ),
            ("L898902C3", "740812", "120415", "UTO")
        );
        assert!(m.failed.is_empty(), "{:?}", m.failed);
    }

    #[test]
    fn a_misread_digit_is_reported_and_letter_for_digit_is_repaired() {
        // 'O' for '0' in the dates is repaired; a changed digit in the number fails its check
        let b = "L898902C36UTO74O8122F12O4159ZE184226B<<<<<10";
        assert!(find(&format!("{A}\n{b}"))[0].failed.is_empty());
        let bad = "L898903C36UTO7408122F1204159ZE184226B<<<<<10";
        assert_eq!(
            find(&format!("{A}\n{bad}"))[0].failed,
            vec!["document number", "composite"]
        );
        // a zero for the letter O in the nationality code is repaired
        let zero = "L898902C36UT07408122F1204159ZE184226B<<<<<10";
        assert_eq!(find(&format!("{A}\n{zero}"))[0].nationality, "UTO");
        // spaces from OCR are ignored
        let spaced = "L898902C36 UTO7408122F1204159ZE184226B<<<<<10";
        assert!(find(&format!("{A}\n{spaced}"))[0].failed.is_empty());
    }

    const TD1: [&str; 3] = [
        "I<UTOD231458907<<<<<<<<<<<<<<<",
        "7408122F1204159UTO<<<<<<<<<<<6",
        "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
    ];

    fn td1_text(l: &[&str; 3]) -> String {
        l.join("\n")
    }

    #[test]
    fn annex_examples_of_the_check_digit() {
        assert_eq!(check_digit("520727"), Some(3));
        assert_eq!(check_digit("AB2134<<<"), Some(5));
    }

    #[test]
    fn td1_specimen_reads_and_all_digits_agree() {
        let m = find(&td1_text(&TD1));
        assert_eq!(m.len(), 1);
        let m = &m[0];
        assert_eq!(m.format, "TD1");
        assert!(m.failed.is_empty(), "{:?}", m.failed);
        assert_eq!(
            (m.surname.as_str(), m.given.as_str(), m.number.as_str()),
            ("ERIKSSON", "ANNA MARIA", "D23145890")
        );
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert!(m.repairs.is_empty());
    }

    #[test]
    fn td1_changed_document_number_fails_document_and_composite_only() {
        let mut l = TD1;
        l[0] = "I<UTOD231458807<<<<<<<<<<<<<<<";
        let m = &find(&td1_text(&l))[0];
        assert_eq!(m.failed, vec!["document number", "composite"]);
    }

    #[test]
    fn td1_long_document_number_continues_in_the_optional_field() {
        // number D23145890X1 (11 characters): 9 in the number field, '<' as check position,
        // then "X1", its check digit, '<'
        let num = "D23145890X1";
        let ck = check_digit(num).unwrap();
        let a = format!("I<UTOD23145890<X1{ck}<<<<<<<<<<<<");
        assert_eq!(a.len(), 30);
        let b2 = "7408122F1204159UTO<<<<<<<<<<<";
        let comp = format!("{}{}{}{}", &a[5..30], &b2[0..7], &b2[8..15], &b2[18..29]);
        let line2 = format!("{b2}{}", check_digit(&comp).unwrap());
        let m = &find(&format!("{a}\n{line2}\n{}", TD1[2]))[0];
        assert_eq!(m.number, num);
        assert!(m.failed.is_empty(), "{:?}", m.failed);
    }

    #[test]
    fn td3_unused_optional_field_accepts_zero_or_fill_and_warns_otherwise() {
        // personal-number field unused: check position '<' (composite recomputed) and '0'
        let body = |chk: char| {
            let b = format!("L898902C36UTO7408122F1204159{}{}", "<".repeat(14), chk);
            let comp = format!("{}{}{}", &b[0..10], &b[13..20], &b[21..43]);
            format!("{b}{}", check_digit(&comp).unwrap())
        };
        for ok in ['<', '0'] {
            let m = &find(&format!("{A}\n{}", body(ok)))[0];
            assert!(m.failed.is_empty() && m.warnings.is_empty(), "{ok}: {m:?}");
        }
        let m = &find(&format!("{A}\n{}", body('7')))[0];
        assert!(m.warnings.iter().any(|w| w.contains("personal number")));
    }

    #[test]
    fn letter_for_digit_and_back_is_reported_and_repair_can_be_off() {
        let zero = "L898902C36UT07408122F1204159ZE184226B<<<<<10";
        let m = &find(&format!("{A}\n{zero}"))[0];
        assert_eq!(m.nationality, "UTO");
        assert!(m.repairs.iter().any(|r| r.contains("nationality")));
        let off = &find_with(&format!("{A}\n{zero}"), false)[0];
        assert!(off.repairs.is_empty());
        assert!(off.warnings.iter().any(|w| w.contains("nationality")));
        let date = "L898902C36UTO74O8122F1204159ZE184226B<<<<<10";
        let m = &find(&format!("{A}\n{date}"))[0];
        assert_eq!(m.birth, "740812");
        assert!(m.repairs.iter().any(|r| r.contains("date of birth")));
        assert!(m.failed.is_empty());
        let off = &find_with(&format!("{A}\n{date}"), false)[0];
        assert!(off.warnings.iter().any(|w| w.contains("date of birth")));
        assert!(off.failed.contains(&"date of birth"));
    }

    #[test]
    fn impossible_dates_and_codes_are_warned_about() {
        let b = "L898902C36UTO7413322F1204159ZE184226B<<<<<10";
        let m = &find(&format!("{A}\n{b}"))[0];
        assert!(m.warnings.iter().any(|w| w.contains("impossible date")));
        let mut l = TD1;
        l[0] = "ACUTOD231458907<<<<<<<<<<<<<<<";
        let m = &find(&td1_text(&l))[0];
        assert!(m.warnings.iter().any(|w| w.contains("C does not follow")));
    }

    #[test]
    fn a_short_line_is_not_a_td1() {
        let l = [TD1[0], &TD1[1][..29], TD1[2]];
        assert!(find(&td1_text(&l)).is_empty());
    }

    #[test]
    fn names_are_written_by_the_punctuation_rules() {
        let f = |p, s| encode_name(p, s, 39).unwrap();
        let line1 = format!("P<UTO{}", f("Smith-Jones", "Susie Margaret").text);
        assert_eq!(line1, "P<UTOSMITH<JONES<<SUSIE<MARGARET<<<<<<<<<<<<");
        assert_eq!(line1.len(), 44);
        let line1 = format!("P<UTO{}", f("O'Connor", "Enya Siobhan").text);
        assert_eq!(line1, "P<UTOOCONNOR<<ENYA<SIOBHAN<<<<<<<<<<<<<<<<<<");
        assert_eq!(line1.len(), 44);
        assert!(!f("Smith", "Ann").truncated);
        assert!(encode_name("Ștefan", "X", 39).is_err());
        // too long: primary cut, "<<" and the first letter of the secondary survive, last is a letter
        let long = encode_name(&"B".repeat(50), "Anna", 39).unwrap();
        assert!(long.truncated);
        assert_eq!(long.text.len(), 39);
        assert!(long.text.contains("<<A"));
        assert!(long.text.ends_with('A'));
        // decoding spots the truncation
        let b = "L898902C36UTO7408122F1204159ZE184226B<<<<<10";
        let a = format!("P<UTO{}", long.text);
        assert!(find(&format!("{a}\n{b}"))[0].name_truncated);
        assert!(!find(&format!("{A}\n{b}"))[0].name_truncated);
    }

    #[test]
    fn plain_text_has_no_mrz() {
        assert!(find("Hello world\nthis is a page").is_empty());
    }
}
