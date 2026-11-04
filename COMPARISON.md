# LegatorOCR compared with the field (as of 2 November 2026; selected cells re-verified 4 November 2026)

> Updated 4 Nov: for the harsher, claim-by-claim critique see `docs/ADVERSARIAL-ANALYSIS.md`. Re-verified against live sources on 2026-11-04 (details and "could not verify" list in `docs/audit/competitors.md`).
>
> **4 Nov conflict resolved:** the ScanTailor Advanced README (vigri and 4lex4 repos, read raw) says "scanning, optical character recognition, and assembling multi-page documents are out of scope", and no release note of v1.0.17-v1.2.1 mentions PDF export. Only the anonymous community site scantailor.net says "Export as high-contrast TIFF, PDF, or layers" in one step, while its own format list says "TIFF, JPEG, PNG" and its OCR advice is "import ScanTailor's TIFFs". So: **no native PDF export is documented by the project; the site's PDF line is unsupported.** Not tested by running the program.
> **New 4 Nov finding that weakens the "ScanTailor stops at images" claim:** a ScanTailor Advanced fork, *ScanTailor FanMode* (hakami55, alpha), does add PDF import and searchable/image-only PDF export with Tesseract (see §3).

This file compares LegatorOCR with the scan post-processing tools (ScanTailor family), the commercial OCR/PDF suites and the
open-source and cloud OCR engines. It was compiled on 2026-11-02 from vendor pages, release notes, package indexes and
third-party articles; LegatorOCR's own claims come from tests run here.

How to read it:

- **✔** = documented on a page we fetched (vendor claim) or tested here (LegatorOCR). **✘** = documented as absent.
  **?** = we could not confirm it; it is *not* a "no". We did not fill any cell from memory.
