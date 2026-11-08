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

# the toolbox on files encrypted by qpdf (another producer): every standard security handler
for k, (enc, objs) in enumerate((e, o) for e in (["40"], ["128", "--use-aes=n"], ["128", "--use-aes=y"], ["256"], ["256", "--force-R5"]) for o in ([], ["--object-streams=generate"])):
    src = f"qe{k}.pdf"
    q = subprocess.run(["qpdf", "--allow-weak-crypto", "--encrypt", "up", "ow", *enc, "--", *objs, "plain.pdf", src], capture_output=True, text=True)
    if q.returncode:
        print("note qpdf cannot make", enc, q.stderr[:60])
        continue
    for pwd in ("up", "ow"):
        r = run(src, "--pdf-password", pwd, "--extract-pages", "1-3", "-o", f"qd{k}{pwd}", must=False)
        out = f"qd{k}{pwd}/{src}"
        good = r.returncode == 0 and os.path.exists(out) and same(poppler(out, ""), ref["poppler"])
        check(good, f"toolbox opens qpdf-encrypted {' '.join(enc)} {' '.join(objs)} with the {'user' if pwd == 'up' else 'owner'} password, pixels identical")

# links between pages: kept where both ends are kept, dropped otherwise (no dangling destinations)
from pikepdf import Array, Dictionary, Name, Pdf

src = Pdf.new()
pgs = [src.make_indirect(Dictionary(Type=Name.Page, MediaBox=[0, 0, 200, 200], Contents=src.make_stream(b"0 0 50 50 re f"))) for _ in range(30)]
tree = src.make_indirect(Dictionary(Type=Name.Pages, Kids=Array(pgs), Count=30))
for p in pgs:
    p.Parent = tree
for i in range(0, 30, 3):
    pgs[i].Annots = Array([src.make_indirect(Dictionary(Type=Name.Annot, Subtype=Name.Link, Rect=[1, 1, 20, 20], Dest=Array([pgs[(i + 5) % 30], Name.Fit])))])
src.trailer.Root = src.make_indirect(Dictionary(Type=Name.Catalog, Pages=tree))
src.save("links.pdf")
for sel, flags in (("1-30", []), ("1-10", []), ("1-30", ["--linearize"])):
    outd = f"lk_{sel}_{len(flags)}"
    run("links.pdf", "--extract-pages", sel, *flags, "-o", outd)
    with pikepdf.open(f"{outd}/links.pdf") as d:
        ids = {p.objgen for p in d.pages}
        ann = [a for p in d.pages for a in p.get("/Annots", [])]
        ok = all(a.get("/Dest") is not None and a["/Dest"][0] is not None and a["/Dest"][0].objgen in ids for a in ann)
        check(ok and (sel != "1-30" or len(ann) == 10), f"pages {sel} {' '.join(flags)}: {len(ann)} links, all pointing at pages of the output")
    q = subprocess.run(["pdftoppm", "-r", "10", "-png", f"{outd}/links.pdf", "lkx"], capture_output=True, text=True)
    check("Bad annotation" not in q.stderr, f"pages {sel} {' '.join(flags)}: poppler finds no bad destination")

# document information survives the toolbox, also out of an encrypted file with object streams
run(*base, "--pdf", "meta.pdf", "--author", "Ana Păun", "--subject", "Subiect (test)", "--object-streams", "--encrypt-pdf", env=PW)
run("meta.pdf", "--pdf-password", "upw", "--extract-pages", "1-2", "-o", "metadec")
with pikepdf.open("metadec/meta.pdf") as d:
    check(str(d.docinfo.get("/Author")) == "Ana Păun" and str(d.docinfo.get("/Subject")) == "Subiect (test)", "toolbox keeps Author and Subject from an encrypted object-stream file")
run("meta.pdf", "--pdf-password", "upw", "--extract-pages", "1", "--author", "Nou", "-o", "metadec2")
with pikepdf.open("metadec2/meta.pdf") as d:
    check(str(d.docinfo.get("/Author")) == "Nou", "--author overrides in the toolbox")

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

# a second and third signature: unique field names, the earlier ones stay valid
run("--sign-pdf", "sig_plain.pdf", "--sign-p12", "s.p12", "--sign-pass-env", "SP", "--pdf", "sig2.pdf", env={"SP": "pp"})
run("--sign-pdf", "sig2.pdf", "--sign-p12", "s.p12", "--sign-pass-env", "SP", "--sign-visible", "1,10,10,50,20", "--pdf", "sig3.pdf", env={"SP": "pp"})
sigs = list(PdfFileReader(open("sig3.pdf", "rb")).embedded_signatures)
check([s.field_name for s in sigs] == ["Signature1", "Signature2", "Signature3"], f"three signatures with distinct names: {[s.field_name for s in sigs]}")
check(all(validate_pdf_signature(s, vc).valid for s in sigs), "all three signatures valid")
check(all(validate_pdf_signature(s, vc).modification_level.name in ("NONE", "FORM_FILLING") for s in sigs), "only form-filling changes between them")

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
