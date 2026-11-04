# Audit: CI workflows (.github/workflows/*.yml, .github/dependabot.yml)

Date read: 2026-11-04. Not run: CI cannot be executed from the audit environment, so nothing here was verified by a green run.
Result: **no workflow line was changed** - every pinned action is already the newest release, every runner label exists.

## Actions (all SHA-pinned; SHA -> tag resolved with `git ls-remote` on github.com)

| Action | Pinned (comment) | Newest tag now | Runtime | Verdict | Source |
|---|---|---|---|---|---|
| actions/checkout | 3d3c42e (v7) = v7.0.1 | v7.0.1 | node24 | current. v7 only adds `allow-unsafe-pr-checkout` (pull_request_target/workflow_run; not used here) | github.com/actions/checkout README + `git ls-remote --tags` |
| actions/upload-artifact | cf430e0 (v7) = v7.0.2 | v7.0.2 | node24 | current | github.com/actions/upload-artifact, action.yml |
| actions/download-artifact | 9000827 (v8) = v8.0.2 | v8.0.2 | node24 | current | github.com/actions/download-artifact, action.yml |
| Swatinem/rust-cache | d1527c8 (v2) = v2.9.3 | v2.9.3 | node24 | current (v2.9.0 moved it to node24) | github.com/Swatinem/rust-cache CHANGELOG.md, `git ls-remote` |
| dtolnay/rust-toolchain | b725eda (stable) | `stable` branch tip is b725eda | composite | current; the `1.95` branch used by the msrv job exists | `git ls-remote` |

Inputs used (`toolchain`, `components`, `targets`, `name`, `path`, `if-no-files-found`, `merge-multiple`, `fetch-tags`): none reported removed. Only READMEs/changelogs were read, not every release note, so "no breaking input" is by absence of a notice, not by an explicit statement.
Not used: softprops/*, setup-* actions (none exist in the workflows). dependabot.yml tracks github-actions + cargo weekly: fine.

## Runner labels (source: github.com/actions/runner-images README, 2026-11-04)

| Label in repo | Status now |
|---|---|
| ubuntu-latest | = ubuntu-24.04 (26.04 exists but is not `latest`) |
| windows-latest | = Windows Server 2025 (now the VS 2026 image) |
| macos-latest | = macOS 26 **arm64** |
| ubuntu-22.04, ubuntu-22.04-arm | still offered. Third-party endoflife page: end of active support 2026-09-17 (already passed), EOL 2027-04-17 (one source, medium confidence) |
| macos-13/14 | not used (macos-14 images were retired 2026-11-02; good that it is not used) |

## Tools downloaded in CI

| Item | Repo says | Source says now | Verdict |
|---|---|---|---|
| veraPDF installer `https://software.verapdf.org/releases/verapdf-installer.zip` | stable installer zip, headless install with auto.xml | software.verapdf.org and docs.verapdf.org/install list exactly this URL as "Release (stable)"; the install page documents the auto-install XML | URL documented. A HEAD from the shell was blocked (proxy 403), so the file's existence was not tested directly. software.verapdf.org carried a notice that it may be offline while a PHP vulnerability is assessed |
| Playwright `npm i playwright@1.49.1` | pinned 1.49.1 | npm latest is 1.64.1 | many minors behind; left alone on purpose (pinned test harness, `web/test-capture.mjs` not tested against newer). Candidate for a separate, run-verified bump |
| apt packages (tesseract-ocr + 8 language packs, djvulibre-bin, fonts-dejavu-core, python3-pil, poppler-utils, qpdf, ffmpeg) | Ubuntu 24.04 names | packages.ubuntu.com blocked | could not verify; these are long-standing Debian/Ubuntu names |
| cargo-audit (`cargo install --locked`) | latest | crates.io index: 0.22.2 | exists |
| winget/choco/brew | none used | - | n/a |

## Doubts (none acted on)

1. The macOS release job builds x86_64-apple-darwin on an arm64 `macos-latest` and then executes it in the smoke test: needs Rosetta on the runner (unverified).
2. windows-latest moved to the VS 2026 image; the release check that the .exe does not reference VCRUNTIME140 should be watched on the next run.
3. Release builds on ubuntu-22.04 are deliberate (glibc 2.35 floor, checked in the Portability step); moving to 24.04 would raise the floor, so it stays until 22.04 is actually removed.

## Could not verify
veraPDF installer file existence (HEAD blocked), apt package availability, docs.github.com runner pages (the runner-images README was read instead), every action's full release notes.

## Changed lines
None, so nothing to mark "unverified until a run is green".
