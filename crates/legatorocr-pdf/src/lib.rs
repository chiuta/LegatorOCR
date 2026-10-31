//! Minimal, dependency-light PDF writer.
//!
//! * colour/gray pages → JPEG (DCT) or Flate
//! * black & white pages → 1-bit image, Flate + PNG "Up" predictor
//! * optional invisible text layer (render mode 3) from OCR words, using a
//!   glyph-less Type0 font with a ToUnicode CMap so any Unicode text (e.g.
//!   Romanian ăâîșț) is searchable and copy-able.

pub mod g4;
pub mod reader;

use flate2::{write::ZlibEncoder, Compression};
use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};
use legatorocr_core::{BitImage, Page, PageImage, Word};
use std::collections::BTreeMap;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compress {
    /// JPEG for colour/gray pages (quality 1..=100); B/W pages stay lossless.
    Jpeg(u8),
    /// Lossless Flate everywhere.
    Flate,
}

pub struct PdfWriter {
    buf: Vec<u8>,
    offsets: Vec<Option<usize>>, // index = object id
    page_ids: Vec<u32>,
    chars: BTreeMap<char, u16>, // char -> CID
    compress: Compress,
    title: Option<String>,
    meta: Meta,
}

/// Optional document-information entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// Seconds since the Unix epoch; written as CreationDate / ModDate (UTC).
    pub created_unix: Option<u64>,
}

/// `D:YYYYMMDDHHmmSSZ` for a Unix time (UTC, proleptic Gregorian).
pub fn pdf_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant's public-domain algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

const CATALOG: u32 = 1;
const PAGES: u32 = 2;
const FONT: u32 = 3;
const CIDFONT: u32 = 4;
const DESCRIPTOR: u32 = 5;
const TOUNICODE: u32 = 6;
const FIRST_FREE: u32 = 7;

impl PdfWriter {
    pub fn new(compress: Compress) -> Self {
        let mut w = Self {
            buf: Vec::new(),
            offsets: vec![None; FIRST_FREE as usize],
            page_ids: vec![],
            chars: BTreeMap::new(),
            compress,
            title: None,
            meta: Meta::default(),
        };
        w.buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
        w
    }

    pub fn set_title(&mut self, t: &str) {
        self.title = Some(t.to_string());
    }

    pub fn set_meta(&mut self, m: Meta) {
        self.meta = m;
    }

    fn alloc(&mut self) -> u32 {
        self.offsets.push(None);
        (self.offsets.len() - 1) as u32
    }

    fn begin(&mut self, id: u32) {
        self.offsets[id as usize] = Some(self.buf.len());
        writeln!(self.buf, "{id} 0 obj").unwrap();
    }

    fn stream_obj(&mut self, id: u32, dict_extra: &str, data: &[u8]) {
        self.begin(id);
        write!(
            self.buf,
            "<< {dict_extra} /Length {} >>\nstream\n",
            data.len()
        )
        .unwrap();
        self.buf.extend_from_slice(data);
        self.buf.extend_from_slice(b"\nendstream\nendobj\n");
    }

    fn plain_obj(&mut self, id: u32, body: &str) {
        self.begin(id);
        write!(self.buf, "{body}\nendobj\n").unwrap();
    }

    /// CID for `c`; `None` once the 16-bit code space is exhausted (the word is then skipped).
    fn cid(&mut self, c: char) -> Option<u16> {
        if let Some(&id) = self.chars.get(&c) {
            return Some(id);
        }
        let next = u16::try_from(self.chars.len() + 1)
            .ok()
            .filter(|&n| n < u16::MAX)?;
        self.chars.insert(c, next);
        Some(next)
    }

