# Word lists

`<lang>.txt.gz`: the 40 000 most frequent alphabetic word forms (lowercase) of the eight interface
languages, extracted by `tools/ocr-train/make_lexicons.py` from the Python package **wordfreq**
(Robyn Speer et al.; code Apache-2.0). Its word-frequency data is distributed under
**Creative Commons Attribution-ShareAlike 4.0** (<https://creativecommons.org/licenses/by-sa/4.0/>),
and so are these files. They are compiled into the programs and read at run time only to repair
isolated recognition errors; they are separate data files, not part of the MIT-licensed source code.
Attribution: *wordfreq* by Robyn Speer, <https://github.com/rspeer/wordfreq>.

If CC BY-SA data is not acceptable for your distribution, build without them: the `lexicon` cargo
feature (on by default) controls it, e.g.

    cargo build --release --no-default-features -p legatorocr-cli -p legatorocr-gui

The programs then contain no word list at all; OCR works the same except that no dictionary-based
repair of single wrong letters is done (and the built-in orientation test falls back to Tesseract's).
