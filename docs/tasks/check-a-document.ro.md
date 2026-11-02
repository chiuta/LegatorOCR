# Verifică imaginea unui document

```
legatorocr --verify-document fotografie.jpg
legatorocr pasaport.png --ocr --lang eng --mrz --format none --txt pasaport.txt
```

`--verify-document` afișează codurile QR și zonele care se repetă în imagine. `--mrz` citește zona
cu citire automată și verifică cifrele de control. Ambele sunt indicii pentru o persoană; niciuna nu
dovedește că un document este autentic sau fals.
