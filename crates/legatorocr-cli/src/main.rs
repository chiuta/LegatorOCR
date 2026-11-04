mod fff;
use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use legatorocr_core::{HAlign, OutputMode, Settings, SplitMode, Threshold, VAlign};
use legatorocr_engine::project::{Project, SheetAdj};
use legatorocr_engine::{
    check_ocr,
    files::{collect_images, detect_dpi},
    ocr::OcrOptions,
    run_export, write_atomic, ExportJob, PdfExport, SheetInput,
};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex};

#[derive(Copy, Clone, ValueEnum)]
enum Mode {
    Color,
    Gray,
    Bw,
    /// 1-bit text + colour picture patches.
    Mixed,
    /// 1-bit text over a low-resolution colour background (smallest colour PDFs).
    Hybrid,
}

#[derive(Copy, Clone, ValueEnum)]
enum Split {
    Auto,
    Off,
    Middle,
}

#[derive(Copy, Clone, ValueEnum)]
enum Fmt {
    Png,
    Tiff,
    Jpeg,
    None,
}

/// LegatorOCR — clean-room scan post-processor: split, deskew, crop, clean,
/// then export to PDF with optional OCR text layer.
#[derive(Parser)]
#[command(name = "legatorocr", version, about)]
struct Cli {
    /// Input page images (PNG, JPEG, TIFF, BMP) and scanned PDFs, in reading order.
    #[arg(required_unless_present_any = ["project", "list_profiles", "save_profile", "make_separator", "ocr_languages", "doctor", "watch", "verify_bag", "verify_ocfl", "timestamp", "timestamp_verify", "sign_pdf", "verify_pdf_signatures", "decode_worker", "fuse_frames", "verify_document", "integrity_hints", "mcp", "measure_edge", "backup_from", "efactura", "merkle_root", "merkle_verify", "verify_sums", "extract_project", "search_preset", "search_help"])]
    inputs: Vec<PathBuf>,

    /// Load settings and page list (with per-page adjustments) from a `.lgr` project
    /// saved by the GUI or `--save-project`, or a ScanTailor project (`.ScanTailor`: scan list and
    /// rotation only; skew, split and content are found again). Its processing settings replace the
    /// command-line ones; output options (`--pdf`, `--ocr`, …) still apply.
    #[arg(long)]
    project: Option<PathBuf>,

    /// Start from a named profile (built-in or saved in the user's profiles folder);
    /// like `--project`, its processing settings replace the command-line ones.
    #[arg(long)]
    profile: Option<String>,

    /// List the available profiles and exit.
    #[arg(long)]
    list_profiles: bool,

    /// Save the effective processing settings as a user profile with this name.
    #[arg(long)]
    save_profile: Option<String>,

    /// Write the effective settings and page list to a `.lgr` project file.
    #[arg(long)]
    save_project: Option<PathBuf>,

    /// Directory for processed page images.
    #[arg(short = 'o', long, default_value = "legatorocr-out")]
    out_dir: PathBuf,

    /// Image format for processed pages (`none` = PDF only).
    #[arg(long, value_enum, default_value = "png")]
    format: Fmt,

    /// JPEG quality for `--format jpeg` page images.
    #[arg(long, default_value_t = 90)]
    jpeg_quality: u8,

    /// Only report pages whose size differs clearly from the rest of the book
    /// (missing corners, foldouts, wrong-size inserts) and exit.
    #[arg(long)]
    report_outliers: bool,

    /// Write an HTML contact sheet (thumbnails, sizes, skew; odd pages highlighted).
    #[arg(long)]
    report: Option<PathBuf>,

    /// Write all (black & white) pages into one multi-page Group 4 TIFF.
    #[arg(long)]
    multitiff: Option<PathBuf>,

    /// Write all processed pages into one PDF. Output paths (also --txt, --hocr, --json,
    /// --docx, --package) may use {date}, {year}, {month}, {day} and {name} (the first input's
    /// name); folders are created: --pdf "archive/{year}/{month}/{date}_{name}.pdf".
    #[arg(long)]
    pdf: Option<PathBuf>,

    /// JPEG quality inside the PDF for colour/gray pages (0 = lossless Flate).
    #[arg(long, default_value_t = 80)]
    pdf_quality: u8,

