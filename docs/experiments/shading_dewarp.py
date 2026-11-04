#!/usr/bin/env python3
"""Experiment (ROADMAP "shape from shading" dewarp hint). SYNTHETIC ground truth only.

Hypothesis: near a book spine the page leans away from the camera, so (a) it is darker and
(b) it is horizontally foreshortened.  For a cylinder page lit/viewed so that I = cos(theta)
(theta = surface slope), dx/du = cos(theta) = I: the column-wise illumination profile IS the
horizontal compression.  Estimator: per-column 85th-percentile grey (paper level between the
glyphs), normalised by the right-hand quarter (assumed flat), smoothed; then u(x) = integral
of dx / I(x), anchored at the flat side; the page is resampled horizontally.

Test: render a known flat page (scripts/ocr-bench.py text and font), bend it with a KNOWN
cylinder model + multiplicative shading, add blur/noise, run the estimator, and compare
  * median horizontal error (px) of glyph centres (components of the flat render, mapped through
    the true warp and through the estimate), without vs with the correction;
  * Tesseract CER (ron / eng) against the known text on warped vs corrected image.
Cases deliberately include model mismatches (off-axis light + ambient, global vignette).

    python3 docs/experiments/shading_dewarp.py [--out DIR]
"""
import os, subprocess, sys, tempfile
import numpy as np, cv2
from PIL import Image, ImageDraw, ImageFont
from rapidfuzz.distance import Levenshtein
from scipy import ndimage

HERE = os.path.dirname(os.path.abspath(__file__))
src = open(os.path.join(HERE, "..", "..", "scripts", "ocr-bench.py")).read()
ns = {"os": os, "Image": Image, "ImageDraw": ImageDraw, "ImageFont": ImageFont}
exec(src[src.index('FONT = '):src.index("def render")], ns)
TEXT, FONT = ns["TEXT"], ns["FONT"]
OUT = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else tempfile.mkdtemp(prefix="shade-")
os.makedirs(OUT, exist_ok=True)


def render_flat(text):
    W, H = 1700, 2200
    font = ImageFont.truetype(FONT, 46)
    img = Image.new("L", (W, H), 245)
    d = ImageDraw.Draw(img)
    y = 190
    for para in text.split("\n"):
        line = ""
        for w in para.split():
            t = (line + " " + w).strip()
            if d.textlength(t, font=font) > W - 340:
                d.text((170, y), line, font=font, fill=20); y += 72; line = w
            else:
                line = t
        d.text((170, y), line, font=font, fill=20); y += 110
    return np.asarray(img, np.float32)


def true_warp(W, theta0, L):
    """flat u -> image x for a right-hand page, spine at u=0 (left); slope theta(u)."""
    u = np.arange(W, dtype=np.float64)
    th = np.where(u < L, theta0 * (1 - u / L), 0.0)
    x = np.concatenate([[0], np.cumsum(np.cos(th))[:-1]])
    return u, th, x


def resample_cols(img, xsrc):
    H = img.shape[0]
    return cv2.remap(img, np.tile(xsrc[None, :], (H, 1)).astype(np.float32),
                     np.tile(np.arange(H, dtype=np.float32)[:, None], (1, len(xsrc))),
                     cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)


def make_case(flat, theta0, L, phi, amb, vignette, seed):
    H, W = flat.shape
    u, th, xu = true_warp(W, theta0, L)
    shade = amb + (1 - amb) * np.cos(th - phi) / np.cos(phi)       # 1 on the flat part
    if vignette:
        shade = shade * (1 - 0.22 * u / W)
    lit = (flat * shade[None, :]).astype(np.float32)
    Wi = int(np.ceil(xu[-1])) + 1
    u_of_x = np.interp(np.arange(Wi, dtype=np.float64), xu, u)      # xu is monotone
    img = resample_cols(lit, u_of_x)
    rng = np.random.default_rng(seed)
    img = cv2.GaussianBlur(img, (0, 0), 0.9) + rng.normal(0, 3.0, img.shape)
    return np.clip(img, 0, 255).astype(np.uint8), u, xu


def flatten(img, smooth=41, flat_frac=0.25):
    """divide by the column illumination profile (so CER differences are geometry, not darkness)."""
    prof = ndimage.uniform_filter1d(np.percentile(img.astype(np.float32), 85, axis=0), smooth, mode="nearest")
    I = np.clip(prof / np.median(prof[int(len(prof) * (1 - flat_frac)):]), 0.3, 1.0)
    return np.clip(img / I[None, :], 0, 255).astype(np.uint8)


