#!/usr/bin/env python3
"""Typeface-spread OCR benchmark: the same Romanian text set in many installed fonts and scanned
badly (blur, noise, tilt, uneven light, low-quality JPEG), then read by LegatorOCR's native
engine and by Tesseract. The text is known exactly, so error rates are exact; the pages are
still SYNTHETIC (rendered, not scanned), so use it to compare engines and spot fonts that fail,
not as accuracy on real books.

    python3 scripts/ocr-bench-fonts.py target/release/legatorocr [--fonts N] [--seed S] [--out DIR]

Needs Pillow, numpy, fontTools, tesseract (ron) and system fonts that contain ă â î ș ț.
"""
import glob, os, random, subprocess, sys, tempfile, json
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter
from fontTools.ttLib import TTFont

BIN = sys.argv[1]
N = int(sys.argv[sys.argv.index("--fonts") + 1]) if "--fonts" in sys.argv else 40
SEED = int(sys.argv[sys.argv.index("--seed") + 1]) if "--seed" in sys.argv else 7
OUT = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else tempfile.mkdtemp(prefix="lgr-fonts-")
os.makedirs(OUT, exist_ok=True)
TEXT = """Clasorul este o mobilă care servește la păstrarea unor documente sau acte. Clasoarele cu rulou se execută în două variante, și anume: varianta întâi, cu pereții laterali și placa superioară din panel sau plăci celulare; varianta a doua, cu pereții laterali și placa superioară din rame cu placaj.
La interior, clasorul are diferite compartimente. Astfel, clasorul poate avea numai sertare sau numai polițe, iar îmbinările se fac prin cuie, șuruburi sau tipsuri de lemn. Ornamentele populare păstrează motive geometrice străvechi, iar țesăturile au culori vii și tiparele trec din generație în generație."""
NEED = set("ăâîșțĂÂÎȘȚ")

def has_glyphs(path):
    try:
        cmap = TTFont(path, fontNumber=0, lazy=True).getBestCmap()
        return cmap is not None and all(ord(c) in cmap for c in NEED)
    except Exception:
        return False

fonts = sorted(set(glob.glob("/usr/share/fonts/**/*.ttf", recursive=True)))
fonts = [f for f in fonts if not any(k in os.path.basename(f).lower() for k in ("bold", "italic", "oblique", "black", "light", "thin", "mono", "symbol", "emoji", "dingbat", "extra", "condensed", "narrow"))]
fonts = [f for f in fonts if has_glyphs(f)]
random.Random(SEED).shuffle(fonts)
fonts = fonts[:N]

def render(font_path, seed):
    rnd = random.Random(seed)
    W, H = 1700, 1500
    font = ImageFont.truetype(font_path, rnd.choice([22, 26, 30]))
    img = Image.new("L", (W, H), rnd.randint(225, 248))
    d = ImageDraw.Draw(img)
    y, lines = 150, []
    for para in TEXT.split("\n"):
        line = ""
        for w in para.split():
            t = (line + " " + w).strip()
            if d.textlength(t, font=font) > W - 320:
                d.text((160, y), line, font=font, fill=rnd.randint(10, 50)); lines.append(line); y += int(font.size * 1.3); line = w
            else:
                line = t
        d.text((160, y), line, font=font, fill=rnd.randint(10, 50)); lines.append(line); y += int(font.size * 2.1)
    # scan damage
    img = img.rotate(rnd.uniform(-1.5, 1.5), resample=Image.BICUBIC, fillcolor=240)
    a = np.asarray(img).astype(np.float32)
    yy, xx = np.mgrid[0:H, 0:W]
    a *= (0.82 + 0.18 * (xx / W) * rnd.choice([0, 1]) + 0.12 * np.sin(yy / 400.0))   # uneven light
    a += np.random.default_rng(seed).normal(0, rnd.uniform(14, 30), a.shape)           # sensor noise
    img = Image.fromarray(np.clip(a, 0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(rnd.uniform(1.1, 1.9)))
    return img, " ".join(lines)

def norm(s):
    return "".join(s.split())

def cer(ref, hyp):
    a, b = norm(ref), norm(hyp)
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1] / max(1, len(a))

rows = []
for k, f in enumerate(fonts):
    img, ref = render(f, SEED * 1000 + k)
    p = os.path.join(OUT, f"f{k:02d}.jpg")
    img.save(p, quality=random.Random(k).choice([35, 50, 70]))
    res = {}
    for eng in ("native", "tesseract"):
        js = os.path.join(OUT, f"f{k:02d}.{eng}.json")
        subprocess.run([BIN, p, "--dpi", "300", "--ocr", "--lang", "ron", "--engine", eng, "--format", "none",
                        "--pdf", os.path.join(OUT, "x.pdf"), "--json", js], capture_output=True)
        try:
            words = json.load(open(js))["pages"][0]["words"]
            res[eng] = cer(ref, " ".join(w["text"] for w in words))
        except Exception:
            res[eng] = 1.0
    rows.append((os.path.basename(f), res["native"], res["tesseract"]))
    print(f"{rows[-1][0]:42s} native {res['native']*100:6.2f} %   tesseract {res['tesseract']*100:6.2f} %", flush=True)
n, t = np.mean([r[1] for r in rows]) * 100, np.mean([r[2] for r in rows]) * 100
w = sum(r[1] < r[2] for r in rows)
print(f"\n{len(rows)} fonts: mean CER native {n:.2f} %  tesseract {t:.2f} %  (native better on {w}, worse on {sum(r[1] > r[2] for r in rows)})")
print("worst native:", ", ".join(f"{r[0]} {r[1]*100:.0f}%" for r in sorted(rows, key=lambda r: -r[1])[:5]))
