# Hide names and numbers before sharing

```
legatorocr scan.png --ocr --lang ron --redact email,phone --redact-regex "RO[0-9]{2}[A-Z]{4}[0-9A-Z]{16}" --format none --pdf redactat.pdf
```

The matching words are blacked out in the picture and removed from the text layer. Check the result by
eye: OCR can miss a word, and a missed word is not hidden. `--redact` needs `--ocr --format none`.
