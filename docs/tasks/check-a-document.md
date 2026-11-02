# Check a document picture

```
legatorocr --verify-document fotografie.jpg
legatorocr pasaport.png --ocr --lang eng --mrz --format none --txt pasaport.txt
```

`--verify-document` prints QR codes and regions that repeat inside the picture. `--mrz` reads the
machine-readable zone and checks its check digits. Both are hints for a person to look at; neither
proves a document is genuine or forged.
