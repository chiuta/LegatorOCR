# LegatorOCR against the field: an adversarial analysis (4 November 2026)

Purpose: argue *against* LegatorOCR as strongly as the evidence allows, so the project does not fool itself.
`COMPARISON.md` (2 Nov) is the feature table; this file attacks it and the project's own claims. Competitor facts below were
fetched on 2026-11-04 (links at the end); LegatorOCR numbers come from `ROADMAP.md` (4.8-4.16) and were measured here.
Vendor figures are marked as vendor claims. Nothing was filled from memory; "not confirmed" is not "no".

## 1. The market on 4 November 2026

| Segment | State today (source in §6) |
|---|---|
| ScanTailor family (clean-up only, with one exception) | **Experimental**: "nail" 1.2026.10.10 (**2026-10-21**), "foot" 1.2026.08.08 (2026-08-07), "boomerang" 1.2026.07.07 (2026-07-07) - roughly monthly. **Universal** 0.2.14 is from **2023-08-19** (corrected 4 Nov: earlier text said "19 Aug, pre-release", year unknown; it is not active). **Advanced** is more alive than the 2 Nov text said: github.com/ScanTailor-Advanced/scantailor-advanced has **v1.2.1 (2026-05-24)**, v1.2.0 (2026-05-24), v1.1.1 (2026-04-03: oblique deskew, Fox/Window/Grad thresholds, relative project paths); the 4lex4 repo is stale (2019 early-access tag). **PDF conflict resolved (4 Nov): the project README says OCR and assembling documents are out of scope and no release note lists PDF export; scantailor.net's "TIFF, PDF" line is an unsourced claim by an anonymous site.** **New: ScanTailor FanMode** (hakami55, "0.1.0-alpha.0", Windows-first, commits Apr-May 2026): a fork that *does* add PDF import and searchable (Arabic/English, Tesseract) or image-only PDF export; alpha, no Romanian data bundled, not run by us. |
| OCR plumbing | **OCRmyPDF 17.13.0** (28 Sep 2026): PDF/A without Ghostscript, `--force-ocr` keeps links, memory cut by a third in 17.11. Active, mature, and the base of Paperless-ngx. |
| Classic engines | **Tesseract 5.5.4** (2026-10-28; 5.5.3 was 2026-07-24; conflict resolved 4 Nov from the release page timestamps), **PP-OCRv6** (PaddleOCR v3.7.0, released 2026-06-11; tiny 1.5M / small 7.7M / medium 34.5M parameters; one model for Chinese, English, Japanese and 46 Latin-script languages, dictionary extended with about 200 diacritical characters; Romanian is **not named**, so it is likely but unconfirmed; CPU speed from its own page: tiny 0.22 s and small 0.61 s per image with ONNX Runtime on a Xeon 8350C, medium 3.31 s). Its accuracy tables are its authors' in-house benchmark (Chinese/English/Japanese-centred), read on 2026-11-04 in the browser. |
| Neural document OCR | The 2026 change. OmniDocBench v1.6 (board updated 11 Sep): TeleOCR 96.91, OvisOCR2 96.47, PaddleOCR-VL-1.6 96.34 (0.8-1.2B parameters, Apache-2.0); olmOCR-Bench: Chandra OCR 2 85.8 (self-reported), dots.mocr 83.9, olmOCR 2 82.4. All are tiny enough for one consumer GPU; several have restrictive licences (Chandra, olmOCR "research/educational", one model grants no rights in the EU). The same source reports PaddleOCR-VL (first version) falling from 91.93 to 72.19 on *photographed* pages. |
| Commercial | FineReader PDF about US$99/yr (aggregator), Acrobat Pro US$19.99/mo annual. Subscription-only, Windows-centred (FineReader), no independent accuracy data; "99.8 %" is a vendor claim. Re-read 4 Nov: ABBYY's own spec lists Romanian **with** dictionary (plus Romanian (Moldova) without), still "FineReader PDF 16"; Acrobat's web OCR page (updated 30 Apr 2026) does **not** list Romanian. Prices not re-verified (the vendor buy page did not render for us). |
| OCR plumbing, extra | Paperless-ngx is now v3.4.2 (2026-11-03). OCRmyPDF 17.13.0 (2026-09-28) and PaddleOCR 3.7.0 (2026-06-11) re-confirmed on PyPI. |

