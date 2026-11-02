# Sign a PDF and time-stamp a file

```
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-name "Nume" --sign-reason "Aprobat"
legatorocr --verify-pdf-signatures registru.signed.pdf --sign-ca ca.pem
legatorocr --timestamp SHA256SUMS.txt --tsa-url https://tsa.example.org/tsr
legatorocr --timestamp-verify SHA256SUMS.txt --tsa-ca tsa.pem
```

The legal level of a signature is that of your certificate. The signature is invisible and carries no
time stamp inside it; the time stamp above is for a separate file such as the checksums list.
