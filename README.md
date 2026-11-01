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
`--dewarp` (straighten curved lines), `--output-dpi N`, `--uniform-size`, `--align top-left`,
`--project FILE.lgr` / `--save-project FILE.lgr`. Multi-page TIFFs are read page by page
(`scan.tif#3` addresses one page).

`--skip-blank`, `--skew DEG` (fixed correction), `--rotate 90`, `--pdf-quality N` (0 = lossless),
`--format png|jpeg|tiff|none`, `--tessdata DIR`, `--psm N`, `--min-conf N`, `--keep-going` (skip unreadable
inputs, exit status 2) — see `legatorocr --help`. A folder can be given instead of files. Resolution is
read from the PNG/JPEG header when it says 150–1200 dpi (otherwise 300; override with `--dpi`); camera
EXIF rotation is applied and transparency is flattened onto white. Out-of-range numbers are rejected
with a clear message rather than producing absurd page sizes.

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
word lists. Its model is trained on synthetic text only: on clean and moderately degraded synthetic
pages it makes under 1 % character errors, but real scans of old or unusual print can be worse than
Tesseract. `--engine auto` (default) therefore uses **Tesseract when it is installed with the
requested languages** and falls back to the native engine otherwise; `--engine tesseract` forces it.
Tesseract (run as a separate program; the app finds it on `PATH`):
`sudo apt install tesseract-ocr tesseract-ocr-ron` · `brew install tesseract tesseract-lang` ·
Windows: `winget install UB-Mannheim.TesseractOCR` (add it to `PATH`).
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
| Delivery | `--checksums` writes `SHA256SUMS.txt` (same format as `sha256sum`) next to the PDF for every file written; `--verify-sums SHA256SUMS.txt` re-checks them later (exit 1 on any mismatch or missing file); `--rights-notice "…"` stores a rights statement (XMP `dc:rights` with `--pdfa`, otherwise the `/Rights` info key and a note icon on page 2); `--txt-each` writes `<pdf name>.txt` beside the PDF for indexers such as DocFetcher / Agent Ransack; `--package FOLDER`, `FILE.zip` or `FILE.iso` (ISO 9660 + Joliet, readable on Windows / macOS / Linux; no Rock Ridge or boot data; files up to 4 GiB) gathers everything written plus `SHA256SUMS.txt` and a `MANIFEST.txt` for handing over (never overwrites; zip limited to 3 GiB) |
| Phone photos | `--phone-paper A4` (or `A5`, `B5`, `Letter`, `210x297` …) derives the resolution of photographs that have none from the paper size, assuming the shot is framed on the page; an estimate, off by the margin left around the page |
| OCR sanity | With `--lang ro`, a warning when the recognised text has almost no ăâîșț or many letter-digit words (typical of a missing Romanian model); a note when `--dpi` is below 200 or above 400 with `--ocr` |
| Picture zones | Always kept as a colour picture in mixed / hybrid output, for photos the detector misses. Drawn with the mouse in the GUI (tick *Draw zones*, choose *Picture*; blue; Undo / Redo shared with erase zones; saved per sheet in the `.lgr` project as `picture =`): they are followed through rotation, page split and deskew, so they stay on the picture. Not followed through perspective correction (then dropped) or dewarping (position approximate). Typed: `--picture-zone X,Y,W,H` (mm of the deskewed page, repeatable), GUI field, `picture_zones=` |
| Text output | `--txt` (plain text), `--hocr` (word boxes + confidences), `--json` (the same as structured JSON: per page size and dpi, per word text, box, confidence, line), `--docx` (editable Word document: paragraphs recovered from the line spacing, one page break per scanned page), PDF author/subject/keywords/dates. With `--lang ron` the cedilla letters ş/ţ are corrected to the proper comma-below ș/ț |
| Archives as input | Give `.zip`/`.cbz`, `.tar`, `.tar.gz`/`.tgz` (built in) or `.7z`, `.rar`/`.cbr`, `.cab`, `.iso`, `.tar.bz2`, `.tar.xz` … (through an installed 7-Zip, unrar, bsdtar or unar) instead of a folder: the images and PDFs inside are processed in natural order of their names, in the CLI and the GUI (add / drag and drop), and are searched by `--search`. Extraction goes to a temporary folder that is removed on exit; names are flattened and sizes capped |
| OCR languages | `--lang` takes Tesseract names or two-letter codes (`ro+en` = `ron+eng`) for the eight interface languages ro, en, fr, it, es, pt, de, hu; `--ocr-languages` shows which models are installed and how to add the missing ones; the GUI warns about missing ones. Recognition is done by Tesseract when installed, else by the built-in native engine (see above); the training and benchmark tools are in `tools/ocr-train` |
| Full-text search | `legatorocr --search "text" FOLDER-OR-FILE…` finds a phrase in PDF text layers (LegatorOCR, ABBYY FineReader, PDF24, Word …), `.txt`, `.hocr`/`.html`, `.docx` and inside archives (`.zip`, `.tar`, `.tgz`, and `.7z`/`.rar` with an extractor installed); ignores case and diacritics (`mobila` finds `Mobilă`, also across a line break); `--search-regex`, `--search-exact`, `--search-max N`; exit status 1 when nothing is found. Scans without a text layer are reported so you know they still need `--ocr` Wildcards (`mobil?`, `mob*`) and proximity (`mobila NEAR/5 veche`) work in plain queries; `--search-preset NAME` searches ready-made patterns (Romanian CUI, IBAN, invoice number, e-mail, phone, date, amount; list with `--search-help`). |
| PDF input | own tolerant reader: pages of scanned PDFs become `book.pdf#N` (Flate/LZW/ASCII85/RunLength, PNG/TIFF predictors, JPEG, CCITT G4, Gray/RGB/CMYK/Indexed, /Rotate, DPI from the page size); object streams and damaged cross-reference tables are fine; **layered / scanner-MRC pages** (several images placed by their transformation matrices, stencil masks in the fill colour, form XObjects) are composited at the sharpest image's resolution; CCITT **Group 3** (1-D, 2-D and byte-aligned) and **JBIG2 generic regions** (arithmetic templates 0–3 with adaptive pixels and typical prediction, MMR) are decoded; **encrypted files** (RC4 40/128, AES-128, AES-256 R5/R6, empty user password) are opened natively; JBIG2 **symbol dictionaries and text regions** (arithmetic coding) and **rotated/skewed placements** are handled; **JPEG 2000** (JP2 or raw codestream: tiles, all five progression orders, layers, precincts, 5/3 and 9/7 wavelets, RCT/ICT) and JBIG2 **generic refinement, refinement/aggregate symbols and halftone regions** are decoded natively; what is still outside the built-in reader (JBIG2 Huffman coding, JPEG 2000 bypass/termall code-block styles, POC/PPM/PPT) is rendered through an installed `pdftoppm`, `mutool` or Ghostscript when one exists, otherwise refused with a clear message |
| Profiles | `--profile NAME` / `--list-profiles` / `--save-profile NAME` (14 built in: doc-birou, schite-desene, registre-manuscrise, book-bw, book-small-color, document-text, photo-archive, newspaper, form-dropout-red, negatives, flux-1-optim, flux-2-draft, flux-3-volum, phone-photos; user profiles in the config folder) and a GUI selector |
| Page-size check | `--report-outliers` lists pages whose size differs clearly from the rest (median/MAD) |
| OCR | word boxes from the built-in engine or Tesseract → invisible text layer (render mode 3, glyph-less Type0 font + ToUnicode, so ăâîșț and any Unicode copy/search correctly) |

