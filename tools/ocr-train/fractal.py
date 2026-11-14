"""Fractal textures for synthetic test pages: paper, ink and the other side of the sheet.

Everything comes from fractional Brownian motion (several octaves of smooth noise, each finer
and fainter than the last), so the detail is there at every scale: the page mottles over
centimetres, the paper fibres run over millimetres and the edge of every letter wobbles over a
few pixels. Used by `makepage.py` (environment FRACTAL=1, strength FRACTAL_STRENGTH, default 1.0;
FRACTAL_TINT=1 gives a warm colour page).

    from fractal import texturize
    out = texturize(page, seed=7, strength=1.0)      # page: PIL "L" image, dark text on light

Only numpy and Pillow; the noise is made by resizing small random grids (bicubic), which is
fast enough for an A4 page at 300 dpi. The ground truth text of the page does not change: the
texture only makes the pixels harder to read, as real scans are.
"""
import numpy as np
from PIL import Image, ImageFilter


def fbm(shape, rng, octaves=8, base=4, persistence=0.55, stretch=(1.0, 1.0)):
    """Fractional Brownian motion in [0, 1]. `base` is the number of grid cells along the longer
    side at the first octave and doubles each octave; `stretch` (y, x) elongates the features
    (fibres: x much larger than y)."""
    h, w = shape
    acc = np.zeros((h, w), np.float32)
    amp, tot = 1.0, 0.0
    for o in range(octaves):
        n = base * (2 ** o)
        gw = max(2, int(round(n * w / max(h, w) / stretch[1])) + 2)
        gh = max(2, int(round(n * h / max(h, w) / stretch[0])) + 2)
        if gw > w or gh > h:
            break
        g = rng.random((gh, gw)).astype(np.float32)
        im = Image.fromarray(g, mode="F").resize((w, h), Image.BICUBIC)
        acc += amp * np.asarray(im)
        tot += amp
        amp *= persistence
    acc /= max(tot, 1e-6)
    lo, hi = float(acc.min()), float(acc.max())
    return (acc - lo) / max(hi - lo, 1e-6)


def _smooth(x, a, b):
    t = np.clip((x - a) / max(b - a, 1e-6), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def texturize(page, seed=0, strength=1.0, tint=False):
    """Return the page with fractal paper, ink and show-through; same size as `page`."""
    rng = np.random.default_rng(seed)
    s = float(strength)
    si = min(s, 1.0)          # ink terms saturate: stronger settings only age the paper
    s = min(s, 2.0)
    g = np.asarray(page.convert("L"), np.float32) / 255.0
    h, w = g.shape
    ink = np.clip((g.max() - g) / max(g.max() - g.min(), 1e-6), 0.0, 1.0)

    # --- ink: ragged edges (threshold wobbles with fine fractal noise), uneven density,
    #     and starved spots where the ink did not take
    soft = np.asarray(Image.fromarray((ink * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.1)),
                      np.float32) / 255.0
    edge = fbm((h, w), rng, octaves=7, base=96, persistence=0.62)
    rough = _smooth(soft + (edge - 0.5) * 0.55 * si, 0.38, 0.62)
    density = 0.78 + 0.45 * fbm((h, w), rng, octaves=6, base=24, persistence=0.6)
    starve = _smooth(fbm((h, w), rng, octaves=7, base=64, persistence=0.65), 0.72, 0.9)
    ink_t = np.clip(rough * density * (1.0 - 0.35 * si * starve), 0.0, 1.0)

    # --- paper: low-frequency mottle, fibres, stains, foxing spots, grain
    mottle = fbm((h, w), rng, octaves=7, base=3, persistence=0.58)
    fibres = fbm((h, w), rng, octaves=5, base=40, persistence=0.6, stretch=(1.0, 14.0))
    fibres2 = fbm((h, w), rng, octaves=5, base=40, persistence=0.6, stretch=(14.0, 1.0))
    stain = _smooth(fbm((h, w), rng, octaves=8, base=2, persistence=0.6), 0.55, 0.95)
    fox_f = fbm((h, w), rng, octaves=8, base=30, persistence=0.7)
    fox = _smooth(fox_f, 0.80, 0.93) * (fbm((h, w), rng, octaves=3, base=3) > 0.5)
    grain = rng.normal(0.0, 1.0, (h, w)).astype(np.float32)
    paper = (0.925 + 0.09 * (mottle - 0.5) * s
             + 0.040 * (fibres - 0.5) * s + 0.024 * (fibres2 - 0.5) * s
             - 0.10 * stain * s - 0.06 * fox * s + 0.006 * grain * s)

    # --- the other side of the sheet shows through, faint, mirrored and blurred
    back = np.asarray(Image.fromarray((ink * 255).astype(np.uint8)).transpose(Image.FLIP_LEFT_RIGHT)
                      .filter(ImageFilter.GaussianBlur(2.2)), np.float32) / 255.0
    paper = paper - 0.07 * s * back

    # --- soft light fall-off over the whole sheet (low-order fractal, one big lobe)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    vign = 1.0 - 0.08 * s * (((xx / w - 0.5) ** 2 + (yy / h - 0.5) ** 2) * 2.0)
    paper = np.clip(paper * vign, 0.0, 1.0)

    out = paper * (1.0 - 0.93 * ink_t) + 0.02 * grain * s * 0.5
    out = np.clip(out, 0.0, 1.0)
    if tint:
        r = out * (1.0 + 0.03 * s)
        gch = out
        b = out * (1.0 - 0.10 * s * (0.4 + 0.6 * stain + 0.5 * fox))
        rgb = np.stack([r, gch, b], axis=-1)
        return Image.fromarray((np.clip(rgb, 0, 1) * 255).astype(np.uint8), mode="RGB")
    return Image.fromarray((out * 255).astype(np.uint8), mode="L")
