#!/usr/bin/env python3
"""Compare OCR text from several engines against reference texts, page by page.

    python3 scripts/compare-ocr-text.py REF_DIR NAME=DIR[@PREFIX] [NAME=DIR[@PREFIX] ...] [--pages p4,p7]

REF_DIR holds `<page>.txt`; each NAME=DIR holds the same file names,
or `PREFIX<page>.txt` when `@PREFIX` is given (e.g. `ocrmypdf=out@o_`). Prints, per engine:
 - CER with diacritics (cedilla and comma forms of s/t are treated as the same letter),
 - CER with diacritics folded,
 - order-free recall of words of 3+ letters (reading order does not matter).
The reference is only as good as the text you give it: with machine-made references (e.g. ABBYY
output) the numbers measure agreement with that engine, not truth. Needs `pip install rapidfuzz`.
"""
import collections, os, re, sys, unicodedata
from rapidfuzz.distance import Levenshtein as L

def norm(s, fold=False):
    s = unicodedata.normalize("NFC", s).replace("\xad", "-")
    s = s.replace("ş", "ș").replace("Ş", "Ș").replace("ţ", "ț").replace("Ţ", "Ț")
    if fold:
        s = "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c))
    return re.sub(r"\s+", " ", s).strip()

def words(s):
    return [w for w in re.findall(r"\w+", norm(s).lower()) if len(w) >= 3]

def read(p):
    try:
        return open(p, encoding="utf8").read()
    except OSError:
        return ""

def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    pages = None
    if "--pages" in argv:
        pages = argv[argv.index("--pages") + 1].split(",")
        args = [a for a in args if a != argv[argv.index("--pages") + 1]]
    if len(args) < 2 or any("=" not in a for a in args[1:]):
        print(__doc__); return 2
    ref = args[0]
    engines = [a.split("=", 1) for a in args[1:]]
    if pages is None:
        pages = sorted(f[:-4] for f in os.listdir(ref) if f.endswith(".txt"))
    print(f"{len(pages)} pages: {','.join(pages)}\n")
    print("| engine | CER % | CER folded % | recall 3+ letters % |\n|---|---|---|---|")
    for name, d in engines:
        d, _, pre = d.partition("@")
        dist = dist_f = n = n_f = hit = tot = 0
        for p in pages:
            r, h = read(f"{ref}/{p}.txt"), read(f"{d}/{pre}{p}.txt")
            R, H = norm(r), norm(h)
            dist += L.distance(R, H); n += len(R)
            Rf, Hf = norm(r, True), norm(h, True)
            dist_f += L.distance(Rf, Hf); n_f += len(Rf)
            a, b = collections.Counter(words(r)), collections.Counter(words(h))
            hit += sum((a & b).values()); tot += sum(a.values())
        print(f"| {name} | {100*dist/max(1,n):.2f} | {100*dist_f/max(1,n_f):.2f} | {100*hit/max(1,tot):.1f} |")
    return 0

if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
