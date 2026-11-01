//! Bar codes: reading (for separator sheets that split a scan batch into documents and
//! name them) of Code 128 (also rendered, to print such sheets), EAN-13, Interleaved 2 of 5
//! and Code 39.
//! Implemented from the public symbology description: 106 symbols of 6 alternating
//! bar/space widths summing to 11 modules, a 7-element stop pattern, and a
//! modulo-103 checksum.

use image::{GrayImage, Luma};

const PATTERNS: [&str; 106] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232",
];
const STOP: &str = "2331112";
const START_A: usize = 103;
const START_B: usize = 104;
const START_C: usize = 105;

fn widths(p: &str) -> [f32; 6] {
    let mut w = [0f32; 6];
    for (i, c) in p.bytes().take(6).enumerate() {
        w[i] = (c - b'0') as f32;
    }
    w
}

/// Symbol values for `text` (start, data, checksum). ASCII 32..=126 only; all-digit
/// even-length text uses the compact numeric set.
fn symbols(text: &str) -> Option<Vec<usize>> {
    if text.is_empty() || text.len() > 80 || !text.bytes().all(|b| (32..=126).contains(&b)) {
        return None;
    }
    let b = text.as_bytes();
    let mut v = Vec::new();
    if b.len().is_multiple_of(2) && b.iter().all(u8::is_ascii_digit) {
        v.push(START_C);
        for p in b.chunks(2) {
            v.push(((p[0] - b'0') * 10 + (p[1] - b'0')) as usize);
        }
    } else {
        v.push(START_B);
        v.extend(b.iter().map(|&c| (c - 32) as usize));
    }
    let sum: usize = v[0]
        + v.iter()
            .enumerate()
            .skip(1)
            .map(|(i, &x)| i * x)
            .sum::<usize>();
    v.push(sum % 103);
    Some(v)
}

/// Module sequence (true = bar) of a complete symbol, without quiet zones.
pub fn encode(text: &str) -> Option<Vec<bool>> {
    let mut mods = Vec::new();
    let mut push = |p: &str| {
        let mut bar = true;
        for c in p.bytes() {
            for _ in 0..(c - b'0') {
                mods.push(bar);
            }
            bar = !bar;
        }
    };
    for s in symbols(text)? {
        push(PATTERNS[s]);
    }
    push(STOP);
    Some(mods)
}

/// Render a printable bar code with 10-module quiet zones.
pub fn render(text: &str, module_px: u32, height_px: u32) -> Option<GrayImage> {
    let m = encode(text)?;
    let mp = module_px.clamp(1, 16);
    let q = 10 * mp;
    let w = m.len() as u32 * mp + 2 * q;
    let h = height_px.clamp(8, 4000);
    let mut img = GrayImage::from_pixel(w, h, Luma([255]));
    for (i, &bar) in m.iter().enumerate() {
        if bar {
            for dx in 0..mp {
                for y in 0..h {
                    img.put_pixel(q + i as u32 * mp + dx, y, Luma([0]));
                }
            }
        }
    }
    Some(img)
}

fn match_symbol(w: &[f32]) -> Option<usize> {
    let t: f32 = w.iter().sum();
    if t <= 0.0 {
        return None;
    }
    let k = 11.0 / t;
    let mut best = (f32::MAX, 0usize);
    for (i, p) in PATTERNS.iter().enumerate() {
        let pw = widths(p);
        let e: f32 = (0..6).map(|j| (w[j] * k - pw[j]).powi(2)).sum();
        if e < best.0 {
            best = (e, i);
        }
    }
    (best.0 < 0.9).then_some(best.1)
}

fn match_stop(w: &[f32]) -> bool {
    let t: f32 = w.iter().sum();
    if t <= 0.0 {
        return false;
    }
    let k = 13.0 / t;
    let sw: Vec<f32> = STOP.bytes().map(|c| (c - b'0') as f32).collect();
    let e: f32 = (0..7).map(|j| (w[j] * k - sw[j]).powi(2)).sum();
    e < 1.2
}

