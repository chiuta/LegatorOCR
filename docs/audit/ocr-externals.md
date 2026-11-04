# Audit: external OCR and language inputs (read 2026-11-04)

Method: live pages through the browser tools (github.com, formulae.brew.sh API, packages.debian.org, packages.ubuntu.com, huggingface.co, ro/en.wikipedia.org, paddleocr.ai) and `curl` for raw.githubusercontent.com and pypi.org. Nothing was downloaded except HTTP headers and READMEs (deleted).

## 1. Tesseract and Romanian models

| Item | Repo said | Source now | URL |
|---|---|---|---|
| Required Tesseract version | none stated; `--doctor` prints the first line of `tesseract --version` and checks only that the binary and the 8 language models exist | unchanged (no minimum is enforced; Debian packages `tesseract-ocr` 5.5.0 pull `tesseract-ocr-osd`, so OSD data comes with the standard package, as README says) | packages.debian.org/trixie/tesseract-ocr |
| Latest Tesseract | ADVERSARIAL: 5.5.4 (conflict with 5.5.3 resolved) | **5.5.4 "Latest"** on the release page (5.5.3, 5.5.2, 5.5.1, 5.5.0 before it); Homebrew formula also 5.5.4 | github.com/tesseract-ocr/tesseract/releases ; formulae.brew.sh/api/formula/tesseract.json |
| `ron` models | COMPARISON: best 9.6 MB, fast 2.4 MB, stamped 2017 | confirmed: `tessdata_best/ron.traineddata` 9,595,755 B, `tessdata_fast` 2,376,323 B; each has ONE commit, 15 Sep 2017 ("Initial import"). The `tessdata` repo: 11,008,634 B, last changed 22 Mar 2018 ("integerized tessdata_best"), earlier 2016 | github.com/tesseract-ocr/{tessdata_best,tessdata_fast,tessdata}/commits/main/ron.traineddata ; raw.githubusercontent.com content-length |
| Newer Romanian model? | not claimed | none found in the three official repos (no commit after 2017/2018); latest tag of tessdata_best/fast is 4.1.0. Community models were not searched | github.com/tesseract-ocr/tessdata_best/releases |
| langdata_lstm #37 (comma-below vs cedilla) | "open when fetched" | **still open**; last visible activity: maintainer questions (can the author send a PR for `ron.training_text`/`singles_text`/`wordlist`; was cedilla used historically). No fix, no linked PR, no assignee. The page shows no dates | github.com/tesseract-ocr/langdata_lstm/issues/37 |
| apt | `sudo apt install tesseract-ocr tesseract-ocr-ron` | both exist. Debian trixie: `tesseract-ocr` 5.5.0-1; `tesseract-ocr-ron` 1:4.1.0-2 (bookworm, trixie, forky, sid). Ubuntu noble: `tesseract-ocr-ron` 1:4.1.0-2 (universe), 990 kB download, 2.3 MB installed = the **fast** model (inferred from size) | packages.debian.org (search + trixie page); packages.ubuntu.com/noble/tesseract-ocr-ron |
| brew | `brew install tesseract tesseract-lang` | both exist. `tesseract` 5.5.4 contains only `eng` and `osd` (its own caveat); `tesseract-lang` 4.1.0 is built from **tessdata_fast** | formulae.brew.sh/api/formula/tesseract(-lang).json |
| winget | `winget install UB-Mannheim.TesseractOCR` | id exists; newest manifest folder is **5.4.0.20240606** (older 5.3.x, v5.x). So winget lags Tesseract 5.5.x by more than two years. Whether the silent winget install offers Romanian: NOT checked (installer YAML not read) | github.com/microsoft/winget-pkgs/tree/master/manifests/u/UB-Mannheim/TesseractOCR |

Changed in repo: README install paragraph now says what each package really delivers (fast model from apt/brew, best only by manual download, winget lag) and marks the claims as measured nowhere for accuracy; COMPARISON notes the issue is still open on 2026-11-04.

## 2. Romanian orthography

| Claim | Source | Verdict |
|---|---|---|
| `â` replaced by `î` everywhere in 1953 | Academy decision / Council of Ministers HCM 3135/1953; decision text dated 16 Sept 1953 (cited by ro.wikipedia "Alfabetul limbii române", note 11); en.wikipedia "Romanian alphabet" (1953 reform, "Romînia"); evz.ro: in force 1954, schools 1955-56 | correct |
| `român` family got `â` back in 1964 | ro.wikipedia ("În 1964 ... reincluderea literei â ... exclusiv ... rădăcina român"); en.wikipedia (1964 minor reform); evz.ro (other words kept `î`: cîntec, pămînt, hotărîre) | repo rule `romîn`->`român` is right but only for 1953-1964 books; README/ROADMAP/code comment now say so |
| 1993 return of `â` inside words | Academy General Assembly, **17 Feb 1993**, published in Monitorul Oficial 8 Mar 1993 (newsweek.ro); also restored `sunt, suntem, sunteți` (evz.ro, newsweek.ro); basis: pre-1948 norms (ro.wikipedia notes 12-14) | period "1953-1993" is correct; dates added |
| `sînt` only in 1953-1993 | the 1993 decision "restored" `sunt` (evz.ro, newsweek.ro) | consistent with the code (`sînt/sîntem/sînteți` -> `sunt/...`). That the 1953 decision itself *introduced* `sînt` was not read in a primary text: NOT verified |
| Some works kept `î`/`sînt` after 1993 | en.wikipedia "Romanian alphabet" | true; the search option is not limited to pre-1993 print |
| Cedilla vs comma-below | Unicode 3.0 added Ș/Ț; Unicode says comma-below is preferred for Romanian (en.wikipedia citing the standard); Academy mandates comma-below; U+0218/0219/021A/021B (ro.wikipedia); Romanian SR 14111:1998 defines the set later registered as ISO/IEC 8859-16 with S/T-comma (en.wikipedia ISO/IEC 8859-16) | repo statements (`ş ţ` U+015F/U+0163 are cedilla, `ș ț` the correct letters) are right. The repo never cites SR 14111; nothing to fix |

