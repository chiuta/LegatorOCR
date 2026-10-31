# LegatorOCR

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
`--format png|tiff|none`, `--tessdata DIR`, `--psm N`, `--min-conf N`, `--keep-going` (skip unreadable
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

**OCR needs Tesseract** (run as a separate program; the app finds it on `PATH`):
`sudo apt install tesseract-ocr tesseract-ocr-ron` · `brew install tesseract tesseract-lang` ·
Windows: `winget install UB-Mannheim.TesseractOCR` (add it to `PATH`).
The app also looks in the usual install folders (Homebrew `/opt/homebrew/bin`, `/usr/local/bin`,
`C:\Program Files\Tesseract-OCR`), so a double-clicked app works without editing `PATH`.
Without Tesseract everything else works; the OCR switch is disabled and says why.
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
| Separator sheets | `--split-on-barcode` (pages carrying a **Code 128** bar code, upright, turned 180° or sideways, read on the raw scan, start a new document named after the code → `book_001_ACT-17.pdf`), `--make-separator TEXT` prints such a sheet (A4 PNG) |
| PDF navigation | `--bookmarks` (one outline entry per source file), `--front-matter N` (pages i, ii, iii … then 1, 2, 3 in PDF viewers) |
| Erase zones | `--erase X,Y,W,H` (mm from the page's top-left, repeatable): painted with the paper colour before anything else is measured; `erase=` in `.lgr` projects; GUI text field |
| Batch | `--skip-duplicates` (consecutive pages with a 96×128 thumbnail difference ≤ 2/255), `--split-on-blank` (blank pages separate documents → `name_001.pdf`, …), `--target-size MB` (JPEG quality chosen from a sample of pages so the PDF fits) |
| OCR input | pages below 250 dpi are enlarged ×2–×4 (Catmull-Rom) before Tesseract; word boxes are mapped back |
| Phone photos | `--perspective`: finds the page as the large bright quadrilateral (Otsu + extreme corners, with convexity/fill checks) and warps it with a homography; does nothing for upright or full-frame scans |
| QC and archive | `--report r.html` (contact sheet, flags blank / size-outlier / strongly deskewed pages), `--multitiff out.tif` (multi-page Group 4 TIFF, black & white only) |
| Text output | `--txt` (plain text), `--hocr` (word boxes + confidences), PDF author/subject/keywords/dates |
| PDF input | own tolerant reader: pages of scanned PDFs become `book.pdf#N` (Flate/LZW/ASCII85/RunLength, PNG/TIFF predictors, JPEG, CCITT G4, Gray/RGB/CMYK/Indexed, /Rotate, DPI from the page size); object streams and damaged cross-reference tables are fine; **layered / scanner-MRC pages** (several images placed by their transformation matrices, stencil masks in the fill colour, form XObjects) are composited at the sharpest image's resolution; CCITT **Group 3** (1-D, 2-D and byte-aligned) and **JBIG2 generic regions** (arithmetic templates 0–3 with adaptive pixels and typical prediction, MMR) are decoded; encrypted files, JPEG 2000, JBIG2 symbol/text/halftone/refinement coding and rotated/skewed image placements are refused with a clear message |
| Profiles | `--profile NAME` / `--list-profiles` / `--save-profile NAME` (8 built in: book-bw, book-small-color, document-text, photo-archive, newspaper, form-dropout-red, negatives, phone-photos; user profiles in the config folder) and a GUI selector |
| Page-size check | `--report-outliers` lists pages whose size differs clearly from the rest (median/MAD) |
| OCR | Tesseract word boxes → invisible text layer (render mode 3, glyph-less Type0 font + ToUnicode, so ăâîșț and any Unicode copy/search correctly) |

Export streams in bounded batches (process → OCR → write), so a several-hundred-page colour book
does not have to fit in memory.

Crates: `legatorocr-core` (pipeline) · `-pdf` (writer) · `-ocr` (Tesseract driver) ·
`-engine` (shared export orchestration) · `-cli` · `-gui` (egui/eframe).

Quality gates: see `FUZZING.md` (10 fuzz/property suites, run weekly in CI with qpdf/poppler cross-checks).

## Known limits

See `COMPARISON.md` for what other tools do that this one does not. In short:

- **Orientation**: the built-in test only finds "page lies on its side", not *which way*. With
  Tesseract's `osd` data installed, enable **Orientation via OCR (OSD)** (GUI) or `--auto-rotate`
  (CLI): it also resolves 90° vs 270° and upside-down pages (needs enough printed text on the page;
  otherwise it leaves the page alone). Manual fallback: *Rotate left/right 90°* / `--rotate`.
- No mouse-drawn zones: erase zones are typed as millimetres (the same rectangles on every page); picture
  zones and dewarp control points cannot be drawn at all. Pictures are detected automatically
  (sensitivity adjustable) and the content box can be nudged per page.
- Dewarp corrects vertical bending of text lines only (no perspective / horizontal stretch).
- PDF/A output is deliberately not claimed (the glyph-less, non-embedded text font would violate it). PDF input: no JPEG 2000, no JBIG2 symbol dictionaries / text regions (what `jbig2enc -s` and most office scanners produce), no encryption; bar codes: Code 128 only (character sets A/B/C, no FNC/shift codes). JBIG2 and Group 3 were verified against poppler / libtiff output; layered-page compositing only against our own writer's pages.
- Perspective correction needs a page that is clearly lighter than its surroundings and shows all four corners; otherwise it leaves the photo alone. It does not unroll curved pages (use `--dewarp`).
- Skin/hole/finger removal are heuristics: `--fingers` is off by default and can eat skin-toned photographs at the page edge.
- DPI is not read from TIFF headers (set it by hand); 16-bit TIFF is reduced to 8 bit.
- A dark scanner-bed border *touching* the text can still enlarge the detected content box
  (borders and corner shadows separated from the text are removed).
- Windows and macOS builds come from the CI workflows (`.github/workflows`); the author's
  development environment verified Linux only.

## Clean-room statement

Algorithms were written from general image-processing knowledge (Sauvola, projection profiles,
connected components) and the publicly described workflow. For `COMPARISON.md` only public
feature lists and release notes of other tools were read; no source code of any existing
scan-processing tool was consulted or copied. Before commercial release, do your own due
diligence, including a trademark search for the name.

MIT licensed. Dependencies and embedded fonts: see `THIRD_PARTY_LICENSES.md`.
