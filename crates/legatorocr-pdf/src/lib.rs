//! Minimal, dependency-light PDF writer.
//!
//! * colour/gray pages → JPEG (DCT) or Flate
//! * black & white pages → 1-bit image, Flate + PNG "Up" predictor
//! * optional invisible text layer (render mode 3) from OCR words, using a
//!   glyph-less Type0 font with a ToUnicode CMap so any Unicode text (e.g.
//!   Romanian ăâîșț) is searchable and copy-able.

pub mod crypt;
pub mod g4;
pub mod jbig2;
pub mod jbig2enc;
pub mod jbig2huff;
pub mod jpx;
pub mod pdfa;

/// Part used by writers created afterwards (set once from the command line).
/// Tagged-PDF default for new writers: `None` = off, `Some("")` = on without a language.
pub static DEFAULT_TAGGED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Structured tagged PDF (paragraphs and headings instead of one element per line) for the
/// engine to feed: set from `--tagged-pdf`.
pub static DEFAULT_TAGGED_STRUCT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
pub static DEFAULT_PDFA_PART: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
/// PDF/A conformance level for new writers: `b'B'` (default), `b'U'` or `b'A'` (see
/// [`PdfWriter::set_pdfa_level`]).
pub static DEFAULT_PDFA_LEVEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(b'B');
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
    /// (title, page index, heading size relative to body text; 0 = not a heading)
    bookmarks: Vec<(String, usize, f32)>,
    labels: Vec<(usize, LabelStyle, String, u32)>,
    pdfa: bool,
    pdfa_part: u8,
    pdfa_level: u8,
    /// Form fields for the next page: (checkbox?, [x0, y0, x1, y1] in points).
    next_fields: Vec<(bool, [f32; 4])>,
    /// Widget object ids of all fields written so far (for /AcroForm).
    field_ids: Vec<u32>,
    /// Tagged PDF: marked content per OCR line plus a structure tree (`set_tagged`).
    tagged: Option<String>,
    /// Marked-content ids used on the page being written.
    page_mc: u32,
    /// Per page: (page object, structure element ids by MCID), for the tree.
    struct_pages: Vec<(u32, Vec<(u32, StructKind)>)>,
    /// Kinds of the marked-content sequences of the page being written, by MCID.
    page_kinds: Vec<StructKind>,
    /// Paragraph/heading ranges for the next page (`set_next_structure`).
    next_struct: Option<Vec<StructPara>>,
    /// Figures of the next page (`set_next_figures`).
    next_figures: Vec<Figure>,
    /// Alternative texts of all figures written, by `StructKind::Figure` index.
    fig_alts: Vec<String>,
}

/// A picture on a page that gets a `/Figure` structure element with alternative text. The box is
/// in points from the top left corner of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub alt: String,
}

/// Tag of a structure element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructKind {
    /// `/P`
    Para,
    /// `/H1` .. `/H3` (the level is clamped to 1..=3)
    Heading(u8),
    /// `/Figure` with the alternative text number `n` of the writer
    Figure(u32),
}

impl StructKind {
    fn tag(self) -> String {
        match self {
            StructKind::Para => "P".into(),
            StructKind::Heading(n) => format!("H{}", n.clamp(1, 3)),
            StructKind::Figure(_) => "Figure".into(),
        }
    }
}

/// One structure element of a page: words `first..end` of the slice given to `add_page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructPara {
    pub kind: StructKind,
    pub first: usize,
    pub end: usize,
}

/// Numbering style of a page-label range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    Decimal,
    RomanLower,
    RomanUpper,
    AlphaLower,
    AlphaUpper,
    /// No number, only the prefix.
    None,
}

/// Optional document-information entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// Seconds since the Unix epoch; written as CreationDate / ModDate (UTC).
    pub created_unix: Option<u64>,
    /// Rights statement (copyright holder, licence, contract reference). Written as XMP
    /// `dc:rights` with `--pdfa`. Without it: Info key `/Rights`, plus a visible note (Text
    /// annotation) on page 2 when the file has more than one page.
    pub rights: Option<String>,
    /// Provenance record (JSON text): program version, inputs with hashes, options. Written as
    /// the custom Info key `/LegatorOCRProvenance` (not with PDF/A, which would need an XMP
    /// extension schema for it).
    pub provenance: Option<String>,
    /// Files embedded in the PDF (`/EmbeddedFiles`, each with `/AFRelationship /Source`), e.g. the
    /// original scans and the project (`--embed-project`). Sorted by name when written.
    pub attachments: Vec<Attachment>,
    /// Reading direction right to left (`/ViewerPreferences /Direction /R2L`): a viewer in
    /// two-page mode then puts the first page on the right.
    pub right_to_left: bool,
}

