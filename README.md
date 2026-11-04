# LegatorOCR

Prezentare / presentation site: <https://legatorocr.chiuta.ro> (sursa în `docs/`).

Clean-room Rust post-processor for scanned books:
**split → deskew → crop → margins → clean / binarize → PDF with a searchable OCR text layer.**
Same *workflow* as classic scan-tailoring tools, independent code. Graphical interface
(Windows, macOS, Linux) and command line.

## Quick start

**GUI** — run `legatorocr-gui` (or open `LegatorOCR.app` on macOS). Drag scans or a folder into the
window (or *Add*), pick settings on the right, check the **Side by side / Original / Processed**
preview, adjust single pages (rotate, page split, manual skew), then **Export PDF…** or
**Export images…**. Settings persist between sessions; **File → Save project…** keeps the page list with
per-page adjustments (rotation, split, skew, content-box correction) in a plain-text `.lgr` file.
Languages (follow the system language; switch in the menu): Română, English, Français, Italiano,
Español, Português, Deutsch, Magyar.

**CLI**

```
legatorocr scan_*.png --mode bw --dpi 300 --margin 6 --page-size a5 \
    --pdf carte.pdf --ocr --lang ron+eng
```

Output modes: `--mode color|gray|bw|mixed|hybrid`. **mixed** = sharp 1-bit text plus colour picture
patches found automatically; **hybrid** = the same over a low-resolution paper background
(`--hybrid-dpi`), which gives colour book pages of roughly 60–100 KB instead of 700 KB+.
More: `--threshold sauvola|wolf|niblack|nick|bradley|otsu`, `--normalize` (even out lighting),
`--dewarp` (straighten curved lines), `--dewarp-guide "X,Y X,Y …"` (you give the bent line as points, fractions 0–1 of the deskewed page; repeatable; replaces the automatic estimate), `--output-dpi N`, `--uniform-size`, `--align top-left`,
`--project FILE.lgr` / `--save-project FILE.lgr`. Multi-page TIFFs are read page by page
(`scan.tif#3` addresses one page).

`--skip-blank`, `--skew DEG` (fixed correction), `--rotate 90`, `--pdf-quality N` (0 = lossless),
`--format png|jpeg|tiff|none`, `--tessdata DIR`, `--psm N`, `--min-conf N`, `--keep-going` (skip unreadable
inputs, exit status 2) — see `legatorocr --help`. A folder can be given instead of files. Resolution is
read from the PNG/JPEG header when it says 150–1200 dpi (otherwise 300; override with `--dpi`); camera
EXIF rotation is applied and transparency is flattened onto white. Out-of-range numbers are rejected
with a clear message rather than producing absurd page sizes.

## Less common options

