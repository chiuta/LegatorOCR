# Audit: external standards and formats (read 2026-11-04)

Method: every standard the repo cites or implements was looked up on the live source with WebFetch / WebSearch
(shell access to most hosts is blocked; `raw.githubusercontent.com` worked). "Repo" = what README / ROADMAP /
COMPARISON / code say. Anything not opened is in the last section; nothing there is marked verified.

## Checked

| Standard | Repo says | Source says now (2026-11-04) | Behind? | Source |
|---|---|---|---|---|
| ALTO | 4.4 (validator); `schemaLocation` in `xmlout.rs` names `alto-4-2.xsd`, namespace `ns-v4#` | 4.4 is the latest official (Apr 2023); no 4.5 / 5.0 file in the schema repo (HTTP 404) | No. Cosmetic: our output points to the 4.2 schema file, same namespace (TODO 3) | loc.gov/standards/alto/ ; altoxml/schema README (raw) ; altoxml documentation wiki "Versions" |
| PAGE XML (PRImA) | writes namespace 2019-07-15; validator rewrites it to 2024-07-15 | master `pagecontent.xsd` has `targetNamespace` ...`/pagecontent/2024-07-15` (read raw). The repo README of PAGE-XML still lists only 2019-07-15 | No (repo statement confirmed). We still emit the older namespace on purpose; consumers (OCR-D, Transkribus, eScriptorium) mostly read 2019 (not checked here) | raw PAGE-XML master `pagecontent/schema/pagecontent.xsd` |
| OCFL | 1.1 (`ocfl-1.1`, validated by ocfl-py) | Zenodo record "Oxford Common File Layout - Specification": version 1.1.1, 2024-11-22. Whether 1.1.1 changes the object `conformance` string was NOT established (ocfl.io blocked); ocfl-py still validates our object as "OCFL v1.1" | Probably not (1.1.1 looks like a patch/editorial release); unverified | zenodo.org/records/7157195/latest |
| MCP protocol | dual era 2024-11-05 + 2026-07-28 since 2026-11-04 (was 2024-11-05 only) | Latest revision is **2026-07-28** (stable, released 2026-07-28). Previous: 2025-11-25. Changes below | **Yes, two or three revisions behind** | modelcontextprotocol.io/specification/2026-07-28/changelog ; github release tag 2026-07-28 |
| TEI P5 | "TEI P5", namespace `http://www.tei-c.org/ns/1.0` | Guidelines release 4.12.0 dated 2026-07-28 (dev branch VERSION 4.13.0a). Namespace unchanged | No. Our output is not schema-checked (RELAX NG could not be fetched: generated file, tei-c.org blocked from shell) | tei-c.org/guidelines/P5/ ; raw TEIC/TEI dev `P5/VERSION` |
| PDF 2.0 / ISO 32000-2 | not implemented (writer stays inside PDF 1.4) | 2020 edition; Amendment 1 in publication | n/a | pdfa.org/iso-status/ |
| PDF/A-1, -2, -3 | -1b / -2b (default) / -3b declared | ISO 19005-1 (2005), -2 (2011), -3 (2012): no revision in progress | No | pdfa.org/iso-status/ |
| PDF/A-4 | not implemented (ROADMAP 6.1 backlog) | ISO 19005-4:2020 published; a revision (DIS) is in progress | Gap by design | pdfa.org/iso-status/ |
| PDF/UA | structure produced, "not validated" | PDF/UA-1 (ISO 14289-1:2014); **PDF/UA-2 (ISO 14289-2:2024) is published** | We only mention PDF/UA-1 era tagging; PDF/UA-2 is a different (PDF 2.0) profile, not planned | pdfa.org/iso-status/ |
| RFC 3161 | cited (tsa.rs) | Proposed Standard, **updated by RFC 5816** (ESSCertIDv2, needed for TSAs that sign with a SHA-2 certificate id); not obsoleted | Unknown whether `tsa.rs` accepts ESSCertIDv2 replies (TODO 4) | datatracker.ietf.org/doc/rfc3161/ |
| RFC 6962 | cited for the Merkle tree (`merkle.rs`) | **Obsoleted by RFC 9162** (CT v2.0, Dec 2021, Experimental). The leaf/node hashing we use is read from 6962; whether 9162 changes it was not compared here | Doc-only: say "RFC 6962 (obsoleted by RFC 9162)" (TODO 5) | rfc-editor.org/rfc/rfc9162.html |
| Tesseract | COMPARISON: 5.5.4; ADVERSARIAL-ANALYSIS: "5.5.3 or 5.5.4, check" | Arch Linux package page: 5.5.4-1, updated 2026-10-28 (a distro page, not the Tesseract release page) | No. ADVERSARIAL-ANALYSIS line updated | archlinux.org/packages/extra/x86_64/tesseract/ |
| GS1 General Specs (EAN-13) | Code 128, EAN-13, ITF, Code 39 decoded | GS1 General Specifications Release 26 published Jan 2026, "no disruptive new rules"; page does not mention EAN-13 | No evidence of change | gs1uk.org news item |
| e-Factura / CIUS-RO | CustomizationID must contain `cius-ro`; test sample uses `...CIUS-RO:1.0.1`; UBL 2.1 | OMF 4092/2022 (read at static.anaf.ro) sets the identifier `urn:cen.eu:en16931:2017#compliant#urn:efactura.mfinante.ro:CIUS-RO:1.0.1`. **No later order or version found**, but the official pages (mfinante.gov.ro, legislatie.just.ro, lege5) refuse robots, so "no newer version" is NOT established | Unknown | static.anaf.ro/static/10/Anaf/legislatie/OMF_4092_2022.pdf |

