# LegatorOCR compared with the field (as of 2 November 2026)

This file compares LegatorOCR with the scan post-processing tools (ScanTailor family), the commercial OCR/PDF suites and the
open-source and cloud OCR engines. It was compiled on 2026-11-02 from vendor pages, release notes, package indexes and
third-party articles; LegatorOCR's own claims come from tests run here.

How to read it:

- **✔** = documented on a page we fetched (vendor claim) or tested here (LegatorOCR). **✘** = documented as absent.
  **?** = we could not confirm it; it is *not* a "no". We did not fill any cell from memory.
- *Vendor claim* is not independent evidence. Prices come mostly from review aggregators and may be out of date.
- Several vendor sites (abbyy.com product pages, Adobe's scanned-PDF settings page, several GitHub release lists) could not
  be read by our tools, hence the many "?" cells for closed products. Anyone with a browser can close them.
- **There is no independent benchmark of any engine on Romanian text with ș ț ă â î that we could find.** Everything about
  Romanian accuracy below is either our own measurement or marked as not measured.

## 1. What LegatorOCR is, and what it is not

A free (open-source) Rust tool that does the ScanTailor job (orientation, split, deskew, content box, dewarp, binarise,
picture zones), then writes **PDF** (CCITT G4, hybrid MRC-like layers, text layer, PDF/A-2b checked with veraPDF on samples),
DOCX, hOCR and text, with batch / audit features, CLI and GUI in 8 languages.

It is **not** a state-of-the-art OCR engine. Its own OCR (`nocr`) is a small native engine; Tesseract can be used instead.

## 2. Measured OCR accuracy (our test, not a benchmark)

12 real scanned Romanian book pages (clean, two-column and difficult layouts), character error rate against a hand-checked
reference where available; ABBYY output was used only as a cross-check because it is not ground truth (it mis-ordered one
page and read a cover differently).

| Engine | CER on the 12 pages |
|---|---|
| Tesseract 5.3.4, `ron` | **14.6 %** |
| LegatorOCR native OCR, before this round of fixes | 38.9 % |
| LegatorOCR native OCR, now | **17.5 %** |

Reading: the native engine is **still behind Tesseract** on this set. Remaining weak spots: a narrow column gutter (one
page), a few pages with heavy noise, and "." read as ",". Twelve pages is a sample, not a benchmark. We did not run
ABBYY, PaddleOCR or any cloud engine on the same pages. Synthetic results for 8 languages are in `docs/OCR-BENCH.md`.

Tesseract's Romanian models (`ron`, both `best` 9.6 MB and `fast` 2.4 MB, stamped 2017) contain both the comma-below
(ș, ț) and the cedilla (ş, ţ) forms, and a public issue (langdata_lstm #37, open when fetched) reports that the training
data maps Ș/ș/Ț/ț to the cedilla forms. We did not measure which form `ron` emits most; a normalisation step after OCR is
justified, not proven.

## 3. Scan post-processing tools

| | ScanTailor Advanced | ScanTailor Experimental | ScanTailor Universal | LegatorOCR |
|---|---|---|---|---|
| Latest we could confirm | 1.0.21 "snake", 2025-12-20 | "nail" 1.2026.10.10, 21 Oct (year not printed on page; 2026 footer) | 0.2.14, 19 Aug (year not shown) | 0.2.0 pre-releases (draft) |
| Price / licence | free, GPL-3 | free, GPL-3 | free, GPL-3 | free, open source |
| Platforms | Linux, Windows (Flathub too) | Debian, Windows x86/ARM64, Flatpak; macOS only in one 2025 build | Windows, macOS, Linux | Linux, Windows, macOS (last two from CI only, see §7) |
| Orientation, split, deskew, content box, margins | ✔ | ✔ (README) | ✔ | ✔ |
| Dewarp | ✔ semi-automatic, mesh / manual; "marginal dewarping" for curved borders | ? (one release note mentions correction parameters) | ? | ◐ automatic from text lines, or guide lines drawn with the mouse; vertical bend only |
| Binarisation | Sauvola, Wolf, Fox, Window, EdgeDiv | many (Niblack, Sauvola, Wolf, Window, LCaM …) | ? | six + auto per page |
| Foreground / background layers | ✔ (for DjVu) | ? | ? | ✔ hybrid PDF (G4 stencil + low-res background + patches) |
| OCR / PDF output | ✘ images only | ✘ ("out of scope" per README) | ✘ | ✔ own PDF writer with text layer |
| Perspective correction of photos | ? | ? | ? | ◐ automatic, conservative |
| Batch / CLI | ? | ? | ? | ✔ |
| Offline, no telemetry | ✔ | ✔ | ✔ ("offline, no registration, ad-free") | ✔ |

The forks keep adding binarisation methods (Experimental has more than LegatorOCR; we do not have EM denoise / deblur).
ScanKromsator (5.92, Windows, "free and proprietary") and Book Restorer (i2S) could not be assessed: no usable page. We found
no commercial book-scan-cleanup product that displaced the ScanTailor family in 2026, but we did not search exhaustively.

## 4. Commercial OCR / PDF suites

Prices are list prices seen on 2026-11-02 mostly via aggregators (currency often not stated; assumed US$).

| | ABBYY FineReader PDF 16 | Adobe Acrobat Pro / Studio | Tungsten Power PDF | Tungsten OmniPage Ultimate | Readiris 17 | PDF24 Creator | Foxit PDF Editor | Nitro PDF Pro | PDFelement 13 | LegatorOCR |
|---|---|---|---|---|---|---|---|---|---|---|
| Latest confirmed | 16.0.14.7295 (Aug 2023); newer build ? | continuous, no version | ? | 19.2 (release notes 2019-10-30) | 17 (date ?) | 11.31.1 (2026-10-08) | 2026.x (numbering inconsistent) | Win 14 (2023); Mac 26.4 | 13 | 0.2.0-pre |
| Price | sub only: ≈US$99/yr Standard, ≈US$165/yr Corporate; Mac US$69/yr; no perpetual | Pro US$19.99–29.99/mo; Studio US$24.99–34.99/mo | US$129 Standard / 179 Advanced, one-time | US$499 one-time | from US$89 one-time | free | US$10.99–13.99/mo; perpetual US$209.99 | US$15/mo; Win 14 perpetual ≈€233; Mac US$139.99/yr | US$79.99/yr; perpetual US$129.99 | free |
| Platforms | Windows; reduced Mac | Win, Mac, web, mobile | Win (Std also Mac) | Windows only | Win (Mac per marketing) | Windows only | Win, Mac, web, mobile | Win, Mac | Win, Mac | Linux, Win, macOS |
| Romanian OCR | ✔ + dictionary (vendor spec, 198 languages) | web OCR: **not listed** (15 languages); desktop ? | ? (102 languages) | ? (120+) | ? (138 claimed) | ? (online tool lists Romanian) | ? | Mac: ✘ (5 languages); Win ? | ✔ (29–30 languages) | ✔ (native engine or Tesseract) |
| Deskew / straighten / perspective / split | ✔ deskew, ✔ "Straighten Text Lines", ✔ "Correct Trapezoid Distortion", ✔ split | partial: "Enhance scan" (details ?) | ? | ? | ? | ? | not mentioned | ? | ? | ✔ / ◐ / ◐ / ✔ |
| PDF/A | ✔ 1a–3u, PDF/UA | ? (third party: A-2b/3b) | ✔ A-1…A-4 | ? | ? | ? | ? | Mac ✔ | ✔ (Win) | ◐ A-2b, veraPDF on samples only, not a certification |
| Compression | ✔ MRC + PreciseScan smoothing; JBIG2 as export format | ? | ✔ MRC, "8× smaller" (vendor) | ? | "compression" ? | ? | downsample only | ? | ? | ✔ G4, hybrid layers, JBIG2/JPEG2000 on input |
| DOCX export | ✔ | ? | ? | ? | Word/Excel ✔ | ? | ? | ? | ? | ✔ with columns and tables (limits §7) |
| hOCR / ALTO | not documented | ? | ? | ? | ? | ? | ? | ? | ? | hOCR ✔, ALTO ✘ |
| Batch / hot folder | Hot Folder (limits and edition ?); CLI saving needs an "Extended CLI" licence | Action Wizard incl. "Optimize Scanned Documents" | watched folder ✔ | Batch Manager ✔ | batch + watch folder | ? | multi-file OCR ✔ | Mac batch ✔ | batch ✔ | CLI + GUI batch |
| Local / offline | local components; not explicitly stated | desktop ?; AI Assistant is cloud | ? | ✔ local | listed | OCR language files are downloaded on demand; empty list without internet | ? | ? | ? | ✔ |

Notes:

- ABBYY is the only suite that publicly documents a full image-editing set close to ScanTailor's (deskew, straighten, trapezoid,
  split). Its claimed 98–99 % character accuracy on clean text and ≈80 % on hard scans come from a single review site, not a
  controlled test.
- Adobe's OCR "Editable text and images" replaced the old ClearScan (staff reply, 2015); its OCR needs Pro or above.
- PDF24 Creator's OCR uses Tesseract 4.0 and fetches language files from the internet, so it is not offline out of the box.
- Microsoft Lens stopped scanning on 9 March 2026 (OneDrive scan replaces it, cloud-saving). Google Drive/Docs OCR is
  reported (by a competitor's article) to cap files at 2 MB and read only 10 PDF pages. Apple Vision's new document
  recognition (OS 26) lists supported languages in its API; we could not confirm Romanian.
- Pricing, versions and language lists change; check the vendor before buying.

## 5. Open-source and cloud OCR engines

| | Tesseract 5.5.4 | OCRmyPDF 17.13.0 | Paperless-ngx 3.4.0 | PaddleOCR 3.7 / PaddleOCR-VL-1.6 | Mistral OCR 4 / 4.1 | Google Document AI | olmOCR 2 / Chandra OCR 2 | Surya |
|---|---|---|---|---|---|---|---|---|
| Date | 28 Oct (year inferred 2026) | 2026-09-28 | 2026-10-29 | 2026-06 / 2026-05 | 2026-06-23 / preview 2026-07-16 | n/a | Oct 2025 / Mar 2026 | 2.5 |
| Licence / price | Apache-2.0 | MPL-2.0 | GPL-3 | Apache-2.0 | cloud, US$4 per 1,000 pages (batch US$2) | US$1.50 per 1,000 pages | open weights (Chandra: modified OpenRAIL-M); GPU needed | code Apache-2.0, weights restricted above US$5M |
| Runs offline | ✔ | ✔ | ✔ self-hosted | ✔ | ✘ | ✘ | ✔ GPU | ✔ |
| Romanian | ✔ `ron` model (2017) | via Tesseract | via Tesseract | not confirmed (46 Latin languages in one model) | claimed (170 languages) | ? | English-first | 90+ languages claimed |
| Searchable PDF output | ✔ | ✔ PDF/A-2b default | ✔ PDF/A archive + untouched original | ✘ (Markdown/JSON) | ✘ | ✘ | ✘ | ✘ |
| Image clean-up before OCR | Sauvola in Leptonica | `--deskew`, `--clean` via unpaper; no dewarp / perspective / split | OCRmyPDF options | – | – | – | – | – |
| Audit / history | – | – | edit history since v2.7 (on by default); checksums per document | – | – | – | – | – |

Observations:

- The 2026 change is vision-language OCR (PaddleOCR-VL, Mistral OCR 4, Chandra, olmOCR 2, GLM-OCR, TeleOCR). On the
  OmniDocBench v1.6 board the top entries score about 96–97, and olmOCR-Bench tops at about 85–88. Those benchmarks are
  English/Chinese-centred, several figures are self-reported by vendors or by gateways that sell access, and none measures
  Romanian diacritics. We did not run any of them on our 12 pages, so **we cannot say how LegatorOCR's native OCR compares
  with them; we expect it to be behind**.
- OCRmyPDF dropped lossy JBIG2 in v17 because of character-substitution errors; LegatorOCR's lossless-only JBIG2 policy is the
  same idea. OCRmyPDF is the mature choice for adding a text layer to existing scans.
- Tesseract is still the practical local engine for Romanian and is what LegatorOCR can call as an alternative.

## 6. Where LegatorOCR is actually different

This list is deliberately narrow, since earlier drafts claimed too much (Paperless-ngx has an audit history; Nitro and Foxit
advertise audit trails of their own kinds).

- **One free tool for the whole chain**: ScanTailor-style clean-up, hybrid PDF writer, text layer, DOCX, hOCR, batch, GUI +
  CLI, 8 interface languages. The ScanTailor forks stop at images; OCRmyPDF/Tesseract start from existing scans.
- **Hybrid layered output** from the clean-up stage (G4 stencil, low-res background, full-resolution patches).
- **Input breadth, native**: PDFs with CCITT, JBIG2 (including Huffman, refinement and aggregation), JPEG 2000 (bypass,
  termination, POC, PPM/PPT styles) and RC4/AES encryption with empty, user or owner password. Palette and high-throughput
  JPEG 2000 still need an external renderer.
- **Batch processing aids**: bar-code separator sheets (Code 128, EAN-13, ITF, Code 39), SHA-256 sums, JSON-lines audit log with
  retention / legal-hold fields, encrypted packaging. We found no evidence of this exact combination elsewhere, but absence of
  evidence in closed products is not proof; do not read it as uniqueness.
- **Mouse-drawn dewarp guides** that follow rotation, split, deskew and perspective.

Where it is **not** better: OCR accuracy (§2), binarisation variety (ScanTailor Experimental), mature dewarp meshes
(ScanTailor Advanced), page-layout analysis and recognition languages (ABBYY), Office-grade DOCX fidelity, support, and track
record. LegatorOCR is young (0.2 pre-release), has a single author and no external users yet.

## 7. Honest limits that remain

- PDF/A: output samples pass veraPDF PDF/A-2b in CI; that is not a certification of every output.
- Windows, macOS and ARM64 builds come from CI. The CLI binary is smoke-tested there; the GUI was exercised by hand only
  on Linux x86-64 (under a virtual display).
- Native OCR: real-page CER 17.5 % vs Tesseract 14.6 % on 12 pages; weaker on narrow gutters and noisy pages.
- Dewarp corrects vertical bend only. Word export of complicated tables is heuristic.
- Palette and high-throughput JPEG 2000 and JBIG2 Huffman refinement with empty symbols need an external renderer.
- 7z was tested with real archives; RAR through a hand-built RAR4 archive and the 7z reader only, not with files made by
  WinRAR.
- No independent Romanian OCR benchmark exists for any engine; measure on your own pages.

## Sources (fetched 2026-11-02 unless stated)

ScanTailor: gitea.com/ImageProcessing-ElectronicPublications/scantailor-advanced/releases · github.com/ScanTailor-Advanced/scantailor-advanced ·
github.com/ImageProcessing-ElectronicPublications/scantailor-experimental (release 1.2026.10.10, README) ·
github.com/trufanov-nok/scantailor/releases · alternativeto.net (ScanTailor Universal, ScanKromsator).

Commercial: pdf.abbyy.com/specifications · help.abbyy.com/en-us/finereader/16/user_guide/{pdf_settings,adjustimage,hotfolder,commandline_convert} ·
en.wikipedia.org/wiki/ABBYY_FineReader · marvienta.com/tools/abbyy-finereader · adobe.com/acrobat/pricing ·
helpx.adobe.com/acrobat-com/kb/using-ocr-exportpdf.html · experienceleague.adobe.com (Optimize Scanned Documents) ·
community.adobe.com (ClearScan thread) · tungstenautomation.com (OmniPage, Power PDF vs Acrobat) ·
docshield.tungstenautomation.com (OmniPage 19.2 release notes) · capterra.com / softwareadvice.com (Readiris, Nitro, Power PDF) ·
tools.pdf24.org/en/creator · help.pdf24.org/en/?p=6056 · resource-cms.foxit.com (Recognize Text, add-ons, auditing logs) ·
apps.apple.com (Nitro Mac) · software-express.de (Nitro 14) · pdf.wondershare.com/pdfelement/tech-spec.html ·
aiproductivity.ai/pricing/{abbyy-finereader,adobe-acrobat,foxit-pdf-editor,wondershare-pdfelement} ·
thurrott.com/?p=331644 (Microsoft Lens) · lido.app/blog/google-ocr-options-explained · cloud.google.com/document-ai/pricing ·
developer.apple.com/documentation/vision.

Open source and models: github.com/tesseract-ocr/tesseract/releases · raw.githubusercontent.com/tesseract-ocr/{tessdata_best,tessdata_fast,tessdoc} (ron files inspected) ·
github.com/tesseract-ocr/langdata_lstm/issues/37 · pypi.org/pypi/ocrmypdf/json · raw.githubusercontent.com/ocrmypdf/OCRmyPDF/main/docs ·
releasebot.io/updates/paperless-ngx · raw.githubusercontent.com/paperless-ngx/paperless-ngx/dev/docs · paddleocr.dev · docsumo.com/blog/best-ocr-models ·
codesota.com/ocr · edenai.co (Mistral OCR 4) · runtimewire.com (Mistral OCR 4.1) · github.com/VikParuchuri/surya ·
aclanthology.org/2023.ldk-1.20.pdf · arxiv.org/abs/1810.03436 · aclanthology.org/2025.nodalida-1.11.pdf · ludditus.com/2020/08/06/ocr.
