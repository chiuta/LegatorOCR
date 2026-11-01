# OCR accuracy benchmark (synthetic, honest limits)

Reproduce: `python3 scripts/ocr-bench.py target/release/legatorocr --out DIR` (needs Tesseract with the 8 language models).

Pages are rendered text (one serif font, 3 paragraphs) in each of the 8 UI languages, in three conditions: *clean*,
*scan* (slight skew, noise, uneven light) and *bad* (rotation, dark scanner-edge bars, shading gradient). "Raw" is
Tesseract on the untouched image; "LegatorOCR" is the full clean-up pipeline followed by the same Tesseract.
Metrics are character/word error rate after normalisation (`--compare-text`).

| lang | scan | raw Tesseract | LegatorOCR (clean-up + Tesseract) |
|---|---|---|---|
| ron | clean | CER 0.00%  WER 0.00% | CER 0.18%  WER 1.16% |
| ron | scan | CER 1.99%  WER 3.49% | CER 0.00%  WER 0.00% |
| ron | bad | CER 0.91%  WER 3.49% | CER 0.00%  WER 0.00% |
| eng | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| eng | scan | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| eng | bad | CER 0.55%  WER 3.33% | CER 0.00%  WER 0.00% |
| fra | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| fra | scan | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| fra | bad | CER 1.27%  WER 4.49% | CER 0.00%  WER 0.00% |
| ita | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| ita | scan | CER 0.00%  WER 0.00% | CER 0.58%  WER 3.61% |
| ita | bad | CER 0.97%  WER 3.61% | CER 0.00%  WER 0.00% |
| spa | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| spa | scan | CER 0.38%  WER 2.30% | CER 0.19%  WER 1.15% |
| spa | bad | CER 0.38%  WER 2.30% | CER 0.00%  WER 0.00% |
| por | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| por | scan | CER 1.40%  WER 5.95% | CER 1.20%  WER 5.95% |
| por | bad | CER 2.20%  WER 10.71% | CER 0.00%  WER 0.00% |
| deu | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| deu | scan | CER 0.38%  WER 1.33% | CER 0.38%  WER 1.33% |
| deu | bad | CER 0.76%  WER 2.67% | CER 0.00%  WER 0.00% |
| hun | clean | CER 0.00%  WER 0.00% | CER 0.00%  WER 0.00% |
| hun | scan | CER 0.22%  WER 1.54% | CER 0.22%  WER 1.54% |
| hun | bad | CER 1.30%  WER 6.15% | CER 0.43%  WER 1.54% |

What this does and does not show:

- It shows the clean-up stage helps on degraded input (bad: 0.0–0.2 % vs 0.6–2.3 % CER) and rarely hurts on easy input.
- Residual differences on *clean* ron (0.18 %) and *scan* ita/por are a few characters per page — too small a sample to rank.
- It is **one font, rendered text, ≈75 words per page**. It says nothing about real historical prints, handwriting,
  or the native engine (not written). Treat it as a regression guard, not a published accuracy figure.
- The benchmark found three real bugs (scanner-edge bars rotating or cropping pages, photo pages rotated); all are fixed
  with tests.
