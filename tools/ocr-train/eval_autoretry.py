#!/usr/bin/env python3
"""Gate experiment for `--auto-retry` (ROADMAP 4.24): CER before (default settings) and after.

  eval_autoretry.py real  [EXE] [IMG_DIR REF_DIR]   12 real Romanian pages (as eval_real.py)
  eval_autoretry.py synth [EXE]                     8 SYNTHETIC degraded pages (known text)

Environment: EVAL_ENGINE (default tesseract), EXTRA (extra CLI args for both runs, e.g. "--mode bw"),
BELOW (value for --auto-retry-below, default 0.8; 1.1 retries every page).
Prints per page: errors before/after, whether the page was retried, and totals. The synthetic
pages are generated here with the renderer of scripts/ocr-bench.py plus blur / skew / uneven light.
Needs rapidfuzz and Pillow. Not a unit test: it takes minutes."""
import os, re, subprocess, sys, tempfile, unicodedata, json, shlex, math, random
from concurrent.futures import ThreadPoolExecutor
from rapidfuzz.distance import Levenshtein as L

kind = sys.argv[1]
exe = sys.argv[2] if len(sys.argv) > 2 else "legatorocr"
engine = os.environ.get("EVAL_ENGINE", "tesseract")
extra = shlex.split(os.environ.get("EXTRA", ""))
below = os.environ.get("BELOW", "0.8")
here = os.path.dirname(os.path.abspath(__file__))


def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()


def read(img, lang, retry):
    with tempfile.TemporaryDirectory() as d:
        cmd = [exe, img, "--pdf", f"{d}/x.pdf", "--ocr", "--lang", lang, "--engine", engine,
               "--json", f"{d}/o.json", "-o", f"{d}/pages", "--explain"] + extra
        if retry:
            cmd += ["--auto-retry", "--auto-retry-below", below]
        r = subprocess.run(cmd, capture_output=True, text=True)
        j = json.load(open(f"{d}/o.json"))
    text = norm(" ".join(w["text"] for pg in j["pages"] for w in pg["words"]))
    retried, changed, kept = 0, 0, ""
    for line in r.stderr.splitlines():
        m = re.match(r"auto-retry: (\d+) page\(s\) retried, (\d+) changed", line)
        if m:
            retried, changed = int(m.group(1)), int(m.group(2))
        if "kept:" in line:
            kept = line.split("kept:")[1].split(";")[0].strip()
    return text, retried, changed, kept


def synth_pages(out):
    src = open(os.path.join(here, "../../scripts/ocr-bench.py"), encoding="utf8").read()
    seg = src[src.index("FONT ="):src.index("def run(")]
    ns = {"math": math, "random": random}
    exec("from PIL import Image, ImageDraw, ImageFont, ImageFilter\n" + seg, ns)
    from PIL import Image, ImageFilter
    jobs = []
    # (lang, condition, extra blur, extra skew, light gradient strength)
    spec = [("ron", "scan", 1.6, 0.0, 0.0), ("ron", "bad", 0.0, 0.0, 0.0), ("eng", "scan", 2.2, 0.0, 0.0),
            ("fra", "scan", 0.0, 3.5, 0.5), ("deu", "bad", 1.4, 0.0, 0.0), ("ron", "clean", 2.6, 0.0, 0.6),
            ("eng", "bad", 0.0, 2.0, 0.0), ("ron", "scan", 0.0, 4.5, 0.8)]
    for n, (lang, cond, blur, skew, light) in enumerate(spec):
        tess, text = ns["TEXT"][lang]
        img, dpi = ns["render"](text, cond, 1000 + n)
        if blur:
            img = img.filter(ImageFilter.GaussianBlur(blur))
        if skew:
            img = img.rotate(skew, resample=Image.BICUBIC, fillcolor=(235, 235, 235))
        if light:
            g = Image.linear_gradient("L").rotate(90).resize(img.size)  # dark left -> light right
            g = g.point(lambda v: int(255 - light * 150 * (1 - v / 255)))
            img = Image.composite(img, Image.new("RGB", img.size, (0, 0, 0)), g)
        png = os.path.join(out, f"s{n}_{lang}_{cond}.png")
        img.save(png, dpi=(dpi, dpi))
        jobs.append((png, tess, norm(text)))
    return jobs


if kind == "real":
    img = sys.argv[3] if len(sys.argv) > 3 else "/tmp/ocr_eval/img"
    ref = sys.argv[4] if len(sys.argv) > 4 else "/tmp/ref3"
    jobs = [(f"{img}/p{p}.png", "ron", norm(open(f"{ref}/p{p}.txt", encoding="utf8").read())) for p in
            (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52)]
else:
    jobs = synth_pages(tempfile.mkdtemp(prefix="autoretry-synth-"))


def one(j):
    png, lang, R = j
    out = []
    for retry in (False, True):
        t, rt, ch, kept = read(png, lang, retry)
        out.append((L.distance(R, t), rt, ch, kept))
    return os.path.basename(png), len(R), out


with ThreadPoolExecutor(2) as ex:
    res = list(ex.map(one, jobs))
tb = ta = tn = 0
better = worse = retried = 0
worst = 0.0
print("page: errors before -> after (CER before -> after); retried; kept")
for name, n, ((eb, _, _, _), (ea, rt, ch, kept)) in res:
    tb += eb; ta += ea; tn += n
    d = 100 * (ea - eb) / n
    retried += rt
    if ea < eb: better += 1
    if ea > eb: worse += 1
    worst = max(worst, d)
    print(f"{name}: {eb} -> {ea} ({100*eb/n:.2f}% -> {100*ea/n:.2f}%) retried={rt} {kept}")
print(f"TOTAL errors {tb} -> {ta} of {tn} chars: CER {100*tb/tn:.2f}% -> {100*ta/tn:.2f}%; "
      f"pages retried {retried}, improved {better}, worse {worse}, worst page change {worst:+.2f} points")
