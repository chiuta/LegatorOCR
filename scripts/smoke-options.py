#!/usr/bin/env python3
"""Runs the command line with many rarely used options on one synthetic page; each run must exit 0 and
write its output. Not a quality test: it proves the option is accepted and does not crash.
Usage: scripts/smoke-options.py [path/to/legatorocr]"""
import subprocess, sys, os, tempfile, pathlib
from PIL import Image, ImageDraw, ImageFont
root = pathlib.Path(__file__).resolve().parent.parent
exe = sys.argv[1] if len(sys.argv) > 1 else str(root / "target/release/legatorocr")
d = pathlib.Path(tempfile.mkdtemp())
im = Image.new("L", (1240, 1754), 235)
dr = ImageDraw.Draw(im)
try: f = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf", 34)
except Exception: f = ImageFont.load_default()
for i in range(18): dr.text((140, 160 + i * 52), "Aceasta este o linie de text de probă numărul %d, cu cifre 2026." % i, fill=40, font=f)
dr.line([(140, 1180), (1100, 1180)], fill=60, width=3)
im.save(d / "p.png")
(d / "p2.png").write_bytes((d / "p.png").read_bytes())
(d / "rules.txt").write_text("[Text]\nmatch = any\nterms = linie, proba\ntag = demo\nfolder = Demo\n")
CASES = {
 "ocrmypdf-aliases": ["--language", "ron", "--force-ocr", "--deskew", "--rotate-pages", "--output-type", "pdfa-2", "-j", "2", "--sidecar", "{o}.txt"],
 "alto": ["--ocr", "--alto", "{o}.alto.xml"], "page-xml": ["--ocr", "--page-xml", "{o}.pagexml.xml"],
 "author": ["--author", "A. Autor", "--subject", "Subiect", "--keywords", "a,b"],
 "auto-levels": ["--auto-levels"], "auto-rotate": ["--auto-rotate"], "invert": ["--invert"],
 "normalize": ["--normalize"], "sharpen": ["--sharpen", "1"], "descreen": ["--descreen"],
 "denoise": ["--mode", "color", "--denoise", "0.6"], "deblur": ["--mode", "color", "--deblur", "0.6"],
 "remove-background": ["--mode", "color", "--remove-background"], "optimize": ["--mode", "color", "--optimize", "3"], "optimize0": ["--mode", "color", "--optimize", "0"],
 "right-to-left": ["--split", "middle", "--right-to-left"],
 "erase-color": ["--mode", "color", "--erase", "20,20,30,30", "--erase-color", "#ff0000"],
 "fold-shadow": ["--mode", "color", "--fold-shadow"],
 "classify": ["--ocr", "--classify", "{d}/rules.txt", "--classify-out", "{o}.tags.json", "--sort-originals", "{d}/sorted"],
 "dewarp-margin": ["--dewarp-margin"],
 "smooth-glyphs": ["--mode", "bw", "--smooth-glyphs", "2"],
 "figure": ["--ocr", "--tagged-pdf", "--figure", "1,10,10,50,40=Diagram"], "ocr-zone": ["--ocr", "--ocr-zone", "0,0,500,500"], "ocr-ignore": ["--ocr", "--ocr-ignore", "0,0,10,10"],
 "lang-auto": ["--ocr", "--lang-auto", "eng,eng+eng"],
 "object-streams": ["--object-streams"], "linearize": ["--linearize"], "pdfa-level": ["--pdfa", "--pdfa-level", "U"], "jbig2": ["--mode", "bw", "--jbig2"], "encrypt-pdf": ["--encrypt-pdf"], "pdf-permissions": ["--encrypt-pdf", "--pdf-permissions", "print,copy"],
 "bernsen": ["--threshold", "bernsen"], "phansalkar": ["--threshold", "phansalkar"], "edgeplus": ["--threshold", "edgeplus"],
 "despeckle": ["--despeckle-strength", "2"], "no-despeckle": ["--no-despeckle"], "no-deskew": ["--no-deskew"],
 "no-auto-orient": ["--no-auto-orient"], "max-skew": ["--max-skew", "5"], "jpeg": ["--format", "jpeg", "--jpeg-quality", "80", "--out-dir", "{d}/o"],
"fit": ["--page-size", "a4", "--fit-width", "--fit-height"],
 "picture": ["--mode", "mixed", "--picture-sensitivity", "0.5"], "remove-lines": ["--remove-lines"],
 "show-through": ["--show-through"], "perspective": ["--perspective"], "uniform": ["--uniform-size"],
 "target-size": ["--target-size", "200"], "timings": ["--timings"], "explain": ["--explain"],
 "keep-going": ["--keep-going"], "min-conf": ["--ocr", "--min-conf", "50"], "clean-text": ["--ocr", "--clean-text"],
 "ocr-mode": ["--ocr", "--ocr-mode", "force"], "ocr-languages": ["--ocr", "--ocr-languages"],
 "page-labels": ["--ocr", "--page-labels"], "doubtful": ["--ocr", "--doubtful-words", "{o}.tsv", "--doubtful-conf", "90"],
 "tei": ["--ocr", "--tei", "{o}.tei.xml"], "tables": ["--ocr", "--tables-xlsx", "{o}.xlsx", "--tables-html", "{o}.html"],
 "mrz": ["--ocr", "--mrz"], "mrz-no-repair": ["--ocr", "--mrz", "--mrz-no-repair"], "verify-text": ["--ocr", "--verify-text"], "quality-report": ["--quality-report"],
 "report": ["--report", "{o}.r.json"], "outliers": ["--report-outliers"], "checksums": ["--checksums"],
  "profile": ["--profile", "diploma"], "save-profile": ["--save-profile", "smokeprof"], "save-project": ["--save-project", "{d}/x.lgr"],
 "retain": ["--audit-log", "{d}/audit.jsonl", "--retain-until", "2040-01-01"], "rights": ["--rights-author", "X", "--rights-death-year", "1950"],
 "legal-hold": ["--audit-log", "{d}/audit2.jsonl", "--legal-hold"], "front-matter": ["--front-matter", "1"], "uniform2": ["--punch-holes"], "fingers": ["--fingers"],
 "phone-paper": ["--phone-paper", "A4"], "djvu-thumbs": ["--djvu", "{o}.djvu", "--djvu-thumbnails"],
 "max-memory": ["--max-memory", "512"], "decoder": ["--isolate-decoders", "--decoder-memory", "512", "--decoder-timeout", "30"],
"proof-sheet": ["--ocr", "--proof-sheet", "{o}.html"],
"ocfl": ["--ocfl", "{d}/ocfl-obj"],
 "sheet-turn": ["--sheet-turn", "-1.5", "--split", "auto"], "auto-sheet-turn": ["--auto-sheet-turn", "--split", "auto"],
 "accurate": ["--ocr", "--accurate"], "cedilla": ["--ocr", "--clean-text", "--cedilla"],
 "epub": ["--ocr", "--epub", "{o}.epub"],
 "vectorize": ["--vectorize", "{d}/svg", "--vectorize-min-area", "20", "--vectorize-tolerance", "1.5"],
 "verify-pack": ["--ocr", "--verify-pack", "{o}.vp.html", "--verify-conf", "95", "--verify-per-page", "10"],
 "skip-dup": ["--skip-duplicates"], "dewarp-guide": ["--dewarp", "--dewarp-guide", "0.1,0.5 0.5,0.5 0.9,0.5"],
}
import os
os.environ.setdefault("LEGATOROCR_PDF_USER_PASSWORD", "smoke")
os.environ.setdefault("LEGATOROCR_PDF_OWNER_PASSWORD", "smoke-owner")
fail = []
# PDF toolbox options work on an existing PDF
base = str(d / "base.pdf")
subprocess.run([exe, str(d / "p.png"), "--pdf", base, "--dpi", "150"], capture_output=True)
subprocess.run([exe, str(d / "p.png"), "--pdf", str(d / "base2.pdf"), "--dpi", "150"], capture_output=True)
TOOLBOX = {"watermark": ["--watermark", "COPIE"], "number-pages": ["--number-pages"], "extract-pages": ["--extract-pages", "1"],
           "header": ["--header", "{n}/{total}"], "footer": ["--footer", "{date}"], "bates": ["--bates", "A-:1:4"],
           "sanitize": ["--sanitize"], "rotate-pdf": ["--rotate-pdf", "90:1"], "crop-pdf": ["--crop-pdf", "5,5,5,5"], "booklet": ["--booklet"],
           "merge-pdf": ["--merge-pdf"], "verify-text": ["--verify-text"]}