- *Vendor claim* is not independent evidence. Prices come mostly from review aggregators and may be out of date.
- Several vendor sites could not be read by our tools on 2 Nov, hence the many "?" cells for closed products. On 4 Nov a
  real browser closed some of them (ABBYY language list, GitHub release dates); the rest are listed as "could not verify" in `docs/audit/competitors.md`.
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
(ș, ț) and the cedilla (ş, ţ) forms, and a public issue (langdata_lstm #37, open when fetched; still open and unanswered since the maintainers' questions, re-read 2026-11-04) reports that the training
data maps Ș/ș/Ț/ț to the cedilla forms. We did not measure which form `ron` emits most; a normalisation step after OCR is
justified, not proven.

## 3. Scan post-processing tools

| | ScanTailor Advanced | ScanTailor Experimental | ScanTailor Universal | LegatorOCR |
|---|---|---|---|---|
| Latest we could confirm (read 2026-11-04) | **v1.2.1, 2026-05-24** (github.com/ScanTailor-Advanced/scantailor-advanced; v1.1.1 was released by user vigri, v1.2.x by pablogventura; whether github.com/vigri/ redirects there was not confirmed; v1.2.0 2026-05-24, v1.1.1 2026-04-03). Separate Gitea fork 1.0.21 "snake", 2025-12-20. *Changed 4 Nov: the 2 Nov row showed only the Gitea fork.* | "nail" 1.2026.10.10, **2026-10-21** (year now confirmed from the release timestamp) | 0.2.14, **2023-08-19** (year now confirmed; previous 2 Nov cell and the ADVERSARIAL file called it a 19 Aug "pre-release" without a year, and its newest listed release is over 3 years old) | 0.2.0 pre-releases (draft) |
| Price / licence | free, GPL-3 | free, GPL-3 | free, GPL-3 | free, open source |
| Platforms | Windows (v1.1.1 portable zip per scantailor.net), Linux (v1.2.1 .deb/AppImage; Flathub for the 4lex4 ID); no official macOS build (scantailor.net) | Debian, Windows x86/ARM64, Flatpak; macOS only in one 2025 build | Windows, macOS, Linux | Linux, Windows, macOS (last two from CI only, see §7) |
| Orientation, split, deskew, content box, margins | ✔ | ✔ (README) | ✔ | ✔ |
| Dewarp | ✔ semi-automatic, mesh / manual; "marginal dewarping" for curved borders | ? (one release note mentions correction parameters) | ? | ◐ automatic from text lines, or guide lines drawn with the mouse; vertical bend only |
| Binarisation | Sauvola, Wolf, Fox, Window, Grad (v1.1.1 notes), EdgeDiv/EdgePlus | many (Niblack, Sauvola, Wolf, Window, LCaM …) | ? | six + auto per page |
| Foreground / background layers | ✔ (for DjVu) | ? | ? | ✔ hybrid PDF (G4 stencil + low-res background + patches) |
| OCR / PDF output | ✘ images (TIFF/PNG/JPEG), DjVu-oriented layers; README: OCR and assembling documents "out of scope"; no PDF export in any release note v1.0.17-v1.2.1. (scantailor.net's "TIFF, PDF" line is unsupported by the project.) | ✘ ("out of scope" per README) | ✘ (same README text) | ✔ own PDF writer with text layer |
| Perspective correction of photos | ? | ? | ? | ◐ automatic, conservative |
| Batch / CLI | ? | ? | ? | ✔ |
| Offline, no telemetry | ✔ | ✔ | ✔ ("offline, no registration, ad-free") | ✔ |

**New on 2026-11-04: ScanTailor FanMode** (github.com/hakami55/ScanTailor-FanMode, GPL-3, "0.1.0-alpha.0", Windows x64 primary, no GitHub release published, commits from 2026-04-20 to at least 2026-05-25). It is a fork of ScanTailor Advanced that, per its own README, adds PDF-to-TIFF project import, covers, right-to-left ordering, extra dewarp models and **searchable (Arabic/English) or image-only PDF export** with Tesseract, Poppler and QPDF bundled. This is the same chain LegatorOCR sells (clean-up, then PDF with text), inside the mature ScanTailor GUI. Limits visible on its page: alpha, Arabic/English data only (no Romanian bundled; others would need user-supplied Tesseract data, not verified), "primary tested platform Windows", we did not run it. If it matures it is the most direct competitor found; the claim "the ScanTailor forks stop at images" in §6 is therefore true of the mainline forks only.

The forks keep adding binarisation methods (Experimental has more than LegatorOCR; we do not have EM denoise / deblur).
ScanKromsator (5.92, Windows, "free and proprietary") and Book Restorer (i2S) could not be assessed: no usable page. On 2026-11-04 we searched again (web search, GitHub topics scantailor and book-scanning sorted by update, 'ScanTailor alternative 2026'): besides FanMode we found only scan_book (ThomasHoussin, Windows WIA scanning with auto deskew/crop and image-only PDF export; no split, no dewarp, no text layer; OCR benchmark of LLMs on French pages, own figures), pdf-book-ocr (multimodal AI to EPUB/Markdown, not a PDF tool) and py-scantailor (2024 Python port, old). No commercial book-scan-cleanup product displaced the ScanTailor family, but the search is not exhaustive.

## 4. Commercial OCR / PDF suites

Prices are list prices seen on 2026-11-02 mostly via aggregators (currency often not stated; assumed US$).

| | ABBYY FineReader PDF 16 | Adobe Acrobat Pro / Studio | Tungsten Power PDF | Tungsten OmniPage Ultimate | Readiris 17 | PDF24 Creator | Foxit PDF Editor | Nitro PDF Pro | PDFelement 13 | LegatorOCR |
|---|---|---|---|---|---|---|---|---|---|---|
| Latest confirmed | 16.0.14.7295 (Aug 2023); pdf.abbyy.com (read 2026-11-04) still presents "FineReader PDF 16 for Windows" as current, no newer major version seen; build number ? | continuous, no version | ? | 19.2 (release notes 2019-10-30) | 17 (date ?; readiris.com did not load and Readiris is not on iriscorporate.com/products on 2026-11-04: could not verify its status) | 11.31.1 (2026-10-08) | 2026.x (numbering inconsistent) | Win 14 (2023); Mac 26.4 | 13 | 0.2.0-pre |
| Price | sub only: ≈US$99/yr Standard, ≈US$165/yr Corporate; Mac US$69/yr; no perpetual | Pro US$19.99–29.99/mo; Studio US$24.99–34.99/mo | US$129 Standard / 179 Advanced, one-time | US$499 one-time | from US$89 one-time | free | US$10.99–13.99/mo; perpetual US$209.99 | US$15/mo; Win 14 perpetual ≈€233; Mac US$139.99/yr | US$79.99/yr; perpetual US$129.99 | free |
| Platforms | Windows; reduced Mac | Win, Mac, web, mobile | Win (Std also Mac) | Windows only | Win (Mac per marketing) | Windows only | Win, Mac, web, mobile | Win, Mac | Win, Mac | Linux, Win, macOS |
| Romanian OCR | ✔ + dictionary: "Romanian" is marked with a dictionary and "Romanian (Moldova)" is listed without one (vendor spec read in full on 2026-11-04, 198 languages, 53 with dictionaries) | web OCR: **not listed** (page "Supported languages for OCR", updated 30 Apr 2026, 15 entries: English US/UK, Danish, Dutch, Spanish, French, Japanese, Russian, Italian, Chinese Traditional, Norwegian, Portuguese, Finnish, Swedish); desktop ? (desktop help pages name a language choice but no list; could not verify) | ? (102 languages) | ? (120+) | ? (138 claimed) | ? (online tool lists Romanian) | ? (vendor blog confirms multilingual OCR with a language picker but names no languages) | Mac: ✘ (5 languages); Win ? (gonitro.com/ocr has an FAQ "supports multiple languages?" that we could not expand) | ✔ (29–30 languages) | ✔ (native engine or Tesseract) |
| Deskew / straighten / perspective / split | ✔ deskew, ✔ "Straighten Text Lines", ✔ "Correct Trapezoid Distortion", ✔ split | partial: "Enhance scan" (details ?) | ? | ? | ? | ? | not mentioned | ? | ? | ✔ / ◐ / ◐ / ✔ |
| PDF/A | ✔ 1a–3u, PDF/UA | ? (third party: A-2b/3b) | ✔ A-1…A-4 | ? | ? | ? | ? | Mac ✔ | ✔ (Win) | ◐ A-2b, veraPDF on samples only, not a certification |
| Compression | ✔ MRC + PreciseScan smoothing; JBIG2 as export format | ? | ✔ MRC, "8× smaller" (vendor) | ? | "compression" ? | ? | downsample only | ? | ? | ✔ G4, hybrid layers, JBIG2/JPEG2000 on input |
| DOCX export | ✔ | ? | ? | ? | Word/Excel ✔ | ? | ? | ? | ? | ✔ with columns and tables (limits §7) |
| hOCR / ALTO | not documented | ? | ? | ? | ? | ? | ? | ? | ? | hOCR ✔, ALTO 4.4 ✔ and PAGE ✔ (schema-validated, see ROADMAP 1.3) |
| Batch / hot folder | Hot Folder (limits and edition ?); CLI saving needs an "Extended CLI" licence | Action Wizard incl. "Optimize Scanned Documents" | watched folder ✔ | Batch Manager ✔ | batch + watch folder | ? | multi-file OCR ✔ | Mac batch ✔ | batch ✔ | CLI + GUI batch |
| Local / offline | local components; not explicitly stated | desktop ?; AI Assistant is cloud | ? | ✔ local | listed | OCR language files are downloaded on demand; empty list without internet | ? | ? | ? | ✔ |

Notes:

- ABBYY Romanian recognition is now confirmed from the vendor's own list; accuracy on Romanian is still unmeasured by anyone independent.
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

| | Tesseract 5.5.4 | OCRmyPDF 17.13.0 | Paperless-ngx 3.4.2 | PaddleOCR 3.7 / PaddleOCR-VL-1.6 | Mistral OCR 4 / 4.1 | Google Document AI | olmOCR 2 / Chandra OCR 2 | Surya |
|---|---|---|---|---|---|---|---|---|
| Date (read 2026-11-04) | 2026-10-28 (5.5.3 was 2026-07-24; confirmed from release timestamps, year no longer inferred) | 2026-09-28 (PyPI) | 2026-11-03 (v3.4.2; v3.4.1 2026-10-31; the 2 Nov file said 3.4.0, 2026-10-29) | PaddleOCR 3.7.0 2026-06-11 (PyPI); VL-1.6 date not re-checked | 2026-06-23 / preview 2026-07-16 | n/a | Oct 2025 / Mar 2026 | 2.5 |
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
  CLI, 8 interface languages. The mainline ScanTailor forks stop at images (the FanMode alpha fork does not, see §3); OCRmyPDF/Tesseract start from existing scans.
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
- Native OCR: see ROADMAP 4.11/4.15/4.16 and `docs/ADVERSARIAL-ANALYSIS.md`: behind Tesseract on single-column pages (5.72 vs 4.54 % CER) and on the 1960 book page (recall 72.5 vs 91.2 %); the 17.5 vs 14.6 % figures in §2 used an older, flawed reference.
- Dewarp corrects vertical bend only. Word export of complicated tables is heuristic.
- Palette and high-throughput JPEG 2000 and JBIG2 Huffman refinement with empty symbols need an external renderer.
- 7z was tested with real archives; RAR through a hand-built RAR4 archive and the 7z reader only, not with files made by
  WinRAR.
- No independent Romanian OCR benchmark exists for any engine; measure on your own pages.

## Sources (fetched 2026-11-02 unless stated)

ScanTailor (re-read 2026-11-04): github.com/ScanTailor-Advanced/scantailor-advanced/releases (release notes v1.0.17-v1.2.1) · raw README of vigri/, 4lex4/, ScanTailor-Advanced/ repos, scantailor-experimental, trufanov-nok/scantailor-universal · scantailor.net and scantailor.net/download/ · github.com/hakami55/ScanTailor-FanMode · github.com/ThomasHoussin/scan_book · github.com/topics/scantailor, /book-scanning · gitea.com/ImageProcessing-ElectronicPublications/scantailor-advanced/releases · github.com/ScanTailor-Advanced/scantailor-advanced ·
github.com/ImageProcessing-ElectronicPublications/scantailor-experimental (release 1.2026.10.10, README) ·
github.com/trufanov-nok/scantailor/releases · alternativeto.net (ScanTailor Universal, ScanKromsator).

Commercial (4 Nov additions: pdf.abbyy.com, helpx.adobe.com/acrobat/web/share-review-and-export/export-and-print/ocr-supported-languages.html, foxit.com/blog/how-to-work-in-multi-language-using-pdf-editor, gonitro.com/ocr): pdf.abbyy.com/specifications · help.abbyy.com/en-us/finereader/16/user_guide/{pdf_settings,adjustimage,hotfolder,commandline_convert} ·
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