fn values_to_text(v: &[usize]) -> Option<String> {
    // v = start, data..., checksum
    if v.len() < 3 {
        return None;
    }
    let (data, chk) = (&v[1..v.len() - 1], v[v.len() - 1]);
    let sum: usize = v[0]
        + data
            .iter()
            .enumerate()
            .map(|(i, &x)| (i + 1) * x)
            .sum::<usize>();
    if sum % 103 != chk {
        return None;
    }
    let mut set = v[0];
    let mut out = String::new();
    for &x in data {
        match x {
            99 => set = START_C,
            100 => set = START_B,
            101 => set = START_A,
            102 | 98 | 103..=105 => return None, // FNC1/shift/start inside: not supported
            _ => match set {
                START_C => {
                    if x > 99 {
                        return None;
                    }
                    out.push_str(&format!("{x:02}"));
                }
                START_B => out.push((32 + x as u8) as char),
                _ => {
                    if x < 64 {
                        out.push((32 + x as u8) as char);
                    } else {
                        return None;
                    }
                }
            },
        }
    }
    (!out.is_empty()).then_some(out)
}

fn decode_runs(runs: &[f32], first_dark: bool) -> Option<String> {
    let mut i = if first_dark { 0 } else { 1 };
    while i + 6 + 7 <= runs.len() {
        if let Some(start) =
            match_symbol(&runs[i..i + 6]).filter(|s| (START_A..=START_C).contains(s))
        {
            let mut vals = vec![start];
            let mut j = i + 6;
            while vals.len() < 90 && j + 6 <= runs.len() {
                if j + 7 <= runs.len() && vals.len() >= 3 && match_stop(&runs[j..j + 7]) {
                    if let Some(t) = values_to_text(&vals) {
                        return Some(t);
                    }
                    break;
                }
                match match_symbol(&runs[j..j + 6]) {
                    Some(s) => vals.push(s),
                    None => break,
                }
                j += 6;
            }
        }
        i += 2;
    }
    None
}

fn row_runs(row: &[u8], bias: f32) -> Option<(Vec<f32>, bool)> {
    let (lo, hi) = row
        .iter()
        .fold((255u8, 0u8), |a, &p| (a.0.min(p), a.1.max(p)));
    if hi < lo.saturating_add(60) {
        return None;
    }
    let thr = ((lo as u16 + hi as u16) / 2) as u8;
    let mut runs: Vec<f32> = Vec::new();
    let first_dark = row[0] < thr;
    let mut cur = first_dark;
    let mut n = 0f32;
    for &p in row {
        let d = p < thr;
        if d == cur {
            n += 1.0;
        } else {
            runs.push(n);
            cur = d;
            n = 1.0;
        }
    }
    runs.push(n);
    if runs.len() < 20 || runs.len() > 4000 {
        return None;
    }
    // printing/scanning makes bars fatter or thinner: undo a constant bias
    let mut dark = first_dark;
    for r in runs.iter_mut() {
        *r = (*r + if dark { bias } else { -bias }).max(0.2);
        dark = !dark;
    }
    Some((runs, first_dark))
}

// ------------------------------------------------------------ other symbologies
// Code 39, Interleaved 2 of 5 and EAN-13 share the run-length front end of Code 128.

const CODE39: [(&str, &str); 43] = [
    ("0", "nnnwwnwnn"),
    ("1", "wnnwnnnnw"),
    ("2", "nnwwnnnnw"),
    ("3", "wnwwnnnnn"),
    ("4", "nnnwwnnnw"),
    ("5", "wnnwwnnnn"),
    ("6", "nnwwwnnnn"),
    ("7", "nnnwnnwnw"),
    ("8", "wnnwnnwnn"),
    ("9", "nnwwnnwnn"),
    ("A", "wnnnnwnnw"),
    ("B", "nnwnnwnnw"),
    ("C", "wnwnnwnnn"),
    ("D", "nnnnwwnnw"),
    ("E", "wnnnwwnnn"),
    ("F", "nnwnwwnnn"),
    ("G", "nnnnnwwnw"),
    ("H", "wnnnnwwnn"),
    ("I", "nnwnnwwnn"),
    ("J", "nnnnwwwnn"),
    ("K", "wnnnnnnww"),
    ("L", "nnwnnnnww"),
    ("M", "wnwnnnnwn"),
    ("N", "nnnnwnnww"),
    ("O", "wnnnwnnwn"),
    ("P", "nnwnwnnwn"),
    ("Q", "nnnnnnwww"),
    ("R", "wnnnnnwwn"),
    ("S", "nnwnnnwwn"),
    ("T", "nnnnwnwwn"),
    ("U", "wwnnnnnnw"),
    ("V", "nwwnnnnnw"),
    ("W", "wwwnnnnnn"),
    ("X", "nwnnwnnnw"),
    ("Y", "wwnnwnnnn"),
    ("Z", "nwwnwnnnn"),
    ("-", "nwnnnnwnw"),
    (".", "wwnnnnwnn"),
    (" ", "nwwnnnwnn"),
    ("$", "nwnwnwnnn"),
    ("/", "nwnwnnnwn"),
    ("+", "nwnnnwnwn"),
    ("%", "nnnwnwnwn"),
];
const CODE39_STAR: &str = "nwnnwnwnn";

