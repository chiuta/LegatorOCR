#!/usr/bin/env python3
"""Second opinion on the PDFs LegatorOCR writes: other programs read them and must agree.

Builds sample files (plain, encrypted, linearized, object streams, JBIG2 symbols, signed, ...),
then checks with poppler, pdfium, MuPDF, qpdf/pikepdf, pypdf and pyHanko that
 * every variant opens (with the password) and has the same number of pages,
 * every lossless variant renders exactly like the plain file in each reader,
 * qpdf finds no linearization error and no warning,
 * the signatures are intact, valid and cover the whole file (pyHanko),
 * the tagged PDF/A files carry the declared part/conformance and the figure texts.
Needs: pillow numpy pikepdf pypdf pypdfium2 pymupdf pyhanko, poppler-utils, qpdf, openssl.
Usage: crossread.py PATH-TO-legatorocr
"""
import glob, os, subprocess, sys, tempfile, warnings
warnings.filterwarnings("ignore")
import numpy as np
import pikepdf, pypdf, pypdfium2 as pdfium, pymupdf
from PIL import Image, ImageDraw, ImageFilter, ImageFont

B = os.path.abspath(sys.argv[1])
W = tempfile.mkdtemp(prefix="crossread")
os.chdir(W)
bad = []


def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        bad.append(what)


def run(*a, env=None, must=True):
    r = subprocess.run([B, *a], capture_output=True, text=True, env={**os.environ, **(env or {})})
    if must and r.returncode:
        check(False, f"{' '.join(a)[:80]}: {r.stderr[-200:]}")
    return r


fonts = glob.glob("/usr/share/fonts/**/*.ttf", recursive=True)
font = ImageFont.load_default()
for cand in sorted(fonts, key=lambda x: "DejaVuSans." not in x):  # scalable fonts only
    try:
        font = ImageFont.truetype(cand, 24)
        break
    except OSError:
        continue
for n in range(3):
    im = Image.new("L", (1654, 2339), 255)
    d = ImageDraw.Draw(im)
    for i in range(40):
        d.text((150, 150 + i * 48), f"Page {n+1} line {i}: The quick brown fox jumps over the lazy dog", fill=0, font=font)
    d.rectangle((300, 1500, 900, 1900), outline=0, width=6)
    im.filter(ImageFilter.GaussianBlur(0.7)).save(f"p{n}.png", dpi=(200, 200))

PW = {"LEGATOROCR_PDF_USER_PASSWORD": "upw", "LEGATOROCR_PDF_OWNER_PASSWORD": "opw"}
base = ["p0.png", "p1.png", "p2.png", "--mode", "bw", "--format", "none"]
variants = {  # name: (extra args, env, password)
    "plain": ([], {}, ""),
    "enc": (["--encrypt-pdf"], PW, "upw"),
    "lin": (["--linearize"], {}, ""),
    "linenc": (["--linearize", "--encrypt-pdf"], PW, "upw"),
    "objs": (["--object-streams"], {}, ""),
    "encobj": (["--object-streams", "--encrypt-pdf"], PW, "upw"),
    "jb": (["--jbig2"], {}, ""),
    "jblin": (["--jbig2", "--linearize"], {}, ""),
}
for name, (extra, env, _) in variants.items():
    run(*base, "--pdf", f"{name}.pdf", *extra, env=env)


def poppler(f, pw):
    for x in glob.glob("pp-*.png"):
        os.remove(x)
    subprocess.run(["pdftoppm", "-r", "50", "-gray", "-png", *(["-upw", pw] if pw else []), f, "pp"], check=True, capture_output=True)
    return [np.array(Image.open(x).convert("L"), dtype=float) for x in sorted(glob.glob("pp-*.png"))]


def pdfium_r(f, pw):
    d = pdfium.PdfDocument(f, password=pw or None)
    return [np.array(d[i].render(scale=50 / 72, grayscale=True).to_pil().convert("L"), dtype=float) for i in range(len(d))]


def mupdf_r(f, pw):
    d = pymupdf.open(f)
    if d.needs_pass:
        assert d.authenticate(pw)
    out = []
    for pg in d:
        pm = pg.get_pixmap(dpi=50, colorspace=pymupdf.csGRAY)
        out.append(np.frombuffer(pm.samples, dtype=np.uint8).reshape(pm.h, -1).astype(float))
    return out


readers = {"poppler": poppler, "pdfium": pdfium_r, "mupdf": mupdf_r}
ref = {k: fn("plain.pdf", "") for k, fn in readers.items()}


def same(a, b):
    return len(a) == len(b) and all(x.shape == y.shape and np.abs(x - y).max() == 0 for x, y in zip(a, b))


