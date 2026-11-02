# Hand-over to an electronic-archiving provider (roadmap 5.10)

```
legatorocr scans/*.png --ocr --lang ron --pdf dosar.pdf --pdfa --checksums --catalog catalog.csv --package predare-dosar -o pages
legatorocr --verify-sums predare-dosar/SHA256SUMS.txt
```

This builds a PDF/A-2b file with a text layer, a `SHA256SUMS.txt`, a catalogue of the files with
their hashes (`--catalog`), and a folder with a readable `MANIFEST.txt` (name it `.zip` for a single archive). **What a provider
accepts (accepted PDF/A part, metadata schema, signature or time-stamp requirements, folder
layout) must be asked of that provider first**; no provider's specification was available to check
against, so this recipe only gives the neutral, self-describing package that most can start from.
Add `--rights-notice` / `--timestamp` if the provider asks for them.
