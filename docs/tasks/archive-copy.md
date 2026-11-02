# Long-term archive copy with proof of integrity

```
legatorocr scans/*.png --profile registre-manuscrise --ocr --lang ron --pdf registru.pdf --pdfa --checksums --archive-package registru-arhiva -o pages
legatorocr --verify-sums registru-arhiva/SHA256SUMS.txt
```

- `--archive-package DIR` keeps the originals, the project (settings and adjustments), the OCR words
  with confidences and the outputs in one folder with hashes, and a `RUN.txt` saying how to run it again.
- `--verify-sums` checks that nothing changed since.
- Keep a second copy elsewhere: `legatorocr --backup-from registru-arhiva --backup-to /mnt/disc1 --backup-to /mnt/disc2`.
