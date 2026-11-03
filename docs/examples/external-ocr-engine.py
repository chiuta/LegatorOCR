#!/usr/bin/env python3
"""Template of an external OCR engine for LegatorOCR: any program that speaks Tesseract's command line.

LegatorOCR calls, for every page:   PROGRAM stdin stdout -l LANG --psm N [--dpi D] tsv
with a PNG on standard input, and expects Tesseract's TSV on standard output
(header line, then level page block par line word left top width height conf text).
It also calls `PROGRAM --version` and `PROGRAM --list-langs` once.

Use it:  legatorocr scan.png --pdf out.pdf --ocr --engine tesseract --tesseract ./external-ocr-engine.py

This template returns one fixed word so that the hook can be tested. Replace `recognise()` with a call to
any model (a vision-language model, PaddleOCR, ...). Whatever it returns ends up in the PDF text layer
as if Tesseract had read it: keep a person in the loop for text that a generative model produced, because
such models can invent words that are not on the page.
"""
import sys

def recognise(png: bytes, lang: str, psm: str):
    """Return a list of (left, top, width, height, confidence 0-100, text)."""
    return [(10, 10, 100, 30, 90, "EXTERN")]

def main(argv):
    if "--version" in argv:
        print("external-ocr-engine 1.0"); return 0
    if "--list-langs" in argv:
        print("List of available languages (1):\nron"); return 0
    lang = argv[argv.index("-l") + 1] if "-l" in argv else "eng"
    psm = argv[argv.index("--psm") + 1] if "--psm" in argv else "3"
    words = recognise(sys.stdin.buffer.read(), lang, psm)
    print("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext")
    for i, (x, y, w, h, c, t) in enumerate(words, 1):
        print(f"5\t1\t1\t1\t1\t{i}\t{x}\t{y}\t{w}\t{h}\t{c}\t{t}")
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
