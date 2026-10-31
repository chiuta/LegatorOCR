# Fuzzing and quality gates

Everything is deterministic (seeded xorshift), so a failure reproduces with the same
`LEGATOROCR_FUZZ_SEED`. Iteration counts scale with `LEGATOROCR_FUZZ_ITERS`; plain
`cargo test` runs a short version of each, the weekly `Fuzz` workflow a long one.

| Target | File | What is thrown at it | What must hold |
|---|---|---|---|
| Whole pipeline | `core/tests/fuzz_extreme.rs` | 16 hostile image kinds (1×1 … 2×4000 lines, noise, salt-pepper, frames, checkerboards, photo blocks…), 4 input formats, every setting at NaN/±inf/±1e30/0/u32::MAX | no panic, page 1 ≤ 2 sheets, sane dimensions, layers consistent with the page, `measure_page` agrees with `process_page`, output bit-identical on a re-run |
| Building blocks | same | all six binarisers with hostile window/k, rotate/crop/downscale/normalise with extreme arguments, picture/gutter/skew/orientation detectors, bit-image ops | no panic, no absurd allocation |
| Quality properties | `core/tests/metamorphic.rs` | rotate by a known angle, re-process, change lighting, turn by 90°/270° | skew recovered within 0.25° (measured 0.04°), second pass ≈ no-op, margins exact ±4 px, orientation direction-independent, B/W invariant to vignetting |
| Image loaders | `engine/tests/fuzz_inputs.rs` | PNG/JPEG/BMP/TIFF/G4-TIFF/multi-page TIFF mutated byte-wise (flips, truncation, zero/FF runs, extreme 32-bit fields, wrong extensions) | `Err` or a bounded image; never a panic or > 400 MP |
| Project / Tesseract parsers | same | mutated and random-byte `.lgr`, TSV, OSD, language strings, file names | accepted input is sanitised and re-serialisable; `natural_cmp` is antisymmetric |
| Export engine | `engine/tests/fuzz_export.rs` | batches of good / corrupt / empty / missing sheets, random options, cancel at a random moment | valid PDF with the right `/Count`, or a clean error; progress never exceeds the sheet count |
| JBIG2 / G3 | `pdf/src/jbig2.rs`, `g4.rs` tests | encoder↔decoder round trips for all four templates, TPGDON, custom AT pixels, 3000 mutated streams; libtiff-written Group 3 (1-D, 2-D, byte-aligned) bit-exact, 2000 mutations | decoded bitmaps equal the source; `pdfimages` (poppler) decodes our JBIG2 to the same bitmap |
| Bar codes | `core/src/barcode.rs` tests | round trip, blur, 180° turn, embedded in a page, noise, python-barcode module strings | exact decode / `None` |
| PDF writer | `pdf/tests/fuzz_pdf.rs` | every page type incl. layered, hostile OCR words (10-digit coordinates, NUL, RTL, astral/CJK, 500-char words, > 64k distinct glyphs), odd titles | own xref/structure check, `qpdf --check`, `qpdf --qdf` content scan (no NaN/inf/absurd numbers), `pdftoppm` without syntax errors, `pdftotext` runs |
| CCITT G4 | `pdf/src/g4.rs` | pages of width 1, 63–65, 1790–1794, 2558–2563, 5000+, long runs, noise | an independent decoder written from the spec reproduces every pixel; poppler agrees; TIFF wrapper well-formed |
| Command line | `scripts/fuzz_cli.py` | random flag combinations with hostile values against good, corrupt, empty, missing and multi-page inputs | exit status 0/1/2 only, no panic message, no signal, no timeout |

Run locally:

```
LEGATOROCR_FUZZ_ITERS=3000 cargo test --release -p legatorocr-core --test fuzz_extreme
LEGATOROCR_FUZZ_ITERS=200  cargo test -p legatorocr-core --test fuzz_extreme      # debug: overflow checks
LEGATOROCR_FUZZ_ITERS=60000 cargo test --release -p legatorocr-engine --test fuzz_inputs
python3 scripts/fuzz_cli.py --runs 400 --seed 1
```

Defects this suite has found so far (all fixed, each with a regression test or a permanent fuzz case):
unbounded skew search (`tan` → 37 GB table), unbounded output canvas (margins/page size × dpi),
absurd coordinates in the OCR text layer.

## Added with the clean-up / PDF-input release

- `core/tests/fuzz_extreme.rs` now randomises every clean-up field (invert, levels, sharpen, punch holes, fingers,
  lines, drop-out, despeckle strength) and the `auto` threshold.
- `core/tests/cleanup_e2e.rs`: end-to-end behaviour of each clean-up stage through `process_page`.
- `pdf` reader: byte-level mutation of valid PDFs plus random bodies (`reader::tests::garbage_never_panics`);
  production CCITT G4 decoder checked against the encoder (300 random pages, damaged streams) and against the independent test decoder.
- `engine/tests/pdf_qpdf.rs`: reads files rewritten by qpdf (`--object-streams=generate`, `--qdf`, `--linearize`).
- Round 2: fuzz_extreme also randomises `show_through` and `descreen`; engine tests cover duplicates, blank-separator splitting and target-size quality; OCR upscaling rules are unit-tested.
- Round 3: fuzz_extreme randomises `perspective`; perspective module tested on a synthetic keystoned photo, upright, full-frame and dark inputs; multi-page TIFF checked with the `tiff` crate, Pillow and ImageMagick.