/// Narrow/wide classification of `w.len()` elements of which exactly `wide` are wide.
fn nw_mask(w: &[f32], wide: usize) -> Option<String> {
    let mut idx: Vec<usize> = (0..w.len()).collect();
    idx.sort_by(|&a, &b| w[b].partial_cmp(&w[a]).unwrap_or(std::cmp::Ordering::Equal));
    let (wmin, nmax) = (w[idx[wide - 1]], w[idx[wide]]);
    let avg = |r: &[usize]| r.iter().map(|&i| w[i]).sum::<f32>() / r.len() as f32;
    let (aw, an) = (avg(&idx[..wide]), avg(&idx[wide..]));
    if wmin <= nmax || aw / an.max(0.1) < 1.6 || aw / an.max(0.1) > 4.5 {
        return None;
    }
    let mut m = vec!['n'; w.len()];
    for &i in &idx[..wide] {
        m[i] = 'w';
    }
    Some(m.into_iter().collect())
}

/// All narrow elements of a symbol must be alike, all wide ones too, at a ratio near 2–3.
fn uniform(narrow: &[f32], wide: &[f32]) -> bool {
    if narrow.len() < 4 || wide.is_empty() {
        return false;
    }
    let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (an, aw) = (avg(narrow), avg(wide));
    narrow.iter().all(|&x| (x / an - 1.0).abs() < 0.4)
        && wide.iter().all(|&x| (x / aw - 1.0).abs() < 0.4)
        && (1.8..3.4).contains(&(aw / an))
        && an >= 1.5
}

fn take_elems(nar: &mut Vec<f32>, wid: &mut Vec<f32>, w: &[f32], wide: usize) {
    if let Some(m) = nw_mask(w, wide) {
        for (x, c) in w.iter().zip(m.chars()) {
            if c == 'w' {
                wid.push(*x)
            } else {
                nar.push(*x)
            }
        }
    }
}

fn code39_char(w: &[f32]) -> Option<&'static str> {
    let m = nw_mask(w, 3)?;
    if m == CODE39_STAR {
        return Some("*");
    }
    CODE39.iter().find(|(_, p)| *p == m).map(|(c, _)| *c)
}

fn decode_39(runs: &[f32], first_dark: bool) -> Option<String> {
    let mut i = if first_dark { 0 } else { 1 };
    while i + 9 * 4 + 3 <= runs.len() {
        if code39_char(&runs[i..i + 9]) == Some("*") {
            let (mut nar, mut wid): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
            take_elems(&mut nar, &mut wid, &runs[i..i + 9], 3);
            let mut out = String::new();
            let mut j = i + 9;
            while j + 10 <= runs.len() {
                let gap = runs[j];
                match code39_char(&runs[j + 1..j + 10]) {
                    Some("*") => {
                        take_elems(&mut nar, &mut wid, &runs[j + 1..j + 10], 3);
                        nar.push(gap);
                        if out.len() >= 2 && uniform(&nar, &wid) {
                            return Some(out);
                        }
                        break;
                    }
                    Some(c) => {
                        take_elems(&mut nar, &mut wid, &runs[j + 1..j + 10], 3);
                        nar.push(gap);
                        out.push_str(c);
                    }
                    None => break,
                }
                j += 10;
                if out.len() > 60 {
                    break;
                }
            }
        }
        i += 2;
    }
    None
}

const ITF: [&str; 10] = [
    "nnwwn", "wnnnw", "nwnnw", "wwnnn", "nnwnw", "wnwnn", "nwwnn", "nnnww", "wnnwn", "nwnwn",
];

