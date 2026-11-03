"""Label real scanned lines by agreement of two engines (no text layer needed).

For every page image: `dump_lines` (the native engine's lines, straightened, with its reading) and
the Tesseract words from the CLI. A line is kept when the Tesseract words lying on it spell nearly the
same letters as the native reading; the label is then Tesseract's text with its word spaces. Only
lines both engines read alike are used, so this adapts the model to a typeface and a scanner, it does
not teach it anything the two engines disagree on.

Usage: agree_labels.py OUT.tsv DPI IMAGE...    (work files go to /tmp/agree/<stem>/)
"""
import json, os, subprocess, sys, re
from concurrent.futures import ThreadPoolExecutor
from rapidfuzz.distance import Levenshtein as L
from charset import CHARSET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "../..")
DUMP = os.path.join(ROOT, "target/release/examples/dump_lines")
CLI = os.path.join(ROOT, "target/release/legatorocr")
MODEL = os.path.join(ROOT, "crates/legatorocr-nocr/models/multi.lnm")
MAXDIFF = 0.10
MAP = {"ş": "ș", "ţ": "ț", "Ş": "Ș", "Ţ": "Ț", "­": "-", " ": " "}


def work(img, dpi):
    stem = os.path.splitext(os.path.basename(img))[0]
    d = f"/tmp/agree/{stem}"
    os.makedirs(d, exist_ok=True)
    if not os.path.exists(f"{d}/{stem}.jsonl"):
        subprocess.run([DUMP, MODEL, img, d, str(dpi)], capture_output=True)
    if not os.path.exists(f"{d}/tess.json"):
        subprocess.run([CLI, img, "--pdf", f"{d}/x.pdf", "--ocr", "--lang", "ron", "--engine", "tesseract",
                        "--json", f"{d}/tess.json"], capture_output=True)
    rows = []
    try:
        words = json.load(open(f"{d}/tess.json"))["pages"][0]["words"]
    except Exception:
        return rows
    cs = set(CHARSET)
    for ln in map(json.loads, open(f"{d}/{stem}.jsonl", encoding="utf8")):
        xc = ln["x"] + ln["w"] / 2
        on = []
        for w in words:
            cx, cy = w["x"] + w["w"] / 2, w["y"] + w["h"] / 2
            if not (ln["x"] - 5 <= cx <= ln["x"] + ln["w"] + 5):
                continue
            u = cx - xc
            want = ln["y"] + ln["h"] / 2 + ln["slope"] * u + ln["curve"] * u * u
            if abs(cy - want) < 0.6 * ln["h"]:
                on.append((cx, "".join(MAP.get(c, c) for c in w["text"])))
        on.sort()
        txt = " ".join(t for _, t in on).strip()
        mo = ln["model"].replace(" ", "")
        tn = txt.replace(" ", "")
        if len(tn) < 8 or not mo or any(c not in cs for c in txt):
            continue
        if L.distance(tn, mo) / len(tn) > MAXDIFF:
            continue
        rows.append(f"{d}/{stem}_{ln['i']}.png\t{txt}")
    return rows


if __name__ == "__main__":
    out, dpi, imgs = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
    with ThreadPoolExecutor(2) as ex:
        res = list(ex.map(lambda i: work(i, dpi), imgs))
    with open(out, "w", encoding="utf8") as f:
        for r in res:
            for row in r:
                f.write(row + "\n")
    print(sum(len(r) for r in res), "lines from", len(imgs), "pages")