for k, a in TOOLBOX.items():
    if k == "merge-pdf": cmd = [exe, base, str(d / "base2.pdf"), "--merge-pdf", "--pdf", str(d / "m.pdf")]
    elif k == "verify-text": continue
    else: cmd = [exe, base, "--pdf", str(d / ("t_" + k + ".pdf"))] + a
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    if r.returncode: fail.append((k, r.returncode, (r.stderr or r.stdout)[:160].strip()))
    print(("ok   " if r.returncode == 0 else "FAIL ") + "toolbox " + k)
for k, a in CASES.items():
    o = str(d / ("o_" + k))
    args = [x.replace("{o}", o).replace("{d}", str(d)) for x in a]
    r = subprocess.run([exe, str(d / "p.png"), "--pdf", o + ".pdf", "--dpi", "150"] + args, capture_output=True, text=True, timeout=300)
    if r.returncode and "tesseract not found" in (r.stderr or ""):
        print("skip " + k + " (needs Tesseract)")
        continue
    ok = r.returncode == 0
    if not ok: fail.append((k, r.returncode, (r.stderr or r.stdout)[-160:].strip()))
    print(("ok   " if ok else "FAIL ") + k)
# --pdfa-check: a --pdfa file passes the built-in preflight, a plain one does not
subprocess.run([exe, str(d / "p.png"), "--pdf", str(d / "a.pdf"), "--pdfa", "--split", "off"], capture_output=True)
subprocess.run([exe, str(d / "p.png"), "--pdf", str(d / "n.pdf"), "--split", "off"], capture_output=True)
r1 = subprocess.run([exe, "--pdfa-check", str(d / "a.pdf")], capture_output=True, text=True)
r2 = subprocess.run([exe, "--pdfa-check", str(d / "n.pdf")], capture_output=True, text=True)
ok = r1.returncode == 0 and r2.returncode == 1
print(("ok   " if ok else "FAIL ") + "pdfa-check")
if not ok: fail.append(("pdfa-check", (r1.returncode, r2.returncode), (r1.stdout + r2.stdout)[-200:]))
# --compare-text with --diff-html: two texts merged word by word
(d / "ref.txt").write_text("Aceasta este o linie de text de proba.\n")
(d / "ocr.txt").write_text("Aceasta este o linie de test de proba.\n")
r = subprocess.run([exe, "--compare-text", str(d / "ref.txt"), str(d / "ocr.txt"), "--diff-html", str(d / "diff.html")], capture_output=True, text=True)
html = (d / "diff.html").read_text() if (d / "diff.html").exists() else ""
ok = r.returncode == 0 and "test" in html and "text" in html
print(("ok   " if ok else "FAIL ") + "diff-html")
if not ok: fail.append(("diff-html", r.returncode, (r.stderr or r.stdout)[-160:]))
# --scan-escl against a fake scanner (scripts/test-escl.py)
r = subprocess.run([sys.executable, str(root / "scripts/test-escl.py"), exe], capture_output=True, text=True, timeout=600)
print(("ok   " if r.returncode == 0 else "FAIL ") + "eSCL against a fake scanner")
if r.returncode: fail.append(("escl", r.returncode, (r.stdout + r.stderr)[-200:]))
# an external engine that speaks Tesseract's command line (docs/examples/external-ocr-engine.py)
import json
r = subprocess.run([exe, str(d / "p.png"), "--pdf", str(d / "ext.pdf"), "--ocr", "--lang", "ron", "--engine", "tesseract",
                    "--tesseract", str(root / "docs/examples/external-ocr-engine.py"), "--json", str(d / "ext.json")], capture_output=True, text=True)
words = [w["text"] for w in json.load(open(d / "ext.json"))["pages"][0]["words"]] if r.returncode == 0 else []
print(("ok   " if "EXTERN" in words else "FAIL ") + "external engine")
if "EXTERN" not in words: fail.append(("external-engine", r.returncode, r.stderr[-160:]))
# the ONNX bridge (scripts/engine-onnx.py) with synthetic models; skipped when onnx/onnxruntime are missing
r = subprocess.run([sys.executable, str(root / "scripts/test-engine-onnx.py"), exe], capture_output=True, text=True, timeout=900)
if r.returncode == 77: print("skip onnx bridge (needs onnx + onnxruntime)")
else:
    print(("ok   " if r.returncode == 0 else "FAIL ") + "onnx bridge (synthetic models)")
    if r.returncode: fail.append(("onnx-bridge", r.returncode, (r.stdout + r.stderr)[-200:]))
print(f"{len(CASES) + 1 - len(fail)}/{len(CASES) + 1} cases passed")
for f_ in fail: print("  ", f_)
sys.exit(1 if fail else 0)
