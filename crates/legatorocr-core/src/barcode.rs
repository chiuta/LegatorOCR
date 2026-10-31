//! Code 128 bar codes: reading (for separator sheets that split a scan batch into
//! documents and name them) and rendering (to print such separator sheets).
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

/// Find and read a Code 128 bar code made of horizontal bars (page upright, or turned
/// by 180°). Returns the text of the first valid one.
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
                if let Some(t) = decode_runs(&runs, fd) {
                    return Some(t);
                }
                let mut rev = runs.clone();
                rev.reverse();
                // after reversal the first run is the old last run
                let last_dark = if runs.len() % 2 == 1 { fd } else { !fd };
                if let Some(t) = decode_runs(&rev, last_dark) {
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
