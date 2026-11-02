# LegatorOCR roadmap (draft of 2 November 2026)

Ordered from simple to complex and by logical dependency. **D** = difficulty 1 (hours) to 5 (months, research risk).
Effort figures are rough estimates by the author of this plan, not measurements. Nothing here is implemented yet unless
the README says so. Ideas that depend on research carry a **gate**: a measurement that decides whether to continue.

Working rule for every item: record a number *before*, build, record a number *after*; if it does not improve, remove it.

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

## Not promised

Leadership, specific accuracy, or adoption. Those are outcomes; this plan only lists work that can be built and measured.
Windows, macOS and ARM64 builds stay CI-tested until someone runs them on real hardware.
