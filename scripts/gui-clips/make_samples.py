#!/usr/bin/env python3
"""Generates the sample scans of the window clips (invented Romanian text). Usage: make_samples.py OUTDIR"""
import sys, pathlib, random, math
from PIL import Image, ImageDraw, ImageFont, ImageOps, ImageFilter
out = pathlib.Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
DJ = "/usr/share/fonts/truetype/dejavu/"
f = ImageFont.truetype(DJ + "DejaVuSerif.ttf", 30); fb = ImageFont.truetype(DJ + "DejaVuSerif-Bold.ttf", 44)
TXT = ["Satul se întindea de-a lungul apei, între dealuri joase,", "iar oamenii știau drumurile după mirosul fânului.", "Primăvara aducea ceață dimineața și clopote seara,", "iar bătrânii povesteau ce se petrecuse înainte de ei.", "Cine pornea la drum lua pâine, sare și o veste bună.", "Nimeni nu se grăbea, fiindcă timpul avea răbdare."]
def page(w, h, title, n, angle=0, paper=238, upper=False):
    im = Image.new("L", (w, h), paper); d = ImageDraw.Draw(im)
    d.text((90, 110), title, font=fb, fill=35)
    for i in range(n): d.text((90, 210 + i * 56), TXT[i % len(TXT)].upper() if upper else TXT[i % len(TXT)], font=f, fill=45)
    return im
# crooked double sheet: two pages side by side, dark gutter, turned 3 degrees
L, R = page(760, 1100, "Capitolul întâi", 17), page(760, 1100, "Capitolul al doilea", 17)
sheet = Image.new("L", (1700, 1250), 60); sheet.paste(L, (60, 70)); sheet.paste(R, (850, 70))
ImageDraw.Draw(sheet).rectangle([820, 70, 850, 1170], fill=90)
sheet.rotate(3, expand=True, fillcolor=60, resample=Image.BICUBIC).save(out / "dublu-strâmb.png", dpi=(200, 200))
# single page with two punch holes
p = page(1000, 1400, "Registru", 20); d = ImageDraw.Draw(p)
for y in (420, 980): d.ellipse([12, y - 22, 56, y + 22], fill=20)
p.save(out / "perforat.png", dpi=(200, 200))
# blank page, faint noise
b = Image.new("L", (1000, 1400), 240); px = b.load(); random.seed(1)
for _ in range(900): px[random.randrange(1000), random.randrange(1400)] = 215
b.save(out / "alba.png", dpi=(200, 200))
# negative (white text on black)
ImageOps.invert(page(1000, 1400, "Microfilm", 20)).save(out / "negativ.png", dpi=(200, 200))
# sideways page
page(1000, 1400, "Pagină culcată", 20).rotate(270, expand=True).save(out / "lateral.png", dpi=(200, 200))
# phone photo: the page on a brown table, seen at an angle
tab = Image.new("RGB", (1600, 1200), (118, 84, 58)); pg = page(900, 1200, "Fotografie", 19).convert("RGB")
coef = Image.new("RGB", (1600, 1200)); 
W, H = pg.size
quad = [(300, 120), (1250, 160), (1330, 1100), (240, 1080)]
import numpy as np
def coeffs(src, dst):
    A = []
    for (x, y), (u, v) in zip(src, dst):
        A.append([x, y, 1, 0, 0, 0, -u * x, -u * y]); A.append([0, 0, 0, x, y, 1, -v * x, -v * y])
    res = np.linalg.solve(np.array(A, float), np.array([c for p_ in dst for c in p_], float)); return res.tolist()
warped = pg.transform((1600, 1200), Image.PERSPECTIVE, coeffs(quad, [(0, 0), (W, 0), (W, H), (0, H)]) , Image.BICUBIC)
mask = Image.new("L", (1600, 1200), 0); ImageDraw.Draw(mask).polygon(quad, fill=255)
tab.paste(warped, (0, 0), mask); tab.filter(ImageFilter.GaussianBlur(0.8)).save(out / "foto-telefon.jpg", quality=88)
# page with bent text lines (for the guide line)
bent = page(1000, 1400, "Pagină curbată", 20).convert("L")
arr = np.array(bent); h, w = arr.shape; res = np.full_like(arr, 238)
for x in range(w):
    s = int(40 * math.sin(math.pi * x / w)); res[:, x] = np.roll(arr[:, x], s)
Image.fromarray(res).save(out / "curbat.png", dpi=(200, 200))
import shutil
shutil.copy(out / "dublu-strâmb.png", out / "dublu-stramb.png")   # ASCII name for typing into file dialogs
print(sorted(x.name for x in out.iterdir()))
