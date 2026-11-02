# Semnează un PDF și pune marcă de timp pe un fișier

```
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-name "Nume" --sign-reason "Aprobat"
legatorocr --verify-pdf-signatures registru.signed.pdf --sign-ca ca.pem
legatorocr --timestamp SHA256SUMS.txt --tsa-url https://tsa.example.org/tsr
legatorocr --timestamp-verify SHA256SUMS.txt --tsa-ca tsa.pem
```

Nivelul juridic al semnăturii este cel al certificatului tău. Semnătura este invizibilă și nu conține
marcă de timp; marca de timp de mai sus este pentru un fișier separat, de exemplu lista de sume de control.