    /// Append one page. `words` (optional) are in the page's pixel coordinates.
    pub fn add_page(&mut self, page: &Page, words: &[Word]) {
        let (pw, ph) = (page.image.width(), page.image.height());
        let dpi = page.dpi.clamp(36, 2400) as f32;
        let (wpt, hpt) = (pw as f32 * 72.0 / dpi, ph as f32 * 72.0 / dpi);

        if let Some(l) = &page.layers {
            self.add_layered(l, dpi, wpt, hpt, words);
            return;
        }

        // --- image XObject -------------------------------------------
        let img_id = self.alloc();
        // JPEG cannot exceed 65535 px per side; such pages stay lossless.
        let compress = if pw > 65_000 || ph > 65_000 {
            Compress::Flate
        } else {
            self.compress
        };
        match &page.image {
            PageImage::Bw(b) => {
                let (filter, z) = encode_1bit(b);
                self.stream_obj(
                    img_id,
                    &format!(
                        "/Type /XObject /Subtype /Image /Width {pw} /Height {ph} \
                         /ColorSpace /DeviceGray /BitsPerComponent 1 {filter}"
                    ),
                    &z,
                );
            }
            PageImage::Gray(g) => match compress {
                Compress::Jpeg(q) => {
                    let mut out = Vec::new();
                    JpegEncoder::new_with_quality(&mut out, q)
                        .encode(g.as_raw(), pw, ph, ExtendedColorType::L8)
                        .expect("jpeg encode");
                    self.stream_obj(img_id, &image_dict(pw, ph, "DeviceGray", "DCTDecode"), &out);
                }
                Compress::Flate => {
                    let z = deflate(g.as_raw());
                    self.stream_obj(img_id, &image_dict(pw, ph, "DeviceGray", "FlateDecode"), &z);
                }
            },
            PageImage::Rgb(c) => match compress {
                Compress::Jpeg(q) => {
                    let mut out = Vec::new();
                    JpegEncoder::new_with_quality(&mut out, q)
                        .encode(c.as_raw(), pw, ph, ExtendedColorType::Rgb8)
                        .expect("jpeg encode");
                    self.stream_obj(img_id, &image_dict(pw, ph, "DeviceRGB", "DCTDecode"), &out);
                }
                Compress::Flate => {
                    let z = deflate(c.as_raw());
                    self.stream_obj(img_id, &image_dict(pw, ph, "DeviceRGB", "FlateDecode"), &z);
                }
            },
        }

        let mut cs = format!("q {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Im0 Do Q\n");
        self.text_ops(&mut cs, words, dpi, wpt, hpt);
        self.finish_page(cs, &format!("/Im0 {img_id} 0 R"), wpt, hpt);
    }

    /// Invisible-text operators for OCR words.
    fn text_ops(&mut self, cs: &mut String, words: &[Word], dpi: f32, wpt: f32, hpt: f32) {
        if words.is_empty() {
            return;
        }
        cs.push_str("BT 3 Tr\n");
        for w in words {
            if w.text.trim().is_empty() || w.w == 0 || w.h == 0 {
                continue;
            }
            let n = w.text.chars().count() as f32;
            let size = (w.h as f32 * 72.0 / dpi).max(1.0);
            let (x, top) = (w.x as f32 * 72.0 / dpi, w.y as f32 * 72.0 / dpi);
            // Words outside the page or taller than it are garbage (a PDF reader may choke on
            // 10-digit coordinates): drop them rather than write absurd numbers.
            if !(x.is_finite() && top.is_finite() && size.is_finite())
                || x >= wpt
                || top >= hpt
                || size > hpt
            {
                continue;
            }
            // Glyph advance is 0.5 em; scale horizontally to the word box (kept within sane bounds).
            let tz = ((w.w as f32 * 72.0 / dpi) / (n * 0.5 * size) * 100.0).clamp(5.0, 2000.0);
            let y = hpt - top - size + size * 0.2;
            let mut hex = String::new();
            for ch in w.text.chars().chain(std::iter::once(' ')) {
                match self.cid(ch) {
                    Some(id) => hex.push_str(&format!("{id:04X}")),
                    None => {
                        hex.clear();
                        break;
                    }
                }
            }
            if hex.is_empty() {
                continue;
            }
            cs.push_str(&format!(
                "/F1 {size:.2} Tf {tz:.1} Tz 1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"
            ));
        }
        cs.push_str("ET\n");
    }

