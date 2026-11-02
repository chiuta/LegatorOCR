# Scanned book → searchable PDF

```
legatorocr scans/*.png --profile book-bw --ocr --lang ron --pdf carte.pdf --pdfa --txt carte.txt -o pages
```

- `--profile book-bw` picks the settings for text books (list them with `--list-profiles`).
- `--ocr --lang ron` adds an invisible text layer in Romanian (needs Tesseract); `--txt` also saves the text.
- `--pdfa` writes PDF/A-2b for archiving (`--pdfa-part 1` or `3` for the other parts).
- For an accessibility base add `--tagged ro` (paragraph structure from the OCR lines).
- For a DjVu copy add `--djvu carte.djvu`.
