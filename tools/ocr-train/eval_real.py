"""CER of a model on the real Romanian pages (reference texts are not hand-verified).
Usage: eval_real.py MODEL.lnm [IMG_DIR REF_DIR]   (default /tmp/ocr_eval/img and /tmp/ocr_eval/ref)"""
import subprocess, sys, os, re, unicodedata
from concurrent.futures import ThreadPoolExecutor
from rapidfuzz.distance import Levenshtein as L

model = sys.argv[1]
img = sys.argv[2] if len(sys.argv) > 2 else "/tmp/ocr_eval/img"
ref = sys.argv[3] if len(sys.argv) > 3 else "/tmp/ocr_eval/ref"
exe = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../target/release/legatorocr")
PAGES = (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52)
GOOD = (4, 7, 8, 19, 27, 31, 38, 50)  # pages where layout is not the problem

def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()

def one(p):
    import json, tempfile
    with tempfile.TemporaryDirectory() as d:
        subprocess.run([exe, f"{img}/p{p}.png", "--pdf", f"{d}/x.pdf", "--ocr", "--lang", "ron", "--engine", "native",
                        "--json", f"{d}/o.json", "-o", f"{d}/pages"], capture_output=True,
                       env={**os.environ, "LEGATOROCR_NATIVE_MODEL": os.path.abspath(model)})
        j = json.load(open(f"{d}/o.json"))
    out = " ".join(w["text"] for pg in j["pages"] for w in pg["words"])
    R = norm(open(f"{ref}/p{p}.txt", encoding="utf8").read())
    return p, L.distance(R, norm(out)), len(R)

with ThreadPoolExecutor(2) as ex:
    res = list(ex.map(one, PAGES))
tot = {"all": [0, 0], "good": [0, 0]}
for p, e, n in res:
    print(f"p{p}: CER {100*e/n:.1f}")
    tot["all"][0] += e; tot["all"][1] += n
    if p in GOOD:
        tot["good"][0] += e; tot["good"][1] += n
print("ALL12 CER %.2f%%   GOOD8 CER %.2f%%" % tuple(100 * v[0] / v[1] for v in (tot["all"], tot["good"])))
