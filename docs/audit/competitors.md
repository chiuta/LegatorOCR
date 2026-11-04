# Audit: competitor facts (read 2026-11-04)

Scope: `COMPARISON.md`, `docs/ADVERSARIAL-ANALYSIS.md`. Method: live pages through a real Chrome browser (GitHub release pages read from their
`datetime` attributes, so years are certain), raw READMEs via curl, PyPI JSON. No program was run. "Repo said" = text before this audit.

## Verified / changed

| Item | Repo said | Source says now | Source (read 2026-11-04) |
|---|---|---|---|
| ScanTailor Advanced PDF export (conflict) | scantailor.net: PDF output; COMPARISON: images only | Project READMEs (vigri, 4lex4, ScanTailor-Advanced): "scanning, optical character recognition, and assembling multi-page documents are out of scope"; no release note v1.0.17-v1.2.1 mentions PDF. scantailor.net itself says both "TIFF, PDF" (step 6) and "Supported formats: TIFF, JPEG, PNG; import ScanTailor's TIFFs to OCR". Verdict: no documented PDF export. Not tested by running. | raw README of the three repos; scantailor.net; scantailor.net/download/ |
| ScanTailor Advanced latest | 1.0.21 "snake" 2025-12-20 | github.com/ScanTailor-Advanced/scantailor-advanced: **v1.2.1 2026-05-24**, v1.2.0 2026-05-24, v1.1.1 2026-04-03 (changelog date), v1.0.19 2023-07-16. Gitea fork 1.0.21 2025-12-20 confirmed (different fork). 4lex4: last release 2019-08-16 (EA). scantailor.net: Windows v1.1.1, Linux v1.2.1, no official macOS | github releases pages; gitea releases; scantailor.net/download |
| ScanTailor Experimental | 1.2026.10.10, "21 Oct", year not shown | 1.2026.10.10 = 2026-10-21; 1.2026.08.08 = 2026-08-07; 1.2026.07.07 = 2026-07-07 | github releases |
| ScanTailor Universal | 0.2.14, "19 Aug", year not shown, "pre-release" | 0.2.14 = **2023-08-19** (0.2.13 2021-10-21); not active | github.com/trufanov-nok/scantailor-universal/releases |
| Tesseract | 5.5.3 vs 5.5.4 conflict | 5.5.4 = 2026-10-28; 5.5.3 = 2026-07-24; 5.5.2 = 2025-12-26 | github.com/tesseract-ocr/tesseract/releases |
| OCRmyPDF | 17.13.0, 2026-09-28 | same (PyPI upload 2026-09-28; 17.12.1 2026-09-16) | pypi.org/pypi/ocrmypdf/json |
| PaddleOCR | 3.7 | 3.7.0 = 2026-06-11 (PyPI). PaddleOCR-VL-1.6 date not re-checked | pypi.org/pypi/paddleocr/json |
| Paperless-ngx | 3.4.0, 2026-10-29 | **v3.4.2 = 2026-11-03** (3.4.1 2026-10-31) | github releases |
| ABBYY FineReader PDF Romanian | vendor spec, 198 languages (from a collapsed list we could not open) | Accordion opened: 198 languages, 53 with dictionaries; "Romanian" has a dictionary mark, "Romanian (Moldova)" listed without | pdf.abbyy.com/specifications |
| ABBYY current version | 16.0.14.7295 (Aug 2023) | Vendor home still says "FineReader PDF 16 for Windows" as current; no newer major seen; build number unknown | pdf.abbyy.com |
| Adobe web OCR languages | Romanian not listed (15) | Confirmed: page "last updated Apr 30, 2026"; 15 entries, no Romanian | helpx.adobe.com/acrobat/web/.../ocr-supported-languages.html |
| Foxit | Romanian OCR ? | Still ?: vendor blog (Apr 2023) confirms multilingual OCR with a language picker, no list | foxit.com/blog/how-to-work-in-multi-language-using-pdf-editor |
| NEW competitor: ScanTailor FanMode | absent | hakami55/ScanTailor-FanMode, GPL-3 fork of ScanTailor Advanced, "0.1.0-alpha.0", Windows x64 primary; adds PDF-to-TIFF import, RTL, covers, dewarp models, **searchable (Arabic/English) or image-only PDF export** (Tesseract, Poppler, QPDF bundled). No GitHub release; commits 2026-04-20 to at least 2026-05-25. Stronger than any earlier ScanTailor-based competitor on the "whole chain" claim; alpha. | github.com/hakami55/ScanTailor-FanMode (README, commits, releases) |
| Other new items | - | scan_book (ThomasHoussin): Windows WIA scanning + auto deskew/crop + image PDF, no text layer, no split/dewarp; its LLM-OCR benchmark on 16 French pages (own figures) shows CER 0.35-0.72 %. pdf-book-ocr (Joffoo): multimodal AI to EPUB/Markdown. Neither is a direct rival. | github.com/ThomasHoussin/scan_book; github.com/topics/book-scanning |