## 2. Attack on LegatorOCR's own claims

Verdict scale: **holds** (evidence supports it), **weak** (supported but narrower than it sounds), **unsupported**, **refuted**.

| # | Claim we could be tempted to make | Strongest counter-argument | Evidence | Verdict |
|---|---|---|---|---|
| 1 | "Our OCR is as good as Tesseract" | On single-column pages it is not. GOOD8: native 5.72 % vs Tesseract 4.54 % CER (26 % more errors). Book page 200: word recall 72.5 % vs 91.2 %. The ALL12 lead (6.93 vs 7.81) comes from layout pages (p13 4.8 vs 18.2) and is pulled the other way by p52 (89.7 % native vs 29.7 %). | ROADMAP 4.11, 4.15 | **refuted** for plain text; "different failure profile" is all we can say |
| 2 | "Native engine beats Tesseract across 12 typefaces" | The font benchmark (mean 1.01 % vs 11.67 %) is *rendered* text, and the native model was trained on rendered fonts, so it is the home ground. Tesseract's 47-52 % collapses look like a segmentation artefact on tilted small type, not a reading failure; cause not investigated. Native lost to Tesseract on 4 of 12 fonts. | ROADMAP 4.16 | **weak** (a stress test, not accuracy) |
| 3 | "Accuracy numbers are reliable" | References are `pdftotext -raw` of an ABBYY layer (not human text); the only hand reference is one page by the assistant. A 1-point CER difference can be the reference's habits. | 4.10, 4.11, 0.1 | **weak**; no hand-verified set exists |
| 4 | "We are competitive with modern OCR" | Only older neural baselines were run (EasyOCR word recall 89.1 % vs Tesseract 91.3 % on GOOD8, ROADMAP 4.17; three OnnxTR/docTR model pairs 4-5 CER points behind Tesseract on plain pages and unable to output ă ș ț, ROADMAP 4.31); PaddleOCR, PP-OCRv6, any 2026 VLM, FineReader or Mistral were never run on a single page (model hosts unreachable here). The 2026 leaders score 96-97 on a benchmark where Tesseract-class engines are not even the reference. | COMPARISON §5 | **unsupported**; expect to be behind |
| 5 | "Works on phone photos" | `--burst` and perspective correction are untuned on real phone captures; VLM literature shows large drops on photos, which does not say we are better. | ROADMAP 1.14 | **unsupported** |
| 6 | "Better dewarp" | Vertical bend only; no perspective unrolling; Advanced has mesh/manual dewarp used for 10+ years. We never ran both on the same pages. Our guide lines are a UI nicety, not a quality result. | README, COMPARISON §3 | **unsupported** |
| 7 | "More binarisers / better clean-up" | Experimental has more methods and ships new ones monthly; our own test of extra local methods showed them *worse*. | ROADMAP 1.4 | **refuted** as variety; unknown as quality |
| 8 | "One tool for the whole chain" | True, and the strongest point. But a user who already has ScanTailor + OCRmyPDF has the chain in two free tools with 10+ years of use. The advantage is convenience and hybrid output, not capability. **New 4 Nov: ScanTailor FanMode also closes the chain in one GUI (PDF import, clean-up, dewarp, Tesseract text layer, PDF export, compression presets), on top of a mature codebase.** It is alpha, one author, Windows-first and Arabic/English only as shipped, so the claim still holds today, but "no other free tool does the whole chain" is no longer a sentence we can write. | COMPARISON §3, §6; github.com/hakami55/ScanTailor-FanMode | **weakened**: holds, narrower than on 2 Nov |
| 9 | "Hybrid layered PDF (G4 stencil + background)" | Advanced already produces foreground/background layers for DjVu workflows (README, re-read 4 Nov); FanMode offers PDF "compression presets" whose internals we did not inspect; FineReader has MRC. What is new is doing it inside the PDF writer with the text layer, in one free tool. | COMPARISON §3-4 | **holds**, not unique |
| 10 | "PDF/A valid" | veraPDF runs only in CI on samples (`pdfa.yml`); not run in this environment; no certification. | workflow | **weak** |
| 11 | "Cross-platform" | Windows and macOS binaries come from CI; the GUI has run only on Linux under Xvfb; macOS is unsigned. No human has used it there. | README, ROADMAP | **unsupported** for the GUI |
| 12 | "Audit / checksums / OCFL is unique" | Paperless-ngx has edit history and checksums; absence of evidence in closed products is not uniqueness. Real archivists have not tried OCFL/ALTO/PAGE output beyond schema validation. | COMPARISON §6, 1.3, OCFL | **weak** |
| 13 | "Production ready" | 0.2.0-pre.15, one author, ~55k lines of Rust, 230 commits, no external users known, GUI tested by scripted pixel assertions. Bus factor 1. | repo | **refuted** |

