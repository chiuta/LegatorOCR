# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private
vulnerability reporting (Security tab > "Report a vulnerability") on
<https://github.com/chiuta/LegatorOCR>, or write to alexio@trom.tf. Include the program version
(`legatorocr --version`), the command line, and, if you can share it, a small file that triggers
the problem. Expect a first answer within 7 days.

## Scope

LegatorOCR reads untrusted files (images, PDF, archives, JBIG2, JPEG 2000). Crashes, hangs,
unbounded memory use, path traversal when unpacking archives, and anything that makes the program
write outside the chosen output folder are in scope. The decoders are written in safe Rust; the
fuzzing targets are described in `FUZZING.md`. External programs (Tesseract, pdftoppm, mutool,
7z) are run only when the user asks for them and are not covered here.

## Supported versions

Only the latest release receives fixes while the project is in pre-release (0.x).

## What a release contains

* Binaries are built by GitHub Actions from the tagged commit only (no build cache), after the
  whole CI matrix passes. The release is a draft until a person publishes it.
* `SHA256SUMS.txt` lists the SHA-256 of every release file; check it with `sha256sum -c`.
* `LegatorOCR-sbom.cdx.json` is a CycloneDX 1.5 list of every Rust crate in `Cargo.lock` with its
  version and checksum, produced by `scripts/sbom.py` (offline, deterministic for a given
  `SOURCE_DATE_EPOCH`).
* `cargo audit` runs weekly in CI (`audit.yml`).

Not provided yet: code signing of the binaries, a signed provenance attestation, and bit-for-bit
reproducible builds across machines. These are listed in ROADMAP item 5.1 as not done.
