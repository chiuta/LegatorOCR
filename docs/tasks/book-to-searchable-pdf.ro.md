# Carte scanată → PDF căutabil

```
legatorocr scans/*.png --profile book-bw --ocr --lang ron --pdf carte.pdf --pdfa --txt carte.txt -o pages
```

- `--profile book-bw` alege setările pentru cărți cu text (lista: `--list-profiles`).
- `--ocr --lang ron` adaugă un strat invizibil de text în română (cere Tesseract); `--txt` salvează și textul.
- `--pdfa` scrie PDF/A-2b pentru arhivare (`--pdfa-part 1` sau `3` pentru celelalte părți).
- Pentru o bază de accesibilitate adaugă `--tagged ro` (structură de paragrafe din liniile OCR).
- Pentru o copie DjVu adaugă `--djvu carte.djvu`.
