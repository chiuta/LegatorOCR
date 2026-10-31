#!/usr/bin/env python3
"""Synthetic OCR benchmark: raw Tesseract vs LegatorOCR (clean-up + Tesseract) on pages
rendered from known text in the eight interface languages, under three scan conditions.

    python3 scripts/ocr-bench.py [path/to/legatorocr] [--out DIR]

Needs Pillow, tesseract with eng/ron/fra/ita/spa/por/deu/hun models and DejaVu Serif.
The text is ground truth, so CER/WER are exact; but the pages are SYNTHETIC (one font, no
real paper): treat the numbers as a regression benchmark, not as accuracy on real books.
"""
import math, os, random, subprocess, sys, tempfile
from PIL import Image, ImageDraw, ImageFont, ImageFilter

BIN = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("--") else "legatorocr"
OUT = tempfile.mkdtemp(prefix="lgr-bench-")
if "--out" in sys.argv:
    OUT = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(OUT, exist_ok=True)
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"

TEXT = {
 "ron": ("ron", """Tehnologia producerii mobilei a evoluat mult în ultimele decenii. Lemnul masiv, plăcile aglomerate și furnirul sunt materialele de bază, iar alegerea lor depinde de scopul și de locul în care va fi folosit mobilierul. Înainte de asamblare, piesele se șlefuiesc cu grijă, apoi se îmbină prin cuie, șuruburi sau tipsuri de lemn.
Ornamentele populare din județul Argeș păstrează motive geometrice străvechi. Țesăturile și cusăturile au culori vii, iar meșterii transmit tiparele din generație în generație, așa cum se face și astăzi în satele de la munte."""),
 "eng": ("eng", """The technology of furniture making has changed a great deal over recent decades. Solid wood, particle board and veneer are the basic materials, and the choice depends on the purpose and the place where the furniture will be used. Before assembly the parts are sanded with care, then joined with nails, screws or wooden dowels.
Traditional ornaments keep very old geometric motifs. Woven and embroidered fabrics have bright colours, and craftsmen hand the patterns down from one generation to the next, as is still done today in the mountain villages."""),
 "fra": ("fra", """La technologie de fabrication des meubles a beaucoup évolué au cours des dernières décennies. Le bois massif, les panneaux de particules et le placage sont les matériaux de base, et le choix dépend de l'usage et de l'endroit où le meuble sera placé. Avant l'assemblage, les pièces sont poncées avec soin, puis réunies par des clous, des vis ou des chevilles.
Les ornements populaires conservent de très anciens motifs géométriques. Les tissus et les broderies ont des couleurs vives, et les artisans transmettent les motifs de génération en génération."""),
 "ita": ("ita", """La tecnologia di produzione dei mobili è cambiata molto negli ultimi decenni. Il legno massiccio, i pannelli truciolari e il impiallacciato sono i materiali di base, e la scelta dipende dallo scopo e dal luogo in cui il mobile sarà usato. Prima del montaggio i pezzi vengono levigati con cura, poi uniti con chiodi, viti o spinotti di legno.
Gli ornamenti popolari conservano motivi geometrici molto antichi. Tessuti e ricami hanno colori vivaci, e gli artigiani tramandano i disegni di generazione in generazione."""),
 "spa": ("spa", """La tecnología de fabricación de muebles ha cambiado mucho en las últimas décadas. La madera maciza, los tableros de partículas y la chapa son los materiales básicos, y la elección depende del uso y del lugar donde se colocará el mueble. Antes del montaje, las piezas se lijan con cuidado y luego se unen con clavos, tornillos o clavijas de madera.
Los adornos populares conservan motivos geométricos muy antiguos. Los tejidos y los bordados tienen colores vivos, y los artesanos transmiten los dibujos de generación en generación."""),
 "por": ("por", """A tecnologia de fabrico de móveis mudou muito nas últimas décadas. A madeira maciça, os aglomerados e a folha de madeira são os materiais básicos, e a escolha depende da finalidade e do lugar onde o móvel será usado. Antes da montagem, as peças são lixadas com cuidado e depois unidas com pregos, parafusos ou cavilhas de madeira.
Os ornamentos populares conservam motivos geométricos muito antigos. Os tecidos e os bordados têm cores vivas, e os artesãos transmitem os desenhos de geração em geração."""),
 "deu": ("deu", """Die Technik der Möbelherstellung hat sich in den letzten Jahrzehnten stark verändert. Massivholz, Spanplatten und Furnier sind die Grundmaterialien, und die Wahl hängt vom Zweck und vom Ort ab, an dem das Möbel stehen wird. Vor dem Zusammenbau werden die Teile sorgfältig geschliffen und dann mit Nägeln, Schrauben oder Holzdübeln verbunden.
Volkstümliche Verzierungen bewahren sehr alte geometrische Muster. Gewebe und Stickereien haben lebhafte Farben, und die Handwerker geben die Muster von Generation zu Generation weiter."""),
 "hun": ("hun", """A bútorgyártás technológiája az utóbbi évtizedekben sokat változott. A tömör fa, a forgácslap és a furnér az alapanyagok, a választás pedig a felhasználás céljától és helyétől függ. Az összeszerelés előtt az alkatrészeket gondosan csiszolják, majd szegekkel, csavarokkal vagy fadübelekkel kötik össze.
A népi díszítések nagyon régi mértani mintákat őriznek. A szövetek és a hímzések élénk színűek, a mesterek pedig nemzedékről nemzedékre adják tovább a mintákat."""),
}

