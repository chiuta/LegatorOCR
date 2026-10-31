//! Minimal, dependency-light PDF writer.
//!
//! * colour/gray pages → JPEG (DCT) or Flate
//! * black & white pages → 1-bit image, Flate + PNG "Up" predictor
//! * optional invisible text layer (render mode 3) from OCR words, using a
//!   glyph-less Type0 font with a ToUnicode CMap so any Unicode text (e.g.
//!   Romanian ăâîșț) is searchable and copy-able.

use legatorocr_core::{BitImage, Page, PageImage, Word};
use flate2::{write::ZlibEncoder, Compression};
use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};
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
        };
        w.buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
        w
    }

    pub fn set_title(&mut self, t: &str) {
        self.title = Some(t.to_string());
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
        write!(self.buf, "<< {dict_extra} /Length {} >>\nstream\n", data.len()).unwrap();
        self.buf.extend_from_slice(data);
        self.buf.extend_from_slice(b"\nendstream\nendobj\n");
    }

    fn plain_obj(&mut self, id: u32, body: &str) {
        self.begin(id);
        write!(self.buf, "{body}\nendobj\n").unwrap();
    }

    fn cid(&mut self, c: char) -> u16 {
        let next = self.chars.len() as u16 + 1;
        *self.chars.entry(c).or_insert(next)
    }

    /// Append one page. `words` (optional) are in the page's pixel coordinates.
    pub fn add_page(&mut self, page: &Page, words: &[Word]) {
        let (pw, ph) = (page.image.width(), page.image.height());
        let dpi = page.dpi.clamp(36, 2400) as f32;
        let (wpt, hpt) = (pw as f32 * 72.0 / dpi, ph as f32 * 72.0 / dpi);

        // --- image XObject -------------------------------------------
        let img_id = self.alloc();
        // JPEG cannot exceed 65535 px per side; such pages stay lossless.
        let compress = if pw > 65_000 || ph > 65_000 { Compress::Flate } else { self.compress };
        match &page.image {
            PageImage::Bw(b) => {
                let data = pack_1bit_up_predicted(b);
                let z = deflate(&data);
                self.stream_obj(
                    img_id,
                    &format!(
                        "/Type /XObject /Subtype /Image /Width {pw} /Height {ph} \
                         /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode \
                         /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 1 /Columns {pw} >>"
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

        // --- content stream --------------------------------------------
        let mut cs = String::new();
        cs.push_str(&format!("q {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Im0 Do Q\n"));
        if !words.is_empty() {
            cs.push_str("BT 3 Tr\n");
            for w in words {
                if w.text.trim().is_empty() || w.w == 0 || w.h == 0 {
                    continue;
                }
                let n = w.text.chars().count() as f32;
                let size = (w.h as f32 * 72.0 / dpi).max(1.0);
                // Glyph advance is 0.5 em; scale horizontally to the word box.
                let tz = (w.w as f32 * 72.0 / dpi) / (n * 0.5 * size) * 100.0;
                let x = w.x as f32 * 72.0 / dpi;
                let y = hpt - (w.y + w.h) as f32 * 72.0 / dpi + size * 0.2;
                let mut hex = String::new();
                for ch in w.text.chars().chain(std::iter::once(' ')) {
                    hex.push_str(&format!("{:04X}", self.cid(ch)));
                }
                cs.push_str(&format!(
                    "/F1 {size:.2} Tf {tz:.1} Tz 1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"
                ));
            }
            cs.push_str("ET\n");
        }
        let cs_id = self.alloc();
        let z = deflate(cs.as_bytes());
        self.stream_obj(cs_id, "/Filter /FlateDecode", &z);

        // --- page object -------------------------------------------------
        let page_id = self.alloc();
        self.plain_obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {wpt:.3} {hpt:.3}] \
                 /Resources << /XObject << /Im0 {img_id} 0 R >> /Font << /F1 {FONT} 0 R >> >> \
                 /Contents {cs_id} 0 R >>"
            ),
        );
        self.page_ids.push(page_id);
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

        let kids: String = self.page_ids.iter().map(|id| format!("{id} 0 R ")).collect();
        self.plain_obj(
            PAGES,
            &format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", self.page_ids.len()),
        );
        self.plain_obj(CATALOG, &format!("<< /Type /Catalog /Pages {PAGES} 0 R >>"));
        let info_id = self.alloc();
        let title = self
            .title
            .as_deref()
            .map(|t| format!(" /Title {}", pdf_text_string(t)))
            .unwrap_or_default();
        self.plain_obj(info_id, &format!("<< /Producer (LegatorOCR {}){title} >>", env!("CARGO_PKG_VERSION")));

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
                let hex: String = c.encode_utf16(&mut u).iter().map(|x| format!("{x:04X}")).collect();
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
    if s.is_ascii() && !s.contains(['(', ')', '\\']) {
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
        Page { image: img, dpi: 300, skew_deg: 0.0, blank: false, split_index: None }
    }

    #[test]
    fn writes_valid_structure() {
        let mut w = PdfWriter::new(Compress::Jpeg(80));
        w.add_page(&page(PageImage::Rgb(RgbImage::from_pixel(30, 40, image::Rgb([200, 10, 10])))), &[]);
        let mut bw = BitImage::new(17, 9);
        bw.set(3, 3, true);
        w.add_page(
            &page(PageImage::Bw(bw)),
            &[Word { text: "șțăîâ".into(), x: 1, y: 1, w: 10, h: 5, conf: 90.0, line: 0 }],
        );
        let pdf = w.finish();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.5"));
        assert!(s.contains("/Count 2"));
        assert!(s.trim_end().ends_with("%%EOF"));
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
        let page = Page { image: PageImage::Bw(bw), dpi: 150, skew_deg: 0.0, blank: false, split_index: None };
        let words: Vec<Word> = ["Școala", "țăranului", "înființată", "Âțâră", "e", "😀ok"]
            .iter()
            .enumerate()
            .map(|(i, t)| Word { text: t.to_string(), x: 20 + (i as u32) * 90, y: 40, w: 80, h: 24, conf: 99.0, line: 0 })
            .collect();
        w.add_page(&page, &words);
        std::fs::write(path, w.finish()).unwrap();
    }
}
