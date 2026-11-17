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
- **Not run:** ScanTailor (no package in Ubuntu 26.04's archive and GitHub release pages are not reachable from the build machine), ABBYY, Acrobat, any vision-language model, PaddleOCR. Dewarp and perspective were not exercised at all (pages are flat).

The decision rule stays the one in `ROADMAP.md`: this table is a regression guard and an answer to one objection, never marketing.
