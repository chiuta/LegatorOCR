"""Synthetic text-line generator: renders text from freely licensed fonts with scanner-like damage.
Only fonts under SIL OFL / GPL-with-exception / Bitstream licences found on the system are used."""
import random, glob, math
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter
from fontTools.ttLib import TTFont
from wordfreq import top_n_list
from charset import CHARSET

LANGS = ["ro", "en", "fr", "it", "es", "pt", "de", "hu"]
H = 32
FONT_GLOBS = [
    "/usr/share/fonts/truetype/dejavu/DejaVu*.ttf",
    "/usr/share/fonts/truetype/liberation/*.ttf",
    "/usr/share/fonts/truetype/freefont/*.ttf",
    "/usr/share/fonts/truetype/crosextra/*.ttf",
    "/usr/share/fonts/opentype/inter/Inter-*.otf",
]
_CS = set(CHARSET)

def load_fonts():
    fonts = []
    for g in FONT_GLOBS:
        for p in sorted(glob.glob(g)):
            if "Mono" in p or "Thin" in p or "ExtraLight" in p or "Black" in p:
                continue
            try:
                cmap = set(TTFont(p, lazy=True).getBestCmap().keys())
            except Exception:
                continue
            fonts.append((p, cmap))
    return fonts

_words = {}
def words(lang):
    if lang not in _words:
        ws = [w for w in top_n_list(lang, 40000) if all(c in _CS for c in w)]
        _words[lang] = ws
    return _words[lang]

def zipf_pick(ws, rng):
    # rank-biased pick: frequent words dominate, tail still appears
    i = int(len(ws) * rng.random() ** 2.4)
    return ws[min(i, len(ws) - 1)]

PUNCT_END = [".", ".", ",", ";", ":", "!", "?", "…"]
def sample_text(lang, rng):
    ws = words(lang)
    n = rng.choice([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14])
    out = []
    for k in range(n):
        r = rng.random()
        if r < 0.05:
            w = str(rng.choice([rng.randint(0, 99), rng.randint(1000, 2030), round(rng.random() * 1000, 2)])).replace(".", rng.choice([",", "."]))
        else:
            w = zipf_pick(ws, rng)
            if k == 0 and rng.random() < 0.6 or rng.random() < 0.06:
                w = w[:1].upper() + w[1:]
            elif rng.random() < 0.012:
                w = w.upper()
        if rng.random() < 0.06:
            w = rng.choice(["(", "„", "«", "“", "'", '"']) + w
        out.append(w)
        if k < n - 1 and rng.random() < 0.10:
            out[-1] += rng.choice([",", ",", ";", ":", "-"])
    s = " ".join(out)
    if rng.random() < 0.5:
        s += rng.choice(PUNCT_END)
    if rng.random() < 0.1 and s[-1].isalnum():
        s += rng.choice([")", "”", "»", '"'])
    return s

def render(text, font_path, size, rng):
    f = ImageFont.truetype(font_path, size)
    w = int(f.getlength(text)) + 4 * size
    h = int(size * 2.2)
    im = Image.new("L", (w, h), 255)
    ImageDraw.Draw(im).text((2 * size, int(size * 0.5)), text, font=f, fill=0)
    if size >= 30:  # ink spread / starvation only where strokes are several pixels wide
        r = rng.random()
        if r < 0.12:
            im = im.filter(ImageFilter.MinFilter(3))
        elif r < 0.22:
            im = im.filter(ImageFilter.MaxFilter(3))
    return im

def crop_normalise(im, rng):
    a = np.asarray(im)
    ink = a < 160
    ys = np.where(ink.any(1))[0]
    xs = np.where(ink.any(0))[0]
    if len(ys) == 0:
        return None
    y0, y1, x0, x1 = ys[0], ys[-1] + 1, xs[0], xs[-1] + 1
    hh = y1 - y0
    pad_y = int(hh * rng.uniform(0.05, 0.30))
    pad_x = int(hh * rng.uniform(0.1, 0.6))
    box = (max(0, x0 - pad_x), max(0, y0 - pad_y), min(a.shape[1], x1 + pad_x), min(a.shape[0], y1 + pad_y))
    c = im.crop(box)
    return c

def degrade(im, rng):
    if rng.random() < 0.5:  # low resolution scan
        s = rng.uniform(0.35, 0.9)
        im = im.resize((max(8, int(im.width * s)), max(8, int(im.height * s))), Image.BILINEAR)
    if rng.random() < 0.6:
        im = im.rotate(rng.uniform(-1.2, 1.2), resample=Image.BILINEAR, expand=True, fillcolor=255)
    if rng.random() < 0.35:
        im = im.filter(ImageFilter.GaussianBlur(rng.uniform(0.3, 1.1)))
    return im

def to_input(im, rng):
    w = max(8, round(im.width * H / im.height))
    im = im.resize((w, H), Image.BILINEAR)
    a = np.asarray(im).astype(np.float32)
    if rng.random() < 0.4:  # hard binarisation like a bitonal scan
        a = np.where(a < rng.uniform(100, 180), 0.0, 255.0).astype(np.float32)
    lo, hi = rng.uniform(0, 60), rng.uniform(200, 255)
    a = lo + a / 255.0 * (hi - lo)
    if rng.random() < 0.5:
        a = a + np.random.default_rng(rng.getrandbits(32)).normal(0, rng.uniform(2, 14), a.shape)
    if rng.random() < 0.15:  # salt and pepper speckle
        g = np.random.default_rng(rng.getrandbits(32))
        m = g.random(a.shape) < 0.004
        a[m] = 255 - a[m]
    return np.clip(a, 0, 255)

def sample(fonts, rng, lang=None):
    while True:
        lang = lang or rng.choice(LANGS)
        text = sample_text(lang, rng)
        path, cmap = rng.choice(fonts)
        if not all(ord(c) in cmap for c in text):
            continue
        size = rng.randint(16, 56)
        im = render(text, path, size, rng)
        c = crop_normalise(im, rng)
        if c is None or c.width < 8:
            continue
        a = to_input(degrade(c, rng), rng)
        if a.shape[1] > 1600 or (a < 128).mean() < 0.01:
            continue
        return a, text
