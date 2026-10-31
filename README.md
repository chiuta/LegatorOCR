# LegatorOCR

Clean-room Rust post-processor for scanned books:
**split → deskew → crop → margins → clean / binarize → PDF with a searchable OCR text layer.**
Same *workflow* as classic scan-tailoring tools, independent code. Graphical interface
(Windows, macOS, Linux) and command line.

## Quick start

**GUI** — run `legatorocr-gui` (or open `LegatorOCR.app` on macOS). Drag scans or a folder into the
window (or *Add*), pick settings on the right, check the **Side by side / Original / Processed**
preview, adjust single pages (rotate, page split, manual skew), then **Export PDF…** or
**Export images…**. Languages: Română, English (follows the system language; switch in the menu).

**CLI**

```
legatorocr scan_*.png --mode bw --dpi 300 --margin 6 --page-size a5 \
    --pdf carte.pdf --ocr --lang ron+eng
```

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
Without Tesseract everything else works; the OCR switch is disabled and says why.

## Pipeline

| Stage | Method (standard, textbook techniques) |
|---|---|
| Orientation | row/column profile energy around the line-pitch band (0° vs a quarter turn) or `--rotate` |
| Page split | gutter = emptiest column band in the centre of a landscape sheet (`auto` / `middle` / `off`) |
| Deskew | projection-profile sharpness, coarse→fine search to 0.01°, bilinear rotation; or a manual angle |
| Content box | ink blocks → dilation → connected components; dust and scanner-edge shadows rejected, page numbers kept |
| Margins / size | mm margins, optional fixed page size with centred content; everything outside the content box is repainted in paper colour |
| Output | colour, gray, or black & white (Sauvola adaptive threshold + despeckle) |
| PDF | own writer: JPEG (colour/gray) or 1-bit Flate with PNG-Up predictor (B/W); true page size from DPI |
| OCR | Tesseract word boxes → invisible text layer (render mode 3, glyph-less Type0 font + ToUnicode, so ăâîșț and any Unicode copy/search correctly) |

Export streams in bounded batches (process → OCR → write), so a several-hundred-page colour book
does not have to fit in memory.

Crates: `legatorocr-core` (pipeline) · `-pdf` (writer) · `-ocr` (Tesseract driver) ·
`-engine` (shared export orchestration) · `-cli` · `-gui` (egui/eframe).

## Known limits (v0.1)

- **Orientation**: auto-detect finds "page lies on its side" but not *which way*; it may leave the
  page upside-down. Use the per-page *Rotate left/right 90°* buttons (GUI) or `--rotate` (CLI).
  180° is not detected automatically.
- **No manual content-box editing** and no per-page margins yet; margins/page size are global.
- No "mixed" mode (picture zones in colour, text in B/W), no page-curl dewarping.
- Multi-page TIFF input: first page only. DPI is not read from TIFF headers (set it by hand).
- B/W PDF pages are lossless Flate, not CCITT G4/JBIG2 (larger files).
- A dark scanner-bed border *touching* the text can still enlarge the detected content box
  (borders and corner shadows separated from the text are removed).
- GUI settings are not persisted between sessions.
- Windows and macOS builds come from the CI workflows (`.github/workflows`); the author's
  development environment verified Linux only.

## Clean-room statement

Algorithms were written from general image-processing knowledge (Sauvola, projection profiles,
connected components) and the publicly described workflow. No source code of any existing
scan-processing tool was consulted or copied. Before commercial release, do your own due
diligence, including a trademark search for the name.

MIT licensed.
