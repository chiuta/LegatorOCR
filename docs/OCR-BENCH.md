# OCR accuracy benchmark (synthetic, honest limits)

Reproduce: `python3 scripts/ocr-bench.py target/release/legatorocr --out DIR` (needs Tesseract with the 8 language models for the Tesseract columns; add `--engine native` to run LegatorOCR with its built-in engine — the Tesseract column then reads `n/a` when Tesseract is not installed).

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
  or on handwriting. Treat it as a regression guard, not a published accuracy figure.
- The benchmark found three real bugs (scanner-edge bars rotating or cropping pages, photo pages rotated); all are fixed
  with tests.

## Built-in native engine (v0.2.0-pre.5)

Same pages and script, `--engine native`; model trained 16 000 steps on synthetic text only. **Tesseract was not installed
where these were measured, so there is no Tesseract column to compare with** — the table above (measured with Tesseract in
pre.3) used the same pages, but numbers from different machines and versions are not a head-to-head.

| lang | clean | scan | bad |
|---|---|---|---|
| ron | CER 0.72%  WER 4.65% | CER 0.36%  WER 2.33% | CER 1.81%  WER 5.81% |
| eng | CER 0.36%  WER 2.22% | CER 0.36%  WER 2.22% | CER 0.36%  WER 2.22% |
| fra | CER 0.72%  WER 4.49% | CER 0.91%  WER 5.62% | CER 1.09%  WER 5.62% |
| ita | CER 0.39%  WER 2.41% | CER 0.58%  WER 3.61% | CER 0.58%  WER 3.61% |
| spa | CER 0.75%  WER 4.60% | CER 0.57%  WER 3.45% | CER 0.38%  WER 2.30% |
| por | CER 0.60%  WER 3.57% | CER 0.60%  WER 3.57% | CER 2.00%  WER 4.76% |
| deu | CER 0.19%  WER 1.33% | CER 0.38%  WER 2.67% | CER 0.38%  WER 2.67% |
| hun | CER 0.43%  WER 3.08% | CER 0.22%  WER 1.54% | CER 0.43%  WER 3.08% |

Mean CER over the 24 pages: **0.63 %** (about 3 % WER). Each cell is one page of ≈75–100 words, so a single wrong word moves it
by ~1 point WER: differences between cells are noise, the overall level is the information. This is **clean synthetic
text in one serif font**; the engine has never been measured on real scans, so on real books, old print, or unusual fonts
it can be clearly worse than Tesseract — which is why `--engine auto` keeps preferring an installed Tesseract.

Other checks (`tools/ocr-train/eval_pages.py`, synthetic pages rendered in all 8 languages, 300 dpi unless noted):

| condition | CER |
|---|---|
| clean pages, mixed 1–2 columns, digits, symbols | 0.26 % |
| same, underlines and vertical table borders drawn on every line | 0.30 % |
| unseen font (Bitstream Charter), blur + noise + slight tilt, 200 dpi | 1.14 % |
| page orientation (0/90/180/270°, ro and de) | 8 of 8 right |

Model v1 on the same new pages scored 0.61 %, 1.75 % and 0.59 % (it had never seen these symbols and ruled lines).
