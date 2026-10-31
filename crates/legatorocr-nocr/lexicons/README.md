# Word lists

`<lang>.txt.gz`: the 40 000 most frequent alphabetic word forms (lowercase) of the eight interface
languages, extracted by `tools/ocr-train/make_lexicons.py` from the Python package **wordfreq**
(Robyn Speer et al.; code Apache-2.0). Its word-frequency data is distributed under
**Creative Commons Attribution-ShareAlike 4.0** (<https://creativecommons.org/licenses/by-sa/4.0/>),
and so are these files. They are separate data files, read at run time to repair isolated
recognition errors; they are not part of the MIT-licensed source code. If CC BY-SA data is not
acceptable for your distribution, delete the files and the `lexicon` step is skipped (build with
`--no-default-features`).
