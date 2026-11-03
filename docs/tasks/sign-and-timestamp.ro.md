# Semnează un PDF și pune marcă de timp pe un fișier

```
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-name "Nume" --sign-reason "Aprobat"
# cu amprentă de timp în interiorul semnăturii (PAdES-B-T):
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-timestamp --tsa-url https://tsa.example.org/tsr
legatorocr --verify-pdf-signatures registru.signed.pdf --sign-ca ca.pem
legatorocr --timestamp SHA256SUMS.txt --tsa-url https://tsa.example.org/tsr
legatorocr --timestamp-verify SHA256SUMS.txt --tsa-ca tsa.pem
```

Nivelul juridic al semnăturii este cel al certificatului tău. Semnătura este invizibilă. Cu `--sign-timestamp` în ea se pune o marcă de timp RFC 3161 (autoritatea vede doar un hash); fără, semnătura nu are dovadă de timp. `--timestamp` ștampilează un fișier separat, de exemplu lista de sume de control.
