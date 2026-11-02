# Fotografii de telefon → pagini curate

```
legatorocr photos/*.jpg --profile phone-photos --pdf pagini.pdf -o pages
```

- `--profile phone-photos` egalizează lumina, lărgește intervalul de înclinare și elimină degetele.
- Mai multe cadre ale aceleiași pagini se pot îmbina înainte: `legatorocr --fuse-frames pagina.png cadru1.jpg cadru2.jpg cadru3.jpg`
  (ajută la zgomot puternic de senzor; la zgomot moderat nu ajută).