## 3. Where each competitor is simply better

- **Tesseract / OCRmyPDF**: accuracy on clean single-column Romanian, maturity, ecosystem (Paperless-ngx), PDF/A path, packaging in every distro. We *call* Tesseract; we do not replace it.
- **ScanTailor Experimental / Advanced**: breadth and years of tuning in dewarp and binarisation, plus a user base that reports bugs (Advanced: v1.2.1 in May 2026, active issue tracker). They are image-only, which is their weakness, not a reason they lose. **ScanTailor FanMode** removes even that weakness for Arabic/English users; if it reaches beta and adds more Tesseract languages, a Romanian user gets the same chain on a better-tested clean-up core than ours. Today it is an alpha, so this is a risk to watch, not a loss.
- **FineReader**: layout analysis, 198 languages with dictionaries, table/DOCX fidelity. Cost and platform are its weakness.
- **Neural document OCR (PaddleOCR-VL, Chandra, olmOCR, Mistral OCR)**: layout, tables, formulas, multilingual; by the numbers above they are in a different league. Weakness: GPU or cloud, licences, no searchable-PDF output by themselves, photo robustness.

## 4. Where competitors are weaker than they look (also attacked)

- ScanTailor sites claim things with no source ("up to 40 % WER reduction"); the community site is anonymous.
- Benchmarks (OmniDocBench, olmOCR-Bench) are English/Chinese-centred, published by model authors or sellers, and none measures ș ț ă â î. A strong leaderboard score is not evidence for Romanian books.
- Accuracy for FineReader/Acrobat comes from vendor pages and aggregators.
- Licences: several of the best neural models cannot be bundled in a free tool (research-only, revenue caps, EU exclusion); LegatorOCR is MIT, which matters for embedding.
- Versions disagreed between sources on 2 Nov; on 4 Nov they were resolved from release timestamps: Tesseract 5.5.4 (2026-10-28), ScanTailor Advanced v1.2.1 (2026-05-24; the Gitea 1.0.21 is a different fork, 2025-12-20; scantailor.net still advertises v1.1.1 for Windows and v1.2.1 for Linux), Universal 0.2.14 is 2023. Treat prices and any figure from aggregators as approximate.
- scantailor.net contradicts itself (PDF export vs TIFF/JPEG/PNG only) and is anonymous: a README or release note beats it.

## 5. Conclusions and what to do

1. **Do not market native OCR as a rival to Tesseract or to neural OCR.** Position LegatorOCR as the clean-up + hybrid-PDF + batch pipeline that can use any OCR engine. This matches the evidence.
2. **The existential risk is not a rival product, it is a free plug-in:** OCRmyPDF already accepts external engines and a small Apache-2.0 neural engine can be dropped in. If someone adds dewarp/split to that chain, the convenience gap closes. The defensible ground is the image stage (hybrid MRC, split, perspective) and the audit/archive outputs.
   *Update 4 Nov:* the "free plug-in" risk is no longer hypothetical in one form: ScanTailor FanMode (alpha, Arabic/English) already chains ScanTailor clean-up to a Tesseract text layer and PDF export. Do not claim a chain nobody else has; claim what is measurably ours (hybrid MRC writer, JBIG2/JPEG 2000 input breadth, audit/archive outputs, Romanian-aware pipeline) and re-check FanMode before each release.