    /// PDF document information: author, subject, keywords.
    #[arg(long)]
    author: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long)]
    keywords: Option<String>,

    /// Also write the recognised text of all pages to this file (needs --ocr).
    #[arg(long, alias = "sidecar")]
    txt: Option<PathBuf>,

    /// Also write an hOCR file with word boxes and confidences (needs --ocr).
    #[arg(long)]
    hocr: Option<PathBuf>,

    /// Clean the .txt and .docx text for Romanian: ș/ț with comma-below, words split at a line end
    /// joined, typographic quotes, spacing around punctuation. The PDF text layer and hOCR keep
    /// the raw words (needs --ocr).
    #[arg(long)]
    clean_text: bool,

    /// Leave out PDF inputs that already have a text layer (only scans without text are
    /// processed), so a half-finished batch can be run again without redoing finished files.
    #[arg(long)]
    skip_text: bool,

    /// What to do with PDF inputs that already have text: `skip` (same as --skip-text), `redo`
    /// or `force` (default: the page is rebuilt from its image and read again; the old text
    /// layer is never kept, so these two behave the same).
    #[arg(long, value_parser = ["skip", "redo", "force"], value_name = "MODE")]
    ocr_mode: Option<String>,

    /// Write a table of the PDFs this run produced (title, file, size, SHA-256) to this file; the
    /// extension picks the format: `.csv`, else HTML (needs --pdf).
    #[arg(long)]
    catalog: Option<PathBuf>,

    /// Write a Dublin Core record `<pdf>.dc.xml` beside every PDF (title, author, subject,
    /// keywords, rights, language) for library catalogues (needs --pdf).
    #[arg(long)]
    dublin_core: bool,

    /// Also write ALTO v4 XML (word boxes and confidences, the library standard) (needs --ocr).
    #[arg(long)]
    alto: Option<PathBuf>,

    /// Write one JSON object per paragraph (JSON Lines) for citation-grounded research tools:
    /// source file, page, printed page label (with --page-labels), text (cleaned with
    /// --clean-text), box in page pixels and as fractions, language, mean OCR confidence, SHA-256
    /// of the page image, a stable passage id and a "cite" string. Needs --ocr. Schema in README.
    #[arg(long, value_name = "FILE")]
    passages_jsonl: Option<PathBuf>,

    /// Title used in the "cite" string of --passages-jsonl ("<title> p.<label>"); default: the
    /// source file name without extension.
    #[arg(long, value_name = "TEXT")]
    passages_title: Option<String>,

    /// Also write PAGE XML, one `page_NNNN.xml` per page, into this folder (needs --ocr).
    #[arg(long)]
    page_xml: Option<PathBuf>,

    /// Also write the recognised words as structured JSON: per page its size and dpi, per word
    /// the text, box (pixels of the processed page), confidence and line (needs --ocr).
    #[arg(long)]
    json: Option<PathBuf>,

    /// Write the recognised text as an FFF envelope (`document/ocr`, `.fff.json`) for the next app in the
    /// source chain: LegatorOCR -> TEZAUR (`tezaur ingest --fff-in`) -> VERIGA -> AcademiSearch. Carries the text
    /// per page and the SHA-256 of the scan; works offline (needs --ocr). Spec: docs/FFF-ENVELOPE.md in the hub.
    #[arg(long, value_name = "FILE")]
    fff_out: Option<PathBuf>,

    /// Title for --fff-out (default: first heading of the recognised text).
    #[arg(long, value_name = "TEXT")]
    fff_title: Option<String>,

    /// Publication year for --fff-out (makes the reference citable as Author (Year)).
    #[arg(long, value_name = "YYYY")]
    fff_year: Option<String>,

    /// Also write the recognised text as an editable Word document, one page break per
    /// scanned page (needs --ocr).
    #[arg(long)]
    docx: Option<PathBuf>,

    /// Also write the recognised text as TEI P5 XML: a page break per scanned page, paragraphs,
    /// tables, columns (needs --ocr). Page structure only; no headings, notes or index markup.
    #[arg(long)]
    tei: Option<PathBuf>,

    /// SEARCH instead of processing: find this phrase in the given files and folders (PDF
    /// text layers, .txt, .hocr/.html, .docx, and all of these inside .zip). Ignores case
    /// and diacritics (`mobila` finds `Mobilă`, also across a line break). Exit status 1
    /// when nothing is found (2 when, in addition, some files could not be read).
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,

    /// SEARCH for a ready-made pattern instead of text: cui, iban-ro, invoice, email,
    /// phone-ro, date, amount (see --search-help). Same files and options as --search.
    #[arg(long, value_name = "NAME", conflicts_with = "search")]
    search_preset: Option<String>,

    /// List the --search-preset names and what each finds.
    #[arg(long)]
    search_help: bool,

    /// With --search: TEXT is a regular expression, matched line by line.
    #[arg(long)]
    search_regex: bool,

    /// With --search: the text is a boolean query: AND / OR / NOT (or + and -), "phrase", ? and *
    /// wildcards, /regex/ and `word~1` for one-letter OCR slips; case and Romanian diacritics are
    /// ignored. Per line. Example: --search 'brasov AND (factura OR contract~1)'.
    #[arg(long)]
    search_query: bool,

    /// With --search: match case and diacritics exactly.
    #[arg(long)]
    search_exact: bool,

    /// With --search: stop after this many hits.
    #[arg(long, value_name = "N", default_value_t = 0)]
    search_max: usize,

    /// Drop a page that is almost identical to the previous one (double feed).
    #[arg(long)]
    skip_duplicates: bool,

    /// With --search: also match the old Romanian spelling of 1953-1993 (textbooks, books of the
    /// time): `sunt` finds `sînt`, `pânză` finds `pînză`, `când` finds `cînd`, and `ş ţ` match
    /// `ș ț`. The query and each line are also compared after rewriting `î` inside a word to
    /// `â` and `sînt/sîntem/sînteți` to `sunt/suntem/sunteți`; the search as typed still runs, so
    /// nothing found before is lost. Works with --search-query, wildcards and NEAR; ignored
    /// with --search-regex.
    #[arg(long)]
    search_old_orthography: bool,

    /// Treat blank pages as separators and write one PDF per document
    /// (`name_001.pdf`, `name_002.pdf`, …). Needs --pdf.
    #[arg(long)]
    split_on_blank: bool,

    /// Treat pages carrying a bar code (Code 128, EAN-13, ITF or Code 39) as separators (the page is dropped) and
    /// write one PDF per document, named after the bar code (`name_001_<code>.pdf`).
    /// Print separators with --make-separator. Needs --pdf.
    #[arg(long)]
    split_on_barcode: bool,

    /// Compare two text files, REFERENCE then OCR-OUTPUT (the two INPUTS), and print the
    /// character and word error rates. Case counts, whitespace and ş/ţ vs ș/ț do not.
    #[arg(long)]
    compare_text: bool,

    /// Show which of the eight interface languages (ro en fr it es pt de hu) have a
    /// recognition model installed, and how to get the missing ones; then exit.
    #[arg(long)]
    ocr_languages: bool,

    /// Check which optional helpers (Tesseract and its language models, pdftoppm, mutool,
    /// Ghostscript, qpdf) are installed and what a missing one costs; then exit.
    #[arg(long)]
    doctor: bool,

    /// List PDFs among the inputs that contain JavaScript, launch actions, embedded files,
    /// actions on open, form submission or rich media; then exit. A static look at the
    /// dictionaries only: nothing is executed, and a clean result is not a guarantee.
    #[arg(long)]
    scan_active: bool,

    /// After export, read the PDF's text layer back and compare it with the OCR text
    /// (needs --ocr and --pdf); warns when copy/search would fail.
    #[arg(long)]
    verify_text: bool,

    /// Use printed page numbers found by OCR as the PDF's page labels (a viewer then shows
    /// "17" for the page that carries 17, even after an unnumbered cover). Needs --ocr.
    #[arg(long)]
    page_labels: bool,

    /// After the run, print for every page what was decided and why: rotation, split, skew
    /// angle, blank or not, where the content was found, resolution.
    #[arg(long)]
    explain: bool,

    /// OCR language per input page, e.g. `1-40=ron,41-44=eng+fra,45-=ron` (pages counted from 1
    /// in input order; the last matching rule wins; other pages use --lang). Needs --ocr.
    #[arg(long, value_name = "RULES")]
    page_lang: Option<String>,

    /// Phone-capture ingest: order the photos by EXIF capture time (when every file has one)
    /// and keep only the sharpest frame of each burst; lists what was left out.
    #[arg(long)]
    burst: bool,

    /// With --burst: frames closer than this many seconds count as one burst.
    #[arg(long, default_value_t = 3, value_name = "SECONDS")]
    burst_gap: i64,

    /// Hot folder: keep running and process every scan that appears in DIR (PNG, JPEG, TIFF,
    /// BMP, PDF) once it has stopped growing, with all the other options. `--pdf` must name the
    /// output with `{name}` (or `{title}`) so each file gets its own. Stop with Ctrl+C.
    #[arg(long, value_name = "DIR")]
    watch: Option<PathBuf>,

    /// With --watch: seconds between looks at the folder.
    #[arg(long, default_value_t = 2, value_name = "SECONDS")]
    watch_interval: u64,

    /// With --watch: ignore the files already in the folder when it starts.
    #[arg(long)]
    watch_skip_existing: bool,

    /// Remember OCR results in DIR, keyed by the page's pixels and the OCR settings: a re-run
    /// whose page images are unchanged skips OCR. Entries stop matching by themselves when
    /// the OCR engine or this program is upgraded. Needs --ocr.
    #[arg(long, value_name = "DIR")]
    ocr_cache: Option<PathBuf>,

    /// Write the rights notice from the author's year of death (Romanian law: life + 70 years,
    /// counted from 1 January of the year after the death) instead of typing it. Information
    /// only; special cases need checking. Needs --pdf; not together with --rights-notice.
    #[arg(long, value_name = "YEAR")]
    rights_death_year: Option<i32>,

    /// Author's name for --rights-death-year.
    #[arg(long, value_name = "NAME", requires = "rights_death_year")]
    rights_author: Option<String>,

    /// Assignment deed number for the rights notice (needs --rights-deed-date).
    #[arg(
        long,
        value_name = "NO",
        requires = "rights_deed_date",
        requires = "rights_death_year"
    )]
    rights_deed_number: Option<String>,

    /// Assignment deed date for the rights notice, printed as written.
    #[arg(long, value_name = "DATE", requires = "rights_deed_number")]
    rights_deed_date: Option<String>,

    /// Also copy everything this run wrote into a BagIt 1.0 bag in DIR (a new folder): files
    /// under data/, SHA-256 manifest and tag manifest. Needs --pdf.
    #[arg(long, value_name = "DIR")]
    bagit: Option<PathBuf>,

    /// Also write everything this run wrote as version v1 of an OCFL 1.1 object in DIR (a new
    /// folder; SHA-512 inventory). The object id is --ocfl-id (default: urn:legatorocr: + the PDF's file name).
    /// Needs --pdf. Not checked against the reference OCFL validator.
    #[arg(long, value_name = "DIR")]
    ocfl: Option<PathBuf>,

    /// Object id for --ocfl (for example a URN or a catalogue number).
    #[arg(long, value_name = "ID", requires = "ocfl")]
    ocfl_id: Option<String>,

    /// Check an OCFL object written by --ocfl (inventory sidecar, every file against its SHA-512,
    /// files not in the manifest); exit 1 on any problem; then exit. Not a general validator.
    #[arg(long, value_name = "DIR")]
    verify_ocfl: Option<PathBuf>,

    /// Check a BagIt bag (such as one made by --bagit): every file against its manifest, the
    /// tag files, and files the manifest does not list; exit 1 on any problem; then exit.
    #[arg(long, value_name = "DIR")]
    verify_bag: Option<PathBuf>,

    /// After export, read the PDF back and check every page against the page that was meant to
    /// be written: page count, pixel size, a thumbnail comparison. Exit 1 on a problem.
    /// (Layered pages are checked for count and size only.) Needs --pdf.
    #[arg(long)]
    certify: bool,

    /// Store a provenance record in the PDF (Info key /LegatorOCRProvenance, JSON): program
    /// version, time, the input files with SHA-256 and the command-line options (passwords are
    /// left out). Documentation, not a signature. Not with --pdfa. Needs --pdf.
    #[arg(long)]
    provenance: bool,

    /// Print the Merkle root (RFC 6962 style, SHA-256) of a SHA256SUMS.txt and the number of
    /// files; one short value that stands for the whole collection (publish or timestamp it
    /// elsewhere; this program does not call a timestamp service). With --merkle-proof NAME,
    /// also print that file's audit path. With --merkle-expect HEX, exit 1 unless the root matches.
    #[arg(long, value_name = "SHA256SUMS")]
    merkle_root: Option<PathBuf>,

    /// With --merkle-root: print the proof that this file name belongs to the collection.
    #[arg(long, value_name = "NAME", requires = "merkle_root")]
    merkle_proof: Option<String>,

    /// Root to compare with (for --merkle-root and --merkle-verify).
    #[arg(long, value_name = "HEX")]
    merkle_expect: Option<String>,

    /// Check a proof line printed by --merkle-proof ("name hash index count path,path,...")
    /// against the root given with --merkle-expect; exit 1 if it does not lead to that root.
    #[arg(long, value_name = "LINE", requires = "merkle_expect")]
    merkle_verify: Option<String>,

    /// Also write the tables found on the pages as an Excel workbook (one sheet per table, named
    /// p<page>-t<n>); plain numbers become numbers, everything else stays text as printed.
    /// Needs --ocr.
    #[arg(long, value_name = "FILE")]
    tables_xlsx: Option<PathBuf>,

    /// Also write the tables found on the pages as one HTML file. Needs --ocr.
    #[arg(long, value_name = "FILE")]
    tables_html: Option<PathBuf>,

    /// Add a PDF bookmark for every page whose upper part carries a heading (a short line
    /// of clearly taller words), titled with its text. A heuristic; pages without a clear
    /// heading get none. Needs --ocr and --pdf.
    #[arg(long)]
    bookmarks_headings: bool,

    /// Print time per stage (load, orientation, clean-up, OCR) and peak memory at the end.
    #[arg(long)]
    timings: bool,

    /// After the run, list scans that may be blurred, cut off at the edge, duplicated or too
    /// low / too high in resolution (heuristics, not verdicts).
    #[arg(long)]
    quality_report: bool,

    /// Add one PDF outline entry (bookmark) per source file.
    #[arg(long)]
    bookmarks: bool,

    /// Write PDF/A-2b structure (embedded text font, XMP metadata, sRGB output intent).
    #[arg(long)]
    pdfa: bool,

    /// Read an e-Factura (RO_CIUS UBL) invoice, .xml or ANAF's .zip: print the key fields and
    /// check that amounts add up and CUI check digits pass (exit 1 on a problem).
    #[arg(long, value_name = "FILE")]
    efactura: Option<PathBuf>,

    /// Black-and-white pages (needs --ocr, --mode bw/mixed/hybrid): read each page with the chosen
    /// threshold and with a global Otsu threshold, keep the one Tesseract is more sure of. About
    /// twice the clean-up and OCR time for pages under 93 % mean confidence.
    #[arg(long)]
    binarize_by_ocr: bool,

    /// Needs --ocr. A page whose read-back looks doubtful (a --certify check fails, or under
    /// --auto-retry-below of its dark pixels lie inside a recognised word) is processed again with
    /// other binarisers (black-and-white modes), deskew flipped, and OCR at another dpi; the
    /// alternative with the best objective score (not Tesseract's confidence) is kept. Printed under
    /// --explain. EXPERIMENTAL and NOT recommended: it failed its gate (ROADMAP 4.24): no gain on
    /// real pages, and it made skewed synthetic pages much worse.
    #[arg(long)]
    auto_retry: bool,

    /// With --auto-retry: retry pages whose ink coverage is under this fraction (0.8 by default;
    /// above 1 retries every page).
    #[arg(long, value_name = "FRACTION", default_value_t = 0.8)]
    auto_retry_below: f64,

    /// Make a scanned paper form fillable: ruled boxes, check boxes and underlines become PDF
    /// form fields over the scan. Needs --pdf; not with --pdfa. Scans with broken or faint
    /// lines lose fields: look at the result in a PDF viewer.
    #[arg(long)]
    form_fields: bool,

    /// Remove personal data for hand-over (needs --ocr): words matching these presets (comma
    /// list from --search-help: cui, iban-ro, email, phone-ro, date, ...) are painted black in
    /// the page and dropped from every text output. Only what OCR read is found: check the
    /// result by eye. Not with image output, --multitiff, --package or --ocr-cache.
    #[arg(long, value_name = "PRESETS", value_delimiter = ',')]
    redact: Vec<String>,

    /// Like --redact, with your own regular expression (repeatable), matched line by line.
    #[arg(long, value_name = "REGEX")]
    redact_regex: Vec<String>,

    /// RFC 3161 time stamp: send the SHA-256 of FILE to --tsa-url (needs curl) and save the
    /// authority's reply as FILE.tsr. Good for a SHA256SUMS.txt or a Merkle root file.
    #[arg(long, value_name = "FILE", requires = "tsa_url")]
    timestamp: Option<PathBuf>,

    /// Address of the time-stamp authority for --timestamp (https://...). Choose one you trust.
    #[arg(long, value_name = "URL")]
    tsa_url: Option<String>,

    /// Check FILE against FILE.tsr (hash and structure; with --tsa-ca also the signature, through
    /// openssl ts -verify). Exit 1 when the file changed.
    #[arg(long, value_name = "FILE")]
    timestamp_verify: Option<PathBuf>,

    /// CA / TSA certificate file (PEM) for --timestamp-verify.
    #[arg(long, value_name = "PEM")]
    tsa_ca: Option<PathBuf>,

    /// Combine the input photos of ONE page (several shots, the first is the reference) into a
    /// single cleaner image written to FILE (.png/.jpg): frames are aligned by translation and
    /// merged by a per-pixel median. Frames that do not match are left out and listed. Then
    /// process FILE as usual.
    #[arg(long, value_name = "FILE")]
    fuse_frames: Option<PathBuf>,

    /// Largest shift between frames, in pixels (--fuse-frames).
    #[arg(long, value_name = "PX", default_value_t = 40)]
    fuse_max_shift: u32,

    /// Look at each input picture for things a person should check: QR code contents, regions
    /// copied inside the picture (copy-move), and for JPEGs regions with a different
    /// re-compression error level (ELA). Hints only, with known false alarms; not proof of
    /// forgery or of authenticity.
    #[arg(long)]
    verify_document: bool,

    /// Look at each input picture (as it is, no cleaning) and write a JSON list of regions a
    /// person may want to check again: JPEG block-pattern offset, second JPEG save, noise level
    /// that differs from the page, a word out of line with its text line (needs --ocr), and
    /// identical patches at two places. Neutral indicators with explanations and common innocent
    /// causes; HIGH false-alarm risk; never a verdict and never the sole basis of a decision
    /// about a person or a diploma. Which signals run: see --integrity-signals.
    #[arg(long, value_name = "FILE.json")]
    integrity_hints: Option<PathBuf>,

    /// Signals for --integrity-hints: comma-separated from jpeg_grid, jpeg_double, noise,
    /// line_size, copy_move, or `all` / `default`. Signals whose false-alarm rate on unedited
    /// test pages was above 10 % are not in `default` (see README).
    #[arg(long, value_name = "LIST", default_value = "default")]
    integrity_signals: String,

    /// Also write the pages as a DjVu document (bundled multi-page): black & white pages as
    /// lossless JB2, grey / colour pages as a JB2 foreground mask over an IW44 background, with
    /// the OCR text as a hidden text layer (--ocr). Uses the MIT-licensed djvu-rs encoders.
    #[arg(long, value_name = "FILE")]
    djvu: Option<PathBuf>,

    /// Add a page preview (TH44, 128 px) to each page of the --djvu file, for the page strip of
    /// DjVu viewers. Makes the file larger (about 6 KB per page; +12 % on 12 black & white pages).
    #[arg(long, requires = "djvu")]
    djvu_thumbnails: bool,

    /// With --ocr: write a tab-separated list of words that are not in the language's word list
    /// and were read with confidence below --doubtful-conf, for a person to check first.
    /// Text is not changed. Needs a built-in word list (ron, eng, fra, ita, spa, por, deu, hun).
    #[arg(long, value_name = "FILE.tsv")]
    doubtful_words: Option<PathBuf>,

    /// With --ocr: write one HTML page for correcting the text by hand: each page picture next to
    /// an editable box with the recognised text, unsure words listed, and a button that saves the
    /// corrected text (pages separated by a form feed). Self-contained, works offline. Corrections
    /// are not checked; a person must read every page against its picture.
    #[arg(long, value_name = "FILE.html")]
    proof_sheet: Option<PathBuf>,

    /// Confidence limit for --doubtful-words (0-100).
    #[arg(long, value_name = "CONF", default_value_t = 85.0)]
    doubtful_conf: f32,

    /// Process only part of the input list so several machines (or runs) can share a big batch:
    /// `I/N` takes the I-th of N consecutive slices (I from 1), e.g. `--shard 2/4`. Give each
    /// shard its own --pdf / output folder, then join the results with --merge-pdf. Pages keep
    /// the order of the full list; the slices differ in size by at most one page.
    #[arg(long, value_name = "I/N")]
    shard: Option<String>,

    /// Read the machine-readable zone (ICAO 9303 passport / ID card lines) from the OCR text and
    /// check every check digit; prints the fields, or which check failed. Needs --ocr. Reads the
    /// printed zone only; does not prove the document is genuine.
    #[arg(long)]
    mrz: bool,

    /// Decode every input in a separate worker process with a memory limit, a CPU-time limit
    /// (Unix) and a timeout, so a hostile or corrupt file can crash only the worker, not the
    /// run. Process isolation, not a security sandbox (no seccomp or namespaces).
    #[arg(long)]
    isolate_decoders: bool,

    /// Memory limit per decoder worker, in MB (--isolate-decoders; Unix only).
    #[arg(long, value_name = "MB", default_value_t = 4096)]
    decoder_memory: u64,

    /// Wall-clock limit per decoder worker, in seconds (--isolate-decoders).
    #[arg(long, value_name = "SECONDS", default_value_t = 120)]
    decoder_timeout: u64,

    /// Run every loaded page through a WebAssembly image filter (exports `memory`,
    /// `lgr_alloc(len)->ptr`, `lgr_filter(ptr,w,h)->status` on packed RGB8; no imports, fuel-limited).
    #[arg(long, value_name = "FILE.wasm")]
    plugin: Option<PathBuf>,

    /// Internal: decode INPUT to a BMP at OUTPUT (used by --isolate-decoders).
    #[arg(long, num_args = 2, value_names = ["INPUT", "OUTPUT"], hide = true)]
    decode_worker: Vec<PathBuf>,

    /// Make the PDF a reversible project: embed the original input files (byte for byte, not
    /// recompressed) and project.lgr (settings, per-sheet adjustments, zones, guide lines, the
    /// resolution used) as PDF attachments (`/EmbeddedFiles`, `/AFRelationship /Source`). Opt-in:
    /// the PDF grows by the size of the originals (the size is printed). With --pdfa it needs
    /// --pdfa-part 3. Not with --split-on-blank / --split-on-barcode. Needs --pdf. Get them back
    /// with --extract-project.
    #[arg(long)]
    embed_project: bool,

    /// Write the files embedded by --embed-project (the originals and project.lgr) into a new or
    /// empty folder, never overwriting; then exit. Re-run with `--project DIR/project.lgr`.
    #[arg(long, num_args = 2, value_names = ["PDF", "DIR"])]
    extract_project: Vec<PathBuf>,

    /// Reprocessable archive package: a NEW folder with the original inputs, project.lgr (settings
    /// and per-page adjustments, relative paths), derived/ (outputs and ocr.json with every word,
    /// box and confidence), RUN.txt (how to run again) and SHA256SUMS.txt. Needs --ocr. Re-run it
    /// later with a better engine and compare. Never overwrites.
    #[arg(long, value_name = "DIR")]
    archive_package: Option<PathBuf>,

    /// Tagged PDF: each OCR line becomes a paragraph element of a structure tree, the scan image
    /// is an artifact, and /Lang is set to CODE (BCP 47, e.g. ro, en-GB; omit for none). A base
    /// for PDF/UA and screen readers, not a conformance claim: no headings, lists, tables or
    /// alt text. Works with --ocr; without OCR there is nothing to tag.
    #[arg(long, value_name = "CODE", num_args = 0..=1, default_missing_value = "")]
    tagged: Option<String>,

    /// Structured tagged PDF (toward PDF/UA, not a conformance claim): like --tagged, but one
    /// element per OCR paragraph instead of per line, the page heading found by the
    /// --bookmarks-headings detector as /H1../H3, one marked-content id per element in reading
    /// order, /Lang taken from --lang (ron -> ro), a document /Title (the output file name) and
    /// DisplayDocTitle. No figures, lists or tables. Needs --ocr and --pdf.
    #[arg(long)]
    tagged_pdf: bool,

    /// Sign FILE with your PKCS#12 certificate (--sign-p12): CAdES detached, SHA-256, added as an
    /// incremental update (the earlier bytes stay unchanged). Output: --pdf NAME or FILE.signed.pdf.
    /// Needs openssl and a PDF whose last xref is a classic table. The legal level (simple,
    /// advanced, qualified) is that of your certificate.
    #[arg(long, value_name = "FILE", requires = "sign_p12")]
    sign_pdf: Option<PathBuf>,

    /// PKCS#12 (.p12/.pfx) file for --sign-pdf.
    #[arg(long, value_name = "P12")]
    sign_p12: Option<PathBuf>,

    /// Environment variable holding the PKCS#12 password (never on the command line).
    #[arg(long, value_name = "VAR", default_value = "LEGATOROCR_P12_PASS")]
    sign_pass_env: String,

    /// Signer name and reason written into the signature (--sign-pdf).
    #[arg(long, value_name = "TEXT")]
    sign_name: Option<String>,
    #[arg(long, value_name = "TEXT")]
    sign_reason: Option<String>,

    /// Put an RFC 3161 time stamp from --tsa-url inside the signature (--sign-pdf): PAdES-B-T.
    /// The authority sees only the hash of the signature.
    #[arg(long, requires = "tsa_url")]
    sign_timestamp: bool,

    /// Check every signature in FILE (data matches the signature; with --sign-ca also the
    /// certificate chain). Exit 1 when one fails or the file has no signature.
    #[arg(long, value_name = "FILE")]
    verify_pdf_signatures: Option<PathBuf>,

    /// CA / signer certificate (PEM) for --verify-pdf-signatures.
    #[arg(long, value_name = "PEM")]
    sign_ca: Option<PathBuf>,

    /// Run as a Model Context Protocol server on stdin/stdout (JSON-RPC, one message per line)
    /// so an AI assistant can call a few narrow tools: scan_to_pdf, pdf_toolbox, efactura,
    /// measure_edge, search. Each tool starts this program with a fixed argument list.
    #[arg(long)]
    mcp: bool,

    /// Sharpness: measure the spatial frequency response (MTF50, MTF10) on a slanted black /
    /// white edge in this image (ISO 12233 style, near-vertical edge, a few degrees of tilt).
    /// Use --edge-roi to select the edge; --dpi converts to line pairs per mm.
    #[arg(long, value_name = "IMAGE")]
    measure_edge: Option<PathBuf>,

    /// Box around the edge for --measure-edge: x,y,width,height in pixels (default: whole image).
    #[arg(long, value_name = "X,Y,W,H")]
    edge_roi: Option<String>,

    /// Backup helper: copy this folder to every --backup-to folder, read each copy back and
    /// compare SHA-256 (exit 1 on a mismatch); writes SHA256SUMS.txt in each destination.
    #[arg(long, value_name = "DIR")]
    backup_from: Option<PathBuf>,

    /// Destination folder for --backup-from (repeat for several).
    #[arg(long, value_name = "DIR")]
    backup_to: Vec<PathBuf>,

    /// Mark the backup copies read-only.
    #[arg(long)]
    backup_read_only: bool,

    /// PDF toolbox: join all input PDFs into one file (name from --pdf, else merged.pdf in -o).
    #[arg(long)]
    merge_pdf: bool,

    /// PDF toolbox: keep only these pages of each input PDF, e.g. 1-3,7 or 5- (order kept).
    #[arg(long, value_name = "RANGES")]
    extract_pages: Option<String>,

    /// PDF toolbox: cut the result into files of N pages (name_001.pdf, name_002.pdf, ...).
    #[arg(long, value_name = "N")]
    split_pdf_every: Option<usize>,

    /// PDF toolbox: diagonal grey text across every page.
    #[arg(long, value_name = "TEXT")]
    watermark: Option<String>,

    /// PDF toolbox: number the pages at the bottom; FORMAT uses {n} and {total} (default {n}).
    #[arg(long, value_name = "FORMAT", num_args = 0..=1, default_missing_value = "{n}")]
    number_pages: Option<String>,

    /// Memory budget in MB: process fewer pages at once so the working set stays near it
    /// (estimate from the largest page; at least one page always runs). 0 = no limit.
    #[arg(long, default_value_t = 0)]
    max_memory: u64,

    /// PDF/A part to declare with --pdfa: 1 (PDF 1.4), 2 (default) or 3. Level is always B.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=3))]
    pdfa_part: u8,

    /// OCRmyPDF compatibility: same as `--ocr-mode force`.
    #[arg(long, hide = true)]
    force_ocr: bool,

    /// OCRmyPDF compatibility: same as `--ocr-mode redo`.
    #[arg(long, hide = true)]
    redo_ocr: bool,

    /// OCRmyPDF compatibility: skew correction is on by default; accepted and ignored.
    #[arg(long, hide = true)]
    deskew: bool,

    /// OCRmyPDF compatibility: same as `--auto-rotate`.
    #[arg(long, hide = true)]
    rotate_pages: bool,

    /// OCRmyPDF compatibility: `pdfa` (or `pdfa-1`, `pdfa-2`, `pdfa-3`) = `--pdfa [--pdfa-part N]`, `pdf` = plain PDF.
    #[arg(long, hide = true, value_parser = ["pdf", "pdfa", "pdfa-1", "pdfa-2", "pdfa-3"], value_name = "TYPE")]
    output_type: Option<String>,

    /// Number of worker threads (default: all cores). Same as OCRmyPDF's --jobs.
    #[arg(short = 'j', long, value_name = "N")]
    jobs: Option<usize>,

    /// Label the first N pages i, ii, iii … in PDF viewers; numbering restarts at 1 after them.
    #[arg(long, value_name = "N", default_value_t = 0)]
    front_matter: u32,

    /// Write a printable Code 128 separator sheet (PNG, 300 dpi A4) for this text and exit.
    #[arg(long, value_name = "TEXT")]
    make_separator: Option<String>,

    /// Choose the JPEG quality so the PDF stays within this many megabytes (estimated
    /// from a sample of pages). Needs --pdf.
    #[arg(long)]
    target_size: Option<f32>,

    /// Run OCR and embed an invisible, searchable text layer in the PDF.
    #[arg(long)]
    ocr: bool,

    /// Tesseract language(s), e.g. `ron+eng`.
    #[arg(short = 'l', long, alias = "language", default_value = "eng")]
    lang: String,

    /// Tesseract page-segmentation mode.
    #[arg(long, default_value_t = 3)]
    psm: u32,

    /// Drop OCR words below this confidence (0–100).
    #[arg(long, default_value_t = 0.0)]
    min_conf: f32,

    /// OCR engine: `auto` (Tesseract when installed with the languages, else the built-in
    /// native engine), `native` (built-in, nothing to install) or `tesseract`.
    #[arg(long, default_value = "auto")]
    engine: legatorocr_engine::ocr::Engine,

    /// Path to the tesseract executable.
    #[arg(long, default_value = "tesseract")]
    tesseract: PathBuf,

    /// tessdata directory (sets TESSDATA_PREFIX).
    #[arg(long)]
    tessdata: Option<PathBuf>,

    /// Scan resolution of the inputs. Default: read from each PNG/JPEG header
    /// when it is 150–1200 dpi, otherwise 300.
    #[arg(long, alias = "image-dpi")]
    dpi: Option<u32>,

    #[arg(long, value_enum, default_value = "color")]
    mode: Mode,

    #[arg(long, value_enum, default_value = "auto")]
    split: Split,

    /// Write recognised Romanian text in today's spelling: `î` inside a word becomes `â`
    /// (`pînză` -> `pânză`), `sînt` -> `sunt`. For scans of books printed 1953-1993. Needs --ocr.
    /// Changes the text layer, the hOCR and the text outputs; search the original spelling with
    /// --search-old-orthography instead when you want to keep it.
    #[arg(long)]
    ocr_modern_spelling: bool,

    /// Disable skew correction.
    #[arg(long)]
    no_deskew: bool,

    /// Fixed skew correction in degrees (clockwise positive); overrides detection.
    #[arg(long, allow_negative_numbers = true)]
    skew: Option<f32>,

    /// Maximum skew searched, in degrees.
    #[arg(long, default_value_t = 7.0)]
    max_skew: f32,

    /// Use Tesseract's orientation detection (needs the `osd` language data) to turn
    /// pages upright, including 90° vs 270° and upside-down pages.
    #[arg(long)]
    auto_rotate: bool,

    /// Disable automatic detection of sideways pages.
    #[arg(long)]
    no_auto_orient: bool,

    /// Rotate inputs by 0/90/180/270 degrees clockwise before anything else.
    #[arg(long, default_value_t = 0)]
    rotate: u16,

    /// Margin around the content in mm (all sides).
    #[arg(long, default_value_t = 5.0)]
    margin: f32,

    /// Fixed page size: a4, a5, letter, or WxH in mm (e.g. 148x210).
    #[arg(long)]
    page_size: Option<String>,

    /// With --page-size: scale the content to fill the page width (8 px of paper left each side).
    #[arg(long)]
    fit_width: bool,

    /// With --page-size: scale the content to fill the page height (8 px of paper left each side).
    #[arg(long)]
    fit_height: bool,

    /// Sauvola sensitivity for black & white output.
    #[arg(long, default_value_t = 0.3)]
    sauvola_k: f32,

    /// Binarisation method: sauvola, wolf, niblack, nick, bradley, otsu, auto (per page).
    #[arg(long, default_value = "sauvola")]
    threshold: String,

    /// Invert negative scans (light text on a dark page).
    #[arg(long)]
    invert: bool,

    /// Stretch contrast so paper turns white and ink black.
    #[arg(long)]
    auto_levels: bool,

    /// Unsharp-mask amount, 0–2 (0 = off).
    #[arg(long, default_value_t = 0.0)]
    sharpen: f32,

    /// Erase punch / binder holes near the page edge.
    #[arg(long)]
    punch_holes: bool,

    /// Erase fingers holding the page at the border.
    #[arg(long)]
    fingers: bool,

    /// Erase ruled lines (black & white / mixed / hybrid output).
    #[arg(long)]
    remove_lines: bool,

    /// Paint a rectangle with the paper colour on every page: X,Y,WIDTH,HEIGHT in mm from the
    /// top-left (repeatable). For stamps, notes, scanner marks.
    #[arg(long, value_name = "X,Y,W,H")]
    erase: Vec<String>,

    /// Photographs from a phone have no usable resolution: assume each shot is framed on a page
    /// of this size (`A4`, `A5`, `B5`, `Letter`, … or `WIDTHxHEIGHT` in mm) and derive the dpi
    /// from it. Only used for images without a resolution in their header and without --dpi.
    /// An estimate: it is off by the margin left around the page.
    #[arg(long, value_name = "PAPER")]
    phone_paper: Option<String>,

    /// Collect everything written (PDFs, text, …) plus SHA256SUMS.txt and a MANIFEST.txt into a
    /// new folder, or into one zip (`.zip`) or ISO image (`.iso`, ISO 9660 with Joliet long names;
    /// the file name is the volume label), ready to hand over. Refuses to overwrite. Needs --pdf.
    /// (A zip is limited to 3 GiB of content, an ISO file to 4 GiB each.)
    #[arg(long, value_name = "FOLDER|FILE.zip|FILE.iso")]
    package: Option<PathBuf>,

    /// Encrypt a `.zip` --package with AES-256 (WinZip format, opens in 7-Zip and others). The
    /// password is read from the first line of this file, or from the environment variable
    /// LEGATOROCR_PACKAGE_PASSWORD when the file is `-`; never from the command line, where
    /// other users could see it. File names stay visible; only the contents are encrypted.
    #[arg(long, value_name = "FILE", requires = "package")]
    package_password_file: Option<PathBuf>,

    /// Rights statement written into the PDF (copyright holder, licence, contract reference):
    /// XMP `dc:rights` with --pdfa, otherwise the `/Rights` info key plus a visible note icon on
    /// page 2. Needs --pdf.
    #[arg(long, value_name = "TEXT")]
    rights_notice: Option<String>,

    /// Write a `SHA256SUMS.txt` next to the PDF listing every file written (same format as
    /// `sha256sum`), so recipients can verify the files. Needs --pdf.
    #[arg(long)]
    checksums: bool,

    /// Append one JSON Lines record per file written (time, tool version, inputs, SHA-256, size,
    /// retention date, legal hold) to this log. The log is append-only evidence; it deletes nothing.
    #[arg(long, value_name = "FILE")]
    audit_log: Option<PathBuf>,

    /// Retention date (YYYY-MM-DD) recorded in the audit log. Needs --audit-log.
    #[arg(long, value_name = "DATE")]
    retain_until: Option<String>,

    /// Mark the audit-log records as under legal hold (do not destroy). Needs --audit-log.
    #[arg(long)]
    legal_hold: bool,

    /// Check the files listed in a SHA256SUMS file (paths relative to it) and exit: status 0 when
    /// all match, 1 otherwise.
    #[arg(long, value_name = "SHA256SUMS")]
    verify_sums: Option<PathBuf>,

    /// Also write the recognised text next to each PDF as `<pdf name>.txt` (for full-text
    /// indexers such as DocFetcher or Agent Ransack). Needs --pdf and --ocr.
    #[arg(long)]
    txt_each: bool,

    /// Always keep this rectangle as a colour picture (mixed / hybrid output): X,Y,WIDTH,HEIGHT in
    /// mm from the top-left of the deskewed page, like --erase (repeatable). For photos the
    /// automatic detector misses.
    #[arg(long, value_name = "X,Y,W,H")]
    picture_zone: Vec<String>,

    /// Straighten a bent text line you point out: "X,Y X,Y …" with X and Y as fractions 0–1 of
    /// the deskewed page's width and height (at least two points; repeat for several lines).
    /// Replaces the automatic --dewarp estimate.
    #[arg(long, value_name = "X,Y X,Y …")]
    dewarp_guide: Vec<String>,

    /// Password for encrypted PDF input (user or owner password; also read from the environment
    /// variable LEGATOROCR_PDF_PASSWORD, which keeps it out of the process list). Files that open
    /// with the empty password need none.
    #[arg(long, value_name = "PASSWORD")]
    pdf_password: Option<String>,

    /// Drop one printed colour: off, red, green, blue.
    #[arg(long, default_value = "off")]
    dropout: String,

    /// Fade print showing through thin paper (also levels the lighting).
    #[arg(long)]
    show_through: bool,

    /// Straighten pages photographed at an angle (phone photos on a darker desk).
    #[arg(long)]
    perspective: bool,

    /// Smooth halftone screens of printed photographs.
    #[arg(long)]
    descreen: bool,

    /// Despeckle strength 0.25–4 (1 = default).
    #[arg(long, default_value_t = 1.0)]
    despeckle_strength: f32,

    /// Even out uneven lighting / shadows in colour, gray and hybrid output.
    #[arg(long)]
    normalize: bool,

    /// Straighten curved text lines (page curl).
    #[arg(long)]
    dewarp: bool,

    /// Downsample output to this dpi (never upsamples).
    #[arg(long)]
    output_dpi: Option<u32>,

    /// Picture detection sensitivity for mixed/hybrid (0–1).
    #[arg(long, default_value_t = 0.5)]
    picture_sensitivity: f32,

    /// Resolution of the hybrid background layer.
    #[arg(long, default_value_t = 100)]
    hybrid_dpi: u32,

    /// Where content sits on a fixed-size page: e.g. `center`, `top-left`, `bottom`.
    #[arg(long, default_value = "center")]
    align: String,

    /// Give every page the same size (the largest ordinary page of the book).
    #[arg(long)]
    uniform_size: bool,

    /// Keep tiny specks in black & white output.
    #[arg(long)]
    no_despeckle: bool,

    /// Drop pages where no content was found.
    #[arg(long)]
    skip_blank: bool,

    /// Skip unreadable/corrupt inputs instead of stopping (exit status 2 if any were skipped).
    #[arg(long)]
    keep_going: bool,
}

