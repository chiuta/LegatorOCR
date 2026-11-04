# Compatibility policy (command line and files)

Status: **proposal written 2026-11-05 by the author's assistant; the author has not yet confirmed it.**
LegatorOCR is a pre-release (0.2.0-pre.N). Until 0.2.0 is declared stable, nothing below is a
promise; it describes how changes are handled so users are not surprised.

## What counts as the public surface
- Command-line options listed by `legatorocr --help` and their documented meaning.
- Exit codes (0 = success; non-zero = failure) and the JSON/JSONL formats written by
  `--passages-jsonl`, the audit log and the MCP server.
- The structure of the output PDF as far as documented (text layer, tags, PDF/A level asked for).

Not part of it: the exact wording of messages on stderr, the GUI layout, file names inside temporary
folders, the order of equal-score OCR alternatives, and anything marked "experimental" in the docs.

## Rules while in pre-release (0.2.0-pre.N)
- An option may be renamed or removed. Every such change is listed under "Changes" in the release
  notes for that pre-release.
- `scripts/check-docs.py` (run in CI) fails when a recipe in `docs/tasks/` uses an option that no
  longer exists, so documentation and options cannot drift silently.

## Rules from 0.2.0 on (intended)
- Within 0.2.x: options are added, not removed or changed in meaning. A renamed option keeps the old
  name working, with a warning, for at least two minor versions.
- JSON/JSONL formats: fields are added, never removed or retyped within a minor series.
- Anything dropped is announced in the release notes one minor version before it goes.
- 0.2.0 stable is to be published only after the checks in `docs/TESTER-REPORT.md` have been
  reported from Linux, Windows and macOS by people other than the author. That has **not** happened yet.
