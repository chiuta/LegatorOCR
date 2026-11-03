"""CER of a model on the real Romanian pages (reference texts are not hand-verified).
Usage: eval_real.py MODEL.lnm [IMG_DIR REF_DIR]   (default /tmp/ocr_eval/img and /tmp/ref3 = `pdftotext -raw` of OCR_rev_06.pdf, page by page)"""
import subprocess, sys, os, re, unicodedata
from concurrent.futures import ThreadPoolExecutor
from rapidfuzz.distance import Levenshtein as L

model = sys.argv[1]
img = sys.argv[2] if len(sys.argv) > 2 else "/tmp/ocr_eval/img"
ref = sys.argv[3] if len(sys.argv) > 3 else "/tmp/ref3"
exe = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../target/release/legatorocr")
PAGES = (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52)
GOOD = (4, 7, 8, 19, 27, 31, 38, 50)  # pages where layout is not the problem

def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()

def one(p):
    import json, tempfile
    with tempfile.TemporaryDirectory() as d:
        subprocess.run([exe, f"{img}/p{p}.png", "--pdf", f"{d}/x.pdf", "--ocr", "--lang", "ron", "--engine", os.environ.get("EVAL_ENGINE", "native"),
                        "--json", f"{d}/o.json", "-o", f"{d}/pages"], capture_output=True,
                       env={**os.environ, "LEGATOROCR_NATIVE_MODEL": os.path.abspath(model)})
        j = json.load(open(f"{d}/o.json"))
    out = " ".join(w["text"] for pg in j["pages"] for w in pg["words"])
    R = norm(open(f"{ref}/p{p}.txt", encoding="utf8").read())
    H = norm(out)
    # space-blind variant: ABBYY's text layer is unreliable about word spaces in either direction
    R2, H2 = R.replace(" ", ""), H.replace(" ", "")
    return p, L.distance(R, H), len(R), L.distance(R2, H2), len(R2)

with ThreadPoolExecutor(2) as ex:
    res = list(ex.map(one, PAGES))
tot = {k: [0, 0] for k in ("all", "good", "all_ns", "good_ns")}
for p, e, n, e2, n2 in res:
    print(f"p{p}: CER {100*e/n:.1f}   no-space CER {100*e2/n2:.1f}")
    for k, a, b in (("all", e, n), ("all_ns", e2, n2)) + ((("good", e, n), ("good_ns", e2, n2)) if p in GOOD else ()):
        tot[k][0] += a; tot[k][1] += b
print("ALL12 CER %.2f%%   GOOD8 CER %.2f%%" % tuple(100 * v[0] / v[1] for v in (tot["all"], tot["good"])))
print("ALL12 no-space CER %.2f%%   GOOD8 no-space CER %.2f%%" % tuple(100 * v[0] / v[1] for v in (tot["all_ns"], tot["good_ns"])))