/// One embedded file (PDF 1.7 file specification; with `--pdfa-part 3` an associated file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Name shown by viewers; also the key in the name tree. Keep it plain ASCII-safe.
    pub name: String,
    /// MIME type, written as the stream `/Subtype` (PDF/A-3 requires one).
    pub mime: String,
    pub description: String,
    pub data: Vec<u8>,
    /// Flate-compress the stream (leave off for PNG / JPEG / ZIP, which are compressed already).
    pub deflate: bool,
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
            bookmarks: Vec::new(),
            labels: Vec::new(),
            pdfa: false,
            pdfa_part: DEFAULT_PDFA_PART.load(std::sync::atomic::Ordering::Relaxed),
            pdfa_level: DEFAULT_PDFA_LEVEL.load(std::sync::atomic::Ordering::Relaxed),
            next_fields: Vec::new(),
            tagged: DEFAULT_TAGGED.lock().ok().and_then(|g| g.clone()),
            page_mc: 0,
            struct_pages: Vec::new(),
            page_kinds: Vec::new(),
            next_struct: None,
            next_figures: Vec::new(),
            fig_alts: Vec::new(),
            field_ids: Vec::new(),
        };
        w.buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
        w
    }

    /// Outline entry pointing at the next page that will be added.
    pub fn add_bookmark(&mut self, title: &str) {
        if self.bookmarks.len() < 5000 {
            self.bookmarks
                .push((title.to_string(), self.page_ids.len(), 0.0));
        }
    }

    /// Outline entry for a detected heading whose letters are `size` times the body text's
    /// height. When the document's headings differ clearly in size (largest at least 1.25 times
    /// the smallest), the larger half become top-level entries and the smaller ones are nested
    /// under the preceding top-level entry.
    pub fn add_heading_bookmark(&mut self, title: &str, size: f32) {
        if self.bookmarks.len() < 5000 {
            self.bookmarks
                .push((title.to_string(), self.page_ids.len(), size.max(0.01)));
        }
    }

    /// From the next page added on, label pages `prefix` + number in `style`, counting
    /// from `start` (>= 1).
    pub fn set_page_labels(&mut self, style: LabelStyle, prefix: &str, start: u32) {
        let at = self.page_ids.len();
        self.labels.retain(|l| l.0 != at);
        self.labels
            .push((at, style, prefix.to_string(), start.max(1)));
    }

    /// Label pages from page index `at` on (0-based, any page already added or still to come).
    pub fn set_page_labels_at(&mut self, at: usize, style: LabelStyle, prefix: &str, start: u32) {
        self.labels.retain(|l| l.0 != at);
        self.labels
            .push((at, style, prefix.to_string(), start.max(1)));
    }

    /// Write PDF/A-2b structure: embedded glyph-less font, XMP metadata, sRGB output intent,
    /// CIE-based grey, trailer ID. (Structure verified against the standard's requirements
    /// by hand and with open tools; no PDF/A validator was available to certify it.)
    pub fn set_pdfa(&mut self, on: bool) {
        self.pdfa = on;
        self.apply_part();
    }

    /// PDF/A part to declare with `set_pdfa`: 1 (PDF 1.4 header), 2 (default) or 3.
    /// The writer uses nothing outside PDF 1.4 (classic xref, no object streams, no
    /// transparency, uncompressed XMP), so the same body is valid for all three parts.
    pub fn set_pdfa_part(&mut self, part: u8) {
        self.pdfa_part = part.clamp(1, 3);
        self.apply_part();
    }

    /// Conformance level to declare with `set_pdfa`: `'B'` (visual reproducibility), `'U'` (adds
    /// Unicode for all text, parts 2 and 3 only) or `'A'` (adds the tagged structure: needs
    /// `set_tagged` with a language). A level whose requirements are not met is **not claimed**:
    /// the file then declares `B`.
    pub fn set_pdfa_level(&mut self, level: char) {
        self.pdfa_level = match level.to_ascii_uppercase() {
            'A' => b'A',
            'U' => b'U',
            _ => b'B',
        };
    }

    /// The level the file really declares (see [`PdfWriter::set_pdfa_level`]).
    fn conformance(&self) -> char {
        match self.pdfa_level {
            b'A' if self.tagged.as_ref().is_some_and(|l| !l.is_empty()) => 'A',
            // every font this writer embeds carries a ToUnicode map
            b'U' if self.pdfa_part >= 2 => 'U',
            _ => 'B',
        }
    }

    fn apply_part(&mut self) {
        // byte 7 of the header is the minor version digit
        self.buf[7] = if self.pdfa && self.pdfa_part == 1 {
            b'4'
        } else {
            b'5'
        };
    }

    fn gray_cs(&self) -> &'static str {
        if self.pdfa {
            "[/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 2.2 >>]"
        } else {
            "/DeviceGray"
        }
    }

    pub fn set_title(&mut self, t: &str) {
        self.title = Some(t.to_string());
    }

    /// Fillable fields (AcroForm text boxes / check boxes) for the next page added, as
    /// `(is_checkbox, [x0, y0, x1, y1])` in points from the bottom-left. Not with PDF/A.
    pub fn set_form_fields(&mut self, fields: Vec<(bool, [f32; 4])>) {
        self.next_fields = fields;
    }

    /// Write a tagged PDF: every OCR line becomes a /P structure element in reading order, the
    /// page image is marked as an artifact, and the catalog gets /MarkInfo, /Lang (BCP 47 code)
    /// and DisplayDocTitle. This is the base of PDF/UA, not a full conformance claim (no
    /// headings, lists, tables or alternative text; the scan itself is an artifact).
    pub fn set_tagged(&mut self, lang: Option<&str>) {
        // (see also `set_next_structure`)
        self.tagged = lang.map(str::to_string).or(Some(String::new()));
    }

    /// Tagged PDF only: for the next page added, group its words into the given structure
    /// elements (paragraphs and headings) instead of one `/P` per OCR line. The ranges index the
    /// word slice passed to `add_page`; words outside every range are tagged line by line.
    /// Ignored when the writer is not tagged.
    /// Mark rectangles of the next page as figures with alternative text (PDF/A level A, PDF/UA).
    /// The picture is painted again inside the rectangle as tagged content, so the figure is a real
    /// part of the page; the page looks the same. Ignored when the writer is not tagged.
    pub fn set_next_figures(&mut self, figs: Vec<Figure>) {
        self.next_figures = figs;
    }

    pub fn set_next_structure(&mut self, paras: Vec<StructPara>) {
        self.next_struct = Some(paras);
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
                let gray = self.gray_cs();
                let (filter, z) = encode_1bit(b);
                self.stream_obj(
                    img_id,
                    &format!(
                        "/Type /XObject /Subtype /Image /Width {pw} /Height {ph} \
                         /ColorSpace {gray} /BitsPerComponent 1 {filter}"
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
                    self.stream_obj(
                        img_id,
                        &image_dict(pw, ph, self.gray_cs(), "DCTDecode"),
                        &out,
                    );
                }
                Compress::Flate => {
                    let z = deflate(g.as_raw());
                    self.stream_obj(
                        img_id,
                        &image_dict(pw, ph, self.gray_cs(), "FlateDecode"),
                        &z,
                    );
                }
            },
            PageImage::Rgb(c) => match compress {
                Compress::Jpeg(q) => {
                    let mut out = Vec::new();
                    JpegEncoder::new_with_quality(&mut out, q)
                        .encode(c.as_raw(), pw, ph, ExtendedColorType::Rgb8)
                        .expect("jpeg encode");
                    self.stream_obj(img_id, &image_dict(pw, ph, "/DeviceRGB", "DCTDecode"), &out);
                }
                Compress::Flate => {
                    let z = deflate(c.as_raw());
                    self.stream_obj(img_id, &image_dict(pw, ph, "/DeviceRGB", "FlateDecode"), &z);
                }
            },
        }

        let mut cs = format!("q {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Im0 Do Q\n");
        self.text_ops(&mut cs, words, dpi, wpt, hpt);
        let figs = std::mem::take(&mut self.next_figures);
        if self.tagged.is_some() {
            for f in figs {
                let (x, w) = (f.x.clamp(0.0, wpt), f.w.min(wpt));
                let (h, y) = (f.h.min(hpt), (hpt - f.y - f.h).clamp(0.0, hpt));
                self.fig_alts.push(f.alt);
                let id = self.fig_alts.len() as u32 - 1;
                cs.push_str(&format!(
                    "/Figure <</MCID {}>> BDC\nq {x:.2} {y:.2} {w:.2} {h:.2} re W n {wpt:.3} 0 0 {hpt:.3} 0 0 cm /Im0 Do Q\nEMC\n",
                    self.page_mc
                ));
                self.page_kinds.push(StructKind::Figure(id));
                self.page_mc += 1;
            }
        }
        self.finish_page(cs, &format!("/Im0 {img_id} 0 R"), wpt, hpt);
    }

    /// Invisible-text operators for OCR words.
    fn text_ops(&mut self, cs: &mut String, words: &[Word], dpi: f32, wpt: f32, hpt: f32) {
        self.page_mc = 0;
        self.page_kinds.clear();
        let plan = self.next_struct.take().filter(|_| self.tagged.is_some());
        if self.tagged.is_some() {
            *cs = format!("/Artifact BMC\n{cs}EMC\n");
        }
        if words.is_empty() {
            return;
        }
        // One text object per recognised line, all its words on the same baseline and in the same
        // font size (like ABBYY's output): readers then select a line as a line instead of a
        // cloud of differently sized words whose boxes happen to have ascenders or descenders.
        let mut i = 0;
        // the structure element whose marked content is open: (plan index or none, kind)
        let mut open: Option<Option<usize>> = None;
        while i < words.len() {
            let mut j = i + 1;
            while j < words.len() && words[j].line == words[i].line {
                j += 1;
            }
            let line_start = i;
            let group: Vec<&Word> = words[i..j]
                .iter()
                .filter(|w| {
                    // boxes off the page or taller than it are garbage (a PDF reader may choke on
                    // 10-digit coordinates): drop them before they can steer the line's baseline
                    let (x, top, h) = (
                        w.x as f32 * 72.0 / dpi,
                        w.y as f32 * 72.0 / dpi,
                        w.h as f32 * 72.0 / dpi,
                    );
                    !w.text.trim().is_empty()
                        && w.w > 0
                        && w.h > 0
                        && x.is_finite()
                        && top.is_finite()
                        && h.is_finite()
                        && x < wpt
                        && top < hpt
                        && h <= hpt
                })
                .collect();
            i = j;
            if group.is_empty() {
                continue;
            }
            let median = |mut v: Vec<f32>| {
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() / 2]
            };
            // a line's size and baseline come from the middle of its words: one tall or low
            // word (a descender, a stray mark) must not shift the rest
            let size = median(group.iter().map(|w| w.h as f32 * 72.0 / dpi).collect()).max(1.0);
            let bottom = median(
                group
                    .iter()
                    .map(|w| (w.y as f32 + w.h as f32) * 72.0 / dpi)
                    .collect(),
            );
            // a baseline that falls off the page (a box at the very bottom edge) is kept on it
            let y = (hpt - bottom + size * 0.2).clamp(0.0, hpt);
            let mut body = String::new();
            for w in group {
                // control characters and the non-characters are not text
                let text: String = w
                    .text
                    .chars()
                    .filter(|c| !c.is_control() && *c != '\u{FFFE}' && *c != '\u{FFFF}')
                    .collect();
                let n = text.chars().count() as f32;
                if n == 0.0 {
                    continue;
                }
                let x = w.x as f32 * 72.0 / dpi;
                // Words outside the page or taller than it are garbage (a PDF reader may choke on
                // 10-digit coordinates): drop them rather than write absurd numbers.
                if !(x.is_finite() && y.is_finite() && size.is_finite())
                    || x >= wpt
                    || (w.y as f32 * 72.0 / dpi) >= hpt
                    || size > hpt
                {
                    continue;
                }
                // Glyph advance is 0.5 em; scale horizontally to the word box (kept within sane bounds).
                let tz = ((w.w as f32 * 72.0 / dpi) / (n * 0.5 * size) * 100.0).clamp(5.0, 2000.0);
                let mut hex = String::new();
                for ch in text.chars().chain(std::iter::once(' ')) {
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
                body.push_str(&format!("{tz:.1} Tz 1 0 0 1 {x:.2} {y:.2} Tm <{hex}> Tj\n"));
            }
            if !body.is_empty() {
                if self.tagged.is_some() {
                    let which = plan.as_ref().and_then(|p| {
                        p.iter()
                            .position(|s| s.first <= line_start && line_start < s.end)
                    });
                    // one marked-content sequence per planned element; lines outside the plan
                    // (or every line, without a plan) get one each
                    if open.is_none() || which.is_none() || open != Some(which) {
                        if open.is_some() {
                            cs.push_str("EMC\n");
                        }
                        let kind = which
                            .and_then(|k| plan.as_ref().map(|p| p[k].kind))
                            .unwrap_or(StructKind::Para);
                        cs.push_str(&format!("/{} <</MCID {}>> BDC\n", kind.tag(), self.page_mc));
                        self.page_kinds.push(kind);
                        self.page_mc += 1;
                        open = Some(which);
                    }
                    cs.push_str(&format!("BT 3 Tr /F1 {size:.2} Tf\n{body}ET\n"));
                } else {
                    cs.push_str(&format!("BT 3 Tr /F1 {size:.2} Tf\n{body}ET\n"));
                }
            }
        }
        if open.is_some() {
            cs.push_str("EMC\n");
        }
    }

    fn finish_page(&mut self, cs: String, xobjects: &str, wpt: f32, hpt: f32) {
        let cs_id = self.alloc();
        let z = deflate(cs.as_bytes());
        self.stream_obj(cs_id, "/Filter /FlateDecode", &z);
        let page_id = self.alloc();
        // A visible rights note: a Text annotation (sticky note) on page 2. PDF/A requires an
        // appearance stream for annotations, so with --pdfa the rights live in the XMP only.
        let mut widget_refs = String::new();
        let fields = std::mem::take(&mut self.next_fields);
        if !self.pdfa {
            for (check, r) in fields {
                let id = self.alloc();
                let n = self.field_ids.len() + 1;
                let body = if check {
                    format!(
                        "<< /Type /Annot /Subtype /Widget /FT /Btn /T (check_{n}) /Rect [{:.2} {:.2} {:.2} {:.2}] /F 4 /V /Off /AS /Off /P {page_id} 0 R /MK << /BC [0] >> >>",
                        r[0], r[1], r[2], r[3]
                    )
                } else {
                    format!(
                        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (field_{n}) /Rect [{:.2} {:.2} {:.2} {:.2}] /F 4 /DA (/Helv 0 Tf 0 g) /P {page_id} 0 R >>",
                        r[0], r[1], r[2], r[3]
                    )
                };
                self.plain_obj(id, &body);
                self.field_ids.push(id);
                widget_refs.push_str(&format!(" {id} 0 R"));
            }
        }
        let annots = match self.meta.rights.as_deref().filter(|r| !r.trim().is_empty()) {
            Some(r) if !self.pdfa && self.page_ids.len() == 1 => {
                let (x0, y1) = (12.0f32, (hpt - 12.0).max(24.0));
                format!(
                    " /Annots [ << /Type /Annot /Subtype /Text /Rect [{x0:.1} {:.1} {:.1} {y1:.1}] \
                     /Contents {} /Name /Comment /Open false /F 28 /C [1 0.85 0] >> ]",
                    y1 - 24.0,
                    x0 + 24.0,
                    pdf_text_string(r)
                )
            }
            _ => String::new(),
        };
        let annots = if widget_refs.is_empty() {
            annots
        } else if let Some(rest) = annots.strip_prefix(" /Annots [") {
            format!(" /Annots [{widget_refs} {rest}")
        } else {
            format!(" /Annots [{widget_refs} ]")
        };
        let mut tag_attrs = String::new();
        if self.tagged.is_some() {
            // structure elements are written in finish(), once the tree root has an id
            let mut ids = Vec::new();
            for k in 0..self.page_mc as usize {
                let kind = self.page_kinds.get(k).copied().unwrap_or(StructKind::Para);
                ids.push((self.alloc(), kind));
            }
            tag_attrs = format!(" /StructParents {} /Tabs /S", self.struct_pages.len());
            self.struct_pages.push((page_id, ids));
        }
        self.plain_obj(
            page_id,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {wpt:.3} {hpt:.3}] \
                 /Resources << /XObject << {xobjects} >> /Font << /F1 {FONT} 0 R >> >> \
                 /Contents {cs_id} 0 R{annots}{tag_attrs} >>"
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
                self.stream_obj(id, &image_dict(w, h, "/DeviceRGB", "DCTDecode"), &out);
            }
            _ => {
                let z = deflate(c.as_raw());
                self.stream_obj(id, &image_dict(w, h, "/DeviceRGB", "FlateDecode"), &z);
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
        let font_file = if self.pdfa {
            let ttf = pdfa::glyphless_ttf(self.chars.len() + 1);
            let id = self.alloc();
            let z = deflate(&ttf);
            self.stream_obj(
                id,
                &format!("/Filter /FlateDecode /Length1 {}", ttf.len()),
                &z,
            );
            format!(" /FontFile2 {id} 0 R")
        } else {
            String::new()
        };
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
            &format!(
                "<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 \
                 /FontBBox [0 0 500 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 \
                 /CapHeight 1000 /StemV 80{font_file} >>"
            ),
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
        let n_pages = self.page_ids.len();
        let marks: Vec<(String, usize, f32)> = self
            .bookmarks
            .iter()
            .filter(|b| b.1 < n_pages)
            .cloned()
            .collect();
        let levels = outline_levels(&marks);
        let mut cat_extra = String::new();
        if !marks.is_empty() {
            let root = self.alloc();
            let ids: Vec<u32> = marks.iter().map(|_| self.alloc()).collect();
            // parent (index) of every entry, and the children of every entry
            let mut parent: Vec<Option<usize>> = vec![None; marks.len()];
            let mut kids: Vec<Vec<usize>> = vec![Vec::new(); marks.len()];
            let mut tops: Vec<usize> = Vec::new();
            let mut last_top: Option<usize> = None;
            for k in 0..marks.len() {
                match (levels[k], last_top) {
                    (1, Some(t)) => {
                        parent[k] = Some(t);
                        kids[t].push(k);
                    }
                    _ => {
                        tops.push(k);
                        last_top = Some(k);
                    }
                }
            }
            self.plain_obj(
                root,
                &format!(
                    "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
                    ids[tops[0]],
                    ids[tops[tops.len() - 1]],
                    marks.len()
                ),
            );
            for (k, (t, pg, _)) in marks.iter().enumerate() {
                let siblings: &Vec<usize> = match parent[k] {
                    Some(p) => &kids[p],
                    None => &tops,
                };
                let pos = siblings.iter().position(|&x| x == k).unwrap_or(0);
                let prev = if pos > 0 {
                    format!(" /Prev {} 0 R", ids[siblings[pos - 1]])
                } else {
                    String::new()
                };
                let next = if pos + 1 < siblings.len() {
                    format!(" /Next {} 0 R", ids[siblings[pos + 1]])
                } else {
                    String::new()
                };
                let par = parent[k].map_or(root, |p| ids[p]);
                let children = if kids[k].is_empty() {
                    String::new()
                } else {
                    format!(
                        " /First {} 0 R /Last {} 0 R /Count {}",
                        ids[kids[k][0]],
                        ids[kids[k][kids[k].len() - 1]],
                        kids[k].len()
                    )
                };
                self.plain_obj(
                    ids[k],
                    &format!(
                        "<< /Title {} /Parent {par} 0 R{prev}{next}{children} /Dest [{} 0 R /Fit] >>",
                        pdf_text_string(t),
                        self.page_ids[*pg]
                    ),
                );
            }
            cat_extra.push_str(&format!(" /Outlines {root} 0 R /PageMode /UseOutlines"));
        }
        if !self.labels.is_empty() {
            let mut labels = self.labels.clone();
            labels.sort_by_key(|l| l.0);
            if labels[0].0 != 0 {
                labels.insert(0, (0, LabelStyle::Decimal, String::new(), 1));
            }
            let nums: String = labels
                .iter()
                .filter(|l| l.0 < n_pages)
                .map(|(at, st, pre, start)| {
                    let s = match st {
                        LabelStyle::Decimal => "/S /D ",
                        LabelStyle::RomanLower => "/S /r ",
                        LabelStyle::RomanUpper => "/S /R ",
                        LabelStyle::AlphaLower => "/S /a ",
                        LabelStyle::AlphaUpper => "/S /A ",
                        LabelStyle::None => "",
                    };
                    let p = if pre.is_empty() {
                        String::new()
                    } else {
                        format!("/P {} ", pdf_text_string(pre))
                    };
                    format!("{at} << {s}{p}/St {start} >> ")
                })
                .collect();
            cat_extra.push_str(&format!(" /PageLabels << /Nums [{nums}] >>"));
        }
        let date = self.meta.created_unix.map(pdf_date);
        if !self.meta.attachments.is_empty() {
            let mut files = std::mem::take(&mut self.meta.attachments);
            files.sort_by(|a, b| a.name.cmp(&b.name));
            let mut names = String::new();
            let mut af = String::new();
            for f in &files {
                let sid = self.alloc();
                let (body, filt) = if f.deflate {
                    (deflate(&f.data), " /Filter /FlateDecode")
                } else {
                    (f.data.clone(), "")
                };
                let mime = f.mime.replace('/', "#2F");
                let moddate = date
                    .as_deref()
                    .map(|d| format!(" /ModDate ({d})"))
                    .unwrap_or_default();
                self.stream_obj(
                    sid,
                    &format!("/Type /EmbeddedFile /Subtype /{mime}{filt} /Params << /Size {}{moddate} >>", f.data.len()),
                    &body,
                );
                let fid = self.alloc();
                let n = pdf_text_string(&f.name);
                self.plain_obj(
                    fid,
                    &format!(
                        "<< /Type /Filespec /F {n} /UF {n} /Desc {} /AFRelationship /Source /EF << /F {sid} 0 R /UF {sid} 0 R >> >>",
                        pdf_text_string(&f.description)
                    ),
                );
                names.push_str(&format!("{n} {fid} 0 R "));
                af.push_str(&format!("{fid} 0 R "));
            }
            cat_extra.push_str(&format!(
                " /Names << /EmbeddedFiles << /Names [{names}] >> >> /AF [{af}]"
            ));
        }
        let producer = format!("LegatorOCR {}", env!("CARGO_PKG_VERSION"));
        if self.pdfa {
            let xmp = pdfa::Xmp {
                part: self.pdfa_part,
                level: self.conformance(),
                title: self.title.as_deref(),
                author: self.meta.author.as_deref().filter(|v| !v.trim().is_empty()),
                subject: self
                    .meta
                    .subject
                    .as_deref()
                    .filter(|v| !v.trim().is_empty()),
                keywords: self
                    .meta
                    .keywords
                    .as_deref()
                    .filter(|v| !v.trim().is_empty()),
                producer: &producer,
                rights: self.meta.rights.as_deref().filter(|v| !v.trim().is_empty()),
                date: date.as_deref(),
            }
            .render();
            let mid = self.alloc();
            self.stream_obj(mid, "/Type /Metadata /Subtype /XML", xmp.as_bytes());
            let icc = pdfa::srgb_icc();
            let iid = self.alloc();
            let z = deflate(&icc);
            self.stream_obj(iid, "/N 3 /Alternate /DeviceRGB /Filter /FlateDecode", &z);
            cat_extra.push_str(&format!(
                " /Metadata {mid} 0 R /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFA1 \
                 /OutputConditionIdentifier (sRGB IEC61966-2.1) /Info (sRGB IEC61966-2.1) \
                 /RegistryName (http://www.color.org) /DestOutputProfile {iid} 0 R >>]"
            ));
        }
        if let Some(lang) = self.tagged.clone() {
            let root = self.alloc();
            let doc = self.alloc();
            let mut kids = String::new();
            let mut parent_nums = String::new();
            let pages = std::mem::take(&mut self.struct_pages);
            for (k, (pg, ids)) in pages.iter().enumerate() {
                let mut arr = String::new();
                for (mc, (id, kind)) in ids.iter().enumerate() {
                    let alt = match kind {
                        StructKind::Figure(n) => format!(
                            " /Alt {}",
                            pdf_text_string(self.fig_alts.get(*n as usize).map_or("", |s| s))
                        ),
                        _ => String::new(),
                    };
                    self.plain_obj(
                        *id,
                        &format!(
                            "<< /Type /StructElem /S /{} /P {doc} 0 R /Pg {pg} 0 R{alt} /K {mc} >>",
                            kind.tag()
                        ),
                    );
                    kids.push_str(&format!("{id} 0 R "));
                    arr.push_str(&format!("{id} 0 R "));
                }
                parent_nums.push_str(&format!("{k} [{arr}] "));
            }
            self.plain_obj(
                doc,
                &format!("<< /Type /StructElem /S /Document /P {root} 0 R /K [{kids}] >>"),
            );
            self.plain_obj(
                root,
                &format!("<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree << /Nums [{parent_nums}] >> /ParentTreeNextKey {} >>", pages.len()),
            );
            cat_extra.push_str(&format!(" /StructTreeRoot {root} 0 R /MarkInfo << /Marked true >> /ViewerPreferences << /DisplayDocTitle true{} >>", if self.meta.right_to_left { " /Direction /R2L" } else { "" }));
            if !lang.is_empty() {
                cat_extra.push_str(&format!(" /Lang {}", pdf_text_string(&lang)));
            }
        }
        if !self.field_ids.is_empty() {
            let helv = self.alloc();
            self.plain_obj(
                helv,
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
            );
            let list: String = self.field_ids.iter().map(|i| format!("{i} 0 R ")).collect();
            cat_extra.push_str(&format!(
                " /AcroForm << /Fields [{list}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv {helv} 0 R >> >> /NeedAppearances true >>"
            ));
        }
        if self.meta.right_to_left && !cat_extra.contains("/ViewerPreferences") {
            cat_extra.push_str(" /ViewerPreferences << /Direction /R2L >>");
        }
        self.plain_obj(
            CATALOG,
            &format!("<< /Type /Catalog /Pages {PAGES} 0 R{cat_extra} >>"),
        );
        let info_id = self.alloc();
        // DisplayDocTitle needs a title: a tagged file without one gets a neutral placeholder
        let fallback = self
            .tagged
            .as_ref()
            .map(|_| "Untitled document".to_string());
        let title = self
            .title
            .clone()
            .or(fallback)
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
        if let Some(r) = self.meta.rights.as_deref().filter(|v| !v.trim().is_empty()) {
            if !self.pdfa {
                // custom Info key (PDF/A would need an XMP extension schema for it)
                extra.push_str(&format!(" /Rights {}", pdf_text_string(r)));
            }
        }
        if let Some(p) = self
            .meta
            .provenance
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            if !self.pdfa {
                extra.push_str(&format!(" /LegatorOCRProvenance {}", pdf_text_string(p)));
            }
        }
        if let Some(d) = &date {
            extra.push_str(&format!(" /CreationDate ({d}) /ModDate ({d})"));
        }
        self.plain_obj(
            info_id,
            &format!("<< /Producer ({producer}){title}{extra} >>"),
        );

        let xref_pos = self.buf.len();
        let n = self.offsets.len();
        write!(self.buf, "xref\n0 {n}\n0000000000 65535 f \n").unwrap();
        for id in 1..n {
            let off = self.offsets[id].expect("every object written");
            writeln!(self.buf, "{off:010} 00000 n ").unwrap();
        }
        let id = if self.pdfa {
            // two copies of a digest of everything written so far
            let mut h = [0u8; 16];
            for (i, b) in self.buf.iter().enumerate() {
                h[i % 16] = h[i % 16].rotate_left(3) ^ b.wrapping_add(i as u8);
            }
            let hex: String = h.iter().map(|b| format!("{b:02X}")).collect();
            format!(" /ID [<{hex}> <{hex}>]")
        } else {
            String::new()
        };
        write!(
            self.buf,
            "trailer\n<< /Size {n} /Root {CATALOG} 0 R /Info {info_id} 0 R{id} >>\nstartxref\n{xref_pos}\n%%EOF\n"
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
        "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace {cs} \
         /BitsPerComponent 8 /Filter /{filter}"
    )
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

