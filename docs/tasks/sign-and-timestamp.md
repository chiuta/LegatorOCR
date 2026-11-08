# Sign a PDF and time-stamp a file

```
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-name "Nume" --sign-reason "Aprobat"
# with a time stamp inside the signature (PAdES-B-T):
LEGATOROCR_P12_PASS=... legatorocr --sign-pdf registru.pdf --sign-p12 certificat.p12 --sign-timestamp --tsa-url https://tsa.example.org/tsr
legatorocr --verify-pdf-signatures registru.signed.pdf --sign-ca ca.pem
legatorocr --timestamp SHA256SUMS.txt --tsa-url https://tsa.example.org/tsr
legatorocr --timestamp-verify SHA256SUMS.txt --tsa-ca tsa.pem
```

The legal level of a signature is that of your certificate. The signature is invisible unless you add `--sign-visible PAGE,X,Y,WIDTH,HEIGHT` (millimetres from the top-left corner), which draws a box with the signer, reason and date. With `--sign-timestamp` an RFC 3161 time stamp is placed inside it (the authority sees only a hash); without it the signature has no time proof. `--timestamp` stamps a separate file such as the checksums list.