### MCP 2026-07-28 versus our server (details)

Our server (`crates/legatorocr-engine/src/mcp.rs`) answers `initialize` with `2024-11-05`, `ping`, `tools/list`,
`tools/call`; every other method gets -32601.

| Change in 2026-07-28 | Effect on us |
|---|---|
| `initialize` / `notifications/initialized` removed; protocol version + client capabilities travel in `_meta` of each request | A client that speaks only the new revision never sends `initialize` and cannot talk to us |
| `server/discover` MUST be implemented (versions, capabilities, identity); also the stdio probe | We return -32601; dual-era clients treat that as "legacy server" and fall back |
| Every result carries `resultType` ("complete" / "input_required"); clients treat a missing field as complete | None for legacy clients |
| `ping`, `logging/setLevel` removed; `tools/list` results get `ttlMs` + `cacheScope`; tool schemas are JSON Schema 2020-12 | Only matters once we answer in the new revision |
| Error `UnsupportedProtocolVersion` is -32022 (codes -32020..-32099 reserved for MCP) | Needed if we implement the new revision |

Verified locally: `scripts/mcp-client-test.py` with the official Python SDK `mcp` 2.3.0 against the existing release
binary still works (initialize, tools/list with 5 tools, a scan_to_pdf call). So today's clients that support old
revisions are fine; the risk is future new-only clients. Not tested: Claude Desktop, TypeScript SDK.

### Validator re-run

`scripts/validate-outputs.py <release binary> <synthetic 800x400 text image>` on 2026-11-04 against the live
schemas: `ok ALTO 4.4`, `ok PAGE page_0001.xml` (against the 2024-07-15 schema), `ok OCFL ... v1.1 Object ... VALID`
(ocfl-py), exit 0. The release binary dates from 09:50 the same day (not necessarily HEAD). Synthetic page, so this
shows schema validity of the structure, not OCR quality.

## Changes made in this audit