Export streams in bounded batches (process → OCR → write), so a several-hundred-page colour book
does not have to fit in memory.

Crates: `legatorocr-core` (pipeline) · `-pdf` (writer) · `-ocr` (engine choice, Tesseract driver, hOCR / JSON output) · `-nocr` (built-in OCR engine) ·
`-engine` (shared export orchestration) · `-cli` · `-gui` (egui/eframe).

Quality gates: see `FUZZING.md` (10 fuzz/property suites, run weekly in CI with qpdf/poppler cross-checks).

## Known limits

See `COMPARISON.md` for what other tools do that this one does not. In short:

- **Orientation**: the built-in test only finds "page lies on its side", not *which way*. **Orientation via OCR**
  (GUI switch / `--auto-rotate`) resolves 90° vs 270° and upside-down pages: with Tesseract's `osd` data when that is
  installed, otherwise with the built-in engine, which reads a reduced copy in all four turns and keeps the one whose
  words are found in the language word lists (needs about a dozen letters of printed text; otherwise the page is left
  alone). Manual fallback: *Rotate left/right 90°* / `--rotate`.
- Erase zones: typed in millimetres (same on every page) or drawn with the mouse per sheet (drag on the Original view, right-click removes; stored in the project); picture zones are drawn the same way (blue); dewarp control points cannot be drawn.
  Pictures are also detected automatically
  (sensitivity adjustable) and the content box can be nudged per page (numeric fields or by dragging its green edges in the GUI; on a split spread the nudge applies to both pages).
- Dewarp corrects vertical bending of text lines only (no perspective / horizontal stretch).
- `--pdfa` writes the PDF/A-2b *structure* (embedded glyph-less TrueType text font, XMP packet matching the Info dictionary, embedded sRGB output-intent profile, CIE-based grey, trailer ID). The font, ICC profile and XMP were checked with fontTools, LittleCMS and an XML parser, and the file with qpdf and poppler, and the CI job `PDF/A validation` runs veraPDF (greenfield 1.30.x, PDF/A-2b profile) over generated sample files — black & white, mixed, hybrid, grey, with and without an OCR layer, one and two pages — and fails when any is rejected. That is a test of those samples, **not a certification** of every possible output. PDF input: no native JBIG2 Huffman coding (external-renderer fallback only), JPEG 2000 bypass/termall code-block styles unsupported, JBIG2 typical prediction for refinement regions (TPGRON) and shifted halftone grids verified only against our own encoder (poppler decodes them differently); only the empty user password is tried; bar codes: Code 128 (sets A/B/C), EAN-13, ITF, Code 39 are read, only Code 128 is printed. JBIG2 and Group 3 were verified against poppler / libtiff output; layered-page compositing only against our own writer's pages.
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