    fn finish_page(&mut self, cs: String, xobjects: &str, wpt: f32, hpt: f32) {
        let cs_id = self.alloc();
        let z = deflate(cs.as_bytes());
        self.stream_obj(cs_id, "/Filter /FlateDecode", &z);
        let page_id = self.alloc();
        self.plain_obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {wpt:.3} {hpt:.3}] \
                 /Resources << /XObject << {xobjects} >> /Font << /F1 {FONT} 0 R >> >> \
                 /Contents {cs_id} 0 R >>"
            ),
        );
        self.page_ids.push(page_id);
    }

    fn rgb_xobject(&mut self, c: &image::RgbImage) -> u32 {
        let (w, h) = c.dimensions();
        let id = self.alloc();
        match self.compress {
            Compress::Jpeg(q) if w <= 65_000 && h <= 65_000 => {
                let mut out = Vec::new();
                JpegEncoder::new_with_quality(&mut out, q)
                    .encode(c.as_raw(), w, h, ExtendedColorType::Rgb8)
                    .expect("jpeg encode");
                self.stream_obj(id, &image_dict(w, h, "DeviceRGB", "DCTDecode"), &out);
            }
            _ => {
                let z = deflate(c.as_raw());
                self.stream_obj(id, &image_dict(w, h, "DeviceRGB", "FlateDecode"), &z);
            }
        }
        id
    }

    /// Layered page: optional low-res background, picture patches, then the
    /// 1-bit text mask painted as a stencil in the ink colour.
    fn add_layered(
        &mut self,
        l: &legatorocr_core::layers::Layers,
        dpi: f32,
        wpt: f32,
        hpt: f32,
        words: &[Word],
    ) {
        let (pw, ph) = (l.mask.w, l.mask.h);
        let mut xo = String::new();
        let mut cs = String::new();
        match &l.bg {
            Some(b) => {
                let id = self.rgb_xobject(&b.img);
                xo.push_str(&format!("/Bg {id} 0 R "));
                cs.push_str(&format!("q {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Bg Do Q\n"));
            }
            None => cs.push_str(&format!("q 1 1 1 rg 0 0 {wpt:.3} {hpt:.3} re f Q\n")),
        }
        for (i, p) in l.patches.iter().enumerate() {
            let id = self.rgb_xobject(&p.img);
            xo.push_str(&format!("/P{i} {id} 0 R "));
            let (w, h) = (
                p.img.width() as f32 * 72.0 / dpi,
                p.img.height() as f32 * 72.0 / dpi,
            );
            let (x, y) = (p.x as f32 * 72.0 / dpi, hpt - (p.y as f32 * 72.0 / dpi) - h);
            cs.push_str(&format!(
                "q {w:.3} 0 0 {h:.3} {x:.3} {y:.3} cm /P{i} Do Q\n"
            ));
        }
        let mask_id = self.alloc();
        let (filter, z) = encode_1bit(&l.mask);
        self.stream_obj(
            mask_id,
            &format!("/Type /XObject /Subtype /Image /Width {pw} /Height {ph} /ImageMask true /BitsPerComponent 1 {filter}"),
            &z,
        );
        xo.push_str(&format!("/Mk {mask_id} 0 R"));
        let [r, g, b] = l.fg.map(|c| c as f32 / 255.0);
        cs.push_str(&format!(
            "q {r:.3} {g:.3} {b:.3} rg {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Mk Do Q\n"
        ));
        self.text_ops(&mut cs, words, dpi, wpt, hpt);
        self.finish_page(cs, &xo, wpt, hpt);
    }

    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    /// Write the shared objects, xref and trailer; returns the finished file.
    pub fn finish(mut self) -> Vec<u8> {
        // Shared font objects.
        self.plain_obj(
            FONT,
            &format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H \
                 /DescendantFonts [{CIDFONT} 0 R] /ToUnicode {TOUNICODE} 0 R >>"
            ),
        );
        self.plain_obj(
            CIDFONT,
            &format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {DESCRIPTOR} 0 R /DW 500 /CIDToGIDMap /Identity >>"
            ),
        );
        self.plain_obj(
            DESCRIPTOR,
            "<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 \
             /FontBBox [0 0 500 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 \
             /CapHeight 1000 /StemV 80 >>",
        );
        let cmap = self.to_unicode_cmap();
        let z = deflate(cmap.as_bytes());
        self.stream_obj(TOUNICODE, "/Filter /FlateDecode", &z);

        let kids: String = self
            .page_ids
            .iter()
            .map(|id| format!("{id} 0 R "))
            .collect();
        self.plain_obj(
            PAGES,
            &format!(
                "<< /Type /Pages /Kids [{kids}] /Count {} >>",
                self.page_ids.len()
            ),
        );
        self.plain_obj(CATALOG, &format!("<< /Type /Catalog /Pages {PAGES} 0 R >>"));
        let info_id = self.alloc();
        let title = self
            .title
            .as_deref()
            .map(|t| format!(" /Title {}", pdf_text_string(t)))
            .unwrap_or_default();
        let mut extra = String::new();
        for (key, v) in [
            ("Author", &self.meta.author),
            ("Subject", &self.meta.subject),
            ("Keywords", &self.meta.keywords),
        ] {
            if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
                extra.push_str(&format!(" /{key} {}", pdf_text_string(v)));
            }
        }
        if let Some(t) = self.meta.created_unix {
            let d = pdf_date(t);
            extra.push_str(&format!(" /CreationDate ({d}) /ModDate ({d})"));
        }
        self.plain_obj(
            info_id,
            &format!(
                "<< /Producer (LegatorOCR {}){title}{extra} >>",
                env!("CARGO_PKG_VERSION")
            ),
        );

        let xref_pos = self.buf.len();
        let n = self.offsets.len();
        write!(self.buf, "xref\n0 {n}\n0000000000 65535 f \n").unwrap();
        for id in 1..n {
            let off = self.offsets[id].expect("every object written");
            writeln!(self.buf, "{off:010} 00000 n ").unwrap();
        }
        write!(
            self.buf,
            "trailer\n<< /Size {n} /Root {CATALOG} 0 R /Info {info_id} 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n"
        )
        .unwrap();
        self.buf
    }

    fn to_unicode_cmap(&self) -> String {
        let mut s = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let entries: Vec<(&char, &u16)> = self.chars.iter().collect();
        for chunk in entries.chunks(100) {
            s.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (c, cid) in chunk {
                let mut u = [0u16; 2];
                let hex: String = c
                    .encode_utf16(&mut u)
                    .iter()
                    .map(|x| format!("{x:04X}"))
                    .collect();
                s.push_str(&format!("<{cid:04X}> <{hex}>\n"));
            }
            s.push_str("endbfchar\n");
        }
        s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        s
    }
}

