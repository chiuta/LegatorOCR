/// One recognised word, in pixel coordinates of the processed page
/// (origin top-left, y down).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Engine confidence 0..100 (negative = unknown).
    pub conf: f32,
    /// Index of the text line the word belongs to (for reading order).
    pub line: u32,
}

/// Romanian text must use comma-below `ș`/`ț`; OCR engines and old fonts often give the
/// cedilla forms `ş`/`ţ` (U+015F/U+0163), which are different characters and break search.
/// Does recognised Romanian text look as if the Romanian model was not used? Real Romanian has
/// about 7 % of its letters among `ăâîșț`; OCR without the Romanian model drops them (plain
/// `a`, `i`, `s`, `t`) or turns them into digits and symbols. Returns a short reason, or `None`
/// when the text looks fine or is too short to judge.
pub fn romanian_sanity(text: &str) -> Option<&'static str> {
    let (mut letters, mut diac, mut words, mut digit_words) = (0usize, 0usize, 0usize, 0usize);
    for w in text.split_whitespace() {
        let (mut l, mut d) = (0usize, 0usize);
        for c in w.chars() {
            if c.is_alphabetic() {
                l += 1;
                if "ăâîșțşţĂÂÎȘȚŞŢ".contains(c) {
                    diac += 1;
                }
            } else if c.is_ascii_digit() {
                d += 1;
            }
        }
        letters += l;
        if l + d >= 3 {
            words += 1;
            if l >= 2 && d >= 1 {
                digit_words += 1; // letters and digits inside one word: "m1ne", "t3xt"
            }
        }
    }
    if letters < 400 {
        return None;
    }
    if (diac as f64) < 0.01 * letters as f64 {
        return Some("almost no Romanian diacritics (ăâîșț) were recognised: is the Romanian language pack installed and selected (--lang ro)?");
    }
    if words >= 50 && (digit_words as f64) > 0.04 * words as f64 {
        return Some("many words mix letters and digits: Romanian letters may be misread; check the language pack and the scan quality");
    }
    None
}

pub fn normalize_ro(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{15F}' => '\u{219}', // ş → ș
            '\u{15E}' => '\u{218}', // Ş → Ș
            '\u{163}' => '\u{21B}', // ţ → ț
            '\u{162}' => '\u{21A}', // Ţ → Ț
            c => c,
        })
        .collect()
}

/// Lower-case and strip diacritics so that "Mobilă", "mobila" and "MOBILA" compare equal
/// (also `ș`/`ş`/`s`, `ț`/`ţ`/`t`, and the usual Western-European accents). Length in
/// `char`s is preserved except for ligatures (`ß`, `æ`, `œ`), which expand.
pub fn fold(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        // combining marks (decomposed accents, as macOS file names and many PDFs write them)
        if ('\u{300}'..='\u{36f}').contains(&c) {
            continue;
        }
        let b: &str =
            match c {
                'ă' | 'â' | 'à' | 'á' | 'ä' | 'ã' | 'å' | 'ā' | 'Ă' | 'Â' | 'À' | 'Á' | 'Ä'
                | 'Ã' | 'Å' | 'Ā' => "a",
                'î' | 'ì' | 'í' | 'ï' | 'ī' | 'Î' | 'Ì' | 'Í' | 'Ï' | 'Ī' => "i",
                'ș' | 'ş' | 'š' | 'ś' | 'Ș' | 'Ş' | 'Š' | 'Ś' => "s",
                'ț' | 'ţ' | 'ť' | 'Ț' | 'Ţ' | 'Ť' => "t",
                'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę'
                | 'Ě' => "e",
                'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ő' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' | 'Ő'
                | 'Ø' => "o",
                'ù' | 'ú' | 'û' | 'ü' | 'ű' | 'ū' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ű' | 'Ū' => {
                    "u"
                }
                'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => "c",
                'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => "n",
                'ý' | 'ÿ' | 'Ý' => "y",
                'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => "z",
                'ł' | 'Ł' => "l",
                'ď' | 'đ' | 'Ď' | 'Đ' => "d",
                'ř' | 'Ř' => "r",
                'ß' => "ss",
                'æ' | 'Æ' => "ae",
                'œ' | 'Œ' => "oe",
                _ => {
                    o.extend(c.to_lowercase());
                    continue;
                }
            };
        o.push_str(b);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cedilla_becomes_comma_below() {
        assert_eq!(normalize_ro("ştiinţă ŞŢ"), "știință ȘȚ");
        assert_eq!(normalize_ro("deja ok ș ț"), "deja ok ș ț");
    }

    #[test]
    fn fold_ignores_case_and_diacritics() {
        assert_eq!(
            fold("Tehnologia Producerii MOBILEI"),
            "tehnologia producerii mobilei"
        );
        assert_eq!(
            fold("Mobilă ȘTIINȚĂ ştiinţă Cluj-Năpoca"),
            "mobila stiinta stiinta cluj-napoca"
        );
        assert_eq!(fold("Straße Œuvre"), "strasse oeuvre");
        // decomposed forms fold like the precomposed ones
        assert_eq!(fold("Mobila\u{306} S\u{326}tiint\u{326}a\u{306}"), "mobila stiinta");
    }

    #[test]
    fn romanian_sanity_flags_missing_diacritics_only() {
        let ok =
            "În această gândire românească, școala și țara își găsesc locul într-o lume mai bună. "
                .repeat(8);
        assert_eq!(romanian_sanity(&ok), None);
        let stripped =
            "In aceasta gandire romaneasca, scoala si tara isi gasesc locul intr-o lume mai buna. "
                .repeat(8);
        assert!(romanian_sanity(&stripped).is_some());
        assert_eq!(romanian_sanity("scurt fara diacritice"), None);
        let digits = "m1ne t3xt c4rte ".repeat(40) + &ok;
        assert!(romanian_sanity(&digits).is_some());
    }
}
