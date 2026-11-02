# LegatorOCR roadmap (draft of 2 November 2026)

Ordered from simple to complex and by logical dependency. **D** = difficulty 1 (hours) to 5 (months, research risk).
Effort figures are rough estimates by the author of this plan, not measurements. Status as of 2 November 2026 is in the next section; items not listed there are not started. Ideas that depend on research carry a **gate**: a measurement that decides whether to continue.

Working rule for every item: record a number *before*, build, record a number *after*; if it does not improve, remove it.

## Status (2 November 2026)

"Done" means implemented, covered by a test and CI green; it does not mean tuned on real material.

| ID | State | Note |
|---|---|---|
| 0.3 | done | as `--timings` (time per stage, peak memory) |
| 0.4 | done | `--doctor` |
| 1.1 | partial | `--skip-text` only; `redo` / `force` modes not done |
| 1.2 | done | `--clean-text`; limits: joins hyphenated lowercase compounds |
| 1.3 | done | `--alto`, `--page-xml`; only checked for well-formedness, not against the XSDs |
| 1.5, 1.13 | done | `--quality-report`; thresholds are untuned heuristics |
| 1.6 | done | `--page-lang` |
| 1.7 | done | `--explain` |
| 1.8 | done | `--page-labels`; needs OCR to see the printed number |
| 1.9 | done | `{title}` naming token (needs `--ocr`), plus `{date}` `{year}` `{month}` `{day}` `{name}` |
| 1.10, 1.19 | done | `--search-query` (boolean, fuzzy, diacritic-insensitive) with CUI and invoice-series finders; words match whole tokens |
| 1.12 | partial | lone page numbers in the head/foot margin recovered (CER on 12 real pages unchanged at 15.2 %); gutter, "." vs ",", I/l, word spacing not fixed |
| 1.14 | done | `--burst`; thresholds untuned on real phone photos |
| 1.15 | done | `--scan-active` (static; no online hash lookup) |
| 1.16, 1.18 | done | `--catalog` (HTML / CSV with SHA-256) and `--dublin-core` sidecars; need `--pdf` |
| 1.17 | done | `--verify-text` |
| 5.8 | partial | `--bagit DIR` (BagIt 1.0 bag with SHA-256 manifest and tag manifest) and `--verify-bag DIR`; no multi-destination 3-2-1 copy helper yet |
| 6.9 | partial | `--rights-death-year` / `--rights-author` write the life + 70 years notice (Law 8/1996); assignment-deed fields not done; informational only |
| 2.1 | partial | `--ocr-cache DIR` caches the OCR stage only (key: page pixels + OCR settings + engine and program version); the clean-up stages are not cached |
| 2.3 | done | `--watch DIR` (needs `--pdf` with `{name}`; runs the program once per new file; polls, no OS file events) |
| 1.4 | deferred | no reference set to prove a gain, and the exact definitions of Fox / LCaM / EdgeDiv could not be checked here |
| 0.5 | done | CI step runs the whole chain in an empty network namespace (Linux) |
| 0.2 | partial | CI job `ocr-regression` runs `scripts/ocr-bench.py --max-cer 0.3` on 24 synthetic pages (8 languages x 3 scan conditions, exact ground truth; mean CER 0.007 %); a regression guard only, no size or time gate yet |
| 0.1, 0.6 | not started | 0.1 (real pages) needs hand-verified text from a person; the synthetic set above is the stand-in |
| 1.11 | blocked | a real PortableApps package needs the Windows PortableApps.com Launcher / Installer; not testable here |

Phase 2 other than 2.3, and everything from Phase 3 on, is not started.

## Phase 0 - Measure first (D1-3)

Everything later is unverifiable without these.