fn in_range(name: &str, v: f32, lo: f32, hi: f32) -> Result<()> {
    if !v.is_finite() || v < lo || v > hi {
        bail!("--{name} must be between {lo} and {hi}");
    }
    Ok(())
}

/// Output file stems: the input stem, made unique (`001_0003`) when two inputs
/// from different folders share one, so no processed page overwrites another.
fn output_stems(paths: &[PathBuf]) -> Vec<String> {
    let stems: Vec<String> = paths
        .iter()
        .map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page")
                .to_string()
        })
        .collect();
    stems
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if stems.iter().filter(|o| o.eq_ignore_ascii_case(s)).count() > 1 {
                format!("{s}_{:04}", i + 1)
            } else {
                s.clone()
            }
        })
        .collect()
}

/// Canonical, lower-cased paths of the real files behind the inputs. A page of a multi-page
/// file (`book.tif#2`) stands for `book.tif`; the first form never exists on disk.
fn canonical_inputs(inputs: &[PathBuf]) -> Vec<String> {
    inputs
        .iter()
        .filter_map(|p| {
            legatorocr_engine::files::split_frame(p)
                .0
                .canonicalize()
                .ok()
        })
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect()
}

/// Canonical form of a file that may not exist yet: its folder, canonicalised, plus its name.
fn canonical_target(p: &Path) -> Option<String> {
    let parent = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .ok()?;
    Some(parent.join(p.file_name()?).to_string_lossy().to_lowercase())
}