fn itf_digit(w: &[f32]) -> Option<usize> {
    let m = nw_mask(w, 2)?;
    ITF.iter().position(|p| *p == m)
}

fn decode_itf(runs: &[f32], first_dark: bool) -> Option<String> {
    let mut i = if first_dark { 0 } else { 1 };
    while i + 4 + 10 + 3 <= runs.len() {
        let st = &runs[i..i + 4];
        let (lo, hi) = (
            st.iter().cloned().fold(f32::MAX, f32::min),
            st.iter().cloned().fold(0.0, f32::max),
        );
        if hi <= lo * 1.4 {
            let mut nar: Vec<f32> = st.to_vec();
            let mut wid: Vec<f32> = Vec::new();
            let narrow = st.iter().sum::<f32>() / 4.0;
            let mut j = i + 4;
            let mut out = String::new();
            while j + 3 <= runs.len() {
                // stop pattern: wide bar, narrow space, narrow bar, then quiet zone
                if runs[j] > narrow * 1.7
                    && runs[j + 1] < narrow * 1.4
                    && runs[j + 2] < narrow * 1.4
                    && (j + 3 == runs.len() || runs[j + 3] > narrow * 4.0)
                {
                    wid.push(runs[j]);
                    nar.push(runs[j + 1]);
                    nar.push(runs[j + 2]);
                    if out.len() >= 6 && uniform(&nar, &wid) {
                        return Some(out);
                    }
                    break;
                }
                if j + 10 > runs.len() {
                    break;
                }
                let bars = [runs[j], runs[j + 2], runs[j + 4], runs[j + 6], runs[j + 8]];
                let spaces = [
                    runs[j + 1],
                    runs[j + 3],
                    runs[j + 5],
                    runs[j + 7],
                    runs[j + 9],
                ];
                match (itf_digit(&bars), itf_digit(&spaces)) {
                    (Some(a), Some(b)) => {
                        out.push(char::from(b'0' + a as u8));
                        out.push(char::from(b'0' + b as u8));
                        for w in [&bars, &spaces] {
                            if let Some(m) = nw_mask(w, 2) {
                                for (x, c) in w.iter().zip(m.chars()) {
                                    if c == 'w' {
                                        wid.push(*x)
                                    } else {
                                        nar.push(*x)
                                    }
                                }
                            }
                        }
                    }
                    _ => break,
                }
                j += 10;
                if out.len() > 60 {
                    break;
                }
            }
        }
        i += 2;
    }
    None
}

const EAN_L: [&str; 10] = [
    "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011",
    "0110111", "0001011",
];
const EAN_PARITY: [&str; 10] = [
    "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL",
    "LGGLGL",
];

fn bits_to_runs(b: &str) -> [f32; 4] {
    let mut r = [0f32; 4];
    let mut k = 0;
    let bytes = b.as_bytes();
    // every EAN digit is space, bar, space, bar (starts with a 0 module)
    let mut cur = bytes[0];
    for &c in bytes {
        if c == cur {
            r[k] += 1.0;
        } else {
            k += 1;
            r[k] += 1.0;
            cur = c;
        }
    }
    r
}

fn ean_digit(w: &[f32], right: bool) -> Option<(usize, char)> {
    let t: f32 = w.iter().sum();
    if t <= 0.0 {
        return None;
    }
    let k = 7.0 / t;
    let mut best = (f32::MAX, 0usize, 'L');
    for d in 0..10 {
        let l: String = EAN_L[d].into();
        let r: String = l
            .chars()
            .map(|c| if c == '0' { '1' } else { '0' })
            .collect();
        let g: String = r.chars().rev().collect();
        let sets: Vec<(char, &String)> = if right {
            vec![('R', &r)]
        } else {
            vec![('L', &l), ('G', &g)]
        };
        for (set, code) in sets {
            let pw = bits_to_runs(code);
            let e: f32 = (0..4).map(|j| (w[j] * k - pw[j]).powi(2)).sum();
            if e < best.0 {
                best = (e, d, set);
            }
        }
    }
    (best.0 < 0.8).then_some((best.1, best.2))
}

