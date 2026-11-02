# LegatorOCR by task

Short recipes, one per job. Every flag in them is checked against `legatorocr --help` by
`scripts/check-docs.py` (run in CI), so a renamed or removed option breaks the build instead of the
documentation silently going stale. The recipes show what the options do; they have not all been run
on real material (see the roadmap for what was measured).

Translations: put a translated recipe next to the English one as `NAME.LANG.md`
(for example `book-to-searchable-pdf.ro.md`). `python3 scripts/check-docs.py --coverage` prints, per
language, how many of the English recipes have a translation.

| Task | Recipe |
|---|---|
| Scanned book → searchable PDF | [book-to-searchable-pdf.md](book-to-searchable-pdf.md) |
| Phone photos → clean pages | [phone-photos.md](phone-photos.md) |
| Long-term archive copy with proof of integrity | [archive-copy.md](archive-copy.md) |
| Sign a PDF and time-stamp a file | [sign-and-timestamp.md](sign-and-timestamp.md) |
| Hide names and numbers before sharing | [redact-before-sharing.md](redact-before-sharing.md) |
| Big batch on several machines | [big-batch.md](big-batch.md) |
| Check a document picture | [check-a-document.md](check-a-document.md) |
