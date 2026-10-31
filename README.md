# LegatorOCR

Clean-room Rust post-processor for scanned books: **split → deskew → crop →
margins → clean/binarize → PDF with searchable OCR text layer.**
Same *workflow* as classic scan "taylor" tools, independent code.

```
legatorocr scan_001.png scan_002.png … \
    --mode bw --dpi 300 --margin 6 --page-size a5 \
    --pdf carte.pdf --ocr --lang ron+eng
```

## Pipeline

| Stage | Method (all standard, textbook techniques) |
|---|---|
| Orientation | row- vs column-projection variance (0°/90° only) or `--rotate` |
| Page split | gutter = emptiest column band in the central 30 % of a landscape sheet (`--split auto|middle|off`) |
| Deskew | projection-profile sharpness, coarse→fine search to 0.01°, bilinear rotation, ±`--max-skew` |
| Content box | ink blocks → dilation → connected components; dust and scanner-edge shadows rejected, page numbers kept |
| Margins / size | mm margins on all sides, optional fixed `--page-size` with centred content; everything outside the content box is repainted in paper colour |
| Output | `color`, `gray`, or `bw` (Sauvola adaptive threshold + despeckle) |
| PDF | own writer: JPEG (colour/gray), or 1-bit Flate with PNG-Up predictor (B/W); real page size from DPI |
| OCR | Tesseract run as an external process; TSV word boxes → invisible text layer (render mode 3, glyph-less Type0 font + ToUnicode, so ăâîșț and any Unicode copy/search correctly) |

Layout: `legatorocr-core` (pipeline) · `legatorocr-pdf` (writer) · `legatorocr-ocr` (Tesseract driver) · `legatorocr-cli`.

## Install / use

```
cargo build --release          # binary: target/release/legatorocr
sudo apt install tesseract-ocr tesseract-ocr-ron     # for --ocr --lang ron
legatorocr --help
```

`--skip-blank`, `--pdf-quality N` (0 = lossless), `--format png|tiff|none`,
`--tessdata DIR`, `--psm N`, `--min-conf N` are available.

## Not implemented (v0.1)

- **No GUI** and no per-page manual adjustment (everything is automatic; use the
  flags). This is the largest gap versus the classic tools.
- No "mixed" mode (picture zones kept in colour, text in B/W), no page-curl
  dewarping, no 180°/270° auto-detection, no multi-page TIFF input (first page only).
- No CCITT G4 / JBIG2 for B/W PDF pages (Flate+predictor is lossless but larger).
- Scanner-bed borders touching the text can enlarge the content box.

## Clean-room statement

The algorithms were written from general image-processing knowledge (Sauvola,
projection profiles, connected components) and the publicly described workflow.
No source code of any existing scan-processing tool was consulted or copied.
Before commercial release, do your own due diligence (and a trademark search
for the name LegatorOCR).

MIT licensed.