`-o, --out-dir DIR` folder for the processed page images · `--no-deskew`, `--max-skew DEG`, `--no-auto-orient`
(sideways pages), `--no-despeckle` (keep specks) · `--sauvola-k K` (black-and-white sensitivity),
`--picture-sensitivity 0-1` (mixed/hybrid) · `--jpeg-quality N` (with `--format jpeg`) · PDF metadata
`--subject`, `--keywords` · `--tesseract PATH` (external engine) · `--burst-gap SECONDS` (with `--burst`),
`--fuse-max-shift PX` (with `--fuse-frames`) · `--watch-interval SECONDS`, `--watch-skip-existing` (with
`--watch`) · `--version`. `--project FILE.ScanTailor` opens a ScanTailor project: the scan list and the rotation per image are taken over; skew, split and content boxes are found again (the reader was written from the format's description and tested on a hand-made project, not on files from every ScanTailor version).
Another OCR engine can be plugged in: `--engine tesseract --tesseract PROGRAM` accepts any program that speaks Tesseract's command line and returns its TSV (template and contract: `docs/examples/external-ocr-engine.py`; a generative model there can invent words, so keep a person in the loop). OCR speed and accuracy on your own pages: `scripts/bench-real.py DIR` (images with a `NAME.txt` reference each; `--partial` for references that cover part of a page).
**ONNX OCR bridge** (`scripts/engine-onnx.py`; Python with onnxruntime, numpy, Pillow): runs a PaddleOCR PP-OCR layout pair (DB text detector, then a CTC recogniser) as that external engine, so a model you chose can be the OCR engine without LegatorOCR bundling any weights. You download the models yourself, e.g. the PP-OCRv6 detection and recognition models from PaddleOCR's model list converted to ONNX (`pip install paddle2onnx`, then `paddle2onnx --model_dir DIR --model_filename inference.pdmodel --params_filename inference.pdiparams --save_file det.onnx --opset_version 14`; or a ready-made ONNX export from a model hub), plus the matching character dictionary (`ppocr_keys_v1.txt`-style: one character per line; the bridge puts the CTC blank at class 0 and appends the space class, as PaddleOCR does). Use: `export LEGATOROCR_ONNX_DET=det.onnx LEGATOROCR_ONNX_REC=rec.onnx LEGATOROCR_ONNX_DICT=dict.txt` then `legatorocr scan.png --pdf out.pdf --ocr --engine tesseract --tesseract scripts/engine-onnx.py`. Settings (variable or the same flag, `--onnx-det/-rec/-dict/-size/-rec-height/-thresh/-unclip/-space`): detector side 960, recogniser height 48 (32 for older models), threshold 0.3, unclip 1.5. Boxes are axis-aligned, no rotated text and no orientation classifier; words are split at the space class using the CTC time step (approximate boxes); confidence is the mean peak probability x 100. **What is verified:** the plumbing only, with tiny synthetic ONNX models (`scripts/test-engine-onnx.py`: wiring, box coordinates after the resize, CTC decoding, TSV through the CLI). **Not verified:** recognition accuracy with real PP-OCR models (none could be downloaded where this was written) and whether Romanian diacritics (ă â î ș ț) come out right: that depends on the model and dictionary you choose; check on your own pages with `scripts/bench-real.py` before relying on it. Like any model engine it can invent text.
**docTR family of the ONNX bridge (ROADMAP 4.33)**: the same script reads docTR/OnnxTR models, including the Romanian-head recogniser of `tools/ocr-train/romanian_head.py` (ă ș ț Ă Ș Ț added to a frozen docTR CRNN). `scripts/get-romanian-doctr-models.sh [mobilenet|resnet] [DIR]` downloads the original models from the OnnxTR GitHub release assets, builds the Romanian recogniser (`romanian_head.py splice`, needs python3 with onnx) and prints the exact command. Use: `export LEGATOROCR_ONNX_FAMILY=doctr LEGATOROCR_ONNX_DET=DIR/db_mobilenet_v3_large-4987e7bd.onnx LEGATOROCR_ONNX_REC=DIR/crnn_mobilenet_v3_small_ro.onnx LEGATOROCR_ONNX_DICT=DIR/vocab_ro.txt` (or `--onnx-family doctr` etc.), then `legatorocr scan.png --ocr --engine tesseract --tesseract scripts/engine-onnx.py`. The detector finds words (1024x1024 input, probability map threshold 0.3), each word crop is read at 32x128 with CTC greedy decoding (blank = last class; `--onnx-dict` is the vocabulary as one string), and the script groups words into lines and columns itself so the TSV is in reading order. Measured through the real CLI on 12 pages of one Romanian magazine (ABBYY references, not hand-verified; 2 CPU cores): db_mobilenet_v3_large + Romanian crnn_mobilenet_v3_small, GOOD8 CER with diacritics 4.26 %, exact word recall 88.2 %, about 3.4 s per page (the `resnet` option: 3.67 %, 89.6 %, about 18 s per page); Tesseract `ron` on the same pages 5.25 %, 91.3 %, 2.7 s per page. Other typefaces and scans: not measured.
OCRmyPDF names are accepted: `-l/--language`, `--force-ocr`, `--redo-ocr`, `--skip-text`, `--deskew` (no-op, on by
default), `--rotate-pages`, `--output-type pdf|pdfa|pdfa-1|2|3`, `-j/--jobs N`, `--sidecar FILE`, `--image-dpi`
(output is named with `--pdf FILE`; there is no positional output). `scripts/check-options.py` lists every option that lacks documentation or a test.

## Install

Download the archive for your platform from the Releases page, or build from source:

```
cargo build --release          # target/release/legatorocr and legatorocr-gui
scripts/package.sh <target>    # e.g. x86_64-unknown-linux-gnu → dist/LegatorOCR-<ver>-<target>.tar.gz
```

| Platform | Artifact | Notes |
|---|---|---|
| Linux x86-64 / ARM64 | `.tar.gz` (+ `.desktop`) | needs OpenGL 2+ and X11 or Wayland |
| Windows x86-64 | `.zip` | needs OpenGL 2+ (any GPU driver; plain RDP sessions may lack it) |
| macOS Apple-silicon / Intel | `.zip` with `LegatorOCR.app` | **unsigned**: first launch via right-click → Open, or `xattr -dr com.apple.quarantine LegatorOCR.app` |

**OCR works out of the box** with the built-in native engine (`legatorocr-nocr`, pure Rust, no
install; `--engine native`). It reads the eight interface languages with one model plus per-language
word lists. Its model was trained on synthetic text and then fine-tuned on real scanned lines of a Romanian magazine and a 1960 textbook (the scans are not in this repository; provenance and open licence questions: `docs/audit/assets-licences.md`): on clean and moderately degraded synthetic
pages it makes under 1 % character errors, but real scans of old or unusual print can be worse than
Tesseract. `--engine auto` (default) therefore uses **Tesseract when it is installed with the
requested languages** and falls back to the native engine otherwise; `--engine tesseract` forces it.
Tesseract (run as a separate program; the app finds it on `PATH`):
`sudo apt install tesseract-ocr tesseract-ocr-ron` · `brew install tesseract tesseract-lang` ·
Windows: `winget install UB-Mannheim.TesseractOCR` (add it to `PATH`).
These package names were checked on 2026-11-04 (`docs/audit/ocr-externals.md`): Debian 13 and Ubuntu 24.04 have `tesseract-ocr` and `tesseract-ocr-ron`; Homebrew `tesseract` is 5.5.4 with only `eng` and `osd`, and `tesseract-lang` carries the *fast* models; the winget package lags (5.4.0 from 2024-06). The `ron` file from `apt`/`brew` is the small *fast* model (about 2.4 MB, 2017); the *best* model (9.6 MB, same year, same repository history) is a manual download of `ron.traineddata` from `tesseract-ocr/tessdata_best` into the tessdata folder (`--tessdata DIR`). Which of the two reads your pages better was not measured here.
The app also looks in the usual install folders (Homebrew `/opt/homebrew/bin`, `/usr/local/bin`,
`C:\Program Files\Tesseract-OCR`), so a double-clicked app works without editing `PATH`.
Without Tesseract the native engine is used (the OCR switch stays enabled).
`--auto-rotate` / the OSD checkbox additionally need `osd.traineddata` (part of the standard packages).

## Pipeline

| Stage | Method (standard, textbook techniques) |
|---|---|
| Orientation | row/column profile energy around the line-pitch band (0° vs a quarter turn); optional Tesseract OSD for the exact direction; or `--rotate` |
| Page split | gutter = emptiest column band in the centre of a landscape sheet (`auto` / `middle` / `off`) |
| Deskew | projection-profile sharpness, coarse→fine search to 0.01°, bilinear rotation; or a manual angle |
| Content box | ink blocks → dilation → connected components; dust and scanner-edge shadows rejected, page numbers kept |
| Margins / size | mm margins, optional fixed page size with centred content; everything outside the content box is repainted in paper colour |
| Analysis | everything is measured on a ≈150 dpi copy (a 600 dpi scan is analysed on 1/16 of the pixels); the result is mapped back to full resolution |
| Dewarp | text lines are followed across narrow vertical strips (row-profile peaks chained strip to strip); each line's wobble around its own mean height becomes a vertical resampling field, interpolated between lines; skipped unless several long lines show a clear bend |
| Lighting | `--normalize`: paper colour per channel from a max-filtered, blurred ≈60 dpi copy, then a per-pixel gain |
| Binarisation | Sauvola (default), Wolf, Niblack, NICK, Bradley, Otsu, **auto** (per page: tries the local methods on a ≈150 dpi copy, keeps the one with the fewest isolated specks and a plausible ink share), all from sliding column sums in O(width) memory |
| Clean-up | `--invert` (negatives: dark median + bright tail), `--auto-levels` (0.5 %/99.5 % stretch, one gain for all channels), `--dropout red|green|blue`, `--punch-holes` (dark round blobs 3–10 mm near the edge, filled with the surrounding paper colour), `--fingers` (skin-coloured regions touching the border, judged relative to the paper colour), `--remove-lines` (runs ≥ 25 mm and ≤ 1.2 mm thick; glyph stems crossing a rule survive), `--sharpen` (unsharp mask ≈0.3 mm), `--despeckle-strength` |
| Pictures | tiles with a high share of mid-tones (text is bimodal, photos are continuous) or solid dark areas, closed and size-filtered → colour patches |
| Output | colour, gray, B/W, **mixed** (1-bit text + colour patches) or **hybrid** (+ low-res background layer); optional down-sampling to `--output-dpi`; fixed or book-wide uniform page size with alignment |
| PDF | own writer: JPEG (colour/gray), CCITT Group 4 for 1-bit (falls back to Flate when smaller), layered pages (stencil mask + background + patches); true page size from DPI |
| Images | PNG with resolution, TIFF Group 4 for black & white |
| Paper problems | `--show-through` (levels the lighting, then pulls pixels only slightly darker than the paper back to paper; real ink is untouched), `--descreen` (≈2 mm tiles that are mostly mid-tone and busy at pixel scale are lightly blurred; text tiles are left alone) |
| Separator sheets | `--split-on-barcode` (pages carrying a bar code, upright, turned 180° or sideways, read on the raw scan — Code 128, EAN-13, ITF, Code 39 —, start a new document named after the code → `book_001_ACT-17.pdf`), `--make-separator TEXT` prints such a sheet (A4 PNG) |
| PDF navigation | `--bookmarks` (one outline entry per source file), `--front-matter N` (pages i, ii, iii … then 1, 2, 3 in PDF viewers) |
| Erase zones | `--erase X,Y,W,H` (mm from the page's top-left, repeatable): painted with the paper colour before anything else is measured; `erase=` in `.lgr` projects; GUI text field |
| Batch | `--skip-duplicates` (consecutive pages with a 96×128 thumbnail difference ≤ 2/255), `--split-on-blank` (blank pages separate documents → `name_001.pdf`, …), `--target-size MB` (JPEG quality chosen from a sample of pages so the PDF fits) |
| OCR input | pages below 250 dpi are enlarged ×2–×4 (Catmull-Rom) before Tesseract; word boxes are mapped back |
| Phone photos | `--perspective`: finds the page as the large bright quadrilateral (Otsu + extreme corners, with convexity/fill checks) and warps it with a homography; does nothing for upright or full-frame scans |
| QC and archive | `--report r.html` (contact sheet, flags blank / size-outlier / strongly deskewed pages), `--multitiff out.tif` (multi-page Group 4 TIFF, black & white only) |
| Delivery | `--checksums` writes `SHA256SUMS.txt` (same format as `sha256sum`) next to the PDF for every file written; `--verify-sums SHA256SUMS.txt` re-checks them later (exit 1 on any mismatch or missing file); `--rights-notice "…"` stores a rights statement (XMP `dc:rights` with `--pdfa`, otherwise the `/Rights` info key and a note icon on page 2); `--txt-each` writes `<pdf name>.txt` beside the PDF for indexers such as DocFetcher / Agent Ransack; `--package FOLDER`, `FILE.zip` or `FILE.iso` (ISO 9660 + Joliet, readable on Windows / macOS / Linux; no Rock Ridge or boot data; files up to 4 GiB) gathers everything written plus `SHA256SUMS.txt` and a `MANIFEST.txt` for handing over (never overwrites; zip limited to 3 GiB); `--package-password-file FILE` (or `-` with `LEGATOROCR_PACKAGE_PASSWORD`) encrypts a `.zip` package with WinZip AES-256; `--audit-log FILE` appends a JSON Lines record per file written (time, tool version, inputs, SHA-256, size) with optional `--retain-until YYYY-MM-DD` and `--legal-hold` – it records evidence and deletes nothing |
| Phone photos | `--phone-paper A4` (or `A5`, `B5`, `Letter`, `210x297` …) derives the resolution of photographs that have none from the paper size, assuming the shot is framed on the page; an estimate, off by the margin left around the page |
| OCR sanity | With `--lang ro`, a warning when the recognised text has almost no ăâîșț or many letter-digit words (typical of a missing Romanian model); a note when `--dpi` is below 200 or above 400 with `--ocr` |
| Picture zones | Always kept as a colour picture in mixed / hybrid output, for photos the detector misses. Drawn with the mouse in the GUI (tick *Draw zones*, choose *Picture*; blue; Undo / Redo shared with erase zones; saved per sheet in the `.lgr` project as `picture =`): they are followed through rotation, page split and deskew, so they stay on the picture. Not followed through perspective correction (then dropped) or dewarping (position approximate). Dewarp guides are likewise dropped when perspective correction is applied, and a stroke is only accepted as a guide when it runs across at least 30 % of the page width. Typed: `--picture-zone X,Y,W,H` (mm of the deskewed page, repeatable), GUI field, `picture_zones=` |
| Text output | `--txt` (plain text), `--hocr` (word boxes + confidences), `--json` (the same as structured JSON: per page size and dpi, per word text, box, confidence, line), `--docx` (editable Word document: paragraphs recovered from the line spacing, aligned columns on three or more rows as real tables, side-by-side text columns as continuous Word sections with column breaks, one page break per scanned page), PDF author/subject/keywords/dates. With `--lang ron` the cedilla letters ş/ţ are corrected to the proper comma-below ș/ț |
| Archives as input | Give `.zip`/`.cbz`, `.tar`, `.tar.gz`/`.tgz` (built in) or `.7z`, `.rar`/`.cbr`, `.cab`, `.iso`, `.tar.bz2`, `.tar.xz` … (through an installed 7-Zip, unrar, bsdtar or unar) instead of a folder: the images and PDFs inside are processed in natural order of their names, in the CLI and the GUI (add / drag and drop), and are searched by `--search`. Extraction goes to a temporary folder that is removed on exit; names are flattened and sizes capped |
| OCR languages | `--lang` takes Tesseract names or two-letter codes (`ro+en` = `ron+eng`) for the eight interface languages ro, en, fr, it, es, pt, de, hu; `--ocr-languages` shows which models are installed and how to add the missing ones; the GUI warns about missing ones. Recognition is done by Tesseract when installed, else by the built-in native engine (see above); the training and benchmark tools are in `tools/ocr-train` |
| Full-text search | `legatorocr --search "text" FOLDER-OR-FILE…` finds a phrase in PDF text layers (LegatorOCR, ABBYY FineReader, PDF24, Word …), `.txt`, `.hocr`/`.html`, `.docx` and inside archives (`.zip`, `.tar`, `.tgz`, and `.7z`/`.rar` with an extractor installed); ignores case and diacritics (`mobila` finds `Mobilă`, also across a line break); `--search-regex`, `--search-exact`, `--search-max N`; exit status 1 when nothing is found. Scans without a text layer are reported so you know they still need `--ocr` Wildcards (`mobil?`, `mob*`) and proximity (`mobila NEAR/5 veche`) work in plain queries; `--search-preset NAME` searches ready-made patterns (Romanian CUI, IBAN, invoice number, e-mail, phone, date, amount; list with `--search-help`). `--search-old-orthography` also matches the Romanian spelling of 1953-1993 (the Academy's rules of the 1953 decision, in force until its decision of 17 February 1993: `î` inside words instead of `â`, `sînt` instead of `sunt`; `român` with `â` came back already in 1964; `sunt` finds `sînt`, `pânză` finds `pînză`; `î` inside a word is read as `â`, `ş ţ` as `ș ț`; books under the earlier (pre-1948) and later (post-1993) norms already use `â`/`sunt`, so the rewrite leaves them alone; the search as typed still runs). `--ocr-modern-spelling` writes OCR output of such books in today's spelling instead. |
| PDF input | own tolerant reader: pages of scanned PDFs become `book.pdf#N` (Flate/LZW/ASCII85/RunLength, PNG/TIFF predictors, JPEG, CCITT G4, Gray/RGB/CMYK/Indexed, /Rotate, DPI from the page size); object streams and damaged cross-reference tables are fine; **layered / scanner-MRC pages** (several images placed by their transformation matrices, stencil masks in the fill colour, form XObjects) are composited at the sharpest image's resolution; CCITT **Group 3** (1-D, 2-D and byte-aligned) and **JBIG2 generic regions** (arithmetic templates 0–3 with adaptive pixels and typical prediction, MMR) are decoded; **encrypted files** (RC4 40/128, AES-128, AES-256 R5/R6; empty password, or the user or owner password given with `--pdf-password` / the GUI field) are opened natively; JBIG2 **symbol dictionaries and text regions** (arithmetic coding) and **rotated/skewed placements** are handled; **JPEG 2000** (JP2 or raw codestream: tiles, all five progression orders and progression order changes (POC), layers, precincts, 5/3 and 9/7 wavelets, RCT/ICT, all six code-block style bits incl. selective arithmetic bypass and termination on every pass, PPM/PPT packed packet headers) and JBIG2 **generic refinement, refinement/aggregate symbols and halftone regions** are decoded natively; **JBIG2 Huffman coding** (standard tables B.1–B.15, custom tables, symbol dictionaries incl. MMR collective bitmaps and refinement/aggregate symbols, text regions incl. per-instance refinement) is decoded natively; what is still outside the built-in reader (JPEG 2000 palettes and the high-throughput code-block style; empty refined symbols in Huffman aggregate dictionaries) is rendered through an installed `pdftoppm`, `mutool` or Ghostscript when one exists, otherwise refused with a clear message |
| Profiles | `--profile NAME` / `--list-profiles` / `--save-profile NAME` (14 built in: doc-birou, schite-desene, registre-manuscrise, book-bw, book-small-color, document-text, photo-archive, newspaper, form-dropout-red, negatives, flux-1-optim, flux-2-draft, flux-3-volum, phone-photos; user profiles in the config folder) and a GUI selector |
| Page-size check | `--report-outliers` lists pages whose size differs clearly from the rest (median/MAD) |
| Diagnostics | `--doctor` (which helpers are installed and what a missing one costs), `--timings` (time per stage, peak memory), `--explain` (per page: rotation, split, skew, blank, content box, dpi), `--quality-report` (possible blur, low dpi, cut-off text, near-duplicates, page-number gaps; thresholds are untuned heuristics) |
| OCR options | `--page-lang 1-40=ron,41-44=eng+fra` (language per input page), `--verify-text` (reads the finished PDF's text layer back and compares it with the OCR text), `--clean-text` (Romanian comma-below, de-hyphenation, quotes, spacing), `--alto FILE` and `--page-xml DIR` (library XML formats, checked for well-formedness only), `--skip-text` (leave out PDF pages that already have text) |
| Library | `--bookmarks-headings` (PDF bookmarks from page headings; heuristic), `--tables-xlsx FILE` and `--tables-html FILE` (tables found on the pages as an Excel workbook, one sheet per table, or HTML), `--page-labels` (printed page numbers become the viewer's page labels), `--catalog FILE` (HTML / CSV with SHA-256) and `--dublin-core` (sidecars; need `--pdf`), `{title}` in output names (first heading of the OCR text, needs `--ocr`), `--search-query` (boolean, fuzzy, diacritic-insensitive; CUI and invoice-series finders) |
| Archive | `--merkle-root SHA256SUMS.txt` (one root for a whole collection; `--merkle-proof` / `--merkle-verify` prove a file belongs to it; no timestamp service is called), `--provenance` (JSON record in the PDF: version, time, input hashes, options; not a signature), `--certify` (reads the finished PDF back and checks every page: count, size, thumbnail), `--bagit DIR` and `--verify-bag DIR` (BagIt 1.0 packaging and check), `--rights-death-year YEAR` (rights notice from the author's death year, Romanian law; informational), `--timestamp FILE --tsa-url URL` / `--timestamp-verify FILE --tsa-ca PEM` (RFC 3161 time stamp of any file, e.g. the Merkle root; you choose the authority; checked with OpenSSL, not yet tried against a public authority), `--sign-pdf FILE --sign-p12 P12` / `--verify-pdf-signatures FILE` (CAdES signature with your own certificate as an incremental update; password from an environment variable; invisible, no timestamp or LTV inside) |
| PDF toolbox | On existing PDFs, without re-rendering: `--merge-pdf`, `--extract-pages 1-3,7`, `--split-pdf-every N`, `--watermark TEXT`, `--number-pages "{n} / {total}"`. Pages are copied as they are; outlines, forms and tags are dropped; encrypted files are refused. Also `--max-memory MB` (fewer pages at once) and `--shard I/N` (split a big batch across machines, join with `--merge-pdf`), `--djvu FILE` [`--djvu-thumbnails`] (bundled DjVu: lossless JB2 for black & white, layered JB2 + IW44 for colour, OCR text layer; checked with DjVuLibre), `--ocfl DIR` [`--ocfl-id ID`] and `--verify-ocfl DIR` (the run's files as version v1 of an OCFL 1.1 object with a SHA-512 inventory; the checker reads only objects this program wrote and the layout was not run through the reference validator), `--proof-sheet FILE.html` (with `--ocr`: page pictures next to editable text, unsure words listed, a button saves the corrected text with pages separated by a form feed; the way to make a hand-checked reference text, corrections are not checked), `--doubtful-words FILE.tsv` (with `--ocr`: words outside the word list and read with low confidence, for a person to check first; text not changed), `--verify-document` (QR contents and copied regions in a picture; hints only), `--fuse-frames OUT.png` (merge several photos of one page: aligned, median; helps with strong noise, neutral otherwise), `--mrz` (read passport / ID-card machine-readable zone from the OCR text and check its check digits; printed zone only), `--plugin FILE.wasm` (image filter in WebAssembly: no imports, instruction budget, run on every loaded page; ABI in `crates/legatorocr-engine/src/plugin.rs`), `--isolate-decoders` (decode each input in a limited worker process; not a security sandbox), `--archive-package DIR` (reprocessable folder: originals, project, OCR words with confidence, hashes, how to re-run), `--embed-project` (with `--pdf`: the PDF also holds the original inputs byte for byte and `project.lgr` as embedded files, `/AFRelationship /Source`; opt-in, the file grows by the size of the originals; with `--pdfa` needs `--pdfa-part 3`) and `--extract-project PDF DIR` (writes them to a new folder; `--project DIR/project.lgr` then processes again), `--tagged [LANG]` (tagged PDF: paragraph structure from OCR lines, language; base for screen readers, not a PDF/UA claim), `--tagged-pdf` (with `--ocr`: structured tagging, one element per OCR paragraph in reading order, the page heading from the `--bookmarks-headings` detector as `/H1`..`/H3`, a marked-content id per element and a `/ParentTree`, `/Lang` from `--lang` (`ron` gives `ro`), a document `/Title` (the output file name) and `DisplayDocTitle`; no figures, lists or tables; self-checked with `scripts/check-tagged-pdf.py`, **not validated as PDF/UA**), `--pdfa-part 1|2|3` |
| e-Factura | `--efactura FILE` reads a Romanian e-Factura invoice (UBL XML or ANAF's ZIP) and prints the key fields; it checks that the amounts add up and that CUI check digits are valid. It does not validate against the official schematron or check signatures |
| Backup | `--backup-from DIR --backup-to D1 --backup-to D2 [--backup-read-only]`: copies a folder to each destination, reads every copy back and compares SHA-256, writes `SHA256SUMS.txt`; it checks bytes only, whether the copies are on different media or off site is yours to arrange |
| Passage export | `--passages-jsonl FILE` (needs `--ocr`; optional `--passages-title TEXT`, `--page-labels`, `--clean-text`): one JSON object per paragraph, for citation-grounded research tools; schema in "Passage export" below, checked by `scripts/check-passages.py`. Import into any particular tool is not checked |
| TEI | `--tei FILE` (needs `--ocr`): TEI P5 XML with a page break per scanned page, paragraphs, tables and columns; page structure only |
| FFF envelope | `--fff-out FILE.fff.json` (needs `--ocr`; optional `--fff-title`, `--author`, `--fff-year`): the recognised text per page and the SHA-256 of the scan as an envelope of type `document/ocr`, the first step of the offline source chain LegatorOCR -> TEZAUR (`tezaur ingest --fff-in`) -> VERIGA (`veriga --fff-in … --fff-out`) -> AcademiSearch (Library -> Receive). Format: `docs/FFF-ENVELOPE.md` and `docs/FLUXURI.md` in github.com/chiuta/chiuta.github.io. The title is the first heading of the OCR text unless given; OCR errors travel with the text |
| Sharpness | `--measure-edge IMAGE [--edge-roi X,Y,W,H] [--dpi N]`: MTF50 / MTF10 from a slanted black / white edge in a scan or test target. Edge only: no target detection, colour or noise measurement, no FADGI rating |
| MCP server | `legatorocr --mcp` speaks the Model Context Protocol on stdin/stdout, so an AI assistant can call `scan_to_pdf`, `pdf_toolbox`, `efactura`, `measure_edge` and `search`. The assistant gets the file access of your account through these tools; only start it for a client you trust. Speaks protocol revision 2024-11-05 only (the newest revision, 2026-07-28, removes the `initialize` handshake; a client that also supports older revisions falls back and works, a client that supports only the newest one will not; see `docs/audit/standards.md`). Tried with the official MCP Python client (SDK 2.3.0), not with Claude Desktop |
| Redaction | `--redact email,phone-ro,iban-ro,cui,date` or `--redact-regex RE` with `--ocr --format none`: matching words are painted black in the page and removed from the text layer and text exports. Only what OCR read is found, so look at the result before handing it over |
| Fillable forms | `--form-fields` with `--pdf`: ruled boxes, check boxes and underlines of a scanned paper form become fillable PDF fields over the scan. Tried on a rendered test form, not on real scanned forms; fields are named field_N, not after the printed labels |
| Binarisation by OCR | `--binarize-by-ocr` (with `--ocr`, black-and-white modes): each page is read with the chosen threshold and with Otsu, the version Tesseract is more confident about is kept. On 12 real Romanian pages: 5438 -> 5245 character errors (-3.5 %); about twice the time for pages under 93 % confidence |
| Retry of doubtful pages (not recommended) | `--auto-retry` (with `--ocr`; `--auto-retry-below FRACTION`): a page whose read-back looks doubtful is processed again with other binarisers / deskew flipped / another OCR dpi and the alternative with the best certify-style score is kept; `--explain` says which won. **Tested and rejected:** on 12 real pages it changed nothing at the default threshold (2803 -> 2803 character errors) and +25 errors when forced on every page; on 8 synthetic degraded pages it raised errors from 59 to 424 (a skewed page went from 0 to 354 errors). It is off by default and kept only as an experiment; do not expect a benefit (ROADMAP 4.24) |
| Intake | `--ocr-cache DIR` (OCR results remembered per page image and settings; a re-run skips OCR for unchanged pages), `--watch DIR` (hot folder: each new scan is processed once it stops growing; `--pdf` needs `{name}`), `--burst` (phone photos ordered by EXIF capture time, sharpest frame of each burst kept; lists what it dropped), `--scan-active` (lists PDFs with JavaScript, launch actions, embedded files; a static look, a clean result is no guarantee) |
| OCR | word boxes from the built-in engine or Tesseract → invisible text layer (render mode 3, glyph-less Type0 font + ToUnicode, so ăâîșț and any Unicode copy/search correctly) |

Export streams in bounded batches (process → OCR → write), so a several-hundred-page colour book
does not have to fit in memory.

Crates: `legatorocr-core` (pipeline) · `-pdf` (writer) · `-ocr` (engine choice, Tesseract driver, hOCR / JSON output) · `-nocr` (built-in OCR engine) ·
`-engine` (shared export orchestration) · `-cli` · `-gui` (egui/eframe).

Quality gates: see `FUZZING.md` (10 fuzz/property suites, run weekly in CI with qpdf/poppler cross-checks).

## Passage export

`--passages-jsonl FILE` (with `--ocr`) writes JSON Lines: UTF-8, one object per line, one line per
paragraph of every kept page, pages in output order, paragraphs in the engine's reading order (on
a two-column page the boxes of the two columns are therefore listed column by column and may
overlap). The paragraphs are the ones `--docx` uses (`words_to_paragraphs`), not a second
detection. Every key is always present; a value that is unknown is `null`.

| Key | Type | Meaning |
|---|---|---|
| `schema` | string | always `"legatorocr.passage/1"` |
| `passage_id` | string, 16 lower-case hex | first 16 digits of SHA-256 of `"<page_image_sha256>:x0,y0,x1,y1"` (the `bbox_px` numbers); the same page image and box always give the same id, unique in the file |
| `source_file` | string | file name of the input (with `#N` for page N of a multi-page input) |
| `page` | integer >= 1 | page number in the output (after blank-page and duplicate removal; a split spread counts two) |
| `page_label` | string or `null` | printed page number, only with `--page-labels` and only where its run rule would label the PDF page; otherwise `null` |
| `paragraph_index` | integer >= 0 | 0, 1, 2, ... within the page |
| `text` | string, not blank | the paragraph; lines separated by `\n`, words by a space. With `--clean-text` the clean-up is applied to the paragraph, so the text is what `--txt` shows |
| `bbox_px` | 4 integers | `[x0, y0, x1, y1]` in pixels of the output page image, origin top left, `x1`/`y1` exclusive; `0 <= x0 < x1 <= page_width_px`, same for y |
| `bbox_frac` | 4 numbers | the same box divided by page width / height, 6 decimals, within 0..1 |
| `page_width_px`, `page_height_px` | integers | size of the output page image |
| `language` | string | the OCR language setting used for the page (Tesseract codes such as `ron+eng`, honouring `--page-lang`); it is the setting, not a detected language |
| `mean_confidence` | number 0..100 or `null` | mean of the engine's word confidences in the paragraph, 2 decimals; `null` if no word has one |
| `page_image_sha256` | string, 64 lower-case hex | SHA-256 of the page image bytes (see `page_image_basis`) |
| `page_image_file` | string or `null` | name of that image inside `-o` when it was written (`--format png/jpeg/tiff`); `null` otherwise |
| `page_image_basis` | `"file"` or `"png-encoding"` | `"file"`: the hash is of the written image file. `"png-encoding"` (with `--format none`): no image was written; the hash is of the PNG bytes `--format png` would write for this page |
| `cite` | string | `"<title> p.<label>"`: the title is `--passages-title` or the source file name without extension; the label is `page_label`, else `page` |

Check a file (stdlib Python only): `python3 scripts/check-passages.py out.jsonl [--txt out.txt] [--images DIR]`.
It checks every key and type, that each box lies inside its page, the fractions, the id hash, the
`cite` form and, with `--images`, the image hashes against the written files; with `--txt` it
compares the passages of each page with the same page of the `--txt` output (white space
collapsed) and prints the match percentage. The format is LegatorOCR's own; whether a given
research tool (for example TEZAUR) can import it has not been checked.

## Phone capture assistant

`web/capture/index.html` is one offline HTML file (no external requests, no storage, no telemetry; English and Romanian, strings in one object). Open it on a phone (a camera needs https or localhost on most browsers; `file://` works in some), start the camera, lay the page on a darker surface. It finds the bright page (Otsu threshold, largest connected region, corners by extreme x+y / x-y, so up to about 45 degrees of rotation), draws the outline, and shoots by itself when the outline has been still for 0.8 s and the page passes the checks; otherwise it says why: no page found, page not fully visible, too blurry (variance of the Laplacian), too dark, too bright. A manual button, a thumbnail strip with delete, and "Save burst as ZIP" (hand-written stored ZIP) give `burst-YYYYMMDD-HHMMSS.zip` with `page-0001.jpg`, `page-0002.jpg`, ... at the camera's stream resolution (a video frame, not necessarily the sensor's full still resolution). Each JPEG carries an EXIF capture time, at least 4 s after the previous one, so that `legatorocr --burst` (default `--burst-gap 3`) keeps the order and does not take two pages for one burst. Then: `legatorocr --burst --pdf book.pdf page-*.jpg`.
Tested only on **synthetic** frames in headless Chromium with a fake camera (`scripts/test-capture-assistant.py`): corner error 1.0-2.1 px at 640 wide on 6 placements (rotation -14 to +31 degrees, shifts, one keystone), the refusals fire with the right reason, the ZIP passes `zipfile.testzip` and `unzip -t`, and `--burst` accepts the unzipped pages. **Not tested:** real phones, real lighting and shadows, motion blur, autofocus, iOS Safari, a bright or white desk (the page then merges with the background), the "too bright" refusal. All thresholds are untuned guesses.

## Integrity hints (`--integrity-hints`)

**HIGH false-alarm risk. Read this before using it.** `--integrity-hints FILE.json` (opt-in, runs on the input pictures as they are, before any cleaning) writes, per page, boxes in page pixels that a person may want to look at again, each with a plain-language "what it means" and "common innocent causes". It is a list of things *to check*, never a verdict; the output does not say that a document is genuine or not. Do not use it as the only basis for any decision about a person or a person's diploma; ask the issuing institution.

Signals (`--integrity-signals LIST`; `all`, `default`, or names):

| Signal | Looks at | Runs by default |
|---|---|---|
| `copy_move` | identical 16x16 blocks (hashed, blank paper and glyphs repeated more than a few times ignored) at two places, at any pixel offset; boxes at least 64 px on both sides | **yes** |
| `jpeg_grid` | JPEG inputs: a 128 px window whose 8x8 block-edge pattern is offset from the rest of the page | no: 4 of 12 unedited pages flagged, 0 of 60 edits found |
| `jpeg_double` | JPEG inputs: comb pattern of a second save (the `--verify-document` statistic, whole page) | no: no false alarm, but 0 of 60 edited pages flagged in the test below |
| `noise` | tiles whose fine noise is 3x the page median | no: 10 of 12 unedited pages flagged |
| `line_size` | needs `--ocr`: a word whose height or position is out of line with its text line | no: 12 of 12 unedited pages flagged |

Measured on **synthetic** edits only (12 real pages from the OCR test set, JPEG q85; edits: a digit string drawn over a word, a 220x140 patch copied elsewhere, a 260x180 patch from a q45 copy pasted at an odd offset, gaussian noise in 240x160; each edited page re-saved at q85; and the clean pages saved once, re-saved once at q85, re-saved again at q75; `docs/experiments/integrity_eval.py`, ROADMAP 7.5c). `copy_move`: found 10 of 12 copied patches, 0 of 36 unedited/re-saved pages flagged; it does **not** find a digit or a word re-typed, and it misses patches smaller than 64 px. The size limit was set after seeing false alarms on the same pages, so that figure is optimistic. These numbers say nothing about real forgeries, which are made with other tools, other fonts and other compression; real stamps, repeated logos and letterheads were not in the test and are the expected source of false alarms. The other signals are available with `--integrity-signals all` for research only.

## Known limits

See `COMPARISON.md` for what other tools do that this one does not. In short:

- **Orientation**: the built-in test only finds "page lies on its side", not *which way*. **Orientation via OCR**
  (GUI switch / `--auto-rotate`) resolves 90° vs 270° and upside-down pages: with Tesseract's `osd` data when that is
  installed, otherwise with the built-in engine, which reads a reduced copy in all four turns and keeps the one whose
  words are found in the language word lists (needs about a dozen letters of printed text; otherwise the page is left
  alone). Manual fallback: *Rotate left/right 90°* / `--rotate`.
- Erase zones: typed in millimetres (same on every page) or drawn with the mouse per sheet (drag on the Original view, right-click removes; stored in the project); picture zones are drawn the same way (blue); dewarp guide lines too (freehand, *Guide line* kind; also typed via `--dewarp-guide` / GUI field). The GUI drawing was not visually tested.
  Pictures are also detected automatically
  (sensitivity adjustable) and the content box can be nudged per page (numeric fields or by dragging its green edges in the GUI; on a split spread the nudge applies to both pages).
- Dewarp corrects vertical bending of text lines only (no perspective / horizontal stretch).
- `--pdfa` writes the PDF/A-2b *structure* (embedded glyph-less TrueType text font, XMP packet matching the Info dictionary, embedded sRGB output-intent profile, CIE-based grey, trailer ID). The font, ICC profile and XMP were checked with fontTools, LittleCMS and an XML parser, and the file with qpdf and poppler, and the CI job `PDF/A validation` runs veraPDF (greenfield 1.30.x, PDF/A-2b profile) over generated sample files — black & white, mixed, hybrid, grey, with and without an OCR layer, one and two pages — and fails when any is rejected. That is a test of those samples, **not a certification** of every possible output. PDF input: JBIG2 Huffman decoding checked against pdfium on generated streams (the MMR collective-bitmap variant only against our own G4 decoder, pdfium renders it blank); Huffman refinement/aggregation (text-region refinement, refined and aggregate symbols; custom tables) checked against poppler and, except for refinement offsets RDW/RDH beyond -2..1, pdfium (pdfium centres the reference with RDW>>2 instead of RDW>>1 in Huffman text regions, and mis-decodes the same-size, zero-offset case; poppler and our decoder follow the standard), with empty refined symbols in Huffman aggregate dictionaries refused; JPEG 2000 code-block styles incl. bypass/termall, POC, PPM and PPT were checked against OpenJPEG 2.5 on about 1,350 generated streams (lossless exact, 9/7 within 2 grey levels; PPM/PPT streams were produced by rewriting OpenJPEG's, which OpenJPEG then decoded identically), JPEG 2000 palettes and the high-throughput (Part 15) code-block style unsupported, JBIG2 typical prediction for refinement regions (TPGRON) and shifted halftone grids verified only against our own encoder (poppler decodes them differently); encrypted files: user and owner passwords were checked against qpdf-made files (RC4-40/128, AES-128, AES-256 R5/R6); bar codes: Code 128 (sets A/B/C), EAN-13, ITF, Code 39 are read, only Code 128 is printed. JBIG2 and Group 3 were verified against poppler / libtiff output; layered-page compositing only against our own writer's pages.
- Perspective correction needs a page that is clearly lighter than its surroundings and shows all four corners; otherwise it leaves the photo alone. It does not unroll curved pages (use `--dewarp`).
- Skin/hole/finger removal are heuristics: `--fingers` is off by default and can eat skin-toned photographs at the page edge.
- DPI is read from PNG/JPEG headers (150–1200 dpi; 50–149 only when the implied page size is plausible, and never 72/96, which editors write by default) but not from TIFF headers: set `--dpi` by hand when in doubt; 16-bit TIFF is reduced to 8 bit.
- Skew: pages with almost no ink are left as they are, and when the text rows and the vertical structure both give a clear but different angle (oblique drawings, hatching) the page is not rotated. Pages that are already single and straight should be processed with `--split off`.
- A dark scanner-bed border *touching* the text can still enlarge the detected content box
  (borders and corner shadows separated from the text are removed).
- Windows and macOS builds come from the CI workflows (`.github/workflows`); the author's
  development environment verified Linux only.
- Releases: **Actions → Release → Run workflow** (enter a tag such as `v0.2.0-pre.1`), or push a `v*`
  tag. The workflow first runs the whole CI matrix, then builds all five platforms and opens a
  **draft** pre-release for review; nothing is published until you press Publish.

## Credits

**Co-author: [Alexandru Dumbravă](https://competentedigitalizare.blogspot.com/)** ([portfolio and digitisation guide](https://dumbravaalex.github.io/Portofoliu/), ORCID 0009-0008-4896-4489; HUB DIGITAL — digitisation, OCR and archiving
workshop). The three ready-made digitisation flows (`flux-1-optim`, `flux-2-draft`, `flux-3-volum`: optimal quality at
300 dpi / quick draft / high volume), the full-text search over scans (the job Agent Ransack and DocFetcher do in his
workflow), the editable Word export (what ABBYY FineReader is used for) and the attention to Romanian diacritics follow his
published workflow and tool recommendations. Only the *workflow ideas* were taken; no code of ABBYY, Agent Ransack, DocFetcher
or PDF24 was consulted.

## Clean-room statement

Algorithms were written from general image-processing knowledge (Sauvola, projection profiles,
connected components) and the publicly described workflow. For `COMPARISON.md` only public
feature lists and release notes of other tools were read; no source code of any existing
scan-processing tool was consulted or copied. Before commercial release, do your own due
diligence, including a trademark search for the name.

MIT licensed. Dependencies and embedded fonts: see `THIRD_PARTY_LICENSES.md`. The built-in OCR engine ships word lists
derived from the *wordfreq* data (**CC BY-SA 4.0**, attribution in `crates/legatorocr-nocr/lexicons/README.md` and in the release
archives as `LEXICONS-NOTICE.md`); build with `--no-default-features` to leave them out.