def estimate(img, smooth=41, flat_frac=0.25):
    """flat-coordinate offset per image column: dist[x] = sum_{x'>=x} du/dx' (right edge = flat)."""
    prof = np.percentile(img.astype(np.float32), 85, axis=0)
    prof = ndimage.uniform_filter1d(prof, smooth, mode="nearest")
    ref = np.median(prof[int(len(prof) * (1 - flat_frac)):])
    I = np.clip(prof / ref, 0.3, 1.0)
    return np.cumsum((1.0 / I)[::-1])[::-1]


def glyph_errors(flat, u_true, xu, u_est_of_x, zone=None):
    """glyph-component centres of the FLAT render -> true image x -> estimated flat coordinate;
    error = estimate - truth (pixels of the flat page)."""
    lab, n = ndimage.label(flat < 128)
    cx = np.array(ndimage.center_of_mass(np.ones_like(lab), lab, range(1, n + 1)))[:, 1]
    sl = ndimage.find_objects(lab)
    area = np.array([(s[0].stop - s[0].start) * (s[1].stop - s[1].start) for s in sl])
    cx = cx[area > 60]
    if zone is not None:
        cx = cx[cx < zone]                                          # glyphs inside the bent zone
    x_img = np.interp(cx, u_true, xu)
    return np.interp(x_img, np.arange(len(u_est_of_x)), u_est_of_x) - cx


def tess(img, lang):
    p = os.path.join(OUT, "t.png")
    Image.fromarray(img).save(p, dpi=(300, 300))
    return subprocess.run(["tesseract", p, "stdout", "-l", lang, "--psm", "3"],
                          capture_output=True, text=True).stdout


def cer(ref, hyp):
    n = lambda s: " ".join(s.split())
    return 100 * Levenshtein.distance(n(ref), n(hyp)) / max(1, len(n(ref)))


CASES = [  # name, theta0 (slope at the spine, rad), L (bent width, px), phi, ambient, vignette
    ("A mild (0.7 rad, L 700), ideal Lambertian", 0.7, 700, 0.0, 0.0, False),
    ("B strong (1.0, L 900), ideal Lambertian", 1.0, 900, 0.0, 0.0, False),
    ("C strong, off-axis light 0.3 + ambient 0.3", 1.0, 900, 0.3, 0.3, False),
    ("D strong, ideal + global vignette (confound)", 1.0, 900, 0.0, 0.0, True),
    ("E extreme (1.3, L 1000), ideal Lambertian", 1.3, 1000, 0.0, 0.0, False),
    ("F extreme, off-axis 0.3 + ambient 0.3", 1.3, 1000, 0.3, 0.3, False),
]

if __name__ == "__main__":
    rows = []
    for lang in ("ron", "eng"):
        text = TEXT[lang][1]
        flat = render_flat(text)
        print(f"[{lang}] flat page CER {cer(text, tess(flat.astype(np.uint8), lang)):.2f}%")
        for name, th0, L, phi, amb, vig in CASES:
            img, u, xu = make_case(flat, th0, L, phi, amb, vig, hash((lang, name)) & 0xFFFF)
            Wi = img.shape[1]
            dist = estimate(img)
            uest = u[-1] - (dist - dist[-1])                        # right edge column -> u[-1]
            base = np.arange(Wi, dtype=np.float64) + (u[-1] - xu[-1])   # no correction, same anchor
            e_base, e_corr = (glyph_errors(flat, u, xu, m, zone=L) for m in (base, uest))
            v = np.arange(uest[0], uest[-1] + 1)
            fl = flatten(img)
            corr = resample_cols(fl, np.interp(v, uest, np.arange(Wi)))
            Image.fromarray(img).save(os.path.join(OUT, f"{lang}_{name[0]}_warped.png"))
            Image.fromarray(corr).save(os.path.join(OUT, f"{lang}_{name[0]}_corrected.png"))
            mb, mc = np.median(np.abs(e_base)), np.median(np.abs(e_corr))
            cw, cf, cc = (cer(text, tess(x, lang)) for x in (img, fl, corr))
            rows.append((lang, name, mb, mc, 100 * (1 - mc / mb), cw, cf, cc))
            print(f"[{lang}] {name}: median |err| {mb:.1f} -> {mc:.1f} px, p90 "
                  f"{np.percentile(np.abs(e_base), 90):.1f} -> {np.percentile(np.abs(e_corr), 90):.1f}, "
                  f"CER {cw:.2f}% -> {cc:.2f}%")
    print("\n| lang | case | median err in bent zone, warped (px) | after (px) | reduction | CER warped | CER flatten only | CER flatten+dewarp |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        print("| %s | %s | %.1f | %.1f | %.0f%% | %.2f%% | %.2f%% | %.2f%% |" % r)
    print("files:", OUT, file=sys.stderr)