/// The first named output file (`--pdf`, `--txt` …) that is also an input.
fn named_output_clobbers_input(
    outputs: &[&Option<PathBuf>],
    inputs: &[PathBuf],
) -> Option<PathBuf> {
    let canon = canonical_inputs(inputs);
    outputs
        .iter()
        .filter_map(|o| o.as_ref())
        .find(|o| canonical_target(o).is_some_and(|t| canon.contains(&t)))
        .cloned()
}

/// The first page-image path (`stem`, `stem_L`, `stem_R` + extension, in `out_dir`) that is one of
/// the input files themselves, compared by canonical path (and case-insensitively, for the
/// case-folding file systems of Windows and macOS).
fn output_clobbers_input(
    out_dir: &Path,
    stems: &[String],
    ext: &str,
    inputs: &[PathBuf],
) -> Option<PathBuf> {
    let dir = out_dir.canonicalize().ok()?;
    let canon = canonical_inputs(inputs);
    for stem in stems {
        for suffix in ["", "_L", "_R"] {
            let cand = dir.join(format!("{stem}{suffix}.{ext}"));
            if canon.contains(&cand.to_string_lossy().to_lowercase()) {
                return Some(cand);
            }
        }
    }
    None
}

fn parse_page_size(s: &str) -> Result<(f32, f32)> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "a4" => (210.0, 297.0),
        "a5" => (148.0, 210.0),
        "letter" => (215.9, 279.4),
        other => {
            let (w, h) = other
                .split_once('x')
                .context("page size must be a4|a5|letter|WxH")?;
            let (w, h): (f32, f32) = (w.parse()?, h.parse()?);
            in_range("page-size width", w, 10.0, 2000.0)?;
            in_range("page-size height", h, 10.0, 2000.0)?;
            (w, h)
        }
    })
}

fn main() -> Result<()> {
    // `legatorocr … | head` closes our stdout early: end quietly like other Unix tools
    // instead of panicking inside println!.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default_hook(info);
    }));
    // The folder with extracted archives is removed on every way out that unwinds, a panic
    // included (`process::exit` paths call `cleanup` themselves).
    struct Scratch;
    impl Drop for Scratch {
        fn drop(&mut self) {
            legatorocr_engine::archive::cleanup();
        }
    }
    let _scratch = Scratch;
    // The command is one large function with many early-return modes; its stack frame outgrew the
    // 1 MB main-thread stack of Windows (found by CI: "main has overflowed its stack"). Run it on a
    // thread with a generous stack (address space is only reserved, not used).
    let worker = std::thread::Builder::new()
        .name("legatorocr-main".into())
        .stack_size(256 << 20)
        .spawn(real_main)
        .context("cannot start the main thread")?;
    match worker.join() {
        Ok(r) => r,
        Err(p) => std::panic::resume_unwind(p),
    }
}

/// Expand `{date}`, `{year}`, `{month}`, `{day}`, `{name}` in an output path and create the
/// folders of a templated path.
fn expand_path(p: &mut Option<PathBuf>, name: &str) -> Result<()> {
    let Some(path) = p.as_ref() else {
        return Ok(());
    };
    let text = path.to_string_lossy().into_owned();
    if !text.contains(['{', '}']) {
        return Ok(());
    }
    let done = legatorocr_engine::naming::expand(&text, name, legatorocr_engine::now_unix())?;
    let path = PathBuf::from(done);
    if done_has_mark(&path) {
        *p = Some(path);
        return Ok(());
    }
    if let Some(parent) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{}: cannot create the folder", parent.display()))?;
    }
    *p = Some(path);
    Ok(())
}

fn done_has_mark(p: &std::path::Path) -> bool {
    p.to_string_lossy()
        .contains(legatorocr_engine::naming::TITLE_MARK)
}

/// Replace the `{title}` stand-in once OCR text exists, and create the folders.
fn resolve_title(p: &mut Option<PathBuf>, title: &str) -> Result<()> {
    let Some(path) = p.as_ref().filter(|x| done_has_mark(x)) else {
        return Ok(());
    };
    let path = PathBuf::from(
        path.to_string_lossy()
            .replace(legatorocr_engine::naming::TITLE_MARK, title),
    );
    if let Some(parent) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{}: cannot create the folder", parent.display()))?;
    }
    *p = Some(path);
    Ok(())
}

