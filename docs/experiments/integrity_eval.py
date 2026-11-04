#!/usr/bin/env python3
"""Gate experiment for --integrity-hints (ROADMAP 7.5c). SYNTHETIC edits only: results do not
carry over to real documents.

  integrity_eval.py gen   IMGDIR WORK     build the test set (clean pages -> JPEG q85; edited copies)
  integrity_eval.py run   WORK BIN        run BIN --integrity-hints on every file
  integrity_eval.py score WORK            per-signal detection / false-alarm table

Sets: clean (saved once, q85) | resave1 (clean re-saved at q85) | resave2 (re-saved again at q75)
      | edits: digit_obvious, digit_subtle (digits drawn over a word, size/baseline off),
        copy_move (220x140 patch copied elsewhere), requality (260x180 patch from a q45 copy
        pasted at an offset that is not a multiple of 8), noise (gaussian sigma 7 in 240x160).
Every edited page is re-saved at q85 at the end. Edits are placed on text-bearing areas.
"""
import sys, os, json, glob, random, subprocess, io
import numpy as np
from PIL import Image, ImageDraw, ImageFont

Q = 85
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"
SIGS = ["jpeg_grid", "jpeg_double", "noise", "line_size", "copy_move"]
KINDS = ("digit_obvious", "digit_subtle", "copy_move", "requality", "noise")


def save(img, path, q=Q):
    img.save(path, "JPEG", quality=q)


def reload(path):
    return Image.open(path).convert("RGB")


def dark(g):
    """ink mask with a per-page threshold (some pages are dark or coloured)"""
    lo, mid = np.percentile(g, 3), np.percentile(g, 60)
    return g < (lo + (mid - lo) * 0.45) if mid - lo > 40 else g < 0


def text_boxes(a, w, h, rnd, n=60):
    """random boxes (w x h) with 4-25 % dark pixels, i.e. on text"""
    g = a.mean(axis=2)
    dk = dark(g)
    H, W = g.shape
    out = []
    for _ in range(4000):
        x = rnd.randrange(60, W - w - 60)
        y = rnd.randrange(60, H - h - 60)
        f = dk[y:y + h, x:x + w].mean()
        if 0.04 < f < 0.25:
            out.append((x, y, w, h))
            if len(out) >= n:
                break
    return out


def line_band(a, rnd):
    """(y0, y1, x0, x1) of a text line: rows with ink, height 15-90 px, wide enough"""
    g = a.mean(axis=2)
    dk = dark(g)
    r = dk.sum(axis=1)
    H = len(r)
    for thr, minw in ((3, 600), (1, 400), (0, 300)):
        y = int(H * 0.15)
        bands = []
        while y < int(H * 0.85):
            if r[y] > thr:
                y0 = y
                while y < H and r[y] > thr:
                    y += 1
                if 15 <= y - y0 <= 90:
                    cols = np.where(dk[y0:y].any(axis=0))[0]
                    if len(cols) > minw and cols[-1] - cols[0] > 700:
                        bands.append((y0, y, int(cols[0]), int(cols[-1])))
            y += 1
        if bands:
            return rnd.choice(bands)
    x, y, w, h = text_boxes(a, 900, 45, rnd)[0]  # dark or textured page: any inked strip
    return (y, y + h, x, x + w)


def digit_edit(img, rnd, big):
    a = np.array(img)
    y0, y1, xa, xb = line_band(a, rnd)
    x0 = rnd.randrange(xa + 100, xb - 300)
    bg = tuple(int(v) for v in np.median(a[max(0, y0 - 8):y0 - 2, xa:xb].reshape(-1, 3), axis=0))
    d = ImageDraw.Draw(img)
    lh = y1 - y0
    size = int(lh * (1.35 if big else 1.08))
    dy = 6 if big else 2
    f = ImageFont.truetype(FONT, size)
    txt = "%d" % rnd.randrange(1000, 9999)
    bw = int(d.textlength(txt, font=f))
    d.rectangle([x0 - 4, y0 - 4, x0 + bw + 4, y1 + 6], fill=bg)
    d.text((x0, y0 - int(size * 0.18) + dy), txt, fill=(20, 20, 20), font=f)
    return (x0 - 4, y0 - 4, bw + 8, lh + 10)


