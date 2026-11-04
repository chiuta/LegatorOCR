# Tester report (Windows, macOS, arm64, anything the author has not tried)

LegatorOCR has been run by its author's assistant on one Linux machine. Reports from other systems
are the most useful thing you can send. Five minutes is enough.

1. Download the latest release for your system, or build from source (`cargo build --release`).
2. Run `legatorocr --doctor` and copy its whole output.
3. Run it on one scan (or phone photo) you can share: `legatorocr INPUT.jpg --ocr --pdf out.pdf -o pages`.
4. Open `out.pdf`: is the page straight, cropped sensibly, is the text selectable and searchable?

Send (as an issue on https://github.com/chiuta/LegatorOCR/issues, or by email):

```
System and version:          (e.g. Windows 11 23H2 / macOS 15 arm64 / Debian 12 arm64)
LegatorOCR version:
Output of --doctor:
Command you ran:
What you expected:
What happened (error text, or a screenshot):
Can the input be shared?     yes / no
```

Do not send documents with personal data. A "nothing went wrong" report is also welcome: it is the only
way the author learns that a system works.