* README (MCP row) and ROADMAP 7.1: now state that only revision **2024-11-05** is
  implemented, name the newest revision, and describe the real-client test (the README previously said "not yet
  tried with a real MCP client" while ROADMAP said it had been tried: contradiction removed).
* `docs/ADVERSARIAL-ANALYSIS.md`: Tesseract 5.5.4 confirmed.
* No change to `scripts/validate-outputs.py`: its schema URLs and the 2024-07-15 rewrite are still correct.

## TODO (needs code; none done here, no rebuild allowed)

1. **MCP 2026-07-28 — DONE 2026-11-04** (see ROADMAP 7.1; original plan kept below): add a dual-era mode like the SDKs do: keep `initialize` for legacy clients and add
   `server/discover` (supported versions list, capabilities, serverInfo), accept `_meta`
   `io.modelcontextprotocol/protocolVersion` + `clientCapabilities` on each request, add `resultType:"complete"` to
   results, answer an unsupported version with -32022 (`UnsupportedProtocolVersion`, `data.supported` /
   `data.requested`), drop `ping` for new-era connections, set `ttlMs` / `cacheScope` on `tools/list`. Spec:
   changelog "Major changes" items 2, 3, 8 and "Minor changes" items 5, 12. Decide per connection from the first
   request (SDK behaviour). Also update the tests at `mcp.rs` ~l.450 and the README row. Gate: run
   `scripts/mcp-client-test.py` against both an old (1.x) and a new (2.x) SDK client.
2. **OCFL 1.1.1**: confirm in the spec text whether `conformsTo`/namaste must say `1.1` or `1.1.1`; ocfl-py accepted `1.1`.
3. **ALTO** `schemaLocation` in `xmlout.rs` ~l.117: `alto-4-2.xsd` to `alto-4-4.xsd` (cosmetic, one line, needs a rebuild and the unit tests that compare output).
4. **RFC 5816**: test `tsa.rs` against a reply whose signing certificate is identified by ESSCertIDv2 (SHA-256); the code was only tested with an OpenSSL throw-away TSA.
5. Doc wording: `merkle.rs` header and README/ROADMAP 5.4: "RFC 6962 (since obsoleted by RFC 9162 CT v2; the Merkle hashing is the same idea)"; compare 9162 section 2.1 before claiming identical hashes.
6. **PAGE namespace**: consider emitting 2024-07-15 only if a downstream tool is shown to read it; today's consumers were not checked.
7. **CycloneDX**: `scripts/sbom.py` writes specVersion 1.5; 1.6 and 1.7 schemas exist in the CycloneDX repository (raw files read, HTTP 200). 1.5 is still valid; bump only after validating output against the 1.7 JSON schema.
8. **e-Factura**: before trusting the CIUS-RO check, a person with browser access should open mfinante.gov.ro/web/efactura/informatii-tehnice and legislatie and note the current CIUS-RO version, the schematron release and any CustomizationID change; our check only looks for the substring `cius-ro`, so a new version string would still pass, but BR-RO rules are not validated at all.

## Could not verify

* Current CIUS-RO version, schematron release notes, ANAF validator version: mfinante.gov.ro, legislatie.just.ro and
  lege5.ro refused automated fetching (robots); other pages found by search did not state a version. Secondary,
  unconfirmed: one blog says the B2B/B2C submission deadline changed to 5 working days on 2026-01-01 (not used by us).
* Whether OCFL 1.1.1 differs from 1.1 in conformance strings (ocfl.io not allowed).
* RFC 8493 (BagIt) status: rfc-editor / datatracker pages for it were not allowed; from memory it is current, not verified.
* RFC 3161 / 6962 statuses were read, RFC 4648, RFC 1321, FIPS 180-4 / 197, RFC 6070 / 6229 (test vectors only): not looked up.
* ISO 15417 (Code 128), ISO/IEC 16388 (Code 39), ISO/IEC 16390 (ITF), ISO/IEC 15420 (EAN/UPC), ITU-T T.4 / T.6 / T.88 / T.800, Dublin Core, XMP, ICC.1: editions not looked up.
* Leptonica version (cited only as "Sauvola in Leptonica", no version): not checked.
* TEI output against the RELAX NG schema: schema file unobtainable here.
* Tesseract release page itself (distro page used instead); ecosyste.ms showed only 5.5.1, an out-of-date mirror.
* veraPDF current version and its PDF/UA-2 profile: not looked up.