Not read from a primary source: the text of the 1953 and 1993 decisions themselves (secondary sources only: Wikipedia, evz.ro, newsweek.ro). Pre-1953 `sunt` usage is inferred from the 1993 "restoration" wording.

## 3. wordfreq

| Item | Repo said | Source | Verdict |
|---|---|---|---|
| Version | no version | PyPI: **3.1.1**, uploaded 2023-11-21 (last release), licence field Apache-2.0 | add to notes |
| Maintenance | not mentioned | README top and SUNSET.md: data is a snapshot "through about 2021"; author states it will not be updated again (note dated Sept 2024) | new fact, recorded here |
| Romanian source | "derive from wordfreq data", CC BY-SA 4.0 | README table: `ro` has 3 sources: Wikipedia, Subtitles (OpenSubtitles 2018), Web (OSCAR); data files redistributable under CC BY-SA 4.0, code Apache-2.0 | consistent with `crates/legatorocr-nocr/lexicons/README.md` |

## 4. PP-OCRv6, docTR, OnnxTR

| Item | Repo said | Source | Verdict |
|---|---|---|---|
| PP-OCRv6 exists | README / ROADMAP 4.30: "PP-OCRv6 ... from PaddleOCR's model list" | PaddleOCR v3.7.0, 2026-06-11: PP-OCRv6 tiny 1.5M / small 7.7M / medium 34.5M parameters, 50 languages (zh, en, ja + 46 Latin-script), dictionary extended by ~200 diacritics | exists. **Romanian not named** on the release page, the docs index or the algorithm page; licence and training data NOT stated on those pages; ONNX files are not stated to be distributed (there is a guide "Obtaining ONNX Models", not read). README wording ("you download the models yourself ... check diacritics yourself") stays correct |
| OnnxTR releases | ROADMAP 4.31: "OnnxTR v0.0.1 release holds docTR detectors and recognisers as ONNX" | `v0.0.1` is a **pre-release**, "Initial release to upload models", 25 assets (db_*, linknet_*, crnn_*, parseq, master, sar, vitstr, rep_fast_*, orientation, yolo_artefact). Newest release v0.9.0 (port of docTR 1.1.0, needs Python >= 3.11) | correct |
| Licence | not stated in the ROADMAP rows | OnnxTR repo: Apache-2.0 (LICENSE file + README); docTR repo: Apache-2.0; PaddleOCR repo LICENSE: Apache-2.0. HF card `Felix92/onnxtr-parseq-multilingual-v1`: `license: apache-2.0`. These are the code/repo and model-card licences; a licence for the training data is NOT stated anywhere I read | recorded; no repo change needed |
| Training data statement | none | none found in OnnxTR README, the HF card, or the docTR README. The 2022-era docTR weights that ROADMAP 4.31 ran use docTR's `french` vocab (checked in `doctr/datasets/vocabs.py` at `main`) | ROADMAP wording ("inferred, not checked in the model's vocabulary file") now partly checked: vocab `french` has no ă ș ț |
| New lead | not in repo | docTR `main` has `VOCABS["romanian"]` (`ăâîșțĂÂÎȘȚ` added to the English set) and `VOCABS["multilingual"]`; the HF model `Felix92/onnxtr-parseq-multilingual-v1` (updated 2026-06-17) lists languages en da fr it es de pt cs pl nl no fi, **no Romanian**. Not downloaded, not run | follow-up candidate, unmeasured |

## Could not verify

- Whether `winget install UB-Mannheim.TesseractOCR` installs Romanian data (installer manifest not read).
- Dates of the activity in langdata_lstm #37 (page text has no timestamps).
- Primary text of the 1953 (HCM 3135/1953) and 1993 decisions; the claim that `sînt` was introduced in 1953.
- Whether Debian/Ubuntu/brew `ron` is byte-identical to `tessdata_fast` (judged only by size and by the `tesseract-lang` formula source).
- PP-OCRv6 Romanian coverage, licence text of the weights, ONNX download location; any community Romanian Tesseract model.
- packages.ubuntu.com/debian.org and formulae.brew.sh are blocked from the shell; they were read through the browser.
