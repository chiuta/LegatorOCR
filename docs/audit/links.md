# Link audit (read 2026-11-04)

Scope: every `http(s)://` string in README, ROADMAP, COMPARISON, SECURITY, FUZZING, THIRD_PARTY_LICENSES,
`docs/`, `web/`, `index.html`, `packaging/`, `.github/`, `scripts/`, `tools/`, `crates/` (54 distinct strings after
de-duplication). Licence texts were not touched.

Methods: **curl** = `curl -sIL -m 15` from the shell (most hosts are blocked there: code 000/403 from the proxy means
"blocked here", not "dead"); **WebFetch** was refused for every host (not on the session allowlist); **Chrome** = the
Chrome browser tools (page loaded and its text read); **gh** = GitHub REST API.

## Checked links

| URL | Result | Method | Action |
|---|---|---|---|
| github.com/chiuta/LegatorOCR (+ `.git`, `/releases`, `/blob/main/COMPARISON.md`) | OK; `.git` redirects to the repo | curl 200, gh | none |
| github.com/chiuta/legatorocr (lowercase) | redirects to canonical `chiuta/LegatorOCR` (gh API returns the canonical name) | gh | **fixed** in `Cargo.toml` `repository` (was the only lowercase use) |
| github.com/felixdittrich92/OnnxTR/releases/download/v0.0.1/crnn_mobilenet_v3_small-bded4d49.onnx | OK (302 to release-assets, then 200) | curl | none |
| github.com/felixdittrich92/OnnxTR/releases/download/v0.7.1/crnn_vgg16_bn-743599aa.onnx | OK (same) | curl | none |
| github.com/rspeer/wordfreq | OK; README now says data is a snapshot "through about 2021", unlikely to be updated (SUNSET.md); licence statement Apache code + CC BY-SA 4.0 data matches what the repo says | curl 403 (proxy), **Chrome** read | none |
| ocfl.io/1.1/spec/ and `#inventory` | OK, spec "7 October 2022, updated 7 November 2024"; section 3.5 Inventory exists and the `type` value is `https://ocfl.io/1.1/spec/#inventory`, exactly what `ocfl.rs` writes | curl 000, **Chrome** read | none |
| dumbravaalex.github.io/Portofoliu/ | OK, title "Portofoliu Digitalizare" | curl 000, **Chrome** | none |
| competentedigitalizare.blogspot.com/ | OK, title "HUB DIGITAL" (ROADMAP said it could not be fetched on 2 Nov) | curl 000, **Chrome** | ROADMAP note updated |
| legatorocr.chiuta.ro (and `/`) | OK, but the **deployed page is stale**: it says "acum 0.2.0-pre.5"; latest release is pre.15 | curl 000, **Chrome** | repo `docs/index.html` updated to pre.15 in all 8 languages; the live site updates only when Pages redeploys (not verified) |
| trom.tf | OK, "TROM.tf – trade-free online services" | curl 000, **Chrome** | none |
| creativecommons.org/licenses/by-sa/4.0/ and `/legalcode` | OK (deed and legal code load; no redirect seen) | curl 000, **Chrome** | none |
| www.loc.gov/standards/alto/v4/alto-4-2.xsd | OK, schema version 4.2 | curl 000, **Chrome** | none |
| software.verapdf.org/releases/verapdf-installer.zip | File is listed in the directory index (32 971 125 bytes, 07-Oct-2026; version dir `1.30/` is new). Zip itself not downloaded; CI step expects a `verapdf-greenfield-*` folder inside, **not verified** against 1.30 | **Chrome** (directory index) | none; see "could not verify" |

## Dead / 404 (not "dead links" but not-yet-published)

| URL | Result | Where | Note |
|---|---|---|---|
| github.com/chiuta/LegatorOCR/releases/download/v0.2.0/LegatorOCR-0.2.0-x86_64-pc-windows-msvc.zip | 404 (curl and gh: only `v0.2.0-pre.1` to `pre.15` exist; no `v0.2.0` tag or release) | `packaging/winget/*.yaml` | Intentional template: file header already says "not submitted... fill InstallerUrl and InstallerSha256 from the published release". No replacement exists; left as is. |
| github.com/chiuta/LegatorOCR/archive/refs/tags/v0.2.0.tar.gz | 404 (tag does not exist) | `packaging/homebrew/legatorocr.rb` | Same: header says "replace the sha256 when the tag is published". Left as is. |
| (flatpak `tag: v0.2.0`) | tag does not exist | `packaging/flatpak/*.yml` | Same placeholder status. |

## Not checked as links

* Test fixtures, not real links: `http://exemplu.ro/a`, `https://example.org/verifica/ABC-123`, `https://tsa.example.org/tsr`,
  `https://x.ro/a?b=c`, `http://localhost:${port}`, bare `https://`, `https://raw.githubusercontent.com/`.
* XML/RDF/XMP namespace identifiers (names, not meant to be fetched): w3.org, purl.org, ns.adobe.com, aiim.org,
  schemas.openxmlformats.org, apple.com DTD, primaresearch.org PAGE 2019-07-15 (also `.xsd`), loc.gov ALTO ns-v4 and xlink,
  openarchives.org, tei-c.org, color.org.

## Could not verify

* The veraPDF zip contents / folder name for 1.30 (the CI step in `.github/workflows/pdfa.yml` was not run).
* That the live GitHub Pages site will redeploy the pre.15 text (depends on the Pages workflow).
* (Resolved) `docs/FFF-ENVELOPE.md` and `docs/FLUXURI.md` in `github.com/chiuta/chiuta.github.io`, named in README without a URL:
  both exist on `main` (raw.githubusercontent.com, HTTP 200, via curl); contents not read.
