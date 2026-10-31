"""Render a synthetic test page (two paragraphs, optional two columns) with known text.
Usage: makepage.py OUT.png OUT.txt lang [columns] [dpi]"""
import sys, random
from PIL import Image, ImageDraw, ImageFont
import synth
out, txt, lang = sys.argv[1:4]
cols = int(sys.argv[4]) if len(sys.argv) > 4 else 1
dpi = int(sys.argv[5]) if len(sys.argv) > 5 else 300
rng = random.Random(int(sys.argv[6]) if len(sys.argv) > 6 else 3)
W, H = int(8.27 * dpi), int(11.69 * dpi)
im = Image.new("L", (W, H), 245)
d = ImageDraw.Draw(im)
import os
fp = os.environ.get("FONT", "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf")
size = round(11 * dpi / 72)
f = ImageFont.truetype(fp, size)
lines_out = []
colw = (W - 2 * dpi) // cols - (dpi // 4 if cols > 1 else 0)
for c in range(cols):
    x = dpi + c * (colw + dpi // 2)
    y = dpi
    col_lines = []
    while y < H - dpi:
        words = []
        while True:
            w = synth.sample_text(lang, rng).split()
            nxt = " ".join(words + w)
            if f.getlength(nxt) > colw:
                if words:
                    break
                continue
            words += w
            if len(words) > 40: break
        if not words: break
        line = " ".join(words)
        d.text((x, y), line, font=f, fill=20)
        col_lines.append(line)
        y += int(size * 1.5)
        if rng.random() < 0.08: y += size
    lines_out += col_lines
open(txt, "w").write("\n".join(lines_out) + "\n")
if os.environ.get("DEGRADE"):
    import numpy as np
    from PIL import ImageFilter
    im = im.rotate(rng.uniform(-0.4, 0.4), resample=Image.BILINEAR, fillcolor=245)
    im = im.filter(ImageFilter.GaussianBlur(0.8))
    a = np.asarray(im).astype(np.float32)
    g = np.random.default_rng(1)
    a = a + g.normal(0, 9, a.shape)
    a[g.random(a.shape) < 0.0008] = 0
    im = Image.fromarray(np.clip(a, 0, 255).astype(np.uint8))
im.save(out, dpi=(dpi, dpi))
