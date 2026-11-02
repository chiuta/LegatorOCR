# Copie de arhivă cu dovada integrității

```
legatorocr scans/*.png --profile registre-manuscrise --ocr --lang ron --pdf registru.pdf --pdfa --checksums --archive-package registru-arhiva -o pages
legatorocr --verify-sums registru-arhiva/SHA256SUMS.txt
```

- `--archive-package DIR` păstrează originalele, proiectul (setări și ajustări), cuvintele OCR cu încrederi
  și rezultatele într-un singur dosar cu sume de control, plus `RUN.txt` care spune cum se reia.
- `--verify-sums` verifică faptul că nimic nu s-a schimbat de la împachetare.
- Păstrează o a doua copie în altă parte: `legatorocr --backup-from registru-arhiva --backup-to /mnt/disc1 --backup-to /mnt/disc2`.