static JBIG2_OUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Also try lossless JBIG2 for 1-bit images and keep it when it is the smallest.
pub fn set_jbig2_output(on: bool) {
    JBIG2_OUT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Best encoding of a 1-bit page: (extra image-dict entries, bytes).
fn encode_1bit(b: &BitImage) -> (String, Vec<u8>) {
    let w = b.w as usize;
    let g4 = g4::encode(
        w,
        (0..b.h as usize).map(|y| b.data[y * w..(y + 1) * w].iter().map(|&v| v != 0).collect()),
    );
    let flate = deflate(&pack_1bit_up_predicted(b));
    if JBIG2_OUT.load(std::sync::atomic::Ordering::Relaxed) {
        let j = jbig2enc::encode_best(w, b.h as usize, &b.data);
        if j.len() < g4.len().min(flate.len()) {
            return ("/Filter /JBIG2Decode".to_string(), j);
        }
    }
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
            content_box: None,
            layers: None,
        }
    }

    #[test]
    fn right_to_left_is_declared_for_viewers() {
        let make = |rtl: bool| {
            let mut w = PdfWriter::new(Compress::Flate);
            w.set_meta(Meta {
                right_to_left: rtl,
                ..Meta::default()
            });
            w.add_page(
                &page(PageImage::Rgb(RgbImage::from_pixel(
                    8,
                    8,
                    image::Rgb([255, 255, 255]),
                ))),
                &[],
            );
            String::from_utf8_lossy(&w.finish()).into_owned()
        };
        assert!(make(true).contains("/ViewerPreferences << /Direction /R2L >>"));
        assert!(!make(false).contains("/Direction"));
    }

    #[test]
    fn figures_get_alternative_text_and_tagged_content() {
        let mut pw = PdfWriter::new(Compress::Flate);
        pw.set_tagged(Some("en"));
        let mut b = BitImage::new(600, 400);
        b.set(1, 1, true);
        pw.set_next_figures(vec![Figure {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
            alt: "Chart of rain (mm)".into(),
        }]);
        pw.add_page(&page(PageImage::Bw(b.clone())), &[]);
        pw.add_page(&page(PageImage::Bw(b)), &[]); // no figures here
        let pdf = pw.finish();
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("/S /Figure ").count(), 1);
        let hex: String = "Chart of rain (mm)"
            .encode_utf16()
            .map(|u| format!("{u:04X}"))
            .collect();
        assert!(text.contains(&format!("/Alt <FEFF{hex}>")));
        let doc = reader::PdfDoc::open(pdf).unwrap();
        assert_eq!(doc.page_count(), 2);
    }

    #[test]
    fn tagged_pdf_has_structure_tree_marked_content_and_language() {
        let w = |t: &str, y: u32, line: u32| Word {
            text: t.into(),
            x: 100,
            y,
            w: 200,
            h: 40,
            conf: 90.0,
            line,
        };
        let mut pw = PdfWriter::new(Compress::Flate);
        pw.set_tagged(Some("ro"));
        let mut b = BitImage::new(600, 400);
        b.set(1, 1, true);
        pw.add_page(
            &page(PageImage::Bw(b.clone())),
            &[w("Bună", 50, 0), w("ziua", 50, 0), w("A doua", 120, 1)],
        );
        pw.add_page(&page(PageImage::Bw(b)), &[w("Pagina", 50, 0)]);
        let pdf = pw.finish();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/StructTreeRoot") && text.contains("/MarkInfo << /Marked true >>"));
        assert!(text.contains("/Lang (ro)"));
        assert_eq!(text.matches("/Type /StructElem /S /P ").count(), 3);
        assert!(text.contains("/StructParents 1"));
        // content streams are compressed; check through the reader instead
        let doc = reader::PdfDoc::open(pdf.clone()).unwrap();
        assert_eq!(doc.page_count(), 2);
    }

    /// All inflatable streams of a PDF, concatenated (test helper).
    fn inflated(pdf: &[u8]) -> String {
        use std::io::Read;
        let mut out = String::new();
        let mut i = 0;
        while let Some(k) = pdf[i..].windows(7).position(|w| w == b"stream\n") {
            let st = i + k + 7;
            let end = st
                + pdf[st..]
                    .windows(9)
                    .position(|w| w == b"endstream")
                    .unwrap_or(0);
            let mut d = String::new();
            if flate2::read::ZlibDecoder::new(&pdf[st..end])
                .read_to_string(&mut d)
                .is_ok()
            {
                out.push_str(&d);
            }
            i = end.max(st) + 9;
        }
        out
    }

    #[test]
    fn structured_tagging_groups_lines_into_paragraphs_and_headings() {
        let w = |t: &str, y: u32, line: u32| Word {
            text: t.into(),
            x: 100,
            y,
            w: 200,
            h: 40,
            conf: 90.0,
            line,
        };
        let words = [
            w("Titlu", 20, 0),
            w("a", 100, 1),
            w("b", 150, 2),
            w("c", 300, 3),
        ];
        let mut pw = PdfWriter::new(Compress::Flate);
        pw.set_tagged(Some("ro"));
        pw.set_next_structure(vec![
            StructPara {
                kind: StructKind::Heading(1),
                first: 0,
                end: 1,
            },
            StructPara {
                kind: StructKind::Para,
                first: 1,
                end: 3,
            },
            StructPara {
                kind: StructKind::Para,
                first: 3,
                end: 4,
            },
        ]);
        let mut b = BitImage::new(600, 400);
        b.set(1, 1, true);
        pw.add_page(&page(PageImage::Bw(b)), &words);
        let pdf = pw.finish();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type /StructElem /S /H1 "));
        assert_eq!(
            text.matches("/Type /StructElem /S /P ").count(),
            2,
            "two paragraphs, not four lines"
        );
        assert!(
            text.contains("/Title (Untitled document)"),
            "DisplayDocTitle needs a title"
        );
        let cs = inflated(&pdf);
        assert!(
            cs.contains("/H1 <</MCID 0>> BDC")
                && cs.contains("/P <</MCID 1>> BDC")
                && cs.contains("/P <</MCID 2>> BDC")
        );
        assert_eq!(cs.matches(" BDC").count(), 3);
        // the two lines of paragraph 1 share one marked-content sequence: 2 BT inside one BDC
        let p1 = cs
            .split("/P <</MCID 1>> BDC")
            .nth(1)
            .unwrap()
            .split("EMC")
            .next()
            .unwrap();
        assert_eq!(p1.matches("BT ").count(), 2);
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
            rights: None,
            provenance: Some("{\"software\":\"x\"}".into()),
            attachments: Vec::new(),
            right_to_left: false,
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
        assert!(
            pdf.contains("/LegatorOCRProvenance ("),
            "provenance missing from the Info dictionary"
        );
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
    fn control_characters_stay_out_of_the_text_layer_and_baselines_stay_on_the_page() {
        let mut w = PdfWriter::new(Compress::Flate);
        let mk = |t: &str, y: u32| Word {
            text: t.into(),
            x: 2,
            y,
            w: 12,
            h: 6,
            conf: 90.0,
            line: y,
        };
        // 100 x 100 px at 72 dpi: a word at the bottom edge would put its baseline below 0
        let words = [mk("a\nb\0c", 10), mk("edge", 98)];
        let mut cs = String::new();
        w.text_ops(&mut cs, &words, 72.0, 100.0, 100.0);
        // "abc" + space: four glyph codes
        assert!(cs.contains("Tj"), "{cs}");
        for l in cs.lines().filter(|l| l.ends_with("Tj")) {
            let hex = l.split('<').nth(1).unwrap().split('>').next().unwrap();
            assert!(hex.len() == 16 || hex.len() == 20, "{l}");
            let y: f32 = l.split_whitespace().nth(6).unwrap().parse().unwrap();
            assert!((0.0..=100.0).contains(&y), "{l}");
        }
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

    #[test]
    fn rights_notice_in_xmp_and_as_a_note_on_page_two() {
        let mk = |pdfa: bool| {
            let mut w = PdfWriter::new(Compress::Flate);
            w.set_pdfa(pdfa);
            w.set_meta(Meta {
                rights: Some("© Biblioteca, contract 12/2026 (Legea 8/1996)".into()),
                ..Meta::default()
            });
            for _ in 0..2 {
                w.add_page(&page(PageImage::Bw(BitImage::new(40, 40))), &[]);
            }
            String::from_utf8_lossy(&w.finish()).into_owned()
        };
        let plain = mk(false);
        assert_eq!(plain.matches("/Subtype /Text").count(), 1);
        assert!(plain.contains("/Rights "));
        let a = mk(true);
        assert!(a.contains("<dc:rights>"));
        assert!(
            !a.contains("/Subtype /Text"),
            "PDF/A needs appearance streams"
        );
    }
}

#[cfg(test)]
mod unicode_file {
    use super::*;
    /// Writes a PDF with Romanian text for external extraction checks:
    fn pdfa_sample() -> Vec<u8> {
        let mut w = PdfWriter::new(Compress::Jpeg(80));
        w.set_pdfa(true);
        w.set_title("Școala & <test>");
        w.set_meta(Meta {
            author: Some("Alexio".into()),
            subject: Some("subiect".into()),
            keywords: Some("a, b".into()),
            created_unix: Some(1_793_000_000),
            rights: Some("© Biblioteca X, contract 12/2026".into()),
            provenance: Some("{\"software\":\"x\"}".into()),
            attachments: Vec::new(),
            right_to_left: false,
        });
        let mut bw = BitImage::new(600, 300);
        bw.set(1, 1, true);
        let mk = |image| Page {
            image,
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        };
        let words: Vec<Word> = ["Școala", "țăranului"]
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
        w.add_page(&mk(PageImage::Bw(bw)), &words);
        w.add_page(&mk(PageImage::Rgb(image::RgbImage::new(40, 30))), &[]);
        w.finish()
    }

    #[test]
    fn pdfa_survives_empty_documents_and_exhausted_code_space() {
        // no pages at all
        let mut w = PdfWriter::new(Compress::Flate);
        w.set_pdfa(true);
        let empty = w.finish();
        assert!(empty.starts_with(b"%PDF-") && empty.ends_with(b"%%EOF\n"));
        // more distinct characters than a 16-bit font can address
        let mut w = PdfWriter::new(Compress::Flate);
        w.set_pdfa(true);
        let words: Vec<Word> = (0..70_000u32)
            .step_by(40)
            .filter_map(|b| {
                let t: String = (b..b + 40).filter_map(char::from_u32).collect();
                (!t.is_empty()).then_some(Word {
                    text: t,
                    x: 1,
                    y: 1,
                    w: 10,
                    h: 10,
                    conf: 90.0,
                    line: 0,
                })
            })
            .collect();
        w.add_page(
            &Page {
                image: PageImage::Bw(BitImage::new(20, 20)),
                dpi: 100,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                content_box: None,
                layers: None,
            },
            &words,
        );
        let big = w.finish();
        assert!(big.windows(10).any(|x| x == b"/FontFile2"));
    }

    #[test]
    fn pdfa_levels_are_declared_only_when_their_requirements_are_met() {
        let mk = |part: u8, level: char, tagged: Option<&str>| {
            let mut w = PdfWriter::new(Compress::Flate);
            w.set_pdfa(true);
            w.set_pdfa_part(part);
            w.set_pdfa_level(level);
            if let Some(l) = tagged {
                w.set_tagged(Some(l));
            }
            w.add_page(
                &Page {
                    image: PageImage::Bw(BitImage::new(8, 8)),
                    dpi: 150,
                    skew_deg: 0.0,
                    blank: false,
                    split_index: None,
                    content_box: None,
                    layers: None,
                },
                &[],
            );
            String::from_utf8_lossy(&w.finish()).into_owned()
        };
        let c = |s: &str| {
            s.contains("<pdfaid:conformance>").then(|| {
                s.split("<pdfaid:conformance>")
                    .nth(1)
                    .unwrap()
                    .chars()
                    .next()
                    .unwrap()
            })
        };
        assert_eq!(c(&mk(2, 'B', None)), Some('B'));
        assert_eq!(c(&mk(2, 'U', None)), Some('U'));
        assert_eq!(c(&mk(3, 'u', None)), Some('U'));
        // PDF/A-1 has no level U
        assert_eq!(c(&mk(1, 'U', None)), Some('B'));
        // level A needs the structure tree and a language
        assert_eq!(c(&mk(2, 'A', None)), Some('B'));
        assert_eq!(c(&mk(2, 'A', Some(""))), Some('B'));
        let a = mk(2, 'A', Some("ro"));
        assert_eq!(c(&a), Some('A'));
        assert!(a.contains("/StructTreeRoot") && a.contains("/Lang (ro)"));
        assert_eq!(c(&mk(1, 'A', Some("ro"))), Some('A'));
    }

    #[test]
    fn pdfa_parts_change_header_and_xmp() {
        let mk = |part: u8| {
            let mut w = PdfWriter::new(Compress::Flate);
            w.set_pdfa(true);
            w.set_pdfa_part(part);
            w.add_page(
                &Page {
                    image: PageImage::Bw(BitImage::new(8, 8)),
                    dpi: 150,
                    skew_deg: 0.0,
                    blank: false,
                    split_index: None,
                    content_box: None,
                    layers: None,
                },
                &[],
            );
            String::from_utf8_lossy(&w.finish()).into_owned()
        };
        let (a1, a2, a3) = (mk(1), mk(2), mk(3));
        assert!(a1.starts_with("%PDF-1.4\n") && a2.starts_with("%PDF-1.5\n"));
        assert!(a1.contains("<pdfaid:part>1</pdfaid:part>"));
        assert!(a2.contains("<pdfaid:part>2</pdfaid:part>"));
        assert!(a3.contains("<pdfaid:part>3</pdfaid:part>"));
        assert!(!a1.contains("/ObjStm") && !a1.contains("/SMask"));
    }

    #[test]
    fn pdfa_output_has_the_required_parts() {
        let pdf = pdfa_sample();
        let t = String::from_utf8_lossy(&pdf);
        for need in [
            "/FontFile2",
            "/Type /Metadata /Subtype /XML",
            "<pdfaid:part>2</pdfaid:part>",
            "/OutputIntents",
            "/DestOutputProfile",
            "/ID [<",
            "/CalGray",
        ] {
            assert!(t.contains(need), "missing {need}");
        }
        assert!(!t.contains("/DeviceGray"));
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(
            &Page {
                image: PageImage::Bw(BitImage::new(8, 8)),
                dpi: 150,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                content_box: None,
                layers: None,
            },
            &[],
        );
        let plain = String::from_utf8_lossy(&w.finish()).into_owned();
        assert!(!plain.contains("/OutputIntents") && !plain.contains("/FontFile2"));
        if let Ok(path) = std::env::var("LEGATOROCR_TEST_PDFA") {
            std::fs::write(path, &pdf).unwrap();
        }
    }

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
            content_box: None,
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
                content_box: None,
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

/// Outline level (0 = top, 1 = nested) of every entry; see `add_heading_bookmark`.
fn outline_levels(marks: &[(String, usize, f32)]) -> Vec<u8> {
    let sizes: Vec<f32> = marks.iter().map(|m| m.2).filter(|&v| v > 0.0).collect();
    let (lo, hi) = sizes
        .iter()
        .fold((f32::MAX, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if sizes.is_empty() || hi < 1.25 * lo {
        return vec![0; marks.len()];
    }
    let cut = (lo + hi) / 2.0;
    marks
        .iter()
        .map(|m| u8::from(m.2 > 0.0 && m.2 < cut))
        .collect()
}

#[cfg(test)]
mod outline_tests {
    use super::*;
    use legatorocr_core::{Page, PageImage};

    fn pg() -> Page {
        Page {
            image: PageImage::Gray(image::GrayImage::from_pixel(40, 40, image::Luma([255]))),
            dpi: 100,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        }
    }

    #[test]
    fn headings_of_clearly_different_size_are_nested() {
        let mut w = PdfWriter::new(Compress::Flate);
        for (t, size) in [
            ("Part one", 2.4f32),
            ("Section a", 1.5),
            ("Section b", 1.5),
            ("Part two", 2.4),
            ("Section c", 1.5),
        ] {
            w.add_heading_bookmark(t, size);
            w.add_page(&pg(), &[]);
        }
        let pdf = w.finish();
        let txt = String::from_utf8_lossy(&pdf).to_string();
        // two top-level entries and five in all; each part lists its sections
        assert!(
            txt.contains("/Type /Outlines") && txt.contains("/Count 5"),
            "{txt}"
        );
        assert_eq!(
            txt.matches("/Count 2").count(),
            1,
            "first part has two sections"
        );
        assert_eq!(
            txt.matches("/Count 1 ").count() + txt.matches("/Count 1>>").count(),
            1,
            "second part has one"
        );
        // similar sizes stay flat
        assert_eq!(
            outline_levels(&[("a".into(), 0, 1.5), ("b".into(), 1, 1.6)]),
            vec![0, 0]
        );
        assert_eq!(
            outline_levels(&[
                ("a".into(), 0, 2.4),
                ("b".into(), 1, 1.5),
                ("c".into(), 2, 0.0)
            ]),
            vec![0, 1, 0]
        );
        if std::process::Command::new("qpdf")
            .arg("--version")
            .output()
            .is_ok()
        {
            let p = std::env::temp_dir().join(format!("lgr-nest-{}.pdf", std::process::id()));
            std::fs::write(&p, &pdf).unwrap();
            let o = std::process::Command::new("qpdf")
                .arg("--check")
                .arg(&p)
                .output()
                .unwrap();
            let _ = std::fs::remove_file(&p);
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
    }

    #[test]
    fn bookmarks_and_labels_are_written_and_stay_valid() {
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_bookmark("Prima (ă)");
        w.set_page_labels(LabelStyle::RomanLower, "", 1);
        w.add_page(&pg(), &[]);
        w.add_page(&pg(), &[]);
        w.add_bookmark("Second");
        w.set_page_labels(LabelStyle::Decimal, "A-", 1);
        w.add_page(&pg(), &[]);
        w.add_bookmark("beyond the last page"); // dropped
        let pdf = w.finish();
        let txt = String::from_utf8_lossy(&pdf);
        assert!(txt.contains("/Type /Outlines"));
        assert!(txt.contains("/Count 2"));
        assert!(txt.contains("/S /r") && txt.contains("/S /D /P (A-) /St 1"));
        assert!(!txt.contains("beyond"));
        if std::process::Command::new("qpdf")
            .arg("--version")
            .output()
            .is_ok()
        {
            let p = std::env::temp_dir().join(format!("lgr-outline-{}.pdf", std::process::id()));
            std::fs::write(&p, &pdf).unwrap();
            let o = std::process::Command::new("qpdf")
                .arg("--check")
                .arg(&p)
                .output()
                .unwrap();
            let _ = std::fs::remove_file(&p);
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
        // no bookmark / label: untouched catalogue
        let mut w = PdfWriter::new(Compress::Flate);
        w.add_page(&pg(), &[]);
        let t = String::from_utf8_lossy(&w.finish()).into_owned();
        assert!(!t.contains("Outlines") && !t.contains("PageLabels"));
    }
}