/// `--watch`: run this program once per new file with the same options.
fn watch_loop(cli: &Cli, dir: &std::path::Path) -> Result<()> {
    let pdf = cli.pdf.as_ref().map(|p| p.to_string_lossy().into_owned());
    if !pdf
        .as_deref()
        .is_some_and(|t| t.contains("{name}") || t.contains("{title}"))
    {
        bail!("--watch needs --pdf with {{name}} (or {{title}}) in it, e.g. --pdf \"out/{{name}}.pdf\", so every file gets its own output");
    }
    if !dir.is_dir() {
        bail!("--watch: {} is not a folder", dir.display());
    }
    // the same command line minus the watch options
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    let mut it = std::env::args_os().skip(1);
    while let Some(a) = it.next() {
        let t = a.to_string_lossy().into_owned();
        if t == "--watch" || t == "--watch-interval" {
            it.next();
        } else if t.starts_with("--watch=")
            || t.starts_with("--watch-interval=")
            || t == "--watch-skip-existing"
        {
        } else {
            args.push(a);
        }
    }
    let exe = std::env::current_exe().context("cannot find this program to run it again")?;
    let mut w = legatorocr_engine::watch::Watcher::new(dir, cli.watch_skip_existing);
    eprintln!("watching {} (Ctrl+C to stop)", dir.display());
    loop {
        for f in w.poll() {
            eprintln!("new file: {}", f.display());
            match std::process::Command::new(&exe)
                .args(&args)
                .arg(&f)
                .status()
            {
                Ok(s) if s.success() => eprintln!("done: {}", f.display()),
                Ok(s) => eprintln!("failed ({s}): {}", f.display()),
                Err(e) => eprintln!("cannot run for {}: {e}", f.display()),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(cli.watch_interval.max(1)));
    }
}

fn real_main() -> Result<()> {
    let mut cli = Cli::parse();
    if cli.force_ocr {
        cli.ocr_mode = Some("force".into());
        cli.ocr = true;
    }
    if cli.redo_ocr {
        cli.ocr_mode = Some("redo".into());
        cli.ocr = true;
    }
    if cli.rotate_pages {
        cli.auto_rotate = true;
    }
    if let Some(t) = cli.output_type.as_deref() {
        if t != "pdf" {
            cli.pdfa = true;
            if let Some(n) = t.strip_prefix("pdfa-").and_then(|n| n.parse::<u8>().ok()) {
                cli.pdfa_part = n;
            }
        }
    }
    if let Some(n) = cli.jobs.filter(|&n| n > 0) {
        // the global pool is built once, before any parallel work
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global();
    }
    if cli.tagged_pdf && !cli.ocr {
        bail!("--tagged-pdf tags the recognised text; add --ocr");
    }
    if cli.tagged_pdf && cli.pdf.is_none() {
        bail!("--tagged-pdf needs --pdf");
    }
    if let Ok(mut g) = legatorocr_engine::DEFAULT_TAGGED.lock() {
        *g = if cli.tagged_pdf && cli.tagged.is_none() {
            Some(legatorocr_engine::tagstruct::bcp47(&cli.lang))
        } else {
            cli.tagged.clone()
        };
    }
    legatorocr_engine::DEFAULT_TAGGED_STRUCT
        .store(cli.tagged_pdf, std::sync::atomic::Ordering::Relaxed);
    legatorocr_engine::DEFAULT_PDFA_PART.store(cli.pdfa_part, std::sync::atomic::Ordering::Relaxed);
    if let Some(dir) = cli.watch.clone() {
        return watch_loop(&cli, &dir);
    }
    if let Some(y) = cli.rights_death_year {
        if cli.rights_notice.is_some() {
            bail!("--rights-death-year and --rights-notice both write the rights notice; use one");
        }
        let today = 1970 + (legatorocr_engine::now_unix() / 31_556_952) as i32;
        cli.rights_notice = Some(legatorocr_engine::rights::notice(
            y,
            today,
            cli.rights_author.as_deref(),
            cli.rights_deed_number
                .as_deref()
                .zip(cli.rights_deed_date.as_deref()),
        )?);
    }
    if let Some(pw) = cli
        .pdf_password
        .clone()
        .or_else(|| std::env::var("LEGATOROCR_PDF_PASSWORD").ok())
    {
        legatorocr_engine::set_pdf_password(&pw);
    }
    {
        let first = cli.inputs.first().cloned();
        let name = first
            .as_deref()
            .and_then(|f| f.file_stem().or_else(|| f.file_name()))
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "scan".into());
        for p in [
            &mut cli.pdf,
            &mut cli.txt,
            &mut cli.hocr,
            &mut cli.alto,
            &mut cli.page_xml,
            &mut cli.json,
            &mut cli.docx,
            &mut cli.tei,
            &mut cli.package,
        ] {
            expand_path(p, &name)?;
        }
        if let Some(stem) = cli
            .pdf
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
        {
            if legatorocr_engine::naming::is_generic(&stem) {
                eprintln!("note: {stem:?} says nothing about the content; a name such as \"{{date}}_{{name}}.pdf\" or \"{{year}}/{{month}}/contract_ACME.pdf\" keeps an archive in order");
            }
        }
    }
    if cli.list_profiles {
        for (n, d) in legatorocr_engine::profiles::list() {
            println!("{n:<20} {d}");
        }
        return Ok(());
    }
    if let Some(sums) = &cli.verify_sums {
        let v = legatorocr_engine::checksums::verify(sums)?;
        for n in &v.mismatched {
            println!("MISMATCH  {n}");
        }
        for n in &v.missing {
            println!("MISSING   {n}");
        }
        println!(
            "{} ok, {} mismatched, {} missing",
            v.ok,
            v.mismatched.len(),
            v.missing.len()
        );
        std::process::exit(if v.all_good() { 0 } else { 1 });
    }
    if cli.audit_log.is_none() && (cli.retain_until.is_some() || cli.legal_hold) {
        bail!("--retain-until and --legal-hold need --audit-log");
    }
    if let Some(d) = &cli.retain_until {
        legatorocr_engine::audit::check_date(d)?;
    }
    if (cli.checksums
        || cli.txt_each
        || cli.rights_notice.is_some()
        || cli.bagit.is_some()
        || cli.ocfl.is_some()
        || cli.package.is_some()
        || cli.catalog.is_some()
        || cli.dublin_core)
        && cli.pdf.is_none()
    {
        bail!("--checksums, --txt-each, --rights-notice, --bagit, --ocfl, --package, --catalog and --dublin-core need --pdf");
    }
    if !cli.ocr
        && [
            &cli.pdf,
            &cli.txt,
            &cli.hocr,
            &cli.alto,
            &cli.page_xml,
            &cli.json,
            &cli.docx,
            &cli.package,
        ]
        .iter()
        .any(|p| p.as_deref().is_some_and(done_has_mark))
    {
        bail!("{{title}} in an output name is taken from the recognised text; add --ocr");
    }
    if (cli.tables_xlsx.is_some() || cli.tables_html.is_some()) && !cli.ocr {
        bail!("--tables-xlsx and --tables-html read the recognised text; add --ocr");
    }
    if cli.txt_each && !cli.ocr {
        bail!("--txt-each needs --ocr");
    }
    if cli.compare_text {
        let [r, h] = cli.inputs.as_slice() else {
            bail!("--compare-text needs exactly two files: REFERENCE and OCR-OUTPUT");
        };
        let rd = |p: &PathBuf| {
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
        };
        let x = legatorocr_engine::eval::rates(&rd(r)?, &rd(h)?);
        println!(
            "CER {:.2}%  WER {:.2}%  (reference: {} characters, {} words)",
            x.cer * 100.0,
            x.wer * 100.0,
            x.ref_chars,
            x.ref_words
        );
        return Ok(());
    }
    if cli.scan_active {
        for f in &cli.inputs {
            let Ok(data) = std::fs::read(f) else {
                println!("{}: cannot read", f.display());
                continue;
            };
            if !data.starts_with(b"%PDF") {
                continue;
            }
            match legatorocr_engine::search::pdf_active_content(data) {
                Ok(hits) => {
                    if hits.is_empty() {
                        println!("{}: nothing active found", f.display());
                    } else {
                        let list: Vec<String> =
                            hits.iter().map(|(k, n)| format!("{k} x{n}")).collect();
                        println!("{}: {}", f.display(), list.join(", "));
                    }
                }
                Err(e) => println!("{}: could not be read ({e})", f.display()),
            }
        }
        return Ok(());
    }
    if let Some(line) = &cli.merkle_verify {
        let f: Vec<&str> = line.split_whitespace().collect();
        let ok = f.len() >= 4
            && match (f[2].parse::<usize>(), f[3].parse::<usize>()) {
                (Ok(m), Ok(n)) => {
                    let path: Vec<String> = f
                        .get(4)
                        .map(|p| p.split(',').map(str::to_string).collect())
                        .unwrap_or_default();
                    legatorocr_engine::merkle::check(
                        f[0],
                        f[1],
                        m,
                        n,
                        &path,
                        cli.merkle_expect.as_deref().unwrap_or(""),
                    )
                }
                _ => false,
            };
        println!(
            "{}",
            if ok {
                "proof is valid: the file belongs to the collection with that root"
            } else {
                "proof does NOT lead to that root"
            }
        );
        std::process::exit(if ok { 0 } else { 1 });
    }
    if let Some(sums) = &cli.merkle_root {
        use legatorocr_engine::merkle as mk;
        let text = std::fs::read_to_string(sums)
            .with_context(|| format!("{}: cannot read", sums.display()))?;
        let entries = mk::parse_sums(&text)?;
        let leaves = mk::leaves_of(&entries);
        let r = mk::hex(&mk::root(&leaves));
        println!("merkle root of {} file(s): {r}", entries.len());
        if let Some(name) = &cli.merkle_proof {
            let Some(i) = entries.iter().position(|(n, _)| n == name) else {
                bail!("{name} is not listed in {}", sums.display());
            };
            let path: Vec<String> = mk::proof(&leaves, i).iter().map(|h| mk::hex(h)).collect();
            println!(
                "proof: {} {} {} {} {}",
                entries[i].0,
                entries[i].1,
                i,
                entries.len(),
                path.join(",")
            );
        }
        if let Some(e) = &cli.merkle_expect {
            if !e.eq_ignore_ascii_case(&r) {
                println!("root does NOT match the expected value");
                std::process::exit(1);
            }
            println!("root matches the expected value");
        }
        return Ok(());
    }
    let plan = legatorocr_engine::pdftools::Plan {
        merge: cli.merge_pdf,
        pages: cli.extract_pages.clone(),
        split_every: cli.split_pdf_every,
        watermark: cli.watermark.clone(),
        number_format: cli.number_pages.clone(),
    };
    if plan.active() {
        let written =
            legatorocr_engine::pdftools::run(&cli.inputs, &cli.out_dir, cli.pdf.as_deref(), &plan)?;
        for w in &written {
            println!("wrote {}", w.display());
        }
        return Ok(());
    }
    if let Some(f) = &cli.timestamp {
        let r = legatorocr_engine::tsa::stamp(f, cli.tsa_url.as_deref().unwrap_or(""))?;
        println!(
            "{}: stamped {} UTC, reply saved as {}",
            f.display(),
            r.gen_time,
            legatorocr_engine::tsa::tsr_path(f).display()
        );
        return Ok(());
    }
    if let Some(f) = &cli.timestamp_verify {
        match legatorocr_engine::tsa::verify(f, cli.tsa_ca.as_deref()) {
            Ok(r) => println!(
                "{}: matches its time stamp of {}{}",
                f.display(),
                r.gen_time,
                if cli.tsa_ca.is_some() {
                    " (signature verified by openssl)"
                } else {
                    " (signature NOT checked: give --tsa-ca)"
                }
            ),
            Err(e) => {
                println!("{}: {e}", f.display());
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let [i, o] = cli.decode_worker.as_slice() {
        if let Err(e) = legatorocr_engine::isolate::worker(i, o) {
            eprintln!("{e}");
            std::process::exit(2);
        }
        return Ok(());
    }
    if let Some(p) = &cli.plugin {
        legatorocr_engine::plugin::set(Some(p));
    }
    if cli.isolate_decoders {
        legatorocr_engine::isolate::enable(legatorocr_engine::isolate::Config {
            exe: std::env::current_exe()?,
            memory_mb: cli.decoder_memory,
            timeout_s: cli.decoder_timeout,
        });
    }
    if let Some(out) = &cli.integrity_hints {
        let signals = legatorocr_engine::integrity::parse_signals(&cli.integrity_signals)
            .map_err(|e| anyhow::anyhow!(e))?;
        let ocr_o = OcrOptions {
            engine: cli.engine,
            lang: legatorocr_engine::ocr::canonical_lang(&cli.lang),
            psm: cli.psm,
            min_conf: cli.min_conf,
            tesseract: cli.tesseract.clone(),
            tessdata: cli.tessdata.clone(),
            modern_spelling: cli.ocr_modern_spelling,
        };
        let want_words = cli.ocr && signals.contains(&"line_size");
        if want_words {
            check_ocr(&ocr_o)?;
        }
        let mut pages = Vec::new();
        for p in &cli.inputs {
            let img = legatorocr_engine::load_image(p)?;
            let g = img.to_luma8();
            let bytes = std::fs::read(p).ok();
            let words = if want_words {
                let pg = legatorocr_core::PageImage::Gray(g.clone());
                legatorocr_engine::ocr::recognize(&pg, cli.dpi.unwrap_or(300), &ocr_o)
                    .map_err(|e| anyhow::anyhow!("OCR failed on {}: {e}", p.display()))?
            } else {
                Vec::new()
            };
            let rep = legatorocr_engine::integrity::analyse(
                &g,
                bytes.as_deref(),
                &words,
                &signals,
                &p.display().to_string(),
            );
            eprintln!("{}: {} region(s) to check", p.display(), rep.hints.len());
            pages.push(rep);
        }
        write_atomic(
            out,
            legatorocr_engine::integrity::to_json(&pages, &signals).as_bytes(),
        )?;
        eprintln!(
            "wrote {} (indicators for a person to check; not a verdict, high false-alarm risk)",
            out.display()
        );
        return Ok(());
    }
    if cli.verify_document {
        for p in &cli.inputs {
            let img = legatorocr_engine::load_image(p)?;
            let g = img.to_luma8();
            println!("{}:", p.display());
            let qr = legatorocr_engine::verifier::qr_read(&g);
            if qr.is_empty() {
                println!("  QR: none read");
            }
            for (q, _) in &qr {
                println!("  QR: {q}");
            }
            // a QR code repeats its three finder squares by design: not a finding
            let cm: Vec<_> = legatorocr_engine::verifier::copy_move(&g, 40, 48)
                .into_iter()
                .filter(|c| {
                    let (cx, cy) = (
                        (c.bbox.0 + c.bbox.2) as i32 / 2,
                        (c.bbox.1 + c.bbox.3) as i32 / 2,
                    );
                    !qr.iter().any(|(_, b)| {
                        cx >= b.0 - 8 && cx <= b.2 + 8 && cy >= b.1 - 8 && cy <= b.3 + 8
                    })
                })
                .collect();
            if cm.is_empty() {
                println!("  copy-move: nothing found");
            }
            if let Some(dq) = std::fs::read(p)
                .ok()
                .and_then(|b| legatorocr_engine::verifier::jpeg_double_quant(&b))
            {
                if dq.likely_double {
                    println!("  double-JPEG: the coefficient histograms show a comb pattern (score {:.1}); this JPEG was probably saved twice  [hint: re-saving is common and harmless; a pasted-in part from another file would need more evidence]", dq.score);
                } else {
                    println!("  double-JPEG: no sign of a second save (score {:.1}; a small or smooth picture, or a second save at a similar or higher quality, hides it)", dq.score);
                }
            }
            let marks = legatorocr_engine::verifier::round_marks(&g, cli.dpi.unwrap_or(300), 0.85);
            if marks.is_empty() {
                println!("  round marks (stamps, seals): none found");
            }
            for m in &marks {
                println!("  round mark: centre ({}, {}), radius {} px, {:.0} % of the outline is ink  [a round stamp or seal, or a large letter O or logo: look at it]", m.cx, m.cy, m.radius, m.coverage * 100.0);
            }
            for c in &cm {
                println!("  copy-move: {} blocks repeat at offset ({}, {}), source area {:?}  [check by eye: repeated letters or stamps are common]", c.blocks, c.dx, c.dy, c.bbox);
            }
        }
        return Ok(());
    }
    if let Some(out) = &cli.fuse_frames {
        if cli.inputs.len() < 2 {
            bail!("--fuse-frames needs two or more photos of the same page");
        }
        let frames: Vec<image::RgbImage> = cli
            .inputs
            .iter()
            .map(|p| legatorocr_engine::load_image(p).map(|i| i.to_rgb8()))
            .collect::<Result<_>>()?;
        let r = legatorocr_core::fusion::fuse(&frames, cli.fuse_max_shift, 2500.0)
            .context("no frames")?;
        for (p, s) in cli.inputs.iter().zip(&r.shifts) {
            match s {
                Some((x, y)) => eprintln!("{}: aligned (shift {x},{y})", p.display()),
                None => eprintln!("{}: left out (did not match the first frame)", p.display()),
            }
        }
        r.image
            .save(out)
            .with_context(|| format!("writing {}", out.display()))?;
        println!(
            "wrote {} from {} of {} frames",
            out.display(),
            r.shifts.iter().flatten().count(),
            r.shifts.len()
        );
        return Ok(());
    }
    if let Some(f) = &cli.sign_pdf {
        let out = match &cli.pdf {
            Some(n) => n.clone(),
            None => f.with_extension("signed.pdf"),
        };
        legatorocr_engine::sign::sign_pdf(
            f,
            &out,
            cli.sign_p12.as_deref().unwrap(),
            &cli.sign_pass_env,
            cli.sign_name.as_deref(),
            cli.sign_reason.as_deref(),
            cli.tsa_url.as_deref().filter(|_| cli.sign_timestamp),
        )?;
        println!("wrote {}", out.display());
        return Ok(());
    }
    if let Some(f) = &cli.verify_pdf_signatures {
        let rs =
            legatorocr_engine::sign::verify_pdf(f, cli.sign_ca.as_deref(), cli.tsa_ca.as_deref())?;
        if rs.is_empty() {
            println!("{}: no signature", f.display());
            std::process::exit(1);
        }
        let mut bad = false;
        for r in &rs {
            let note = if r.covers_whole_file {
                ""
            } else {
                " (content was added after this signature)"
            };
            match &r.result {
                Ok(()) => println!(
                    "signature {} [{}]: data matches{}{}",
                    r.index,
                    r.sub_filter,
                    if cli.sign_ca.is_some() {
                        ", chain verified"
                    } else {
                        ", chain NOT checked: give --sign-ca"
                    },
                    note
                ),
                Err(e) => {
                    bad = true;
                    println!("signature {} [{}]: FAILED: {e}", r.index, r.sub_filter);
                }
            }
            match &r.timestamp {
                None => println!("  time stamp inside: none"),
                Some(Ok(t)) => println!(
                    "  time stamp inside: {t} UTC, for this signature{}",
                    if cli.tsa_ca.is_some() {
                        " (authority verified)"
                    } else {
                        " (authority NOT checked: give --tsa-ca)"
                    }
                ),
                Some(Err(e)) => {
                    bad = true;
                    println!("  time stamp inside: FAILED: {e}");
                }
            }
        }
        if bad {
            std::process::exit(1);
        }
        return Ok(());
    }
    if cli.mcp {
        let exe = std::env::current_exe()?;
        legatorocr_engine::mcp::serve(&exe, std::io::stdin().lock(), std::io::stdout().lock())?;
        return Ok(());
    }
    if let Some(f) = &cli.measure_edge {
        let img = image::open(f)
            .with_context(|| format!("reading {}", f.display()))?
            .to_luma8();
        let roi = match &cli.edge_roi {
            Some(r) => {
                let v: Vec<u32> = r
                    .split(',')
                    .map(|t| t.trim().parse())
                    .collect::<Result<_, _>>()
                    .context("--edge-roi: expected X,Y,W,H")?;
                if v.len() != 4 {
                    bail!("--edge-roi: expected X,Y,W,H");
                }
                (v[0], v[1], v[2], v[3])
            }
            None => (0, 0, img.width(), img.height()),
        };
        let Some(r) = legatorocr_core::sfr::measure(&img, roi) else {
            bail!("no clear straight edge in that region (needs a near-vertical edge with strong contrast)");
        };
        println!("edge angle {:.2} deg", r.angle_deg);
        println!("MTF50 {:.3} cycles/pixel", r.mtf50);
        println!("MTF10 {:.3} cycles/pixel", r.mtf10);
        if let Some(d) = cli.dpi {
            println!("MTF50 {:.1} lp/mm at {d} dpi", r.mtf50 * d as f64 / 25.4);
        }
        return Ok(());
    }
    if let Some(src) = &cli.backup_from {
        let reports = legatorocr_engine::backup::run(src, &cli.backup_to, cli.backup_read_only)?;
        let mut bad = false;
        for r in &reports {
            println!(
                "{}: {} files, {} bytes verified",
                r.dest.display(),
                r.files,
                r.bytes
            );
            for p in &r.problems {
                bad = true;
                println!("  PROBLEM: {p}");
            }
        }
        print!("{}", legatorocr_engine::backup::checklist(reports.len()));
        if bad {
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(f) = &cli.efactura {
        let xml = legatorocr_engine::efactura::read_xml(f)?;
        let inv = legatorocr_engine::efactura::parse(&xml)?;
        let problems = legatorocr_engine::efactura::check(&inv);
        print!("{}", legatorocr_engine::efactura::render(&inv, &problems));
        if !problems.is_empty() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(obj) = &cli.verify_ocfl {
        let problems = legatorocr_engine::ocfl::verify(obj)?;
        if problems.is_empty() {
            println!("{}: OCFL object is intact", obj.display());
            return Ok(());
        }
        for p in &problems {
            println!("{}: {p}", obj.display());
        }
        std::process::exit(1);
    }
    if !cli.extract_project.is_empty() {
        let (pdf, dir) = (&cli.extract_project[0], &cli.extract_project[1]);
        let (proj, n) = legatorocr_engine::embedproject::extract(pdf, dir)?;
        println!(
            "{n} files written to {}; run again with: legatorocr --project {} --pdf NEW.pdf",
            dir.display(),
            proj.display()
        );
        return Ok(());
    }
    if let Some(bag) = &cli.verify_bag {
        let problems = legatorocr_engine::bagit::verify(bag)?;
        if problems.is_empty() {
            println!("{}: bag is intact", bag.display());
            return Ok(());
        }
        for p in &problems {
            println!("{}: {p}", bag.display());
        }
        std::process::exit(1);
    }
    if cli.doctor {
        let checks = legatorocr_engine::doctor::run(&cli.tesseract);
        print!("{}", legatorocr_engine::doctor::render(&checks));
        return Ok(());
    }
    if cli.ocr_languages {
        let o = OcrOptions {
            lang: "eng".into(),
            ..OcrOptions::default()
        };
        let v = check_ocr(&o).context("no OCR engine is available")?;
        let have = legatorocr_engine::ocr::list_languages(&o)?;
        println!("{v}");
        for (iso, tess, name) in legatorocr_engine::ocr::UI_LANGUAGES {
            if have.iter().any(|h| h == tess) {
                println!("{iso}  {tess}  {name:<11} installed");
            } else {
                println!(
                    "{iso}  {tess}  {name:<11} MISSING — {}",
                    legatorocr_engine::ocr::install_hint(tess)
                );
            }
        }
        return Ok(());
    }
    if (cli.search_regex || cli.search_exact || cli.search_max > 0)
        && cli.search.is_none()
        && cli.search_preset.is_none()
    {
        bail!(
            "--search-regex, --search-exact and --search-max go with --search or --search-preset"
        );
    }
    if cli.search_help {
        for (name, what, pat) in legatorocr_engine::search::PRESETS {
            println!("{name:<9} {what}\n          {pat}");
        }
        println!(
            "\nIn --search text: ? = one letter, * = any letters, `A NEAR/5 B` = within 5 words."
        );
        return Ok(());
    }
    if let Some(name) = &cli.search_preset {
        return run_search(&cli, legatorocr_engine::search::preset(name)?);
    }
    if let Some(q) = &cli.search {
        return run_search(&cli, q);
    }
    if let Some(text) = &cli.make_separator {
        let Some(bc) = legatorocr_core::barcode::render(text, 4, 500) else {
            bail!("--make-separator: use 1–80 printable ASCII characters");
        };
        if bc.width() > 2480 {
            bail!(
                "--make-separator: this text needs a {}-px wide code, more than the 2480-px page; use at most about 45 characters",
                bc.width()
            );
        }
        let mut sheet = image::GrayImage::from_pixel(2480, 3508, image::Luma([255]));
        let (ox, oy) = (
            2480u32.saturating_sub(bc.width()) / 2,
            (3508 - bc.height()) / 2,
        );
        image::imageops::replace(&mut sheet, &bc, ox as i64, oy as i64);
        let safe: String = text
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(40)
            .collect();
        let out = std::path::PathBuf::from(format!("separator_{safe}.png"));
        sheet.save(&out)?;
        eprintln!("wrote {} (A4, 300 dpi: print at 100 %)", out.display());
        return Ok(());
    }
    if !matches!(cli.rotate, 0 | 90 | 180 | 270) {
        bail!("--rotate must be 0, 90, 180 or 270");
    }
    if cli.dpi.is_some_and(|d| !(36..=2400).contains(&d)) {
        bail!("--dpi must be between 36 and 2400");
    }
    if cli.pdf_quality > 100 {
        bail!("--pdf-quality must be 0..=100");
    }
    if !(1..=13).contains(&cli.psm) {
        bail!("--psm must be 1..=13 (0 is orientation-only and yields no text)");
    }
    in_range("margin", cli.margin, 0.0, 200.0)?;
    in_range("max-skew", cli.max_skew, 0.0, 15.0)?;
    in_range("sauvola-k", cli.sauvola_k, 0.0, 1.0)?;
    in_range("min-conf", cli.min_conf, 0.0, 100.0)?;
    if let Some(s) = cli.skew {
        in_range("skew", s, -45.0, 45.0)?;
    }
    let threshold = Threshold::from_name(&cli.threshold).with_context(|| {
        format!(
            "unknown --threshold '{}' (sauvola, wolf, niblack, nick, bradley, otsu, auto)",
            cli.threshold
        )
    })?;
    in_range("picture-sensitivity", cli.picture_sensitivity, 0.0, 1.0)?;
    in_range("sharpen", cli.sharpen, 0.0, 2.0)?;
    in_range("despeckle-strength", cli.despeckle_strength, 0.25, 4.0)?;
    if let Some(d) = cli.output_dpi {
        if !(36..=1200).contains(&d) {
            bail!("--output-dpi must be between 36 and 1200 (got {d})");
        }
    }
    if !(36..=300).contains(&cli.hybrid_dpi) {
        bail!(
            "--hybrid-dpi must be between 36 and 300 (got {})",
            cli.hybrid_dpi
        );
    }
    if !(1..=100).contains(&cli.jpeg_quality) {
        bail!(
            "--jpeg-quality must be between 1 and 100 (got {})",
            cli.jpeg_quality
        );
    }
    let align = parse_align(&cli.align)?;
    let settings = Settings {
        dpi: cli.dpi.unwrap_or(300),
        rotate_quarters: (cli.rotate / 90) as u8,
        auto_orient: !cli.no_auto_orient,
        split: match cli.split {
            Split::Auto => SplitMode::Auto,
            Split::Off => SplitMode::Off,
            Split::Middle => SplitMode::Middle,
        },
        deskew: !cli.no_deskew,
        manual_skew_deg: cli.skew,
        max_skew_deg: cli.max_skew,
        margins_mm: [cli.margin; 4],
        page_size_mm: cli.page_size.as_deref().map(parse_page_size).transpose()?,
        fit_width: cli.fit_width,
        fit_height: cli.fit_height,
        mode: match cli.mode {
            Mode::Color => OutputMode::Color,
            Mode::Gray => OutputMode::Gray,
            Mode::Bw => OutputMode::BlackWhite,
            Mode::Mixed => OutputMode::Mixed,
            Mode::Hybrid => OutputMode::Hybrid,
        },
        sauvola_k: cli.sauvola_k,
        despeckle: !cli.no_despeckle,
        dewarp: cli.dewarp,
        content_adjust_mm: [0.0; 4],
        align,
        threshold,
        normalize_bg: cli.normalize,
        output_dpi: cli.output_dpi,
        picture_sensitivity: cli.picture_sensitivity,
        hybrid_bg_dpi: cli.hybrid_dpi,
        auto_invert: cli.invert,
        auto_levels: cli.auto_levels,
        sharpen: cli.sharpen,
        punch_holes: cli.punch_holes,
        fingers: cli.fingers,
        remove_lines: cli.remove_lines,
        sheet_erase_zones: Vec::new(),
        sheet_dewarp_guides: Vec::new(),
        sheet_picture_zones: Vec::new(),
        dewarp_guides: legatorocr_core::dewarp::parse_guides(&cli.dewarp_guide.join(";"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--dewarp-guide needs at least two X,Y points (fractions 0-1) separated by spaces"
                )
            })?,
        erase_zones: legatorocr_core::cleanup::parse_zones(&cli.erase.join(";")).ok_or_else(
            || anyhow::anyhow!("--erase needs X,Y,WIDTH,HEIGHT in mm (width and height > 0)"),
        )?,
        picture_zones: legatorocr_core::cleanup::parse_zones(&cli.picture_zone.join(";"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--picture-zone needs X,Y,WIDTH,HEIGHT in mm (width and height > 0)"
                )
            })?,
        dropout: legatorocr_core::Dropout::from_name(&cli.dropout)
            .ok_or_else(|| anyhow::anyhow!("--dropout must be off, red, green or blue"))?,
        despeckle_strength: cli.despeckle_strength,
        show_through: cli.show_through,
        descreen: cli.descreen,
        perspective: cli.perspective,
    };

    let project = cli.project.as_deref().map(Project::load).transpose()?;
    let profile = cli
        .profile
        .as_deref()
        .map(legatorocr_engine::profiles::load)
        .transpose()?;
    if project.is_some() || profile.is_some() {
        // the loaded settings replace the processing options: say so for those given explicitly
        const PROCESSING: &[&str] = &[
            "--mode",
            "--dpi",
            "--margin",
            "--page-size",
            "--split",
            "--no-deskew",
            "--skew",
            "--max-skew",
            "--threshold",
            "--sauvola-k",
            "--sharpen",
            "--despeckle-strength",
            "--output-dpi",
            "--hybrid-dpi",
            "--dropout",
            "--descreen",
            "--perspective",
            "--dewarp",
            "--fingers",
            "--punch-holes",
            "--remove-lines",
            "--erase",
            "--picture-zone",
        ];
        let given: Vec<String> = std::env::args()
            .skip(1)
            .filter_map(|a| {
                let name = a.split('=').next().unwrap_or("").to_string();
                PROCESSING.contains(&name.as_str()).then_some(name)
            })
            .collect();
        if !given.is_empty() {
            eprintln!(
                "warning: --{} settings replace the processing options given on the command line ({}); change them in the {} instead",
                if project.is_some() { "project" } else { "profile" },
                given.join(", "),
                if project.is_some() { "project file" } else { "profile" }
            );
        }
    }
    if cli.skew.is_some() && cli.no_deskew {
        eprintln!("warning: --skew sets the angle by hand and is applied even though --no-deskew is given");
    }
    let settings = match (&project, &profile) {
        (Some(p), _) => p.settings.clone(),
        (None, Some(p)) => p.settings.clone(),
        _ => settings,
    };
    if let Some(name) = &cli.save_profile {
        let p = Project {
            settings: settings.clone(),
            extra: vec![("pdf_quality".into(), cli.pdf_quality.to_string())],
            sheets: Vec::new(),
        };
        let f = legatorocr_engine::profiles::save_user(name, &p)?;
        eprintln!("profile '{name}' saved: {}", f.display());
        if cli.inputs.is_empty() && project.is_none() && cli.pdf.is_none() {
            return Ok(());
        }
    }

    let ocr_opts = OcrOptions {
        engine: cli.engine,
        lang: legatorocr_engine::ocr::canonical_lang(&cli.lang),
        psm: cli.psm,
        min_conf: cli.min_conf,
        tesseract: cli.tesseract.clone(),
        tessdata: cli.tessdata.clone(),
        modern_spelling: cli.ocr_modern_spelling,
    };
    if let Some(d) = cli.dpi.filter(|_| cli.ocr) {
        if d < 200 {
            eprintln!("note: {d} dpi is low for OCR; letters are often misread below 200 dpi (pages are enlarged for OCR, but a rescan at 300 dpi is better)");
        } else if d > 400 {
            eprintln!("note: above 400 dpi plain text gets much slower to recognise without becoming more accurate; 300 dpi is enough for most print");
        }
    }
    if cli.ocr {
        let v = check_ocr(&ocr_opts)?;
        eprintln!("OCR engine: {v}, language: {}", cli.lang);
    }
    if cli.auto_rotate {
        let o = OcrOptions {
            lang: "osd".into(),
            ..ocr_opts.clone()
        };
        check_ocr(&o).context("--auto-rotate needs Tesseract with the 'osd' language data")?;
    }
    if let Some(d) = &cli.archive_package {
        if !cli.ocr {
            bail!("--archive-package needs --ocr (it keeps the recognised words)");
        }
        legatorocr_engine::reprocess::prepare(d)?;
        if cli.json.is_none() {
            cli.json = Some(d.join("derived").join("ocr.json"));
        }
    }
    if (cli.txt.is_some()
        || cli.hocr.is_some()
        || cli.alto.is_some()
        || cli.page_xml.is_some()
        || cli.docx.is_some()
        || cli.tei.is_some()
        || cli.json.is_some()
        || cli.passages_jsonl.is_some()
        || cli.fff_out.is_some())
        && !cli.ocr
    {
        bail!("--txt, --hocr, --alto, --page-xml, --json, --docx, --tei, --passages-jsonl and --fff-out need --ocr");
    }
    for p in cli
        .txt
        .iter()
        .chain(cli.hocr.iter())
        .chain(cli.alto.iter())
        .chain(cli.json.iter())
        .chain(cli.docx.iter())
        .chain(cli.tei.iter())
        .chain(cli.passages_jsonl.iter())
    {
        legatorocr_engine::check_writable(p)?;
    }
    if cli.pdf.is_none()
        && cli.djvu.is_none()
        && matches!(cli.format, Fmt::None)
        && cli.txt.is_none()
        && cli.hocr.is_none()
        && cli.alto.is_none()
        && cli.page_xml.is_none()
        && cli.json.is_none()
        && cli.fff_out.is_none()
        && cli.docx.is_none()
        && cli.tei.is_none()
        && cli.passages_jsonl.is_none()
        && cli.report.is_none()
        && cli.multitiff.is_none()
        && !cli.report_outliers
    {
        bail!("nothing to do: give --pdf and/or an image --format");
    }

    // --- run: process → (save images) → OCR → PDF ---------------------------
    // Fail now, not after an hour of OCR: make sure the outputs can be written.
    if let Some(pdf) = cli.pdf.as_ref().filter(|p| !done_has_mark(p)) {
        legatorocr_engine::check_writable(pdf)?;
    }
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut adjusts: Vec<SheetAdj> = Vec::new();
    if let Some(pr) = &project {
        for (p, a) in &pr.sheets {
            inputs.push(p.clone());
            adjusts.push(a.clone());
        }
    }
    for p in &cli.inputs {
        if p.is_dir() {
            let found = collect_images(std::slice::from_ref(p));
            if found.is_empty() {
                bail!("no images (png, jpg, tif, bmp) in folder {}", p.display());
            }
            inputs.extend(found);
        } else {
            // multi-page TIFFs become one sheet per page; anything else is passed on as is
            let pages = collect_images(std::slice::from_ref(p));
            if pages.is_empty() {
                inputs.push(p.clone());
            } else {
                inputs.extend(pages);
            }
        }
    }
    adjusts.resize(inputs.len(), SheetAdj::default());
    if cli.burst {
        if project.is_some() {
            bail!("--burst cannot be combined with --project (its per-page adjustments follow the old order)");
        }
        let imgs: Vec<PathBuf> = inputs
            .iter()
            .filter(|p| {
                !legatorocr_engine::files::is_pdf(&legatorocr_engine::files::split_frame(p).0)
            })
            .cloned()
            .collect();
        if imgs.len() == inputs.len() && !imgs.is_empty() {
            let r = legatorocr_engine::burst::ingest(&imgs, cli.burst_gap);
            if r.reordered {
                eprintln!("--burst: pages put in capture-time order (EXIF)");
            }
            for (d, k) in &r.dropped {
                eprintln!(
                    "--burst: left out {} (weaker copy of {})",
                    d.display(),
                    k.display()
                );
            }
            if r.dropped.is_empty() {
                eprintln!("--burst: no repeated frames found");
            }
            inputs = r.paths;
            adjusts = Vec::new();
        } else {
            eprintln!("--burst: only applies to plain image inputs; skipped");
        }
    }
    adjusts.resize(inputs.len(), SheetAdj::default());
    if cli.skip_text || cli.ocr_mode.as_deref() == Some("skip") {
        // inputs are PDF pages as `file.pdf#N`; decide once per file
        let mut seen: std::collections::HashMap<PathBuf, bool> = std::collections::HashMap::new();
        let mut keep = Vec::with_capacity(inputs.len());
        for p in &inputs {
            let base = legatorocr_engine::files::split_frame(p).0;
            let has = legatorocr_engine::files::is_pdf(&base)
                && *seen
                    .entry(base.clone())
                    .or_insert_with(|| legatorocr_engine::search::pdf_has_text(&base));
            keep.push(!has);
        }
        let skipped = keep.iter().filter(|k| !**k).count();
        if skipped > 0 {
            let files = seen.values().filter(|h| **h).count();
            eprintln!(
                "--skip-text: left out {skipped} page(s) of {files} PDF(s) that already have text"
            );
            let mut it = keep.iter();
            inputs.retain(|_| *it.next().unwrap());
            let mut it = keep.iter();
            adjusts.retain(|_| *it.next().unwrap());
            if inputs.is_empty() {
                eprintln!("nothing to do: every PDF already has text");
                return Ok(());
            }
        }
    }
    if let Some(spec) = &cli.shard {
        let (i, n) = spec
            .split_once('/')
            .and_then(|(a, b)| {
                Some((
                    a.trim().parse::<usize>().ok()?,
                    b.trim().parse::<usize>().ok()?,
                ))
            })
            .filter(|&(i, n)| n >= 1 && (1..=n).contains(&i))
            .context("--shard needs I/N with 1 <= I <= N, e.g. 2/4")?;
        let total = inputs.len();
        let (from, to) = ((i - 1) * total / n, i * total / n);
        eprintln!("shard {i}/{n}: pages {}..{} of {total}", from + 1, to);
        inputs = inputs[from..to].to_vec();
        adjusts = adjusts[from..to].to_vec();
    }
    if inputs.is_empty() {
        bail!("the project lists no pages");
    }
    let mut archive_project: Option<Project> = None;
    if let Some(path) = cli.save_project.as_ref().or(cli.archive_package.as_ref()) {
        let mut pr = Project {
            settings: settings.clone(),
            extra: Vec::new(),
            sheets: Vec::new(),
        };
        pr.extra
            .push(("pdf_quality".into(), cli.pdf_quality.to_string()));
        pr.extra
            .push(("skip_blank".into(), cli.skip_blank.to_string()));
        pr.extra
            .push(("uniform_size".into(), cli.uniform_size.to_string()));
        for (p, a) in inputs.iter().zip(&adjusts) {
            pr.sheets.push((
                std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()),
                a.clone(),
            ));
        }
        if cli.save_project.is_some() {
            pr.save(path)?;
            eprintln!("project saved: {}", path.display());
        }
        if cli.archive_package.is_some() {
            archive_project = Some(pr);
        }
    }
    let stems = output_stems(&inputs);
    let from_project = project.is_some();
    let phone_paper = match &cli.phone_paper {
        Some(n) => Some(legatorocr_engine::files::paper_mm(n).ok_or_else(|| {
            anyhow::anyhow!("--phone-paper: unknown paper '{n}' (A3, A4, A5, A6, B5, Letter, Legal or WIDTHxHEIGHT in mm)")
        })?),
        None => None,
    };
    let sheets: Vec<SheetInput> = inputs
        .iter()
        .zip(&adjusts)
        .map(|(p, a)| {
            // a project fixes the resolution; otherwise read it from the image header
            let dpi = cli
                .dpi
                .or_else(|| if from_project { None } else { detect_dpi(p) })
                .or_else(|| {
                    phone_paper
                        .filter(|_| !from_project)
                        .and_then(|pp| legatorocr_engine::files::dpi_from_paper(p, pp))
                })
                .unwrap_or(settings.dpi);
            SheetInput {
                path: p.clone(),
                settings: a.apply(&Settings {
                    dpi,
                    ..settings.clone()
                }),
            }
        })
        .collect();
    if cli.report_outliers {
        let out = legatorocr_engine::find_outliers(
            &sheets,
            cli.auto_rotate.then_some(&ocr_opts),
            &AtomicBool::new(false),
        )?;
        if out.is_empty() {
            println!("no outlier pages among {}", sheets.len());
        }
        for (i, (w, h)) in &out {
            println!("{}: {:.0} x {:.0} mm", inputs[*i].display(), w, h);
        }
        return Ok(());
    }
    let save_images = !matches!(cli.format, Fmt::None);
    if save_images {
        std::fs::create_dir_all(&cli.out_dir)?;
    }
    let ext = match cli.format {
        Fmt::Png => "png",
        Fmt::Jpeg => "jpg",
        _ => "tif",
    };
    if save_images {
        if let Some(clash) = output_clobbers_input(&cli.out_dir, &stems, ext, &inputs) {
            bail!(
                "the page image {} would overwrite an input scan; choose another folder with -o (or --format none)",
                clash.display()
            );
        }
    }
    if let Some(o) = named_output_clobbers_input(
        &[
            &cli.pdf,
            &cli.txt,
            &cli.hocr,
            &cli.alto,
            &cli.json,
            &cli.passages_jsonl,
            &cli.fff_out,
            &cli.docx,
            &cli.multitiff,
            &cli.package,
            &cli.report,
            &cli.save_project,
        ],
        &inputs,
    ) {
        bail!(
            "{} is also an input; choose another output name, it would replace the original scan",
            o.display()
        );
    }
    if cli.multitiff.is_some() && !matches!(settings.mode, legatorocr_core::OutputMode::BlackWhite)
    {
        bail!("--multitiff needs black & white pages (use --mode bw, or a profile / project that is black & white)");
    }
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    let saved_images: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    if cli.multitiff.is_some()
        && (cli.skip_duplicates || cli.split_on_blank || cli.split_on_barcode || cli.skip_blank)
    {
        bail!("--multitiff writes every processed page; do not combine it with --skip-duplicates, --split-on-blank or --skip-blank");
    }
    type Key = (usize, u8);
    let report_pages: Mutex<Vec<(Key, legatorocr_engine::report::ReportPage)>> =
        Mutex::new(Vec::new());
    let tiff_pages: Mutex<Vec<(Key, Option<legatorocr_engine::g4::G4Strip>)>> =
        Mutex::new(Vec::new());
    // A profile or project carries the PDF quality and two export switches; an explicit
    // command-line flag still wins.
    let given = |flag: &str| {
        std::env::args()
            .skip(1)
            .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
    };
    let carried = |k: &str| {
        project
            .as_ref()
            .and_then(|p| p.get(k))
            .or_else(|| profile.as_ref().and_then(|p| p.get(k)))
            .map(str::to_string)
    };
    let mut pdf_quality = cli.pdf_quality;
    if !given("--pdf-quality") {
        if let Some(q) = carried("pdf_quality").and_then(|v| v.parse::<u8>().ok()) {
            pdf_quality = q.min(100);
        }
    }
    let skip_blank = cli.skip_blank || carried("skip_blank").as_deref() == Some("true");
    let uniform_size = cli.uniform_size || carried("uniform_size").as_deref() == Some("true");
    if let Some(mb) = cli.target_size {
        if cli.pdf.is_none() || !(mb.is_finite() && mb > 0.0) {
            bail!("--target-size needs --pdf and a positive number of megabytes");
        }
        if pdf_quality > 0 {
            pdf_quality = legatorocr_engine::quality_for_target(
                &sheets,
                cli.auto_rotate.then_some(&ocr_opts),
                (mb as f64 * 1e6) as u64,
                &AtomicBool::new(false),
            )?;
            eprintln!("target {mb} MB: JPEG quality {pdf_quality}");
        }
    }
    if (cli.split_on_blank || cli.split_on_barcode) && cli.pdf.is_none() {
        bail!("--split-on-blank / --split-on-barcode need --pdf");
    }
    let provenance = if cli.provenance && cli.pdfa {
        eprintln!("note: --provenance is not written into PDF/A files (it would need an XMP extension schema)");
        None
    } else if cli.provenance {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Some(legatorocr_engine::provenance::build(
            &inputs,
            &args,
            legatorocr_engine::now_unix(),
        ))
    } else {
        None
    };
    let mut meta_attachments = Vec::new();
    if cli.embed_project {
        if cli.pdf.is_none() {
            bail!("--embed-project needs --pdf");
        }
        if cli.pdfa && cli.pdfa_part != 3 {
            bail!("--embed-project with --pdfa needs --pdfa-part 3 (PDF/A-1 and -2 do not allow embedded files of this kind)");
        }
        if cli.split_on_blank || cli.split_on_barcode {
            bail!("--embed-project cannot be combined with --split-on-blank / --split-on-barcode (several PDFs)");
        }
        let pairs: Vec<(PathBuf, SheetAdj)> = inputs
            .iter()
            .zip(&adjusts)
            .zip(&sheets)
            .map(|((p, a), sh)| {
                (
                    p.clone(),
                    SheetAdj {
                        dpi: Some(sh.settings.dpi),
                        ..a.clone()
                    },
                )
            })
            .collect();
        let extra = vec![
            ("pdf_quality".to_string(), pdf_quality.to_string()),
            ("skip_blank".to_string(), skip_blank.to_string()),
            ("uniform_size".to_string(), uniform_size.to_string()),
        ];
        let (atts, info) = legatorocr_engine::embedproject::build(&settings, extra, &pairs)?;
        eprintln!(
            "--embed-project: {} original file(s), {} bytes, plus project.lgr ({} bytes)",
            info.files, info.original_bytes, info.project_bytes
        );
        meta_attachments = atts;
    }
    let job = ExportJob {
        sheets: &sheets,
        pdf: cli.pdf.as_ref().map(|pdf_path| PdfExport {
            quality: pdf_quality,
            ocr: cli.ocr.then(|| ocr_opts.clone()),
            title: pdf_path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.contains(legatorocr_engine::naming::TITLE_MARK))
                .map(str::to_string),
            meta: legatorocr_engine::PdfMeta {
                author: cli.author.clone(),
                subject: cli.subject.clone(),
                keywords: cli.keywords.clone(),
                created_unix: Some(legatorocr_engine::now_unix()),
                rights: cli.rights_notice.clone(),
                provenance: provenance.clone(),
                attachments: meta_attachments,
            },
            bookmarks: cli.bookmarks,
            front_matter: cli.front_matter,
            pdfa: cli.pdfa,
        }),
        text_ocr: cli.ocr.then(|| ocr_opts.clone()),
        skip_blank,
        keep_going: cli.keep_going,
        osd: cli.auto_rotate.then(|| ocr_opts.clone()),
        uniform_size,
        skip_duplicates: cli.skip_duplicates,
        split_on_blank: cli.split_on_blank,
        split_on_barcode: cli.split_on_barcode,
    };
    legatorocr_engine::profile::enable(cli.timings);
    legatorocr_engine::ropost::set_enabled(cli.clean_text);
    legatorocr_engine::pagenum::set_enabled(cli.page_labels);
    legatorocr_engine::memcap::set(cli.max_memory);
    if cli.auto_retry {
        if !cli.ocr {
            bail!("--auto-retry needs --ocr");
        }
        legatorocr_engine::autoretry::set_below(cli.auto_retry_below);
        legatorocr_engine::autoretry::set_enabled(true);
    }
    if cli.binarize_by_ocr {
        if !cli.ocr {
            bail!("--binarize-by-ocr needs --ocr");
        }
        legatorocr_engine::altbin::set_enabled(true);
    }
    if cli.form_fields {
        if cli.pdf.is_none() {
            bail!("--form-fields needs --pdf");
        }
        if cli.pdfa {
            bail!("--form-fields cannot be combined with --pdfa (form widgets need appearance streams there)");
        }
        legatorocr_engine::formfields::set_enabled(true);
    }
    if !cli.redact.is_empty() || !cli.redact_regex.is_empty() {
        if !cli.ocr {
            bail!("--redact needs --ocr (it works on the words OCR reads)");
        }
        if !matches!(cli.format, Fmt::None)
            || cli.multitiff.is_some()
            || cli.package.is_some()
            || cli.ocr_cache.is_some()
        {
            bail!("--redact cannot be combined with image output (add --format none), --multitiff, --package or --ocr-cache: those would hold the original pixels or words");
        }
        let pats = legatorocr_engine::redact::compile(&cli.redact, &cli.redact_regex)?;
        legatorocr_engine::redact::set(pats);
    }
    if cli.bookmarks_headings && !cli.ocr {
        bail!("--bookmarks-headings reads the recognised text; add --ocr");
    }
    legatorocr_engine::headings::set_enabled(cli.bookmarks_headings);
    if let Some(dir) = &cli.ocr_cache {
        if !cli.ocr {
            bail!("--ocr-cache needs --ocr");
        }
        let v = check_ocr(&ocr_opts).context("no OCR engine is available")?;
        legatorocr_engine::ocrcache::set(dir.clone(), &v)
            .with_context(|| format!("{}: cannot create the cache folder", dir.display()))?;
    }
    if let Some(spec) = &cli.page_lang {
        if !cli.ocr {
            bail!("--page-lang needs --ocr");
        }
        let rules = legatorocr_engine::pagelang::parse(spec)?;
        for l in rules
            .iter()
            .flat_map(|r| r.lang.split('+'))
            .collect::<std::collections::BTreeSet<_>>()
        {
            check_ocr(&OcrOptions {
                lang: l.into(),
                ..ocr_opts.clone()
            })
            .with_context(|| format!("--page-lang: language {l}"))?;
        }
        legatorocr_engine::pagelang::set(rules);
    }
    legatorocr_engine::qa::enable(cli.quality_report);
    legatorocr_engine::explain::enable(cli.explain);
    legatorocr_engine::certify::enable(cli.certify);
    if cli.doubtful_words.is_some() {
        legatorocr_engine::doubtful::begin(cli.doubtful_conf);
    }
    if cli.proof_sheet.is_some() {
        legatorocr_engine::proofsheet::begin();
    }
    if cli.djvu.is_some() {
        legatorocr_engine::djvu::set_thumbnails(cli.djvu_thumbnails);
        legatorocr_engine::djvu::begin();
    }
    if cli.passages_jsonl.is_some() {
        let ext_name = match cli.format {
            Fmt::None => "png",
            _ => ext,
        };
        legatorocr_engine::passages::begin(
            ext_name,
            cli.jpeg_quality,
            save_images.then(|| stems.clone()),
        );
    }
    let res = run_export(
        &job,
        &AtomicBool::new(false),
        &|i, pg| {
            let stem = &stems[i];
            let name = match pg.split_index {
                Some(0) => format!("{stem}_L"),
                Some(_) => format!("{stem}_R"),
                None => stem.to_string(),
            };
            eprintln!(
                "{name}: {}x{} px @ {} dpi, skew {:+.2}°{}",
                pg.image.width(),
                pg.image.height(),
                pg.dpi,
                pg.skew_deg,
                if pg.blank { ", blank" } else { "" }
            );
            let key = (i, pg.split_index.unwrap_or(0));
            if cli.report.is_some() {
                report_pages.lock().unwrap().push((
                    key,
                    legatorocr_engine::report::ReportPage::from_page(name.clone(), pg),
                ));
            }
            if cli.multitiff.is_some() {
                let strip = match &pg.image {
                    legatorocr_core::PageImage::Bw(b) => {
                        Some(legatorocr_engine::g4::G4Strip::from_bitimage(b, pg.dpi))
                    }
                    _ => None,
                };
                tiff_pages.lock().unwrap().push((key, strip));
            }
            if save_images {
                let path = cli.out_dir.join(format!("{name}.{ext}"));
                match legatorocr_engine::save_page_image_q(pg, &path, cli.jpeg_quality) {
                    Ok(_) => saved_images.lock().unwrap().push(path),
                    Err(e) => {
                        first_err
                            .lock()
                            .unwrap()
                            .get_or_insert(format!("{}: {e}", path.display()));
                    }
                }
            }
        },
        &|_, _| {},
    )?;
    {
        let title = res
            .text
            .as_deref()
            .and_then(legatorocr_engine::naming::title_from_text)
            .unwrap_or_else(|| "untitled".into());
        for p in [
            &mut cli.pdf,
            &mut cli.txt,
            &mut cli.hocr,
            &mut cli.alto,
            &mut cli.page_xml,
            &mut cli.json,
            &mut cli.docx,
            &mut cli.tei,
            &mut cli.package,
        ] {
            resolve_title(p, &title)?;
        }
    }
    if let Some(p) = &cli.fff_out {
        let text = res.text.as_deref().unwrap_or("");
        let first = inputs.first().cloned().unwrap_or_default();
        let sha = if inputs.len() == 1 {
            std::fs::read(&first)
                .map(|b| legatorocr_engine::checksums::sha256_hex(&b))
                .unwrap_or_default()
        } else {
            let all: Vec<String> = inputs
                .iter()
                .filter_map(|i| std::fs::read(i).ok())
                .map(|b| legatorocr_engine::checksums::sha256_hex(&b))
                .collect();
            legatorocr_engine::checksums::sha256_hex(all.join("\n").as_bytes())
        };
        let title = cli
            .fff_title
            .clone()
            .or_else(|| legatorocr_engine::naming::title_from_text(text))
            .unwrap_or_else(|| "untitled".into());
        let file = if inputs.len() == 1 {
            first
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            format!("{} files", inputs.len())
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let json = fff::document_ocr(
            &fff::DocMeta {
                title: &title,
                author: cli.author.as_deref().unwrap_or(""),
                year: cli.fff_year.as_deref().unwrap_or(""),
                lang: &cli.lang,
                file: &file,
                sha256: &sha,
                text,
                engine: "LegatorOCR",
                version: env!("CARGO_PKG_VERSION"),
            },
            now,
        );
        write_atomic(p, json.as_bytes())?;
        eprintln!(
            "wrote {} (FFF envelope document/ocr, {} page(s))",
            p.display(),
            text.split('\x0c').count()
        );
    }
    if cli.tables_xlsx.is_some() || cli.tables_html.is_some() {
        let found = res
            .docx_blocks
            .as_deref()
            .map(legatorocr_engine::tables::find)
            .unwrap_or_default();
        if let Some(p) = &cli.tables_xlsx {
            write_atomic(p, &legatorocr_engine::tables::xlsx(&found))?;
            eprintln!("wrote {} ({} table(s))", p.display(), found.len());
        }
        if let Some(p) = &cli.tables_html {
            let title = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            write_atomic(
                p,
                legatorocr_engine::tables::html(&found, &title).as_bytes(),
            )?;
            eprintln!("wrote {} ({} table(s))", p.display(), found.len());
        }
    }
    if cli.ocr_cache.is_some() {
        let (h, m) = legatorocr_engine::ocrcache::stats();
        eprintln!("OCR cache: {h} page(s) reused, {m} recognised");
    }
    if cli.explain {
        for l in legatorocr_engine::explain::report() {
            eprintln!("{l}");
        }
    }
    if cli.auto_retry {
        for l in legatorocr_engine::autoretry::report() {
            eprintln!("{l}");
        }
    }
    if cli.certify {
        match &res.pdf {
            Some(pdf) => {
                let (ok, lines) = legatorocr_engine::certify::check(pdf);
                for l in &lines {
                    eprintln!("{l}");
                }
                if !ok {
                    std::process::exit(1);
                }
            }
            None => eprintln!(
                "--certify needs a single --pdf output (not --split-on-blank / --split-on-barcode)"
            ),
        }
    }
    if cli.verify_text {
        match (&res.pdf, &res.text) {
            (Some(pdf), Some(t)) => match legatorocr_engine::textcheck::check(pdf, t) {
                Ok(r) => eprintln!("{}", r.summary().1),
                Err(e) => eprintln!("text layer check: could not read the PDF back ({e})"),
            },
            _ => eprintln!("--verify-text needs --ocr and a single --pdf output"),
        }
    }
    if cli.timings {
        eprint!("{}", legatorocr_engine::profile::report());
    }
    if cli.quality_report {
        let alerts = legatorocr_engine::qa::report();
        if alerts.is_empty() {
            eprintln!("quality report: nothing stands out");
        } else {
            eprintln!("quality report ({} note(s)):", alerts.len());
            for a in alerts {
                eprintln!("  - {a}");
            }
        }
    }
    let save_error = first_err.into_inner().unwrap();
    for (p, e) in &res.failed {
        eprintln!("SKIPPED {}: {e}", p.display());
    }
    if res.skipped_blank > 0 {
        eprintln!("{} blank page(s) skipped", res.skipped_blank);
    }
    if save_images {
        eprintln!(
            "wrote {} page image(s) to {}",
            res.pages,
            cli.out_dir.display()
        );
    }
    if let Some(path) = &cli.report {
        let mut v = report_pages.into_inner().unwrap();
        v.sort_by_key(|(k, _)| *k);
        let pages: Vec<_> = v.into_iter().map(|(_, p)| p).collect();
        let (html, flagged) = legatorocr_engine::report::html(&pages);
        write_atomic(path, html.as_bytes())?;
        eprintln!(
            "wrote {} ({} page(s), {} flagged)",
            path.display(),
            pages.len(),
            flagged
        );
    }
    if let Some(path) = &cli.multitiff {
        let mut v = tiff_pages.into_inner().unwrap();
        v.sort_by_key(|(k, _)| *k);
        if v.iter().any(|(_, s)| s.is_none()) {
            bail!("--multitiff needs black & white pages (use --mode bw)");
        }
        let strips: Vec<_> = v.into_iter().filter_map(|(_, s)| s).collect();
        write_atomic(path, &legatorocr_engine::g4::tiff_g4_multi(&strips))?;
        eprintln!("wrote {} ({} page(s))", path.display(), strips.len());
    }
    if res.skipped_duplicates > 0 {
        eprintln!("{} duplicate page(s) skipped", res.skipped_duplicates);
    }
    let mut written: Vec<PathBuf> = Vec::new();
    let saved_images = saved_images.into_inner().unwrap();
    if let (Some(pdf_path), true) = (&cli.pdf, !res.docs.is_empty()) {
        let stem = pdf_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document")
            .to_string();
        for (i, bytes) in res.docs.iter().enumerate() {
            let name = res.doc_names.get(i).and_then(|n| n.as_deref());
            let p = pdf_path.with_file_name(legatorocr_engine::doc_file_name(&stem, i, name));
            write_atomic(&p, bytes)?;
            eprintln!("wrote {}", p.display());
            written.push(p);
        }
        if cli.txt_each {
            eprintln!("--txt-each: not available with one PDF per document (split); use --txt");
        }
    }
    if let (Some(pdf_path), Some(bytes)) = (&cli.pdf, &res.pdf) {
        write_atomic(pdf_path, bytes)?;
        written.push(pdf_path.clone());
        if let (true, Some(t)) = (cli.txt_each, &res.text) {
            let tp = pdf_path.with_extension("txt");
            write_atomic(&tp, t.replace('\x0c', "\n").as_bytes())?;
            eprintln!("wrote {}", tp.display());
            written.push(tp);
        }
        eprintln!(
            "wrote {} ({} page(s){})",
            pdf_path.display(),
            res.pages,
            if cli.ocr {
                format!(", {} OCR words", res.words)
            } else {
                String::new()
            }
        );
    }
    if let Some(t) = &res.text {
        let wants_ro = ocr_opts
            .lang
            .split('+')
            .any(|l| l.eq_ignore_ascii_case("ron"));
        if wants_ro {
            if let Some(why) = legatorocr_core::text::romanian_sanity(t) {
                eprintln!("warning: {why}");
            }
        }
    }
    if let (Some(p), Some(t)) = (&cli.txt, &res.text) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let (Some(p), Some(t)) = (&cli.json, &res.json) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let (Some(p), Some(t)) = (&cli.hocr, &res.hocr) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let (Some(p), Some(t)) = (&cli.alto, &res.alto) {
        write_atomic(p, t.as_bytes())?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let Some(dir) = &cli.page_xml {
        std::fs::create_dir_all(dir)?;
        for (n, x) in res.page_xml.iter().enumerate() {
            let f = dir.join(format!("page_{:04}.xml", n + 1));
            write_atomic(&f, x.as_bytes())?;
            written.push(f);
        }
        eprintln!(
            "wrote {} PAGE XML file(s) to {}",
            res.page_xml.len(),
            dir.display()
        );
    }
    if let (Some(p), Some(t)) = (&cli.docx, res.paragraph_text.as_ref().or(res.text.as_ref())) {
        let pages: Vec<String> = t
            .split('\x0c')
            .map(|s| s.trim_start_matches('\n').to_string())
            .collect();
        let title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("LegatorOCR");
        let bytes = match &res.docx_blocks {
            Some(b) if !b.is_empty() => legatorocr_engine::docx::from_blocks(b, title),
            _ => legatorocr_engine::docx::from_pages(&pages, title),
        };
        write_atomic(p, &bytes)?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let Some(p) = &cli.proof_sheet {
        let title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Proof sheet");
        match legatorocr_engine::proofsheet::finish(title) {
            Some(h) => {
                write_atomic(p, h.as_bytes())?;
                eprintln!("wrote {}", p.display());
                written.push(p.clone());
            }
            None => eprintln!("--proof-sheet needs --ocr: no page was read, nothing written"),
        }
    }
    if let Some(p) = &cli.passages_jsonl {
        let ps = legatorocr_engine::passages::finish(cli.clean_text);
        let title = cli.passages_title.clone();
        write_atomic(
            p,
            legatorocr_engine::passages::to_jsonl(&ps, title.as_deref()).as_bytes(),
        )?;
        eprintln!("wrote {} ({} passages)", p.display(), ps.len());
        written.push(p.clone());
    }
    if let Some(p) = &cli.doubtful_words {
        if let Some((tsv, n, total)) = legatorocr_engine::doubtful::finish() {
            write_atomic(p, tsv.as_bytes())?;
            eprintln!(
                "wrote {} ({n} doubtful words of {total} judged)",
                p.display()
            );
            written.push(p.clone());
        }
    }
    if let (Some(p), Some(d)) = (&cli.djvu, &res.djvu) {
        write_atomic(p, d)?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let (Some(p), Some(b)) = (&cli.tei, &res.docx_blocks) {
        let title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("LegatorOCR");
        let lang = cli.lang.split('+').next().map(|l| match l {
            "ron" => "ro",
            "eng" => "en",
            "fra" => "fr",
            "deu" => "de",
            "ita" => "it",
            "spa" => "es",
            "hun" => "hu",
            other => other,
        });
        write_atomic(
            p,
            legatorocr_engine::tei::render(b, title, lang, &[]).as_bytes(),
        )?;
        eprintln!("wrote {}", p.display());
        written.push(p.clone());
    }
    if let (Some(dir), Some(pr)) = (&cli.archive_package, archive_project.take()) {
        let mut cmd = String::from("legatorocr --project project.lgr --ocr");
        cmd.push_str(&format!(" --lang {}", cli.lang));
        if cli.pdf.is_some() {
            cmd.push_str(&format!(" --pdf new.pdf --pdf-quality {}", cli.pdf_quality));
        }
        cmd.push_str(" --json new_ocr.json");
        let b = legatorocr_engine::reprocess::finish(
            dir,
            pr,
            &written,
            legatorocr_engine::now_unix(),
            &cmd,
        )?;
        eprintln!(
            "archive package: {} ({} files, {} originals)",
            dir.display(),
            b.files,
            b.originals
        );
    }
    if cli.mrz {
        match res.text.as_deref() {
            None => eprintln!("--mrz: no OCR text was produced"),
            Some(t) => {
                let found = legatorocr_engine::mrz::find(t);
                if found.is_empty() {
                    println!("mrz: none found");
                }
                for m in &found {
                    println!(
                        "mrz {}: {} {} {} | number {} | nationality {} | born {} | sex {} | expires {} | {}",
                        m.format, m.doc_type, m.country, m.surname, m.number, m.nationality, m.birth, m.sex, m.expiry,
                        if m.failed.is_empty() { "all check digits agree".to_string() } else { format!("CHECK FAILED: {}", m.failed.join(", ")) }
                    );
                }
            }
        }
    }
    if let Some(dest) = &cli.package {
        let password: Option<String> = match &cli.package_password_file {
            None => None,
            Some(f) if f.as_os_str() == "-" => Some(
                std::env::var("LEGATOROCR_PACKAGE_PASSWORD")
                    .context("--package-password-file -: LEGATOROCR_PACKAGE_PASSWORD is not set")?,
            ),
            Some(f) => Some(
                std::fs::read_to_string(f)
                    .with_context(|| format!("{}: cannot read the password file", f.display()))?
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string(),
            ),
        };
        let p = legatorocr_engine::package::build_with(
            dest,
            &written,
            legatorocr_engine::now_unix(),
            cli.rights_notice.as_deref().unwrap_or(""),
            password.as_deref(),
        )?;
        eprintln!(
            "package: {} ({} files, {} bytes of content)",
            p.dest.display(),
            p.files,
            p.bytes
        );
    }
    if let (true, Some(pdf_path)) = (cli.checksums, &cli.pdf) {
        let base = pdf_path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let (inside, outside) = legatorocr_engine::checksums::below(base, &written);
        for f in &outside {
            eprintln!(
                "note: {} is outside {}: not listed in SHA256SUMS.txt",
                f.display(),
                base.display()
            );
        }
        let sums = legatorocr_engine::checksums::render(base, &inside)?;
        let sp = base.join("SHA256SUMS.txt");
        if inside.is_empty() {
            eprintln!("note: nothing to list: SHA256SUMS.txt not written");
        } else {
            // a folder that collects several documents keeps the hashes of the earlier ones
            let merged = match std::fs::read_to_string(&sp) {
                Ok(old) => legatorocr_engine::checksums::merge(&old, &sums),
                Err(_) => sums,
            };
            write_atomic(&sp, merged.as_bytes())?;
            eprintln!(
                "wrote {} ({} file(s) added or updated)",
                sp.display(),
                inside.len()
            );
            // the sums file is itself a file this run wrote
            written.push(sp.clone());
        }
    }
    if let Some(bag) = &cli.bagit {
        let s =
            legatorocr_engine::bagit::write(bag, &written, legatorocr_engine::now_unix(), None)?;
        eprintln!(
            "bag: {} ({} files, {} bytes)",
            bag.display(),
            s.files,
            s.bytes
        );
    }
    if let Some(obj) = &cli.ocfl {
        let id = cli.ocfl_id.clone().unwrap_or_else(|| {
            cli.pdf
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("legatorocr".into(), |n| {
                    n.to_string_lossy().replace(' ', "_")
                })
        });
        // an OCFL id is a URI
        let id = if id.contains(':') {
            id
        } else {
            format!("urn:legatorocr:{id}")
        };
        let s = legatorocr_engine::ocfl::write(obj, &written, &id, legatorocr_engine::now_unix())?;
        eprintln!(
            "ocfl: {} ({} files, {} bytes)",
            obj.display(),
            s.files,
            s.bytes
        );
    }
    if cli.catalog.is_some() || cli.dublin_core {
        let pdfs: Vec<PathBuf> = written
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
            .cloned()
            .collect();
        let rows = legatorocr_engine::catalog::rows(&pdfs, None)?;
        if let Some(c) = &cli.catalog {
            let is_csv = c.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"));
            let text = if is_csv {
                legatorocr_engine::catalog::csv(&rows)
            } else {
                legatorocr_engine::catalog::html(&rows, "Catalog")
            };
            write_atomic(c, text.as_bytes())?;
            eprintln!("wrote {} ({} file(s))", c.display(), rows.len());
            written.push(c.clone());
        }
        if cli.dublin_core {
            let first = legatorocr_engine::ocr::canonical_lang(&cli.lang);
            let first = first.split('+').next().unwrap_or("").to_string();
            let language = legatorocr_engine::ocr::UI_LANGUAGES
                .iter()
                .find(|(_, t, _)| *t == first)
                .map(|(iso, _, _)| iso.to_string())
                .or_else(|| (!first.is_empty()).then_some(first));
            let m = legatorocr_engine::catalog::Meta {
                author: cli.author.clone(),
                subject: cli.subject.clone(),
                keywords: cli.keywords.clone(),
                rights: cli.rights_notice.clone(),
                language,
            };
            for (r, p) in rows.iter().zip(&pdfs) {
                let f = legatorocr_engine::catalog::write_dublin_core(p, r, &m)?;
                eprintln!("wrote {}", f.display());
                written.push(f);
            }
        }
    }
    if let Some(log) = &cli.audit_log {
        let o = legatorocr_engine::audit::Options {
            retain_until: cli.retain_until.clone(),
            legal_hold: cli.legal_hold,
            note: String::new(),
        };
        // every file this run wrote, page images and the sums file included
        let mut logged = written.clone();
        logged.extend(saved_images.iter().cloned());
        let text = legatorocr_engine::audit::render(
            concat!("LegatorOCR ", env!("CARGO_PKG_VERSION")),
            &cli.inputs,
            &logged,
            legatorocr_engine::now_unix(),
            &o,
        )?;
        legatorocr_engine::audit::append(log, &text)?;
        eprintln!(
            "audit log: {} record(s) appended to {}",
            logged.len(),
            log.display()
        );
    }
    if let Some(e) = save_error {
        legatorocr_engine::archive::cleanup();
        bail!("saving page image failed (the PDF and text outputs above were written): {e}");
    }
    if !res.failed.is_empty() {
        eprintln!("{} input(s) could not be processed", res.failed.len());
        legatorocr_engine::archive::cleanup();
        std::process::exit(2);
    }
    Ok(())
}

fn run_search(cli: &Cli, query: &str) -> Result<()> {
    if cli.inputs.is_empty() {
        bail!("--search needs at least one file or folder to look in");
    }
    let o = legatorocr_engine::search::SearchOptions {
        regex: cli.search_regex || cli.search_preset.is_some(),
        exact: cli.search_exact,
        max_hits: cli.search_max,
        query: cli.search_query,
        old_orthography: cli.search_old_orthography,
    };
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut write_err = None;
    let s = legatorocr_engine::search::search(&cli.inputs, query, &o, &mut |h| {
        use std::io::Write;
        let page = h.page.map(|p| format!(" p.{p}")).unwrap_or_default();
        if let Err(e) = writeln!(out, "{}{page}:{}: {}", h.source, h.line, h.text) {
            write_err.get_or_insert(e);
        }
    })?;
    use std::io::Write;
    let _ = out.flush();
    if let Some(e) = write_err {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(e.into());
        }
    }
    eprintln!(
        "{} hit(s) in {} file(s){}",
        s.hits,
        s.files,
        if s.truncated {
            " (stopped at the limit)"
        } else {
            ""
        }
    );
    if s.image_only_pdfs > 0 {
        eprintln!(
            "{} PDF(s) are scans without a text layer: run them through LegatorOCR with --ocr first",
            s.image_only_pdfs
        );
    }
    if s.unreadable > 0 {
        eprintln!("{} file(s) could not be read", s.unreadable);
    }
    if s.hits == 0 {
        legatorocr_engine::archive::cleanup();
        // 1: looked everywhere, nothing there; 2: nothing found, but some files were not read
        std::process::exit(if s.unreadable > 0 { 2 } else { 1 });
    }
    Ok(())
}

fn parse_align(s: &str) -> Result<(HAlign, VAlign)> {
    let (mut h, mut v) = (HAlign::Center, VAlign::Center);
    for part in s.to_ascii_lowercase().split('-') {
        match part {
            "center" | "centre" => {}
            "left" => h = HAlign::Left,
            "right" => h = HAlign::Right,
            "top" => v = VAlign::Top,
            "bottom" => v = VAlign::Bottom,
            _ => bail!("--align: '{s}' is not valid (e.g. center, top-left, bottom-right)"),
        }
    }
    Ok((h, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_stems_get_unique_names() {
        let v = output_stems(&["a/001.png".into(), "b/001.PNG".into(), "b/002.png".into()]);
        assert_eq!(v, ["001_0001", "001_0002", "002"]);
    }

    #[test]
    fn page_sizes() {
        assert_eq!(parse_page_size("A5").unwrap(), (148.0, 210.0));
        assert_eq!(parse_page_size("100x150").unwrap(), (100.0, 150.0));
        for bad in ["0x0", "100000x100000", "nanx5", "5", "axb"] {
            assert!(parse_page_size(bad).is_err(), "{bad}");
        }
    }
}