def build(imgdir, work):
    os.makedirs(work, exist_ok=True)
    truth = {}
    for p in sorted(glob.glob(imgdir + "/*.png")):
        name = os.path.basename(p)[:-4]
        rnd = random.Random(sum(map(ord, name)) * 7919)
        base = Image.open(p).convert("RGB")
        cp = f"{work}/{name}.clean.jpg"
        save(base, cp)
        save(reload(cp), f"{work}/{name}.resave1.jpg")
        save(reload(f"{work}/{name}.resave1.jpg"), f"{work}/{name}.resave2.jpg", 75)
        for k in ("clean", "resave1", "resave2"):
            truth[f"{name}.{k}.jpg"] = {"set": k, "boxes": []}
        src = reload(cp)
        a = np.array(src)
        low = None
        for kind in KINDS:
            img = src.copy()
            boxes = []
            if kind.startswith("digit"):
                boxes.append(digit_edit(img, rnd, kind == "digit_obvious"))
            elif kind == "copy_move":
                cand = text_boxes(a, 220, 140, rnd)
                sx, sy, w, h = cand[0]
                for tx, ty, _, _ in cand[1:]:
                    if abs(tx - sx) > 300 or abs(ty - sy) > 300:
                        break
                img.paste(src.crop((sx, sy, sx + w, sy + h)), (tx, ty))
                boxes = [(tx, ty, w, h), (sx, sy, w, h)]
            elif kind == "requality":
                if low is None:
                    b = io.BytesIO()
                    src.save(b, "JPEG", quality=45)
                    low = Image.open(b).convert("RGB")
                cand = text_boxes(a, 260, 180, rnd)
                tx, ty, w, h = cand[0]
                tx += (3 - tx % 8) % 8
                ty += (5 - ty % 8) % 8  # offsets 3 and 5 modulo 8
                sx, sy = cand[1][0], cand[1][1]
                img.paste(low.crop((sx, sy, sx + w, sy + h)), (tx, ty))
                boxes = [(tx, ty, w, h)]
            else:
                tx, ty, w, h = text_boxes(a, 240, 160, rnd)[0]
                arr = np.array(img).astype(np.float32)
                arr[ty:ty + h, tx:tx + w] += np.random.RandomState(1).normal(0, 7, (h, w, 3))
                img = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8))
                boxes = [(tx, ty, w, h)]
            fn = f"{name}.{kind}.jpg"
            save(img, f"{work}/{fn}")
            truth[fn] = {"set": kind, "boxes": boxes}
    json.dump(truth, open(work + "/truth.json", "w"), indent=1)
    print(len(truth), "files")


def run(work, binp):
    truth = json.load(open(work + "/truth.json"))
    os.makedirs(work + "/out", exist_ok=True)
    for fn, t in sorted(truth.items()):
        o = f"{work}/out/{fn}.json"
        if not os.path.exists(o):
            subprocess.run([binp, f"{work}/{fn}", "--integrity-hints", o, "--integrity-signals", "jpeg_grid,jpeg_double,noise"], check=True, capture_output=True)
        o3 = f"{work}/out/{fn}.cm.json"
        if not os.path.exists(o3):
            subprocess.run([binp, f"{work}/{fn}", "--integrity-hints", o3, "--integrity-signals", "copy_move"], check=True, capture_output=True)
        # line_size needs OCR: only run where it is scored (unedited sets and the digit edits)
        if t["set"] in ("clean", "resave1", "resave2", "digit_obvious", "digit_subtle"):
            o2 = f"{work}/out/{fn}.line.json"
            if not os.path.exists(o2):
                subprocess.run([binp, f"{work}/{fn}", "--ocr", "--engine", "tesseract", "--lang", "ron", "--integrity-hints", o2, "--integrity-signals", "line_size"], check=True, capture_output=True)


def hits(work, fn):
    h = []
    for suf in (".json", ".line.json", ".cm.json"):
        p = f"{work}/out/{fn}{suf}"
        if os.path.exists(p):
            for pg in json.load(open(p))["pages"]:
                h += [x for x in pg["to_check"] if not (suf == ".json" and x["signal"] == "copy_move")]
    return h


def overlap(b, t):
    return b[0] < t[0] + t[2] and t[0] < b[0] + b[2] and b[1] < t[1] + t[3] and t[1] < b[1] + b[3]


def score(work):
    truth = json.load(open(work + "/truth.json"))
    sets = {}
    for fn, t in truth.items():
        sets.setdefault(t["set"], []).append(fn)
    print("signal      | pages with any hint: clean, resave1, resave2 | detection on edits (a hint overlaps the edit box)")
    for sig in SIGS:
        fp = []
        for s in ("clean", "resave1", "resave2"):
            fns = sets[s]
            n = sum(1 for fn in fns if any(h["signal"] == sig for h in hits(work, fn)))
            fp.append(f"{n}/{len(fns)}")
        det = []
        for kind in KINDS:
            if sig == "line_size" and not kind.startswith("digit"):
                continue
            n = 0
            fns = sets[kind]
            for fn in fns:
                hs = [h for h in hits(work, fn) if h["signal"] == sig]
                if sig == "jpeg_double":
                    n += bool(hs)
                else:
                    n += any(overlap(h["box"], b) for h in hs for b in truth[fn]["boxes"])
            det.append(f"{kind} {n}/{len(fns)}")
        print(f"{sig:11} | {', '.join(fp):28} | " + ", ".join(det))
    for sig in SIGS:
        for s in ("clean", "resave1", "resave2"):
            c = [sum(1 for h in hits(work, fn) if h["signal"] == sig) for fn in sets[s]]
            print(f"  hints per page, {sig}, {s}: {c}")


if __name__ == "__main__":
    c = sys.argv[1]
    if c == "gen":
        build(sys.argv[2], sys.argv[3])
    elif c == "run":
        run(sys.argv[2], sys.argv[3])
    elif c == "score":
        score(sys.argv[2])
