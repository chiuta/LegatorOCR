"""Reference lines from the ABBYY text layer of OCR_rev_06.pdf (pdftotext -bbox-layout).

pdftotext hands ABBYY's text over as word fragments ("p", "e", "n", "tru"); joining fragments with
spaces (what plain `pdftotext` does) puts spurious blanks inside words. Here fragments of one line
are merged when the gap between them is small against the line height.

Usage: abbyy.py PDF OUT_DIR FIRST LAST     writes OUT_DIR/pN.txt (reading order) and pN.lines.json
       (lines: text and box in 300 dpi pixels).
"""
import json, os, re, subprocess, sys
import xml.etree.ElementTree as ET

GAP = 0.28  # a gap wider than this fraction of the line height is a word space
DPI = 300 / 72


def page_lines(pdf, n):
    out = subprocess.run(["pdftotext", "-bbox-layout", "-f", str(n), "-l", str(n), pdf, "-"],
                         capture_output=True, text=True, check=True).stdout
    out = re.sub(r"<!DOCTYPE[^>]*>", "", out)
    out = out.replace(' xmlns="http://www.w3.org/1999/xhtml"', "")
    root = ET.fromstring(out)
    lines = []
    for ln in root.iter("line"):
        ws = [(float(w.get("xMin")), float(w.get("xMax")), w.text or "") for w in ln.iter("word")]
        y0, y1 = float(ln.get("yMin")), float(ln.get("yMax"))
        h = max(y1 - y0, 1.0)
        text = ""
        for i, (a, b, t) in enumerate(ws):
            if i and a - ws[i - 1][1] > GAP * h:
                text += " "
            text += t
        lines.append({
            "text": text,
            "x": float(ln.get("xMin")) * DPI, "y": y0 * DPI,
            "w": (float(ln.get("xMax")) - float(ln.get("xMin"))) * DPI, "h": h * DPI,
        })
    return lines


if __name__ == "__main__":
    pdf, outdir, a, b = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    os.makedirs(outdir, exist_ok=True)
    for n in range(a, b + 1):
        ls = page_lines(pdf, n)
        open(f"{outdir}/p{n}.txt", "w", encoding="utf8").write("\n".join(l["text"] for l in ls) + "\n")
        json.dump(ls, open(f"{outdir}/p{n}.lines.json", "w"), ensure_ascii=False)
