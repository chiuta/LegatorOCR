//! A self-contained HTML contact sheet of the processed pages for quick quality
//! control of big batches: thumbnail, size, resolution and skew per page, with
//! suspicious pages (blank, size outliers, strong skew correction) highlighted.

use legatorocr_core::Page;

pub struct ReportPage {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub dpi: u32,
    pub skew: f32,
    pub blank: bool,
    /// JPEG thumbnail.
    pub thumb: Vec<u8>,
}

impl ReportPage {
    pub fn from_page(name: String, pg: &Page) -> Self {
        let g = pg.image.to_gray();
        let (w, h) = (g.width().max(1), g.height().max(1));
        let tw = 180u32.min(w);
        let th = ((h as f64 * tw as f64 / w as f64).round() as u32).clamp(1, 400);
        let small = image::imageops::resize(&g, tw, th, image::imageops::FilterType::Triangle);
        let mut buf = Vec::new();
        let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60).encode(
            small.as_raw(),
            tw,
            th,
            image::ExtendedColorType::L8,
        );
        ReportPage {
            name,
            w,
            h,
            dpi: pg.dpi,
            skew: pg.skew_deg,
            blank: pg.blank,
            thumb: buf,
        }
    }
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        o.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    o
}

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Render the report. Returns the HTML and the number of flagged pages.
pub fn html(pages: &[ReportPage]) -> (String, usize) {
    let sizes: Vec<Option<(f32, f32)>> = pages
        .iter()
        .map(|p| {
            (!p.blank && p.dpi > 0).then(|| {
                (
                    p.w as f32 * 25.4 / p.dpi as f32,
                    p.h as f32 * 25.4 / p.dpi as f32,
                )
            })
        })
        .collect();
    let outliers = crate::outlier_indices(&sizes);
    let mut flagged = 0;
    let mut cards = String::new();
    for (i, p) in pages.iter().enumerate() {
        let mut flags: Vec<&str> = Vec::new();
        if p.blank {
            flags.push("blank");
        }
        if outliers.contains(&i) {
            flags.push("size differs");
        }
        if p.skew.abs() > 3.0 {
            flags.push("strong skew fix");
        }
        if !flags.is_empty() {
            flagged += 1;
        }
        cards.push_str(&format!(
            "<figure class=\"{}\"><img alt=\"\" src=\"data:image/jpeg;base64,{}\"><figcaption><b>{}</b><br>{}×{} px · {} dpi<br>skew {:+.2}°{}</figcaption></figure>\n",
            if flags.is_empty() { "" } else { "flag" },
            base64(&p.thumb),
            esc(&p.name),
            p.w,
            p.h,
            p.dpi,
            p.skew,
            if flags.is_empty() { String::new() } else { format!("<br><em>{}</em>", flags.join(", ")) }
        ));
    }
    let doc = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>LegatorOCR report</title><style>\
         body{{font:14px system-ui,sans-serif;margin:16px;background:#f4f4f4;color:#222}}\
         main{{display:flex;flex-wrap:wrap;gap:12px}}figure{{margin:0;width:190px;background:#fff;padding:6px;border:2px solid #ddd}}\
         figure.flag{{border-color:#e08a00;background:#fff6e5}}img{{width:100%;display:block;background:#ccc}}\
         figcaption{{font-size:12px;margin-top:4px}}@media(prefers-color-scheme:dark){{body{{background:#1c1c1c;color:#ddd}}figure{{background:#2a2a2a;border-color:#444}}figure.flag{{background:#3a2e14}}}}\
         </style></head><body><h1>LegatorOCR report</h1><p>{} page(s), {} flagged.</p><main>\n{}</main></body></html>\n",
        pages.len(),
        flagged,
        cards
    );
    (doc, flagged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use legatorocr_core::{Page, PageImage};

    fn pg(w: u32, h: u32, blank: bool, skew: f32) -> Page {
        Page {
            image: PageImage::Gray(image::GrayImage::from_pixel(w, h, image::Luma([200]))),
            dpi: 150,
            skew_deg: skew,
            blank,
            split_index: None,
            layers: None,
        }
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn report_flags_the_odd_pages() {
        let mut pages: Vec<ReportPage> = (0..10)
            .map(|i| ReportPage::from_page(format!("p<{i}>"), &pg(900, 1300, false, 0.1)))
            .collect();
        pages.push(ReportPage::from_page(
            "small".into(),
            &pg(400, 500, false, 0.0),
        ));
        pages.push(ReportPage::from_page(
            "empty".into(),
            &pg(900, 1300, true, 0.0),
        ));
        pages.push(ReportPage::from_page(
            "tilted".into(),
            &pg(900, 1300, false, 5.0),
        ));
        let (html, flagged) = html(&pages);
        assert_eq!(flagged, 3);
        assert!(
            html.contains("size differs")
                && html.contains("blank")
                && html.contains("strong skew fix")
        );
        assert!(html.contains("p&lt;0&gt;") && !html.contains("p<0>"));
        assert!(html.contains("data:image/jpeg;base64,/9j/"));
    }
}