## Could not verify (left as "?" in COMPARISON.md; do not treat as "no")

- ABBYY edition prices (vendor buy page redirected back to the spec page; aggregator figures kept) and the newest 16.x build.
- Adobe Acrobat **desktop** OCR language list (help pages name a language choice without a list; page guesses returned 404).
- Foxit and Nitro Romanian OCR (Nitro's FAQ "supports multiple languages?" would not expand; a cookie banner was in the way and was not accepted).
- Readiris 17: readiris.com did not load, and Readiris was not on iriscorporate.com/products; status unknown.
- scantailor.net/ScanTailor Advanced: gh API is blocked, so release assets and download counts were not read; v1.1.1 vs v1.2.1 binaries for Windows not checked.
- PaddleOCR-VL-1.6, Mistral OCR 4/4.1, Chandra, olmOCR dates and Google Document AI price (not re-read this pass).
- GitHub "trending" has no stable page; new-competitor search was web search plus GitHub topics (scantailor, book-scanning) sorted by update. It is not exhaustive.
- Whether FanMode can use `ron` Tesseract data (README mentions only Arabic/English data), and its real output quality.

Deleted afterwards: scratch downloads in the session scratchpad.

## Added 2026-11-05 (live pages in a real browser; GitHub search by recent activity; nothing was run)

| Tool | What the page says | Read |
|---|---|---|
| ScanTailor OCR (github.com/2ndmax/scantailor-ocr, scantailorocr.eu) | GPL-3, built on ScanTailor Advanced; built-in PDF export (lossless JBIG2 for B/W, layered for mixed, PDF/A-2b), Tesseract text layer, watched folder, `scantailor-ocr-cli`; Windows + Linux; UI English and German; 1.0.11 on 2026-11-01; 356 commits, 0 stars, 0 forks. Romanian OCR not confirmed. | **Direct competitor**: it does what COMPARISON.md called LegatorOCR's distinguishing chain, on a mature interactive base. |
| EVB Viewer (github.com/evb0110/evb-viewer) | MIT; Rust cleanup engine (deskew, despeckle, binarise, crop, split), Tesseract OCR in 31 languages (English and Russian bundled), searchable PDF, DOCX, DjVu, annotations, local MCP server; macOS (Apple Silicon), Windows, Linux x64/ARM64, browser build; v0.1.502 on 2026-11-02; SHA256SUMS and provenance files. Romanian not named in the page text read. | Direct competitor with more platforms than LegatorOCR has verified. |
| Aglaïa (github.com/yb85/aglaia) | Webcam/PDF book scanner, deskew + dewarp, searchable PDF; PolyForm Shield licence (not an OSI open-source licence); 0.1.0rc11 on 2026-10-28; "alpha, it will crash", tested mainly on macOS. | Overlaps on book scanning; different capture focus. |
| ScanTailor PDF+ (github.com/Atelier-Recherche/scantailor-pdf-plus) | GPL-3 fork of ScanTailor Experimental; PDF import/export; Windows x64 zip; 1.2026.08.08 on 2026-08-20; OCR "out of scope". | PDF without OCR. |
| NAPS2 | 8.6.0 on 2026-10-25. | Scanning + OCR; not a post-processor. |

Not read, seen only in lists or search results: scantailor-deviant, mrcpdf / mrc-pdf-kompressor (MRC compression), scanbutler, ocrmypdf-rs (Rust crate), franken_ocr, hocr-files-template-repo, ScanPostProcess. Commercial book tools (BookDrive Editor Pro, Booksorber, LIMB, ScanGate LWF) come from an awesome-list; activity today not checked.

Measured on 2026-11-05 on the 12 evaluation pages: OCRmyPDF 17.13.0 (Tesseract `ron` fast, PDF/A-2b, no unpaper/pngquant/JBIG2) CER 4.82 % GOOD8, 7.88 % ALL12, against 4.54 % / 7.81 % for LegatorOCR with the same Tesseract; PDFs 4.2-7.6 MB against 0.7-1.4 MB (not equal image quality); 3.3-7.7 s against 2.0-5.7 s (Tesseract) and 1.6-3.3 s (native) per page. Prices read from vendor pages: ABBYY FineReader PDF 99 EUR/yr Standard, 165 EUR/yr Corporate, 69 EUR/yr Mac (pdf.abbyy.com/pricing); Adobe Acrobat Pro 24.19 EUR/month, Standard 18.30 EUR/month incl. VAT, annual billed monthly (adobe.com/acrobat/pricing, Romania). IRIS lists Readiris 16 and its Readiris 17 page returned 404.
