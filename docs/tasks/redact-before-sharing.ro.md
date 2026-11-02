# Ascunde nume și numere înainte de partajare

```
legatorocr scan.png --ocr --lang ron --redact email,phone --redact-regex "RO[0-9]{2}[A-Z]{4}[0-9A-Z]{16}" --format none --pdf redactat.pdf
```

Cuvintele găsite sunt înnegrite în imagine și scoase din stratul de text. Verifică rezultatul cu ochii:
OCR poate rata un cuvânt, iar un cuvânt ratat nu este ascuns. `--redact` cere `--ocr --format none`.
