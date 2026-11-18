#!/usr/bin/env python3
"""Run the same pages through several OCR pipelines and print character error rate (CER) and time per page.

    python3 scripts/duel.py PAGES_DIR --legatorocr target/release/legatorocr --tessdata DIR [--lang ron] [--only A,B]

PAGES_DIR holds `NAME.png` (or .jpg) with a reference `NAME.txt` each. Pipelines run (each skipped when its program is missing):
  tesseract            tesseract on the untouched image (psm 3)
  ocrmypdf             ocrmypdf, no image options, text taken back with pdftotext
  ocrmypdf+unpaper     ocrmypdf --deskew --clean (needs unpaper)
  legatorocr+tesseract legatorocr --ocr --engine tesseract
  legatorocr+native    legatorocr --ocr --engine native
The first letter of a page name is its group (A1, A2 ... = group A), so one folder can hold several typefaces or conditions;
the mean is printed per group. CER = edit distance / reference length after collapsing white space and treating the cedilla and
comma-below forms of s and t as the same letter. The references of synthetic pages are exact; with real pages they are only as
good as the text you give. Needs `pip install rapidfuzz`; nothing is written outside --work.
"""
import argparse, collections, glob, os, re, shutil, subprocess, sys, tempfile, time, unicodedata
from rapidfuzz.distance import Levenshtein as L


def norm(s):
    s = unicodedata.normalize("NFC", s).replace("\xad", "-")
    s = s.replace("ş", "ș").replace("Ş", "Ș").replace("ţ", "ț").replace("Ţ", "Ț")
    return re.sub(r"\s+", " ", s).strip()


def run(cmd, env=None, timeout=600):
    t = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)
    return r, time.time() - t


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pages")
    ap.add_argument("--legatorocr", default="target/release/legatorocr")
    ap.add_argument("--tessdata", required=True, help="folder with <lang>.traineddata and configs/")
    ap.add_argument("--lang", default="ron")
    ap.add_argument("--only", help="comma list of pipelines to run")
    ap.add_argument("--work", default=None)
    ap.add_argument("--dewarp", action="store_true",
                    help="also run legatorocr --dewarp (both engines) and page-dewarp followed by tesseract; use with curved pages "
                         "(tools/ocr-train/warp.py). Combine with --only to skip the other pipelines")
    a = ap.parse_args()
    work = a.work or tempfile.mkdtemp(prefix="duel-")
    os.makedirs(work, exist_ok=True)
    env = dict(os.environ, TESSDATA_PREFIX=a.tessdata)
    pages = sorted(p for p in glob.glob(os.path.join(a.pages, "*")) if p.lower().endswith((".png", ".jpg", ".jpeg")))
    have = lambda p: shutil.which(p) is not None
    pipes = {}
    if have("tesseract"):
        pipes["tesseract"] = lambda img, base: (
            run(["tesseract", img, base, "-l", a.lang, "--psm", "3"], env), open(base + ".txt", encoding="utf8").read())
    if have("ocrmypdf") and have("pdftotext"):
        def omp(extra):
            def f(img, base):
                r = run(["ocrmypdf", "-l", a.lang, "--force-ocr", "--output-type", "pdf", "--image-dpi", "300", *extra, img, base + ".pdf"], env)
                txt = subprocess.run(["pdftotext", "-raw", base + ".pdf", "-"], capture_output=True, text=True).stdout
                return r, txt
            return f
        pipes["ocrmypdf"] = omp([])
        if have("unpaper"):
            pipes["ocrmypdf+unpaper"] = omp(["--deskew", "--clean"])
    if os.path.exists(a.legatorocr):
        for eng in ("tesseract", "native"):
            def f(img, base, eng=eng):
                r = run([a.legatorocr, img, "--ocr", "--engine", eng, "--lang", a.lang, "--tessdata", a.tessdata,
                         "--split", "off", "--txt", base + ".txt", "--mode", "gray", "--format", "none"], env)
                try:
                    txt = open(base + ".txt", encoding="utf8").read()
                except OSError:
                    txt = ""
                return r, txt
            pipes["legatorocr+" + eng] = f
    if a.dewarp and os.path.exists(a.legatorocr):
        for eng in ("tesseract", "native"):
            def f(img, base, eng=eng):
                r = run([a.legatorocr, img, "--ocr", "--engine", eng, "--lang", a.lang, "--tessdata", a.tessdata, "--dewarp",
                         "--split", "off", "--txt", base + ".txt", "--mode", "gray", "--format", "none"], env)
                try:
                    txt = open(base + ".txt", encoding="utf8").read()
                except OSError:
                    txt = ""
                return r, txt
            pipes["legatorocr+dewarp+" + eng] = f
    if a.dewarp and have("page-dewarp") and have("tesseract"):
        def pdw(img, base):
            od = base + "_pd"
            os.makedirs(od, exist_ok=True)
            r, secs = run(["page-dewarp", "-nb", "1", "-o", od, img], env, timeout=900)
            outs = sorted(glob.glob(os.path.join(od, "*_thresh.png")) + glob.glob(os.path.join(od, "*.png")))
            if not outs:
                return (r, secs), ""
            r2, s2 = run(["tesseract", outs[0], base, "-l", a.lang, "--psm", "3"], env)
            return (r2, secs + s2), open(base + ".txt", encoding="utf8").read()
        pipes["page-dewarp+tesseract"] = pdw
    if a.only:
        pipes = {k: v for k, v in pipes.items() if k in a.only.split(",")}
    res = collections.defaultdict(lambda: collections.defaultdict(list))   # pipe -> group -> [(cer, secs)]
    for img in pages:
        name = os.path.splitext(os.path.basename(img))[0]
        refp = os.path.splitext(img)[0] + ".txt"
        if not os.path.exists(refp):
            continue
        ref = norm(open(refp, encoding="utf8").read())
        for pn, fn in pipes.items():
            base = os.path.join(work, pn.replace("+", "_") + "_" + name)
            try:
                (r, secs), txt = fn(img, base)
            except Exception as e:
                print("  %s %s failed: %s" % (pn, name, e), file=sys.stderr)
                continue
            cer = 100.0 * L.distance(norm(txt), ref) / max(1, len(ref))
            res[pn][name[0]].append((cer, secs))
            print("%-22s %-6s CER %6.2f %%  %5.1f s" % (pn, name, cer, secs), file=sys.stderr)
    groups = sorted({g for p in res.values() for g in p})
    print("\n| pipeline | " + " | ".join("group %s CER / s per page" % g for g in groups) + " |")
    print("|---|" + "---|" * len(groups))
    for pn in pipes:
        cells = []
        for g in groups:
            v = res[pn].get(g, [])
            cells.append("%.2f %% / %.1f s (n=%d)" % (sum(c for c, _ in v) / len(v), sum(s for _, s in v) / len(v), len(v)) if v else "-")
        print("| %s | %s |" % (pn, " | ".join(cells)))


if __name__ == "__main__":
    main()
