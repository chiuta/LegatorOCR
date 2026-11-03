"""Label real scanned lines with the ABBYY text layer of OCR_rev_06.pdf.

Input: the lines dumped by `cargo run --release -p legatorocr-nocr --example dump_lines` (PNG + JSONL
per page, the page rendered at 300 dpi). ABBYY fragments (`pdftotext -bbox-layout`) whose centre
lies in a line box give the characters; ABBYY's text layer is unreliable about word spaces, so
spaces are taken from what the current model reads, mapped onto the ABBYY characters by edit
alignment. Lines where model and ABBYY disagree by more than MAXDIFF (wrong box, headline, noise)
are dropped.

Usage: real_labels.py PDF LINES_DIR OUT.tsv   (OUT.tsv: png path <TAB> label)
"""
import json, glob, os, re, subprocess, sys, unicodedata
import xml.etree.ElementTree as ET
from rapidfuzz.distance import Levenshtein as L
from charset import CHARSET

DPI = 300 / 72
MAXDIFF = 0.20
MAP = {"ş": "ș", "ţ": "ț", "Ş": "Ș", "Ţ": "Ț", "­": "-", " ": " "}


def fragments(pdf, n):
    out = subprocess.run(["pdftotext", "-bbox", "-f", str(n), "-l", str(n), pdf, "-"],
                         capture_output=True, text=True, check=True).stdout
    out = re.sub(r"<!DOCTYPE[^>]*>", "", out).replace(' xmlns="http://www.w3.org/1999/xhtml"', "")
    root = ET.fromstring(out)
    fr = []
    for w in root.iter("word"):
        t = "".join(MAP.get(c, c) for c in (w.text or ""))
        fr.append(((float(w.get("xMin")) + float(w.get("xMax"))) / 2 * DPI,
                   (float(w.get("yMin")) + float(w.get("yMax"))) / 2 * DPI, t))
    return fr


def label(model_txt, abbyy):
    """ABBYY characters with the model's word spaces."""
    m = model_txt.strip()
    mc = [i for i, c in enumerate(m) if c != " "]
    mns = "".join(m[i] for i in mc)
    space_after = set()  # indices into mns after which the model put a space
    k = -1
    for c in m:
        if c == " ":
            if k >= 0:
                space_after.add(k)
        else:
            k += 1
    m2a = {}
    for op in L.opcodes(mns, abbyy):
        if op.tag in ("equal", "replace"):
            for d in range(op.src_end - op.src_start):
                if op.dest_start + d < op.dest_end:
                    m2a[op.src_start + d] = op.dest_start + d
    cut = set()
    for k in space_after:
        j = k
        while j >= 0 and j not in m2a:
            j -= 1
        if j >= 0:
            cut.add(m2a[j])
    return "".join(c + (" " if i in cut else "") for i, c in enumerate(abbyy)).strip()


if __name__ == "__main__":
    pdf, ldir, out = sys.argv[1:4]
    cs = set(CHARSET)
    kept = tot = 0
    with open(out, "w", encoding="utf8") as f:
        for jl in sorted(glob.glob(f"{ldir}/*.jsonl")):
            stem = os.path.basename(jl)[:-6]
            n = int(re.match(r"p(\d+)-", stem).group(1))
            fr = fragments(pdf, n)
            for row in map(json.loads, open(jl, encoding="utf8")):
                tot += 1
                x0, x1, y0, y1 = row["x"], row["x"] + row["w"], row["y"], row["y"] + row["h"]
                inside = sorted((fx, t) for fx, fy, t in fr if x0 <= fx <= x1 and y0 <= fy <= y1)
                ab = "".join(t for _, t in inside)
                mo = row["model"]
                mns = mo.replace(" ", "")
                if len(ab) < 4 or not mns or any(c not in cs for c in ab):
                    continue
                if L.distance(ab, mns) / max(len(ab), 1) > MAXDIFF:
                    continue
                f.write(f"{ldir}/{stem}_{row['i']}.png\t{label(mo, ab)}\n")
                kept += 1
    print(f"kept {kept} of {tot} lines")
