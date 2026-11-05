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

/// Digits that OCR returns instead of letters, in the country / nationality codes (no check digit
/// covers these, so the repair is by alphabet only).
fn to_letters(s: &str) -> String {
    s.replace('<', "")
        .chars()
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

fn td3(a: &str, b: &str) -> Mrz {
    let (surname, given) = name_parts(&a[5..44]);
    let num = &b[0..9];
    let (birth, expiry) = (to_digits(&b[13..19]), to_digits(&b[21..27]));
    let mut failed = Vec::new();
    if !digit_ok(num, b.as_bytes()[9]) {
        failed.push("document number");
    }
    if !digit_ok(&birth, b.as_bytes()[19]) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, b.as_bytes()[27]) {
        failed.push("expiry date");
    }
    // personal number: the check digit may be '<' when the field is empty
    let pn = &b[28..42];
    if !(pn.bytes().all(|c| c == b'<') && matches!(b.as_bytes()[42], b'<' | b'0')
        || digit_ok(pn, b.as_bytes()[42]))
    {
        failed.push("personal number");
    }
    let comp = format!("{}{}{}", &b[0..10], &b[13..20], &b[21..43]);
    if !digit_ok(&comp, b.as_bytes()[43]) {
        failed.push("composite");
    }
    Mrz {
        format: "TD3",
        doc_type: a[0..2].replace('<', ""),
        country: to_letters(&a[2..5]),
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: to_letters(&b[10..13]),
        birth,
        sex: b[20..21].replace('<', ""),
        expiry,
        failed,
    }
}

fn td2(a: &str, b: &str) -> Mrz {
    let (surname, given) = name_parts(&a[5..36]);
    let num = &b[0..9];
    let (birth, expiry) = (to_digits(&b[13..19]), to_digits(&b[21..27]));
    let mut failed = Vec::new();
    if !digit_ok(num, b.as_bytes()[9]) {
        failed.push("document number");
    }
    if !digit_ok(&birth, b.as_bytes()[19]) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, b.as_bytes()[27]) {
        failed.push("expiry date");
    }
    let comp = format!("{}{}{}", &b[0..10], &b[13..20], &b[21..35]);
    if !digit_ok(&comp, b.as_bytes()[35]) {
        failed.push("composite");
    }
    Mrz {
        format: "TD2",
        doc_type: a[0..2].replace('<', ""),
        country: to_letters(&a[2..5]),
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: to_letters(&b[10..13]),
        birth,
        sex: b[20..21].replace('<', ""),
        expiry,
        failed,
    }
}

fn td1(a: &str, b: &str, c: &str) -> Mrz {
    let (surname, given) = name_parts(c);
    let num = &a[5..14];
    let (birth, expiry) = (to_digits(&b[0..6]), to_digits(&b[8..14]));
    let mut failed = Vec::new();
    if !digit_ok(num, a.as_bytes()[14]) {
        failed.push("document number");
    }
    if !digit_ok(&birth, b.as_bytes()[6]) {
        failed.push("date of birth");
    }
    if !digit_ok(&expiry, b.as_bytes()[14]) {
        failed.push("expiry date");
    }
    let comp = format!("{}{}{}{}", &a[5..30], &b[0..7], &b[8..15], &b[18..29]);
    if !digit_ok(&comp, b.as_bytes()[29]) {
        failed.push("composite");
    }
    Mrz {
        format: "TD1",
        doc_type: a[0..2].replace('<', ""),
        country: to_letters(&a[2..5]),
        surname,
        given,
        number: num.trim_end_matches('<').to_string(),
        nationality: to_letters(&b[15..18]),
        birth,
        sex: b[7..8].replace('<', ""),
        expiry,
        failed,
    }
}

/// Every MRZ found in `text` (lines are matched after removing blanks).
pub fn find(text: &str) -> Vec<Mrz> {
    let lines: Vec<String> = text.lines().map(clean).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len()
            && plausible(&lines[i], 30)
            && plausible(&lines[i + 1], 30)
            && plausible(&lines[i + 2], 30)
            && lines[i].starts_with(['I', 'A', 'C'])
        {
            out.push(td1(&lines[i], &lines[i + 1], &lines[i + 2]));
            i += 3;
        } else if i + 1 < lines.len()
            && plausible(&lines[i], 44)
            && plausible(&lines[i + 1], 44)
            && lines[i].starts_with('P')
        {
            out.push(td3(&lines[i], &lines[i + 1]));
            i += 2;
        } else if i + 1 < lines.len()
            && plausible(&lines[i], 36)
            && plausible(&lines[i + 1], 36)
            && lines[i].starts_with(['I', 'A', 'C', 'P'])
        {
            out.push(td2(&lines[i], &lines[i + 1]));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
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

    #[test]
    fn plain_text_has_no_mrz() {
        assert!(find("Hello world\nthis is a page").is_empty());
    }
}
