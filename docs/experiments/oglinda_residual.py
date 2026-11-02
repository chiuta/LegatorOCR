#!/usr/bin/env python3
"""Roadmap 4.7 gate: does the residual between a scanned word and the same word re-typeset in a
font separate correct from wrong OCR words?

For every Tesseract word (TSV) of each page: crop the word from the scan, render the recognised text
in a serif font at the box size, compare binarised crop and rendering (1 - best normalised
correlation over small shifts), and label the word correct when it lies in an 'equal' block of an
alignment against the page's reference text. Reports AUC of the residual, of the Tesseract
confidence, and of both combined (logistic regression, leave-one-page-out).

usage: oglinda_residual.py IMG_DIR REF_DIR [FONT.ttf]
"""
import csv, difflib, glob, io, os, subprocess, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont

img_dir, ref_dir = sys.argv[1:3]
font_path = sys.argv[3] if len(sys.argv) > 3 else "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"


def tsv_words(path):
    out = subprocess.run(["tesseract", path, "-", "-l", "ron", "--psm", "3", "tsv"], capture_output=True, text=True).stdout
    rows = list(csv.DictReader(io.StringIO(out), delimiter="\t", quoting=csv.QUOTE_NONE))
    return [r for r in rows if r.get("level") == "5" and r["text"].strip() and float(r["conf"]) >= 0]


def norm(w):
    return "".join(c for c in w.lower() if c.isalnum())


def render(text, w, h):
    f = ImageFont.truetype(font_path, max(8, int(h * 0.8)))
    im = Image.new("L", (max(w * 2, 40), h * 2), 255)
    d = ImageDraw.Draw(im)
    d.text((2, 0), text, font=f, fill=0)
    a = np.asarray(im) < 128
    ys, xs = np.where(a)
    if len(xs) == 0:
        return None
    a = a[ys.min(): ys.max() + 1, xs.min(): xs.max() + 1]
    return np.asarray(Image.fromarray((a * 255).astype(np.uint8)).resize((w, h), Image.BILINEAR)) > 100


def residual(crop_dark, text):
    h, w = crop_dark.shape
    r = render(text, w, h)
    if r is None:
        return None
    best = 1.0
    for dy in (-1, 0, 1):
        for dx in (-2, -1, 0, 1, 2):
            s = np.roll(np.roll(r, dy, 0), dx, 1).astype(float)
            c = crop_dark.astype(float)
            a, b = s - s.mean(), c - c.mean()
            den = np.sqrt((a * a).sum() * (b * b).sum())
            if den > 0:
                best = min(best, 1 - (a * b).sum() / den)
    return best


def auc(score, label):
    # probability that a random WRONG word has a higher score than a random correct one
    s, l = np.asarray(score), np.asarray(label)
    pos, neg = s[l == 0], s[l == 1]
    if len(pos) == 0 or len(neg) == 0:
        return float("nan")
    ranks = np.argsort(np.argsort(np.concatenate([pos, neg]))) + 1
    return (ranks[: len(pos)].sum() - len(pos) * (len(pos) + 1) / 2) / (len(pos) * len(neg))


pages = []
for ip in sorted(glob.glob(os.path.join(img_dir, "*.png"))):
    stem = os.path.splitext(os.path.basename(ip))[0]
    rp = os.path.join(ref_dir, stem + ".txt")
    if not os.path.exists(rp):
        continue
    ref = [norm(w) for w in open(rp, encoding="utf-8").read().split() if norm(w)]
    words = tsv_words(ip)
    hyp = [norm(r["text"]) for r in words]
    sm = difflib.SequenceMatcher(None, hyp, ref, autojunk=False)
    ok = np.zeros(len(hyp), bool)
    for tag, i1, i2, _, _ in sm.get_opcodes():
        if tag == "equal":
            ok[i1:i2] = True
    gray = np.asarray(Image.open(ip).convert("L")).astype(float)
    thr = gray.mean() * 0.75
    rows = []
    for r, good, h_ in zip(words, ok, hyp):
        if len(h_) < 3:
            continue
        x, y, w, h = (int(r[k]) for k in ("left", "top", "width", "height"))
        if w < 12 or h < 8 or h > 120:
            continue
        crop = gray[y: y + h, x: x + w] < thr
        if crop.sum() < 10:
            continue
        res = residual(crop, r["text"])
        if res is None:
            continue
        rows.append((res, float(r["conf"]), int(good)))
    pages.append((stem, np.array(rows)))
    print(stem, len(rows), "words,", int(sum(1 - np.array([r[2] for r in rows]))), "wrong", flush=True)

allr = np.vstack([p for _, p in pages])
print(f"\nwords {len(allr)}, wrong {int((allr[:, 2] == 0).sum())}")
print(f"AUC residual           {auc(allr[:, 0], allr[:, 2]):.3f}   (0.5 = no information)")
print(f"AUC 100 - confidence   {auc(100 - allr[:, 1], allr[:, 2]):.3f}")

# simple combination without fitting: per-page z-scores of residual and (100 - confidence)
def z(v):
    return (v - v.mean()) / (v.std() + 1e-9)

zs, zl = [], []
for _, p in pages:
    if len(p) < 5:
        continue
    zs.extend(z(p[:, 0]) + z(100 - p[:, 1]))
    zl.extend(p[:, 2])
zr, zc, zll = [], [], []
for _, p in pages:
    if len(p) < 5:
        continue
    zr.extend(z(p[:, 0]))
    zc.extend(z(100 - p[:, 1]))
    zll.extend(p[:, 2])
print(f"per-page z: residual {auc(zr, zll):.3f}, 100-confidence {auc(zc, zll):.3f}, sum of both {auc(zs, zl):.3f}")

# combined: logistic regression trained on the other pages
def fit(X, y, it=400, lr=0.1):
    X = np.c_[np.ones(len(X)), (X - X.mean(0)) / (X.std(0) + 1e-9)]
    w = np.zeros(X.shape[1])
    for _ in range(it):
        p = 1 / (1 + np.exp(-X @ w))
        w -= lr * X.T @ (p - y) / len(y)
    return w


def predict(X, w, mu, sd):
    return np.c_[np.ones(len(X)), (X - mu) / (sd + 1e-9)] @ w


sc_comb, sc_conf, labels = [], [], []
for k in range(len(pages)):
    tr = np.vstack([p for j, (_, p) in enumerate(pages) if j != k])
    te = pages[k][1]
    if len(te) == 0:
        continue
    for cols, bucket in (([0, 1], sc_comb), ([1], sc_conf)):
        w = fit(tr[:, cols], 1 - tr[:, 2])
        bucket.extend(predict(te[:, cols], w, tr[:, cols].mean(0), tr[:, cols].std(0)))
    labels.extend(te[:, 2])
print(f"AUC confidence only (LOO page)        {auc(sc_conf, labels):.3f}")
print(f"AUC confidence + residual (LOO page)  {auc(sc_comb, labels):.3f}")
