# LegatorOCR against the field: an adversarial analysis (4 November 2026)

Purpose: argue *against* LegatorOCR as strongly as the evidence allows, so the project does not fool itself.
`COMPARISON.md` (2 Nov) is the feature table; this file attacks it and the project's own claims. Competitor facts below were
fetched on 2026-11-04 (links at the end); LegatorOCR numbers come from `ROADMAP.md` (4.8-4.16) and were measured here.
Vendor figures are marked as vendor claims. Nothing was filled from memory; "not confirmed" is not "no".

## 1. The market on 4 November 2026

| Segment | State today (source in §6) |
|---|---|
| ScanTailor family (clean-up only) | **Experimental** is the active one: "nail" 1.2026.10.10 (21 Oct), "foot" 1.2026.08.08, "boomerang" 1.2026.07.07 - roughly monthly releases, new binarisation/threshold, manual scaling, "RIS undefect". **Universal** 0.2.14 (19 Aug, pre-release, year not printed). The original **Advanced** repo's newest release page shows a 2019 early-access tag; the community site scantailor.net promotes "vigri" v1.1.1 (Windows/Linux, no official macOS) and says OCR is *not* built in but that PDF output exists. |
| OCR plumbing | **OCRmyPDF 17.13.0** (28 Sep 2026): PDF/A without Ghostscript, `--force-ocr` keeps links, memory cut by a third in 17.11. Active, mature, and the base of Paperless-ngx. |
| Classic engines | **Tesseract 5.5.x** (5.5.3 per one July source, 5.5.4 per the 2 Nov file: conflicting, check), **PP-OCRv6** (PaddleOCR; 1.5M-34.5M parameters; the page lists 46 Latin-script languages without naming them, **Romanian not confirmed**). |
| Neural document OCR | The 2026 change. OmniDocBench v1.6 (board updated 11 Sep): TeleOCR 96.91, OvisOCR2 96.47, PaddleOCR-VL-1.6 96.34 (0.8-1.2B parameters, Apache-2.0); olmOCR-Bench: Chandra OCR 2 85.8 (self-reported), dots.mocr 83.9, olmOCR 2 82.4. All are tiny enough for one consumer GPU; several have restrictive licences (Chandra, olmOCR "research/educational", one model grants no rights in the EU). The same source reports PaddleOCR-VL (first version) falling from 91.93 to 72.19 on *photographed* pages. |
| Commercial | FineReader PDF about US$99/yr (aggregator), Acrobat Pro US$19.99/mo annual. Subscription-only, Windows-centred (FineReader), no independent accuracy data; "99.8 %" is a vendor claim. |

## 2. Attack on LegatorOCR's own claims

Verdict scale: **holds** (evidence supports it), **weak** (supported but narrower than it sounds), **unsupported**, **refuted**.

| # | Claim we could be tempted to make | Strongest counter-argument | Evidence | Verdict |
|---|---|---|---|---|
| 1 | "Our OCR is as good as Tesseract" | On single-column pages it is not. GOOD8: native 5.72 % vs Tesseract 4.54 % CER (26 % more errors). Book page 200: word recall 72.5 % vs 91.2 %. The ALL12 lead (6.93 vs 7.81) comes from layout pages (p13 4.8 vs 18.2) and is pulled the other way by p52 (89.7 % native vs 29.7 %). | ROADMAP 4.11, 4.15 | **refuted** for plain text; "different failure profile" is all we can say |
| 2 | "Native engine beats Tesseract across 12 typefaces" | The font benchmark (mean 1.01 % vs 11.67 %) is *rendered* text, and the native model was trained on rendered fonts, so it is the home ground. Tesseract's 47-52 % collapses look like a segmentation artefact on tilted small type, not a reading failure; cause not investigated. Native lost to Tesseract on 4 of 12 fonts. | ROADMAP 4.16 | **weak** (a stress test, not accuracy) |
| 3 | "Accuracy numbers are reliable" | References are `pdftotext -raw` of an ABBYY layer (not human text); the only hand reference is one page by the assistant. A 1-point CER difference can be the reference's habits. | 4.10, 4.11, 0.1 | **weak**; no hand-verified set exists |
| 4 | "We are competitive with modern OCR" | Never measured against PaddleOCR, PP-OCRv6, any VLM, FineReader or Mistral on a single page. The 2026 leaders score 96-97 on a benchmark where Tesseract-class engines are not even the reference. | COMPARISON §5 | **unsupported**; expect to be behind |
| 5 | "Works on phone photos" | `--burst` and perspective correction are untuned on real phone captures; VLM literature shows large drops on photos, which does not say we are better. | ROADMAP 1.14 | **unsupported** |
| 6 | "Better dewarp" | Vertical bend only; no perspective unrolling; Advanced has mesh/manual dewarp used for 10+ years. We never ran both on the same pages. Our guide lines are a UI nicety, not a quality result. | README, COMPARISON §3 | **unsupported** |
| 7 | "More binarisers / better clean-up" | Experimental has more methods and ships new ones monthly; our own test of extra local methods showed them *worse*. | ROADMAP 1.4 | **refuted** as variety; unknown as quality |
| 8 | "One tool for the whole chain" | True, and the strongest point. But a user who already has ScanTailor + OCRmyPDF has the chain in two free tools with 10+ years of use. The advantage is convenience and hybrid output, not capability. | COMPARISON §6 | **holds**, narrow |
| 9 | "Hybrid layered PDF (G4 stencil + background)" | Advanced already produces foreground/background layers for DjVu workflows; FineReader has MRC. What is new is doing it inside the PDF writer with the text layer, in one free tool. | COMPARISON §3-4 | **holds**, not unique |
| 10 | "PDF/A valid" | veraPDF runs only in CI on samples (`pdfa.yml`); not run in this environment; no certification. | workflow | **weak** |
| 11 | "Cross-platform" | Windows and macOS binaries come from CI; the GUI has run only on Linux under Xvfb; macOS is unsigned. No human has used it there. | README, ROADMAP | **unsupported** for the GUI |
| 12 | "Audit / checksums / OCFL is unique" | Paperless-ngx has edit history and checksums; absence of evidence in closed products is not uniqueness. Real archivists have not tried OCFL/ALTO/PAGE output beyond schema validation. | COMPARISON §6, 1.3, OCFL | **weak** |
| 13 | "Production ready" | 0.2.0-pre.15, one author, ~55k lines of Rust, 230 commits, no external users known, GUI tested by scripted pixel assertions. Bus factor 1. | repo | **refuted** |