fn image_dict(w: u32, h: u32, cs: &str, filter: &str) -> String {
    format!(
        "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /{cs} \
         /BitsPerComponent 8 /Filter /{filter}"
    )
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// Best encoding of a 1-bit page: (extra image-dict entries, bytes).
fn encode_1bit(b: &BitImage) -> (String, Vec<u8>) {
    let w = b.w as usize;
    let g4 = g4::encode(
        w,
        (0..b.h as usize).map(|y| b.data[y * w..(y + 1) * w].iter().map(|&v| v != 0).collect()),
    );
    let flate = deflate(&pack_1bit_up_predicted(b));
    if g4.len() <= flate.len() {
        (
            format!("/Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns {} /Rows {} /BlackIs1 false >>", b.w, b.h),
            g4,
        )
    } else {
        (
            format!(
                "/Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 1 /Columns {} >>",
                b.w
            ),
            flate,
        )
    }
}

/// 1-bit DeviceGray rows (0 = black = ink), each prefixed with PNG filter
/// type 2 ("Up") so Flate sees long zero runs between similar rows.
fn pack_1bit_up_predicted(b: &BitImage) -> Vec<u8> {
    let rb = (b.w as usize).div_ceil(8);
    let mut prev = vec![0u8; rb];
    let mut out = Vec::with_capacity((rb + 1) * b.h as usize);
    for y in 0..b.h as usize {
        let mut row = vec![0xFFu8; rb]; // padding bits stay white
        for x in 0..b.w as usize {
            if b.data[y * b.w as usize + x] != 0 {
                row[x / 8] &= !(0x80 >> (x % 8));
            }
        }
        out.push(2);
        out.extend(row.iter().zip(&prev).map(|(c, p)| c.wrapping_sub(*p)));
        prev = row;
    }
    out
}

/// PDF text string: PDFDocEncoding if ASCII, else UTF-16BE hex string.
fn pdf_text_string(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) && !s.contains(['(', ')', '\\']) {
        format!("({s})")
    } else {
        let mut h = String::from("<FEFF");
        for u in s.encode_utf16() {
            h.push_str(&format!("{u:04X}"));
        }
        h.push('>');
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    fn page(img: PageImage) -> Page {
        Page {
            image: img,
            dpi: 300,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        }
    }

    #[test]
    fn pdf_dates_are_correct() {
        assert_eq!(pdf_date(0), "D:19700101000000Z");
        assert_eq!(pdf_date(951_782_400), "D:20000229000000Z"); // leap day
        assert_eq!(pdf_date(1_782_864_000 + 3_661), "D:20260701010101Z");
    }

    #[test]
    fn metadata_lands_in_the_info_dictionary() {
        let mut w = PdfWriter::new(Compress::Flate);
        w.set_meta(Meta {
            author: Some("Ana".into()),
            subject: Some("Să\nvăd".into()),
            keywords: None,
            created_unix: Some(0),
        });
        w.add_page(
            &page(PageImage::Rgb(RgbImage::from_pixel(
                8,
                8,
                image::Rgb([9, 9, 9]),
            ))),
            &[],
        );
        let pdf = String::from_utf8_lossy(&w.finish()).to_string();
        assert!(pdf.contains("/Author (Ana)") && pdf.contains("/Subject <FEFF"));
        assert!(pdf.contains("/CreationDate (D:19700101000000Z)"));
        assert!(!pdf.contains("/Keywords"));
    }

    #[test]
    fn writes_valid_structure() {
        let mut w = PdfWriter::new(Compress::Jpeg(80));
        w.add_page(
            &page(PageImage::Rgb(RgbImage::from_pixel(
                30,
                40,
                image::Rgb([200, 10, 10]),
            ))),
            &[],
        );
        let mut bw = BitImage::new(17, 9);
        bw.set(3, 3, true);
        w.add_page(
            &page(PageImage::Bw(bw)),
            &[Word {
                text: "șțăîâ".into(),
                x: 1,
                y: 1,
                w: 10,
                h: 5,
                conf: 90.0,
                line: 0,
            }],
        );
        let pdf = w.finish();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.5"));
        assert!(s.contains("/Count 2"));
        assert!(s.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn code_space_exhaustion_skips_words_instead_of_corrupting() {
        let mut w = PdfWriter::new(Compress::Flate);
        for i in 0..70_000u32 {
            if let Some(c) = char::from_u32(0x4E00 + i) {
                w.cid(c);
            }
        }
        assert!(w.chars.len() < 65_535 && w.cid('\u{1F600}').is_none());
        let words = [Word {
            text: "😀😀".into(),
            x: 1,
            y: 1,
            w: 10,
            h: 5,
            conf: 90.0,
            line: 0,
        }];
        w.add_page(
            &page(PageImage::Gray(image::GrayImage::new(20, 20))),
            &words,
        );
        assert!(w.finish().ends_with(b"%%EOF\n"));
    }

    #[test]
    fn predictor_rows() {
        let mut b = BitImage::new(9, 2);
        b.set(0, 0, true);
        b.set(0, 1, true);
        let d = pack_1bit_up_predicted(&b);
        assert_eq!(d.len(), 2 * (2 + 1));
        assert_eq!(&d[..3], &[2, 0x7F, 0xFF]); // first row vs. zero row
        assert_eq!(&d[3..], &[2, 0, 0]); // identical second row → zeros
    }
}

#[cfg(test)]
mod unicode_file {
    use super::*;
    /// Writes a PDF with Romanian text for external extraction checks:
    /// `LEGATOROCR_TEST_PDF=/path/x.pdf cargo test -p legatorocr-pdf -- --ignored`
    #[test]
    #[ignore]
    fn write_unicode_pdf() {
        let path = std::env::var("LEGATOROCR_TEST_PDF").expect("set LEGATOROCR_TEST_PDF");
        let mut w = PdfWriter::new(Compress::Flate);
        let mut bw = BitImage::new(600, 300);
        bw.set(1, 1, true);
        let page = Page {
            image: PageImage::Bw(bw),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            layers: None,
        };
        let words: Vec<Word> = ["Școala", "țăranului", "înființată", "Âțâră", "e", "😀ok"]
            .iter()
            .enumerate()
            .map(|(i, t)| Word {
                text: t.to_string(),
                x: 20 + (i as u32) * 90,
                y: 40,
                w: 80,
                h: 24,
                conf: 99.0,
                line: 0,
            })
            .collect();
        w.add_page(&page, &words);
        std::fs::write(path, w.finish()).unwrap();
    }
}

#[cfg(test)]
mod g4_roundtrip {
    use super::*;

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// Random, striped, blank, solid and very wide pages must decode exactly.
    /// Needs `pdfimages` (poppler); the test is skipped when it is not installed.
    #[test]
    fn g4_decodes_exactly() {
        if std::process::Command::new("pdfimages")
            .arg("-v")
            .output()
            .is_err()
        {
            eprintln!("pdfimages not installed: skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("locr-g4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let cases: [(u32, u32, u8); 10] = [
            (1, 1, 0),
            (7, 5, 1),
            (64, 40, 2),
            (300, 200, 3),
            (3000, 8, 4),
            (5000, 6, 3),
            (640, 480, 5),
            (100, 100, 6),
            (129, 77, 1),
            (2600, 3, 7),
        ];
        for (i, &(w, h, kind)) in cases.iter().enumerate() {
            let mut b = BitImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let ink = match kind {
                        0 => true,
                        1 => xorshift(&mut seed).is_multiple_of(2),
                        2 => (x / 3 + y / 5) % 2 == 0,
                        3 => xorshift(&mut seed).is_multiple_of(40),
                        4 => (x / 1100) % 2 == 1, // runs longer than 2560
                        5 => (x * x + y * y) % 97 < 20,
                        6 => false,
                        _ => x >= 2562 || (x / 2).is_multiple_of(7),
                    };
                    b.set(x, y, ink);
                }
            }
            let page = Page {
                image: PageImage::Bw(b.clone()),
                dpi: 300,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                layers: None,
            };
            let mut wr = PdfWriter::new(Compress::Flate);
            wr.add_page(&page, &[]);
            let pdf = dir.join(format!("c{i}.pdf"));
            std::fs::write(&pdf, wr.finish()).unwrap();
            let prefix = dir.join(format!("c{i}"));
            let st = std::process::Command::new("pdfimages")
                .arg("-png")
                .arg(&pdf)
                .arg(&prefix)
                .status()
                .unwrap();
            assert!(st.success(), "case {i}");
            let img = image::open(dir.join(format!("c{i}-000.png")))
                .unwrap()
                .to_luma8();
            assert_eq!(img.dimensions(), (w, h), "case {i}");
            for y in 0..h {
                for x in 0..w {
                    let ink = img.get_pixel(x, y)[0] < 128;
                    assert_eq!(ink, b.get(x, y), "case {i} ({w}x{h}) at {x},{y}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