def render(text, cond, seed):
    random.seed(seed)
    W, H, dpi = 1700, 2200, 300           # roughly A4 at 300 dpi
    font = ImageFont.truetype(FONT, 46)    # ≈ 11 pt
    img = Image.new("L", (W, H), 245)
    d = ImageDraw.Draw(img)
    y = 190
    for para in text.split("\n"):
        words, line = para.split(), ""
        for w in words:
            t = (line + " " + w).strip()
            if d.textlength(t, font=font) > W - 340:
                d.text((170, y), line, font=font, fill=20); y += 72; line = w
            else:
                line = t
        d.text((170, y), line, font=font, fill=20); y += 110
    if cond == "clean":
        return img.convert("RGB"), dpi
    # scanner-like degradation: skew, uneven light, noise, dark border
    img = img.rotate(2.4, resample=Image.BICUBIC, fillcolor=60, expand=False)
    px = img.load()
    for yy in range(0, H):
        for xx in range(0, W, 1):
            v = px[xx, yy]
            g = 1.0 - 0.22 * (xx / W) - 0.12 * math.sin(yy / 380.0)
            px[xx, yy] = max(0, min(255, int(v * g)))
    img = img.filter(ImageFilter.GaussianBlur(0.9))
    if cond == "bad":
        px = img.load()
        for _ in range(26000):
            x, y2 = random.randrange(W), random.randrange(H)
            px[x, y2] = random.choice((10, 40, 200))
        d = ImageDraw.Draw(img)
        d.rectangle([0, 0, 90, H], fill=15)        # scanner edge shadow
        d.rectangle([W - 40, 0, W, H], fill=30)
    return img.convert("RGB"), dpi

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)

def cer(ref, hyp):
    r = run([BIN, "--compare-text", ref, hyp])
    return r.stdout.strip().split("  (")[0] if r.returncode == 0 else "error: " + r.stderr.strip()[:80]

rows = []
for lang, (tess, text) in TEXT.items():
    ref = os.path.join(OUT, f"{lang}.ref.txt"); open(ref, "w").write(text)
    for cond in ("clean", "scan", "bad"):
        img, dpi = render(text, cond, hash((lang, cond)) & 0xFFFF)
        png = os.path.join(OUT, f"{lang}_{cond}.png"); img.save(png, dpi=(dpi, dpi))
        raw = os.path.join(OUT, f"{lang}_{cond}.raw.txt")
        open(raw, "w").write(run(["tesseract", png, "stdout", "-l", tess, "--psm", "3"]).stdout)
        out = os.path.join(OUT, f"{lang}_{cond}.lgr.txt")
        r = run([BIN, png, "--mode", "bw", "--ocr", "--lang", tess, "--txt", out, "--format", "none", "-o", os.path.join(OUT, "o")])
        if r.returncode != 0:
            print(lang, cond, r.stderr[-200:], file=sys.stderr); continue
        rows.append((lang, cond, cer(ref, raw), cer(ref, out)))

print("| lang | scan | raw Tesseract | LegatorOCR (clean-up + Tesseract) |")
print("|---|---|---|---|")
for r in rows:
    print("| %s | %s | %s | %s |" % r)
print("\nfiles:", OUT, file=sys.stderr)
