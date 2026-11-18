# Duel: LegatorOCR against OCRmyPDF and plain Tesseract on the same pages (17 November 2026)

**What this is:** a reproducible head-to-head on synthetic Romanian pages, answering one objection of `docs/ADVERSARIAL-ANALYSIS.md`
(the native engine was only compared on the magazine it was fine-tuned on). **What it is not:** a measurement on real scans, phone
photos or other publishers.

Reproduce (needs Tesseract with `ron`, `pip install ocrmypdf rapidfuzz`, optionally `unpaper`, a release build):

```
cd tools/ocr-train
for s in 1 2 3 4 5 6; do
  FONT=/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf FRACTAL=1 DEGRADE=1 python3 makepage.py pages/A$s.png pages/A$s.txt ron 1 300 $s
  FONT=/usr/share/fonts/truetype/google-fonts/Poppins-Regular.ttf       FRACTAL=1 DEGRADE=1 python3 makepage.py pages/B$s.png pages/B$s.txt ron 1 300 $s
done
python3 ../../scripts/duel.py pages --tessdata TESSDATA_DIR --legatorocr ../../target/release/legatorocr
```

## Setup

- 12 pages, 300 dpi, one column of Romanian text with a known reference (`tools/ocr-train/makepage.py`), fractal paper and ink texture (`FRACTAL=1`), slight tilt, blur, noise and specks (`DEGRADE=1`). Group A: Liberation Serif. Group B: Poppins Regular.
- Programs: legatorocr 0.2.0-pre.67 (release build), Tesseract 5.x with `ron` (the same model for every pipeline), OCRmyPDF 17.14.2 (`--force-ocr`; `+unpaper` adds `--deskew --clean`, unpaper 7.0.0), text taken back from the PDFs with `pdftotext -raw`.
- LegatorOCR was run with `--split off --mode gray --format none`; its two engines are `--engine tesseract` (clean-up, then the same Tesseract) and `--engine native` (built-in model).
- CER = edit distance / reference length, white space collapsed, cedilla and comma-below s/t treated as equal. Mean over 6 pages per group, one run, 2-core machine.

## Result

| Pipeline | A (Liberation Serif) CER / s per page | B (Poppins) CER / s per page |
|---|---|---|
| tesseract (untouched image) | 1.60 % / 1.5 s | 1.09 % / 1.3 s |
| ocrmypdf | 1.63 % / 3.0 s | 1.11 % / 2.9 s |
| ocrmypdf + unpaper | 1.71 % / 8.7 s | 1.14 % / 8.5 s |
| legatorocr + Tesseract | 1.37 % / 2.2 s | 1.19 % / 2.0 s |
| legatorocr native engine | 0.28 % / 1.6 s | 0.50 % / 1.6 s |

## Reading, with the strongest objections

- **Clean-up before Tesseract barely matters here:** LegatorOCR's pre-processing gives 1.37 % against 1.60 % in group A and is slightly *worse* in group B (1.19 % against 1.09 %). The pages are only mildly degraded, so there is little for any clean-up to repair; this says nothing about heavy shadows, folds or perspective. OCRmyPDF with or without unpaper changes nothing measurable and unpaper multiplies the time by about three.
- **The native engine's lead is large on these pages, and it is not neutral ground:** the words on the test pages are sampled from the same wordfreq lists the native model was trained on and its word list is built from; both typefaces' *kind* (rendered text from fonts) is what it was trained on. Liberation Serif is in its training fonts; Poppins is not, and the lead shrinks (0.50 % against 1.09 %) but stays. Rendered text with a perfect reference is easier than any scan. Do not read this as "better than Tesseract" on books.
- **Speed:** 1.6 s per page for the native engine against 1.3-1.5 s for bare Tesseract and 2.0-2.2 s for LegatorOCR's clean-up plus Tesseract on this machine; OCRmyPDF is about twice bare Tesseract.
- **Not run:** ScanTailor (no package in Ubuntu 26.04's archive and GitHub release pages are not reachable from the build machine), ABBYY, Acrobat, any vision-language model, PaddleOCR. Dewarp and perspective are not exercised by the table above (flat pages); see the curved-page duel below.

The decision rule stays the one in `ROADMAP.md`: this table is a regression guard and an answer to one objection, never marketing.

## Curved pages (dewarp), 18 November 2026

Flat pages from `makepage.py` (Liberation Serif, 300 dpi, as in group A), then bent by `tools/ocr-train/warp.py` (an arch: the lines rise towards the middle of the page, strongest at the top; amplitude 3 % or 6 % of the page height). Group F = the four flat originals, C = the same four bent 3 %, D = bent 6 %. LegatorOCR was run with `--dewarp`; `page-dewarp` 0.3.4 (MIT, `page-dewarp -nb 1`, then Tesseract on its output) is added as the open-source specialist. Run: `python3 scripts/duel.py DIR --dewarp`. The pages themselves were made in a scratch directory and are not in the repository; `warp.py` makes more.

| Pipeline | F flat | C 3 % arch | D 6 % arch |
|---|---|---|---|
| tesseract (untouched) | 1.56 % | 43.47 % | 55.36 % |
| ocrmypdf | 1.58 % | 37.07 % | 56.77 % |
| ocrmypdf + unpaper | 1.57 % | 33.63 % | 60.42 % |
| legatorocr + Tesseract (no dewarp) | 0.97 % | 39.87 % | 55.08 % |
| legatorocr native (no dewarp) | 0.29 % | 43.42 % | 92.22 % |
| legatorocr `--dewarp` + Tesseract | 0.82 % | 1.31 % | 1.79 % |
| legatorocr `--dewarp` + native | 0.29 % | 0.98 % | 3.85 % |
| page-dewarp + Tesseract | 1.67 % | 20.40 % | 40.57 % |

Mean CER of 4 pages per cell; seconds per page: Tesseract 1.5-2.2 s, LegatorOCR with dewarp 1.4-2.4 s, OCRmyPDF 5-9 s (up to 26 s on single pages), page-dewarp 4 s flat and 15-31 s curved.

Reading, with the objections:

- **What is solid:** on a curved page every tool without a dewarp step reads 33-92 % wrong, and LegatorOCR with `--dewarp` reads 1-4 % wrong in the same time as a plain run. On flat pages the dewarp step does not hurt (0.82 % against 0.97 %; native 0.29 % both).
- **Home ground, again:** the bend is one smooth arch made by a formula, the kind of curve the dewarp model fits (a parabola per text line). Real book pages bend near the spine, with a shadow, a changing line spacing and a bent page edge. This table proves the step works on an arch; it does not prove that it works on photographed books.
- **page-dewarp was not tuned:** it ran with its defaults on pages with no margin shadow and no page frame (it expects a page on a darker background), which may be why it repaired only about half of the damage. A specialist given its own kind of input and settings could well do better; the honest statement is "with defaults, on these pages".
- **A harness bug found on the way:** the first ocrmypdf runs read 100 % wrong because ocrmypdf refuses a PNG without a resolution (`--image-dpi` is now passed); the numbers above are the corrected ones.
- **Still not run:** ScanTailor, ABBYY, Acrobat, real phone photos of open books.