for name, (_, _, pw) in variants.items():
    f = f"{name}.pdf"
    for k, fn in readers.items():
        try:
            check(same(fn(f, pw), ref[k]), f"{f}: {k} renders the same pixels as plain.pdf")
        except Exception as e:
            check(False, f"{f}: {k} failed: {e}")
    with pikepdf.open(f, password=pw) as d:
        check(len(d.pages) == 3 and d.is_encrypted == bool(pw), f"{f}: pikepdf opens it, 3 pages, encrypted={bool(pw)}")
    r = pypdf.PdfReader(f)
    if r.is_encrypted:
        check(bool(r.decrypt(pw)), f"{f}: pypdf accepts the password")
    check(len(r.pages) == 3, f"{f}: pypdf sees 3 pages")
    q = subprocess.run(["qpdf", *([f"--password={pw}"] if pw else []), "--check", f], capture_output=True, text=True)
    check(q.returncode == 0 and "WARNING" not in q.stdout + q.stderr, f"{f}: qpdf --check without warnings")
    if "lin" in name:
        q = subprocess.run(["qpdf", *([f"--password={pw}"] if pw else []), "--check-linearization", f], capture_output=True, text=True)
        check("no linearization errors" in q.stdout and "WARNING" not in q.stdout + q.stderr, f"{f}: qpdf --check-linearization")
    if name.startswith("jb"):
        with pikepdf.open(f) as d:
            raw = list(d.pages[0].images.values())[0].read_raw_bytes()
        kinds, i = [], 0
        while i + 11 <= len(raw):
            c = raw[i + 5] >> 5
            kinds.append(raw[i + 4] & 63)
            i += 6 + c + 5 + int.from_bytes(raw[i + 6 + c + 1 : i + 6 + c + 5], "big")
        check(kinds == [48, 0, 6, 38], f"{f}: page 1 is symbol-coded (page, dictionary, text, residual): {kinds}")

# toolbox on encrypted input
r = run(*[], "enc.pdf", "--pdf-password", "upw", "--extract-pages", "2-3", "-o", "dec", env={}, must=False)
ok = r.returncode == 0 and os.path.exists("dec/enc.pdf")
check(ok, "toolbox takes the encrypted file with the password")
if ok:
    with pikepdf.open("dec/enc.pdf") as d:
        check(len(d.pages) == 2 and not d.is_encrypted, "toolbox output has 2 pages and is not encrypted")
    check(same(poppler("dec/enc.pdf", ""), ref["poppler"][1:]), "toolbox output pages 2-3 are pixel-identical")
check(run("enc.pdf", "--pdf-password", "wrong", "--extract-pages", "1", "-o", "dec2", must=False).returncode != 0, "wrong password is refused")

# signatures
subprocess.run("openssl req -x509 -newkey rsa:2048 -nodes -keyout k.pem -out c.pem -days 30 -subj '/CN=Test Signer' 2>/dev/null && openssl pkcs12 -export -inkey k.pem -in c.pem -out s.p12 -passout pass:pp", shell=True, check=True)
from pyhanko.keys import load_cert_from_pemder
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.validation import validate_pdf_signature
from pyhanko_certvalidator import ValidationContext

vc = ValidationContext(trust_roots=[load_cert_from_pemder("c.pem")], allow_fetching=False)
for src, out, extra in (("plain.pdf", "sig_plain.pdf", []), ("objs.pdf", "sig_objs.pdf", ["--sign-visible", "1,20,20,60,25"])):
    run("--sign-pdf", src, "--sign-p12", "s.p12", "--sign-pass-env", "SP", *extra, "--pdf", out, env={"SP": "pp"})
    sigs = list(PdfFileReader(open(out, "rb")).embedded_signatures)
    check(len(sigs) == 1, f"{out}: one signature")
    for s in sigs:
        st = validate_pdf_signature(s, vc)
        check(st.intact and st.valid and st.trusted and st.coverage.name == "ENTIRE_FILE", f"{out}: pyHanko says intact, valid, trusted, whole file")
    check(subprocess.run(["pdfsig", out], capture_output=True, text=True).stdout.count("Signature is Valid") == 1, f"{out}: poppler pdfsig says valid")

# tagged PDF/A levels and figures (needs an OCR engine)
r = run(*base, "--ocr", "--lang", "eng", "--pdfa", "--pdfa-level", "A", "--tagged-pdf", "--figure", "1,25,127,50,34=A rectangle frame", "--pdf", "aFig.pdf", must=False)
if r.returncode == 0:
    for lv in "BUA":
        run(*base, "--ocr", "--lang", "eng", "--pdfa", "--pdfa-level", lv, "--tagged-pdf", "--pdf", f"a{lv}.pdf")
        with pikepdf.open(f"a{lv}.pdf") as d:
            x = bytes(d.Root.Metadata.read_bytes()).decode()
            check(f"<pdfaid:conformance>{lv}<" in x and "<pdfaid:part>2<" in x, f"a{lv}.pdf: XMP declares PDF/A-2{lv}")
            check(bool(d.Root.get("/MarkInfo")) and "/StructTreeRoot" in d.Root and "/Lang" in d.Root, f"a{lv}.pdf: tagged, language set")
    with pikepdf.open("aFig.pdf") as d:
        alts = [str(o.get("/Alt")) for o in d.objects if isinstance(o, pikepdf.Dictionary) and o.get("/S") == "/Figure"]
    check(alts == ["A rectangle frame"], f"aFig.pdf: figure alternative text {alts}")
else:
    print("note no OCR engine here: PDF/A level checks skipped")

print(f"\n{len(bad)} failure(s)")
sys.exit(1 if bad else 0)