3. **Measurements that would change the verdict (nothing here has been done):** (a) PaddleOCR / PP-OCRv6 and one VLM on the same 12 pages and the book page, with Romanian diacritics counted separately; (b) ScanTailor Advanced vs LegatorOCR dewarp on the same curved pages; (c) a hand-verified reference set (ROADMAP 0.1; needs a person); (d) a real Windows and macOS GUI run; (e) a real phone-photo burst.
4. **Stop-doing candidates:** further native-OCR work is judged by the gate rule in ROADMAP; four such experiments (4.2-4.4, 4.10) were already rejected and the fifth (fonts) only reproduced a synthetic win.
5. Treat the font benchmark and the GOOD8/ALL12 numbers as internal regression checks, never as marketing.

## 6. Sources (fetched 2026-11-04)

- Re-verification of 4 Nov (full table and "could not verify" list: `docs/audit/competitors.md`): GitHub release pages and raw READMEs of ScanTailor-Advanced, 4lex4, vigri, Experimental, Universal; github.com/hakami55/ScanTailor-FanMode; github.com/ThomasHoussin/scan_book; tesseract-ocr/tesseract/releases; paperless-ngx/paperless-ngx/releases; pypi.org (ocrmypdf, paddleocr); pdf.abbyy.com/specifications and pdf.abbyy.com; helpx.adobe.com OCR supported-languages page (Apr 2026); scantailor.net and /download/.
- ScanTailor Experimental releases: github.com/ImageProcessing-ElectronicPublications/scantailor-experimental/releases
- ScanTailor Advanced community site: scantailor.net; 4lex4 and trufanov-nok (Universal) release pages on github.com
- OCRmyPDF releases: github.com/ocrmypdf/OCRmyPDF/releases
- OCR model leaderboards summary: docsumo.com/blog/best-ocr-models (30 Sep 2026; secondary, vendor-published, figures not independently checked)
- PP-OCRv6: raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/docs/version3.x/algorithm/PP-OCRv6/PP-OCRv6.en.md
- Commercial prices: aiproductivity.ai/blog/best-ocr-software-2026 (no hands-on tests, conflicting totals on the page)
- Own data: `ROADMAP.md` 4.8-4.16, `docs/OCR-BENCH.md`, `scripts/ocr-bench-fonts.py`

## 7. Update, 4 November 2026, evening (after ROADMAP 4.31-4.33)

Two rows of section 2 changed. Everything is on the same 12 magazine pages with ABBYY references (not hand-verified), so none of it says anything about other typefaces.

- **"Tesseract is hard to beat on plain Romanian pages" — corrected the same evening (ROADMAP 4.36): the bridge is level with Tesseract on character error once page p31 is set aside; it is better only where Tesseract is confused by an illustration. The next sentence is the original, stronger claim, which the per-page table does not support.** A docTR detector plus a small CRNN recogniser whose last layer was retrained for ă ș ț (frozen backbone, synthetic words, ROADMAP 4.32) and run through the real CLI by `scripts/engine-onnx.py --onnx-family doctr` (4.33) gives GOOD8 CER with diacritics **4.26 %** against Tesseract's **5.25 %** (ALL12 6.02 % vs 8.20 %), at 3.4 s per page vs 2.7 s. Reproduced here from a fresh model download, same numbers.
- **It is still behind on word recall:** exact word recall 88.2 % vs 91.3 % (GOOD8). The two metrics disagree because they weigh different errors (CER counts every wrong character, recall counts whole words), so the honest summary is "different error profile, not strictly better".
- **Costs and limits:** needs onnxruntime and a 25 MB model download, is not the default engine, is untested on other typefaces, tables, three-column layouts and phone photos, and the Tesseract-versus-bridge gap on CER is about one point on 8 pages. Page p21 (a page with a diagram) shows 142 % CER with the bridge, but the ABBYY reference omits the diagram text (458 reference characters against 1087 read); the bridge recovers 93.5 % of the reference's own words (ROADMAP 4.35), so p21 says more about the reference than about the engine.
- **What it does not change:** the native Rust engine is still behind Tesseract (section 2, row 1); the 2026 vision-language models were not run.
- **Against the better Tesseract model (added the same evening):** `tessdata_best` gives GOOD8 CER 4.87 % and exact recall 91.6 % (ROADMAP 4.34). The docTR bridge is still ahead on CER by 0.6 points (4.26 %) and still behind on recall by 3.4 points (88.2 %); the gap is smaller than the one quoted against the fast model that apt installs.
