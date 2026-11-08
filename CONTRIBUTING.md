# Contributing

LegatorOCR is an independently implemented Rust tool (scan post-processing like ScanTailor, PDF export, OCR) with
one main author. Help is welcome, in this order of usefulness:

1. **Test on systems the author lacks** (Windows, macOS, arm64): see `docs/TESTER-REPORT.md`.
2. **Texts with a hand-checked reference** for OCR testing, if you hold the rights and can share them.
3. **Bug reports** with the smallest input that shows the problem.
4. **Code**: open an issue first for anything bigger than a small fix.

## Independent-implementation rule
Do not copy code, data structures or wording from ScanTailor (GPL) or from other tools whose licence
is incompatible with MIT, and do not paste text or images you have no right to publish. If a change is
inspired by another project, say which, and describe the idea in your own words.

## Before a pull request
```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check-docs.py        # options used in docs/tasks must exist
```
Do not run `cargo fmt --all` inside a functional change: the workspace is not fmt-clean, and a mass
reformat hides the real diff. Format only the files you touch, or send formatting as its own commit.

## Where things are
- `crates/` — engine, CLI, GUI, OCR, native OCR (`legatorocr-nocr`), and the rest of the workspace.
- `ROADMAP.md` — what was done, what was measured, what was tried and rejected, with numbers.
- `docs/audit/` — licence, security and competitor audits. `docs/ADVERSARIAL-ANALYSIS.md` — the
  author's own critical review.
- `docs/COMPATIBILITY.md` — how options and file formats are allowed to change.
- `scripts/compare-ocr-text.py` — compare several OCR engines against reference texts.

Security problems: see `SECURITY.md`, not a public issue.
