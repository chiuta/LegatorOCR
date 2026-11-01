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
SYMS = list("<>|\\^~`{}£¥©®™×÷‘‚‹›·¼½²³ªº")
def numeric_line(rng):
    parts = []
    for _ in range(rng.randint(1, 6)):
        r = rng.random()
        if r < 0.25: parts.append(f"{rng.randint(1, 31):02d}.{rng.randint(1, 12):02d}.{rng.randint(1900, 2030)}")
        elif r < 0.5: parts.append(f"{rng.randint(1, 999)} {rng.randint(0, 999):03d},{rng.randint(0, 99):02d}")
        elif r < 0.65: parts.append(rng.choice(["€", "$", "£", "%", "§", "©", "®", "™", "×", "÷", "="]) + str(rng.randint(0, 9999)))
        elif r < 0.8: parts.append(f"ISBN {rng.randint(100, 999)}-{rng.randint(0, 9)}-{rng.randint(10000, 99999)}-{rng.randint(0, 9)}")
        elif r < 0.9: parts.append(f"{rng.randint(1, 400)}-{rng.randint(1, 400)}")
        else: parts.append("".join(rng.choice("0123456789ABCDEFGHJKLMNPRSTUVWXYZ") for _ in range(rng.randint(3, 9))))
    return " ".join(parts)

def sample_text(lang, rng):
    r0 = rng.random()
    if r0 < 0.07:
        return numeric_line(rng)
    ws = words(lang)
    n = rng.choice([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14] + ([18, 22, 26] if rng.random() < 0.12 else []))
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
    if rng.random() < 0.05:
        out.insert(rng.randrange(len(out) + 1), "".join(rng.choice(SYMS) for _ in range(rng.randint(1, 3))))
    if rng.random() < 0.03:
        out = [w.upper() for w in out]
    s = " ".join(out)
    if rng.random() < 0.5:
        s += rng.choice(PUNCT_END)
    if rng.random() < 0.1 and s[-1].isalnum():
        s += rng.choice([")", "”", "»", '"'])
    return s

CEDILLA = {"ș": "ş", "ț": "ţ", "Ș": "Ş", "Ț": "Ţ"}
def render(text, font_path, size, rng, cmap=None):
    if cmap is not None and any(c in text for c in CEDILLA) and rng.random() < 0.5:
        alt = "".join(CEDILLA.get(c, c) for c in text)
        if all(ord(c) in cmap for c in alt):
            text = alt   # draw the legacy cedilla form; the label keeps the correct comma-below letter
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

def artifacts(im, rng):
    """Things that really sit in a line crop: underline, a table rule, slivers of the lines above/below."""
    a = np.array(im)
    h, w = a.shape
    r = rng.random()
    if r < 0.06:            # underline just under the baseline zone
        y = min(h - 1, int(h * rng.uniform(0.82, 0.96)))
        a[y:y + max(1, h // 18), :] = rng.randint(0, 60)
    elif r < 0.10:          # table rule along the top or bottom edge
        t = max(1, h // 16)
        if rng.random() < 0.5: a[:t, :] = rng.randint(0, 60)
        else: a[h - t:, :] = rng.randint(0, 60)
    elif r < 0.14:          # vertical table border at one side
        t = max(1, h // 14)
        if rng.random() < 0.5: a[:, :t] = rng.randint(0, 60)
        else: a[:, w - t:] = rng.randint(0, 60)
    elif r < 0.22:          # neighbouring line's descender/ascender tips
        for _ in range(rng.randint(1, 4)):
            x = rng.randrange(w); ww = rng.randint(2, max(3, h // 6)); hh = rng.randint(2, max(3, h // 5))
            if rng.random() < 0.5: a[:hh, x:x + ww] = rng.randint(0, 80)
            else: a[h - hh:, x:x + ww] = rng.randint(0, 80)
    return Image.fromarray(a)

def jpeg(im, rng):
    import io
    b = io.BytesIO()
    im.convert("L").save(b, "JPEG", quality=rng.randint(40, 92))
    b.seek(0)
    return Image.open(b).convert("L")

def degrade(im, rng):
    if rng.random() < 0.5:  # low resolution scan
        s = rng.uniform(0.42, 0.9)
        im = im.resize((max(8, int(im.width * s)), max(8, int(im.height * s))), Image.BILINEAR)
    if rng.random() < 0.6:
        im = im.rotate(rng.uniform(-2.0, 2.0), resample=Image.BILINEAR, expand=True, fillcolor=255)
    if rng.random() < 0.35 and im.height > 30:
        im = im.filter(ImageFilter.GaussianBlur(rng.uniform(0.3, 1.0)))
    if rng.random() < 0.3:
        im = jpeg(im, rng)
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
        im = render(text, path, size, rng, cmap)
        if rng.random() < 0.25:  # condensed / extended faces and loose tracking
            k = rng.uniform(0.82, 1.18)
            im = im.resize((max(8, int(im.width * k)), im.height), Image.BILINEAR)
        c = crop_normalise(im, rng)
        if c is not None:
            c = artifacts(c, rng)
        if c is None or c.width < 8:
            continue
        a = to_input(degrade(c, rng), rng)
        if a.shape[1] > 2300 or (a < 128).mean() < 0.01:
            continue
        return a, text
