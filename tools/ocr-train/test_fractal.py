"""Sanity test for fractal.texturize: same size, deterministic per seed, keeps text darker than paper."""
import numpy as np
from PIL import Image, ImageDraw

from fractal import texturize


def page():
    im = Image.new("L", (400, 300), 255)
    ImageDraw.Draw(im).rectangle((40, 100, 360, 130), fill=0)
    return im


def test_shape_and_determinism():
    a = np.asarray(texturize(page(), seed=1))
    b = np.asarray(texturize(page(), seed=1))
    c = np.asarray(texturize(page(), seed=2))
    assert a.shape == (300, 400)
    assert (a == b).all() and (a != c).any()


def test_ink_stays_readable_at_high_strength():
    out = np.asarray(texturize(page(), seed=5, strength=3.0)).astype(float)
    ink, paper = out[105:125, 50:350].mean(), out[:60].mean()
    assert paper - ink > 100


def test_tint_is_rgb():
    assert texturize(page(), seed=1, tint=True).mode == "RGB"