| ID | Item | D | Depends on |
|---|---|---|---|
| 0.1 | Public reference set: pages with hand-verified text (several languages, old book, phone photo, modern document) | 2 | - (needs human proofreading) |
| 0.2 | CER / time / size regression report in CI; fail on regressions | 2 | 0.1 |
| 0.3 | `--profile` (time and memory per stage and page) | 1 | - |
| 0.4 | `doctor` command (Tesseract, pdftoppm, mutool, languages, versions) | 1 | - |
| 0.5 | CI test that runs the whole chain with the network blocked | 2 | - |
| 0.6 | Fuzzing of PDF, JBIG2, JPEG 2000, archive readers; differential tests against poppler / pdfium / OpenJPEG in CI | 3 | - |

## Phase 1 - Quick wins (D1-2, hours to 2 days each)

| ID | Item | D | Depends on |
|---|---|---|---|
| 1.1 | `--mode skip / redo / force` for OCR | 1 | - |
| 1.2 | Romanian text post-processing: comma-below s/t, de-hyphenation, quotes, spaces before punctuation | 1 | 0.1 to measure |
| 1.3 | ALTO XML and PAGE XML output | 2 | - |
| 1.4 | More binarisers (Niblack, Gatos, LCaM, Window, Fox, EdgeDiv) | 2 | - |
| 1.5 | Quality alerts: blur, low dpi, cut-off text, page-number gaps, duplicates | 2 | - |
| 1.6 | OCR language chosen per page | 2 | - |
| 1.7 | `--explain` per page (what each stage decided and why) | 2 | - |
| 1.8 | PDF `/PageLabels` from detected page numbers | 2 | - |
| 1.9 | Output naming templates (`{date}_{title}`) | 1 | - |
| 1.10 | Fuzzy, diacritic-insensitive `grep` over processed PDFs (reuses the search engine) | 2 | - |
| 1.11 | Portable package (PortableApps) built in CI | 1 | - |
| 1.12 | Fix observed OCR defects: narrow gutter (p13), "." read as ",", I/l, word spacing | 3 | 0.1 |
| 1.13 | Effective-dpi advisor: from pixel size and page size, warn below ~200 dpi (OCR misreads) and note that above ~400 dpi plain text gains nothing but time (guide's figures) | 1 | - |
| 1.14 | Burst ingest for phone capture: order by EXIF time, honour EXIF orientation, drop near-duplicate and blurred frames, keep the sharpest of a burst | 2 | - |
| 1.15 | Static scan of input PDFs for active content (JavaScript, launch actions, embedded files) and an offline hash report; any online hash lookup (e.g. VirusTotal) only as opt-in, hashes never files | 2 | - |
| 1.16 | Integrity catalogue: titles + SHA-256 as HTML / CSV ready to publish, built on `--checksums` | 1 | - |
| 1.17 | Text-layer self-test: pick rare words from the OCR text and confirm they are found in the PDF (the guide's Ctrl+F check, automated) | 2 | 1.10 |
| 1.18 | Library metadata: Dublin Core / MODS sidecar and CSV catalogue from PDF metadata | 2 | - |
| 1.19 | Search operators in `grep` as in DocFetcher (AND / OR / NOT, wildcards, regex) with ready patterns for Romanian CUI / fiscal code and invoice series | 2 | 1.10 |

## Phase 2 - Throughput (D2-4)

| ID | Item | D | Depends on |
|---|---|---|---|
| 2.1 | Content-hash cache per stage (re-run only what changed) | 3 | 0.3 |
| 2.2 | Page-parallel scheduling with a memory cap | 3 | 0.3 |
| 2.3 | `watch` command (hot folder) | 2 | 1.1 |
| 2.4 | Streaming PDF decoding for multi-GB files | 4 | 0.3 |
| 2.5 | SIMD for thresholding / resampling | 4 | 0.3 shows it matters |
| 2.6 | GPU or multi-machine batches | 5 | only if 0.3 shows a need |

## Phase 3 - Interface (D2-4)

| ID | Item | D | Depends on |
|---|---|---|---|
| 3.1 | Start screen, 3-step wizard, simple / expert mode | 3 | - |
| 3.2 | Before / after slider, drag and drop, plain-language errors, tooltips with examples | 2 | - |
| 3.3 | Full undo / redo and per-page reset for every setting | 3 | - |
| 3.4 | Accessibility: contrast, font size, keyboard-only use, screen reader | 3 | - |
| 3.5 | "Needs attention" panel and book map (green / yellow / red) | 3 | 1.5 |
| 3.6 | Proofreading in the window (cropped image + editable word) | 4 | 4.x confidences |
| 3.7 | Tuning by comparison (3-4 variants side by side) | 3 | 2.1 |
| 3.8 | Scanning-station mode (live verdict per new page) | 4 | 1.5, 2.1 |
| 3.9 | Usability test with 5 new users; fix what they trip on | 2 | 3.1-3.3 |
| 3.10 | Pre-scan checklist inside the app (opening bindings, staples, flattening folds) and capture tips per document type | 1 | - |
| 3.11 | Scan from a device: network scanners over eSCL (AirScan) first, SANE on Linux; WIA / TWAIN later (replaces the NAPS2 step of the guide's workflow) | 4 | - |
| 3.12 | PDF toolbox actions: merge / split, watermark, page numbering, web-optimised compression (the PDF24 Toolbox step) | 3 | - |

## Phase 4 - OCR accuracy (D3-5, research risk)

| ID | Item | D | Gate |
|---|---|---|---|
| 4.1 | Per-word confidence stored in text layer / hOCR | 2 | - |
| 4.2 | Ensemble: native engine + Tesseract, word-level voting | 3 | CER below the best single engine on 0.1 |
| 4.3 | Romanian dictionary post-correction on low-confidence words (flag, do not auto-change proper nouns) | 3 | CER improves, no new errors on names |
| 4.4 | Glyph clusters per book, label voting across all instances | 4 | CER improves on single-font books |
| 4.5 | "Teach the font from 10 pages" (user labels clusters once) | 4 | 4.4 |
| 4.6 | Calibrated per-page error estimate (no reference needed) | 4 | estimate tracks real CER on 0.1 |
| 4.7 | "Oglinda": re-typeset the page from the book's own glyphs, compare with the scan, search alternatives where the residual is high | 5 | prototype on one page: does the residual map separate correct from wrong text? If not, stop |

No gate promises near-zero CER. Report CER with a confidence interval; zero errors in N characters only proves a rate below about 3/N.

## Phase 5 - Archive and trust (D3-5)

| ID | Item | D | Depends on |
|---|---|---|---|
| 5.1 | Reproducible builds, SBOM, signed releases, `SECURITY.md` | 3 | - |
| 5.2 | Per-page verification certificate: re-decode compressed output and compare with the source; render the text layer and compare with the image | 3 | 0.2 |
| 5.3 | Provenance in the PDF (XMP: parameters, hashes, versions), reproducible output bytes | 3 | 5.1 |
| 5.4 | Signed timestamp (RFC 3161) and a Merkle-tree integrity register for collections | 4 | 5.3 |
| 5.5 | Sandboxed decoders for untrusted files | 4 | 0.6 |
| 5.6 | Reprocessable archive package (original pixels + transform graph + text with per-word provenance; re-run when engines improve) | 5 | 2.1, 5.3 |
| 5.7 | PAdES signing with the user's own certificate (PKCS#12) and RFC 3161 timestamp, plus signature verification on input files (eIDAS levels are the user's certificate, not ours) | 4 | 5.3 |
| 5.8 | BagIt / OCFL packaging and a 3-2-1 backup helper: copies to several destinations, hash-verified, with a read-only / cold-storage checklist | 3 | 1.16 |
| 5.9 | Target-based quality report in the spirit of FADGI / Metamorfoze: detect a standard test target in a scan and measure sharpness (SFR), colour error (delta E), noise and illumination uniformity, then state the level reached. Needs the official standard documents before any threshold is coded (see note below) | 5 | - |
| 5.10 | Hand-over package for licensed electronic-archiving providers (e.g. TRANSFOND e-Arhiv@, Law 135/2007); accepted formats must be checked with the provider first | 3 | 5.8 |

## Phase 6 - Outputs (D3-5)

| ID | Item | D | Depends on |
|---|---|---|---|
| 6.1 | PDF/A-1, -3, -4 and PDF/UA tagging | 4 | - |
| 6.2 | PDF outline / bookmarks from headings and page numbers | 3 | 1.8 |
| 6.3 | Table export (XLSX, HTML) | 4 | - |
| 6.4 | TEI-XML and faithful reflow (headings, footnote links, bibliography, index) | 4 | 6.2 |
| 6.5 | Accessibility pipeline: tagged PDF, braille, audiobook in reading order | 4 | 6.1 |
| 6.6 | DjVu foreground / background layers | 3 | - |
| 6.7 | Vector font reconstruction from glyph clusters (note: Adobe's ClearScan did something similar; our difference would be verified fidelity) | 5 | 4.4, 5.2 |
| 6.8 | Scan a paper form, detect lines and boxes, produce a fillable PDF (AcroForm fields); replaces the OnlyOffice step and goes further | 5 | 6.1 |
| 6.9 | Rights wizard: public-domain date from the author's death year (life + 70 years under Law 8/1996), fields for assignment deed number and date, printed in the page-2 notice (we already have `--rights-notice`); informational, not legal advice | 2 | - |
| 6.10 | e-Factura (RO) invoice XML / PDF recognition and extraction of key fields | 4 | - |

## Phase 7 - Ecosystem and moonshots (D3-5)

| ID | Item | D | Depends on |
|---|---|---|---|
| 7.1 | Python and Node bindings, local REST API, MCP server | 3 | stable CLI |
| 7.2 | Native packages (winget, Homebrew, Flathub, AUR, Debian) | 3 | 5.1 |
| 7.3 | Documentation by task, community translations with coverage indicator | 2 | - |
| 7.4 | Domain profiles (diplomas, civil registers, old newspapers, maps, scores) each with its own tests | 3 | 0.1 |
| 7.5 | Document verifier: seals, stamps, copy-paste and double-JPEG detection, MRZ and QR reading | 4 | 7.4 |
| 7.6 | Multi-frame fusion for phone photos | 4 | - |
| 7.7 | Browser (WASM) build of the same core | 4 | 7.1 |
| 7.8 | WASM plugin interface for engines and formats | 4 | 7.1 |
| 7.9 | Peer-to-peer proofreading between libraries | 5 | 3.6, 4.4 |
| 7.10 | Open compression format with a reference WASM decoder | 5 | 5.2; needs standardisation effort, adoption not guaranteed |
| 7.11 | Phone capture app (PWA / WASM): burst, remote trigger, live page verdict, same core as the desktop | 4 | 7.7, 3.8 |
| 7.12 | PII finder and redaction (true removal of pixels and text) for GDPR hand-over | 4 | 6.1 |

## Source of these additions

Added on 2 November 2026 after reading Alexandru Dumbravă's portfolio and digitisation guide
(<https://dumbravaalex.github.io/Portofoliu/>, 12 pages). His blog (competentedigitalizare.blogspot.com) could not be fetched from this
environment, so nothing here is taken from it. Only the *workflow ideas* are used, never code of the tools he names.

What the guide recommends and where we stand:

| Guide step | Status in LegatorOCR |
|---|---|
| Capture with a phone, burst and Bluetooth remote | not covered: 1.14, 7.11 |
| ScanTailor clean-up, dewarp, crop | covered |
| NAPS2: frames to PDF, direct scanning | frames to PDF covered; scanner acquisition not: 3.11 |
| PDF24: OCR (Romanian + English), compression, watermark, page numbers | OCR and compression covered; watermark / numbering not: 3.12 |
| DocFetcher: full-text search | covered (`--txt-each`, search engine); fuzzy search planned: 1.10 |
| Folder2Iso | covered (`--package` .iso) |
| OnlyOffice fillable forms | not covered: 6.8 |
| Electronic signature with a valid certificate (eIDAS) | not covered: 5.7 |
| VirusTotal and published SHA-256 table | hashes covered; catalogue and active-content scan not: 1.15, 1.16 |
| Cold-storage backups | not covered: 5.8 |
| Assignment notice on page 2 | covered (`--rights-notice`); wizard planned: 6.9 |
| Three document profiles (office / sketches / registers) | covered (profiles `doc-birou`, `schite-desene`, `registre-manuscrise`) |
| 300 / 400 / 300-600 dpi advice | profiles cover it; advisor planned: 1.13 |

The portfolio guide does not cover naming conventions, PDF/A, e-Factura, 3-2-1 backup, GDPR or retention periods. Items 6.10, 7.12 and 5.8
come from the wider HUB DIGITAL course package, whose topics (eIDAS signature levels, retention under the archival nomenclature, RO e-Factura
structure, the 3-2-1 rule, DocFetcher query syntax, ransomware hygiene, an electronic-archiving provider such as TRANSFOND e-Arhiv@) are recorded in an
earlier conversation of the author of this repository. That record is a summary of an AI-assisted review, so treat the topics as
the course's, but confirm details with the author before attributing a recommendation to him.

### Further sources found on 2 Nov 2026

Read in full (page content, not just titles):

- eIDAS 2 (certyneo.com summary, secondary source): Regulation (EU) 2024/1183 in force since 20 May 2024; each Member State must offer an EU Digital Identity Wallet **no later than November 2026**; implementing acts still being published, so details may change. A qualified signature carries a presumption of reliability (Art. 26) and can be given remotely (Art. 29a/29b). Affects 5.7: plan for remote qualified signing through a provider, not only a local PKCS#12 certificate.
- FADGI Technical Guidelines for Digitizing Cultural Heritage Materials, Third Edition (2023, page updated 2024-02-27): the page links to the free evaluation tools **OpenDICE** and **AutoSFR**. Affects 5.9: read them and the guideline PDF first, then decide whether to wrap, import or reimplement their measurements.
- PDF Association (March 2024): LibreOffice 24.2 exports PDF that is both PDF/A and PDF/UA-1; veraPDF 1.26 release candidate had draft PDF/UA-2 checks. The page says nothing about scanned or OCR documents. Affects 6.1: PDF/UA for scans is our own work, with no reference implementation found.

Found as titles only (not read; do not rely on them): Quarto/Posit note on PDF accessibility (Mar 2026), Europeana content-tier pages, an
IIIF Content Search API 1.0 page, Australian National Archives preservation digitisation standards (page refuses robots), assorted OCR
benchmark roundups for 2026 (docsumo, codesota, gigagpu, others), and book-scanner software lists (spreads, YASW, BookScanWizard). A 2026 search for a pure-Rust OCR
engine found nothing relevant.

No Romanian-specific OCR benchmark, Romanian national digitisation norm or new ScanTailor-class tool turned up in these searches.

### Sources we could not read (2 Nov 2026)

- The blog competentedigitalizare.blogspot.com, its RSS feed and the Internet Archive copy: blocked by this environment's allowlist; search engines do not index the blog.
- The PMC article on documentary heritage in Romanian libraries (PMC9870124) and Europeana's tier requirements for text: not readable here.
- FADGI / Metamorfoze: only a vendor overview (Digital Transitions) was read. It gives the star levels and the measured quantities
  (SFR, delta E, noise, uniformity) but not the standard's full thresholds, and it states no ppi values. Item 5.9 must wait for the
  official documents. Europeana requirements should be checked before the library-oriented outputs (1.18, 6.x) are finalised.

## Not promised

Leadership, specific accuracy, or adoption. Those are outcomes; this plan only lists work that can be built and measured.
Windows, macOS and ARM64 builds stay CI-tested until someone runs them on real hardware.