fn decode_ean13(runs: &[f32], first_dark: bool) -> Option<String> {
    let mut i = if first_dark { 0 } else { 1 };
    while i + 59 <= runs.len() {
        let r = &runs[i..i + 59];
        let total: f32 = r.iter().sum();
        let m = total / 95.0;
        let near = |v: f32, n: f32| (v / m - n).abs() < 0.6;
        if near(r[0], 1.0)
            && near(r[1], 1.0)
            && near(r[2], 1.0)
            && (27..32).all(|k| near(r[k], 1.0))
            && near(r[56], 1.0)
            && near(r[57], 1.0)
            && near(r[58], 1.0)
        {
            let mut digits = Vec::new();
            let mut parity = String::new();
            let mut ok = true;
            for g in 0..6 {
                match ean_digit(&r[3 + 4 * g..7 + 4 * g], false) {
                    Some((d, 'L')) => {
                        digits.push(d);
                        parity.push('L');
                    }
                    Some((d, 'G')) => {
                        digits.push(d);
                        parity.push('G');
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                for g in 0..6 {
                    match ean_digit(&r[32 + 4 * g..36 + 4 * g], true) {
                        Some((d, 'R')) => digits.push(d),
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            if ok {
                if let Some(first) = EAN_PARITY.iter().position(|p| *p == parity) {
                    let mut all = vec![first];
                    all.extend(digits);
                    let sum: usize = all[..12]
                        .iter()
                        .enumerate()
                        .map(|(k, &d)| if k % 2 == 0 { d } else { 3 * d })
                        .sum();
                    if (10 - sum % 10) % 10 == all[12] {
                        return Some(all.iter().map(|d| char::from(b'0' + *d as u8)).collect());
                    }
                }
            }
        }
        i += 2;
    }
    None
}

fn decode_any_runs(runs: &[f32], first_dark: bool) -> Option<String> {
    decode_runs(runs, first_dark)
        .or_else(|| decode_ean13(runs, first_dark))
        .or_else(|| decode_itf(runs, first_dark))
        .or_else(|| decode_39(runs, first_dark))
}

/// Find and read a bar code made of horizontal bars (Code 128, EAN-13, Interleaved 2 of 5
/// or Code 39; page upright, or turned by 180°). Returns the text of the first valid one.
pub fn decode(img: &GrayImage) -> Option<String> {
    let (w, h) = img.dimensions();
    if w < 60 || h < 8 {
        return None;
    }
    let raw = img.as_raw();
    let rows = 48u32.min(h);
    for k in 0..rows {
        let y = ((k as u64 * 2 + 1) * h as u64 / (rows as u64 * 2)) as usize;
        let row = &raw[y * w as usize..(y + 1) * w as usize];
        for bias in [0.0f32, 0.5, -0.5, 1.0, -1.0] {
            if let Some((runs, fd)) = row_runs(row, bias) {
                if let Some(t) = decode_any_runs(&runs, fd) {
                    return Some(t);
                }
                let mut rev = runs.clone();
                rev.reverse();
                // after reversal the first run is the old last run
                let last_dark = if runs.len() % 2 == 1 { fd } else { !fd };
                if let Some(t) = decode_any_runs(&rev, last_dark) {
                    return Some(t);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_symbol_widths() {
        // checked against an independent implementation's tables
        assert_eq!(PATTERNS[0], "212222");
        assert_eq!(PATTERNS[103], "211412");
        assert_eq!(PATTERNS[105], "211232");
        assert_eq!(PATTERNS.len(), 106);
        assert!(PATTERNS
            .iter()
            .all(|p| p.bytes().map(|c| (c - b'0') as u32).sum::<u32>() == 11));
    }

    #[test]
    fn matches_independent_encoder_output() {
        // module strings produced by the python-barcode package (13-module stop, no quiet zone)
        let cases = [
            ("Hello World", "110100100001100010100010110010000110010100001100101000010001111010110110011001110100011010001111010100100111101100101000010000100110101100011101100011101011"),
            ("1234567890", "110100111001011001110010001011000111000101101100001010011011110110100111100101100011101011"),
        ];
        for (t, bits) in cases {
            let m: String = encode(t)
                .unwrap()
                .iter()
                .map(|&b| if b { '1' } else { '0' })
                .collect();
            assert_eq!(m, bits, "{t}");
        }
    }

    #[test]
    fn reads_codes_that_switch_character_sets() {
        // python-barcode starts in set B and switches to C for the digit run
        let bits = "1101001000010110001000100011101101000100011010011011100101110111101101100110011001101100101100100001100011101011";
        let mut img = GrayImage::from_pixel(bits.len() as u32 * 2 + 40, 30, Luma([255]));
        for (i, c) in bits.bytes().enumerate() {
            if c == b'1' {
                for dx in 0..2 {
                    for y in 0..30 {
                        img.put_pixel(20 + i as u32 * 2 + dx, y, Luma([0]));
                    }
                }
            }
        }
        assert_eq!(decode(&img).as_deref(), Some("DOC-0001"));
    }

    fn bits_img(bits: &str, px: u32, quiet: u32) -> GrayImage {
        let mut img = GrayImage::from_pixel(bits.len() as u32 * px + 2 * quiet, 30, Luma([255]));
        for (i, c) in bits.bytes().enumerate() {
            if c == b'1' {
                for dx in 0..px {
                    for y in 0..30 {
                        img.put_pixel(quiet + i as u32 * px + dx, y, Luma([0]));
                    }
                }
            }
        }
        img
    }

    #[test]
    fn reads_code39_itf_and_ean13_from_independent_encoder() {
        // module strings from the python-barcode package
        let c39 = "100010111011101011101010001011101011101000101110100010101110111011101000101011101011100010101110100010111011101";
        let itf = "11001100111110011000001100110011111000001111100111110011000001100110000011111001100000111110000011001100111110011";
        let ean = "10100010110100111011001100100110111101001110101010110011011011001000010101110010011101000100101";
        for px in [2u32, 3] {
            assert_eq!(
                decode(&bits_img(c39, px, 25)).as_deref(),
                Some("AB-12"),
                "code39 px{px}"
            );
            assert_eq!(
                decode(&bits_img(itf, px, 25)).as_deref(),
                Some("123456"),
                "itf px{px}"
            );
            assert_eq!(
                decode(&bits_img(ean, px, 25)).as_deref(),
                Some("5901234123457"),
                "ean px{px}"
            );
        }
        // upside down and blurred
        let turned = image::imageops::rotate180(&bits_img(ean, 3, 25));
        assert_eq!(decode(&turned).as_deref(), Some("5901234123457"));
        let blur = image::imageops::blur(&bits_img(c39, 3, 25), 0.8);
        assert_eq!(decode(&blur).as_deref(), Some("AB-12"));
        // a wrong EAN check digit must not be accepted
        let bad = ean.replacen("1000100101", "1001000101", 1);
        assert!(decode(&bits_img(&bad, 3, 25)).is_none_or(|t| t != "5901234123457"));
    }

    #[test]
    fn roundtrip_text_and_digits() {
        for t in [
            "DOC-0001",
            "Hello World",
            "1234567890",
            "12345",
            "a~Z 9",
            "0",
        ] {
            let img = render(t, 2, 60).unwrap();
            assert_eq!(decode(&img).as_deref(), Some(t), "{t}");
        }
    }

    #[test]
    fn turned_blurred_and_embedded() {
        let bc = render("LOT-77", 3, 80).unwrap();
        let mut page = GrayImage::from_pixel(900, 1200, Luma([240]));
        for (x, y, p) in bc.enumerate_pixels() {
            page.put_pixel(100 + x, 500 + y, *p);
        }
        assert_eq!(decode(&page).as_deref(), Some("LOT-77"));
        let turned = image::imageops::rotate180(&page);
        assert_eq!(decode(&turned).as_deref(), Some("LOT-77"));
        let blur = image::imageops::blur(&page, 1.0);
        assert_eq!(decode(&blur).as_deref(), Some("LOT-77"));
    }

    #[test]
    fn rejects_noise_and_bad_input() {
        assert!(render("", 2, 40).is_none());
        assert!(render("é", 2, 40).is_none());
        assert!(decode(&GrayImage::from_pixel(500, 500, Luma([255]))).is_none());
        let mut s = 12345u64;
        let noise = GrayImage::from_fn(400, 100, |_, _| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            Luma([(s & 255) as u8])
        });
        assert!(decode(&noise).is_none());
    }
}
