"""Order-free word recall (and the CER, which depends on reading order) of EasyOCR (CRAFT + latin_g2 recogniser, a 2020-era neural baseline) and of Tesseract on the same 12 real pages
and the same references/normalisation as eval_real.py. EasyOCR boxes are put in reading order by line clustering
(single column assumed), so the CER is meaningless on two-column pages (it measured 67 % on GOOD8 for that reason alone); the word recall, which ignores order, is the comparable figure.
Usage: eval_neural.py [IMG_DIR REF_DIR]. Needs easyocr (+ torchvision, or a stand-in), tesseract with ron."""
import sys, os, re, subprocess, unicodedata, json
from rapidfuzz.distance import Levenshtein as L
img = sys.argv[1] if len(sys.argv) > 1 else "/tmp/ocr_eval/img"
ref = sys.argv[2] if len(sys.argv) > 2 else "/tmp/ref3"
PAGES = (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52); GOOD = (4, 7, 8, 19, 27, 31, 38, 50)
def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()
def strip_dia(s):
    return "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c))
import easyocr
rd = easyocr.Reader(["ro"], gpu=False, verbose=False)
def easy(p):
    res = rd.readtext(f"{img}/p{p}.png")
    boxes = [(sum(pt[1] for pt in b) / 4, min(pt[0] for pt in b), max(pt[1] for pt in b) - min(pt[1] for pt in b), t) for b, t, c in res]
    boxes.sort(); lines, cur, last = [], [], None
    for y, x, h, t in boxes:
        if last is not None and abs(y - last) > 0.6 * h: lines.append(cur); cur = []
        cur.append((x, t)); last = y if not cur[:-1] else last
    lines.append(cur)
    return " ".join(t for ln in lines for x, t in sorted(ln))
def tess(p):
    return subprocess.run(["tesseract", f"{img}/p{p}.png", "-", "-l", "ron"], capture_output=True, text=True).stdout
from collections import Counter
def words(t): return Counter(w for w in re.findall(r"\w+", t.lower()) if len(w) >= 3)
rows = {"easy": {}, "tess": {}}; rec = {"easy": {}, "tess": {}}
os.makedirs("/tmp/neural_out", exist_ok=True)
for p in PAGES:
    R = norm(open(f"{ref}/p{p}.txt", encoding="utf8").read())
    for k, f in (("easy", easy), ("tess", tess)):
        H = norm(f(p)); open(f'/tmp/neural_out/{k}_p{p}.txt','w').write(H); rw = words(R); rec[k][p] = (sum((rw & words(H)).values()), sum(rw.values())); rows[k][p] = (L.distance(R, H), len(R), L.distance(strip_dia(R), strip_dia(H)))
    print(p, {k: round(100 * rows[k][p][0] / rows[k][p][1], 1) for k in rows}, flush=True)
for k in rows:
    for name, ps in (("ALL12", PAGES), ("GOOD8", GOOD)):
        e = sum(rows[k][p][0] for p in ps); n = sum(rows[k][p][1] for p in ps); d = sum(rows[k][p][2] for p in ps)
        a = sum(rec[k][p][0] for p in ps); b = sum(rec[k][p][1] for p in ps)
        print(f"{k:5s} {name} word recall {100*a/b:.1f} %")
        print(f"{k:5s} {name} CER {100*e/n:.2f} %   (diacritic-blind {100*d/n:.2f} %)")