## 3. Where each competitor is simply better

- **Tesseract / OCRmyPDF**: accuracy on clean single-column Romanian, maturity, ecosystem (Paperless-ngx), PDF/A path, packaging in every distro. We *call* Tesseract; we do not replace it.
- **ScanTailor Experimental / Advanced**: breadth and years of tuning in dewarp and binarisation, plus a user base that reports bugs. They are image-only, which is their weakness, not a reason they lose.
- **FineReader**: layout analysis, 198 languages with dictionaries, table/DOCX fidelity. Cost and platform are its weakness.
- **Neural document OCR (PaddleOCR-VL, Chandra, olmOCR, Mistral OCR)**: layout, tables, formulas, multilingual; by the numbers above they are in a different league. Weakness: GPU or cloud, licences, no searchable-PDF output by themselves, photo robustness.

## 4. Where competitors are weaker than they look (also attacked)

- ScanTailor sites claim things with no source ("up to 40 % WER reduction"); the community site is anonymous.
- Benchmarks (OmniDocBench, olmOCR-Bench) are English/Chinese-centred, published by model authors or sellers, and none measures ș ț ă â î. A strong leaderboard score is not evidence for Romanian books.
- Accuracy for FineReader/Acrobat comes from vendor pages and aggregators.
- Licences: several of the best neural models cannot be bundled in a free tool (research-only, revenue caps, EU exclusion); LegatorOCR is MIT, which matters for embedding.
- Versions disagree between sources (Tesseract 5.5.3 vs 5.5.4; Advanced 1.0.21 vs "vigri" 1.1.1): treat every competitor version as approximate.

## 5. Conclusions and what to do

1. **Do not market native OCR as a rival to Tesseract or to neural OCR.** Position LegatorOCR as the clean-up + hybrid-PDF + batch pipeline that can use any OCR engine. This matches the evidence.
2. **The existential risk is not a rival product, it is a free plug-in:** OCRmyPDF already accepts external engines and a small Apache-2.0 neural engine can be dropped in. If someone adds dewarp/split to that chain, the convenience gap closes. The defensible ground is the image stage (hybrid MRC, split, perspective) and the audit/archive outputs.
3. **Measurements that would change the verdict (nothing here has been done):** (a) PaddleOCR / PP-OCRv6 and one VLM on the same 12 pages and the book page, with Romanian diacritics counted separately; (b) ScanTailor Advanced vs LegatorOCR dewarp on the same curved pages; (c) a hand-verified reference set (ROADMAP 0.1; needs a person); (d) a real Windows and macOS GUI run; (e) a real phone-photo burst.
4. **Stop-doing candidates:** further native-OCR work is judged by the gate rule in ROADMAP; four such experiments (4.2-4.4, 4.10) were already rejected and the fifth (fonts) only reproduced a synthetic win.
5. Treat the font benchmark and the GOOD8/ALL12 numbers as internal regression checks, never as marketing.

## 6. Sources (fetched 2026-11-04)

- ScanTailor Experimental releases: github.com/ImageProcessing-ElectronicPublications/scantailor-experimental/releases
- ScanTailor Advanced community site: scantailor.net; 4lex4 and trufanov-nok (Universal) release pages on github.com
- OCRmyPDF releases: github.com/ocrmypdf/OCRmyPDF/releases
- OCR model leaderboards summary: docsumo.com/blog/best-ocr-models (30 Sep 2026; secondary, vendor-published, figures not independently checked)
- PP-OCRv6: raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/docs/version3.x/algorithm/PP-OCRv6/PP-OCRv6.en.md
- Commercial prices: aiproductivity.ai/blog/best-ocr-software-2026 (no hands-on tests, conflicting totals on the page)
- Own data: `ROADMAP.md` 4.8-4.16, `docs/OCR-BENCH.md`, `scripts/ocr-bench-fonts.py`
