#!/usr/bin/env python3
"""Compare OCR engines on real pages with a reference text, one `NAME.txt` per `NAME.(png|jpg|tif)`.

Usage: scripts/bench-real.py DIR [--engines native,tesseract] [--model MODEL.lnm] [--lang ron] [--dpi N] [--exe PATH] [--partial]

For every page and engine it prints
  CER          edit distance / reference length (whitespace collapsed),
  CER-nospace  the same with all spaces removed (a reference without reliable word spaces),
  char recall  reference characters found in order (like score2.py: for partial references),
  word recall  reference words (3+ letters) found in the output (bag of words).
Recall metrics do not penalise extra output, so read them together with CER on full-page references.
A reference may cover only part of a page: pass --partial (CER columns are then left out, recall remains). Pages, references and engines
are listed in the table; nothing is downloaded. To add your own pages: put the image and a text file with the same
name in DIR (a second person should proofread each reference).
"""
import sys, os, re, json, glob, subprocess, tempfile, unicodedata, collections, pathlib
from rapidfuzz.distance import Levenshtein as L

root = pathlib.Path(__file__).resolve().parent.parent
a = sys.argv[1:]
d = pathlib.Path(a[0])
def opt(k, default):
    return a[a.index(k) + 1] if k in a else default
engines = opt("--engines", "native,tesseract").split(",")
lang = opt("--lang", "ron")
exe = opt("--exe", str(root / "target/release/legatorocr"))
model = opt("--model", None)
dpi = opt("--dpi", None)
partial = "--partial" in a
fmt = lambda v: "     -" if partial else f"{v:6.2f}%"

def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț").replace("Ş", "Ș").replace("Ţ", "Ț")
    return re.sub(r"\s+", " ", s).strip()
def toks(s):
    return [t for t in re.findall(r"[^\W\d_]+", norm(s).lower()) if len(t) >= 3]
def recall(ref, hyp):
    r = re.sub(r"[\s\-—–]+", "", norm(ref)).lower(); h = re.sub(r"[\s\-—–]+", "", norm(hyp)).lower()
    m = sum(op.src_end - op.src_start for op in L.opcodes(r, h) if op.tag == "equal")
    rc, hc = collections.Counter(toks(ref)), collections.Counter(toks(hyp))
    return 100 * m / max(len(r), 1), 100 * sum(min(c, hc[t]) for t, c in rc.items()) / max(sum(rc.values()), 1)

def read(img, engine):
    with tempfile.TemporaryDirectory() as t:
        cmd = [exe, img, "--pdf", f"{t}/x.pdf", "--ocr", "--lang", lang, "--engine", engine, "--json", f"{t}/o.json", "-o", f"{t}/pg"]
        if dpi: cmd += ["--dpi", dpi]
        env = dict(os.environ)
        if model and engine == "native": env["LEGATOROCR_NATIVE_MODEL"] = os.path.abspath(model)
        subprocess.run(cmd, capture_output=True, env=env)
        j = json.load(open(f"{t}/o.json"))
    return " ".join(w["text"] for pg in j["pages"] for w in pg["words"])

pages = sorted(p for p in glob.glob(str(d / "*")) if p.lower().endswith((".png", ".jpg", ".jpeg", ".tif", ".tiff")) and os.path.exists(os.path.splitext(p)[0] + ".txt"))
if not pages: sys.exit(f"no NAME.txt next to an image in {d}")
print(f"{'page':<18}{'engine':<11}{'CER':>7}{'CER-ns':>8}{'chars':>8}{'words':>8}")
tot = {e: [0, 0, 0, 0, 0, 0.0, 0.0] for e in engines}
for p in pages:
    ref = open(os.path.splitext(p)[0] + ".txt", encoding="utf8").read()
    R = norm(ref)
    for e in engines:
        H = norm(read(p, e))
        ce = L.distance(R, H); cn = L.distance(R.replace(" ", ""), H.replace(" ", ""))
        rc, rw = recall(ref, H)
        print(f"{os.path.basename(p)[:17]:<18}{e:<11}{fmt(100*ce/max(len(R),1))}{fmt(100*cn/max(len(R.replace(' ','')),1)):>8}{rc:7.1f}%{rw:7.1f}%")
        t = tot[e]; t[0] += ce; t[1] += len(R); t[2] += cn; t[3] += len(R.replace(" ", "")); t[4] += 1; t[5] += rc; t[6] += rw
print("-" * 52)
for e, t in tot.items():
    print(f"{'ALL':<18}{e:<11}{fmt(100*t[0]/max(t[1],1))}{fmt(100*t[2]/max(t[3],1)):>8}{t[5]/t[4]:7.1f}%{t[6]/t[4]:7.1f}%")
