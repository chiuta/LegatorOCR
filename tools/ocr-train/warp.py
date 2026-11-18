#!/usr/bin/env python3
"""Bend a flat test page like a curled book page and write it with its (unchanged) reference.

    python3 warp.py IN.png OUT.png AMPLITUDE [--tilt DEG]

AMPLITUDE is the height of the arch as a fraction of the page height (0.03 = the middle of every text line is lifted 3 % of the
page height, relative to the page edges). Lines become parabolas (`y_src = y + A*H*arch(x)*(0.6+0.4*y/H)`), the strongest at the
bottom of the page, as when a thick book's page lifts from the platen. This is one simple model of curl, not a physical one
(no perspective, no shading, no horizontal squeeze); it exists to compare straightening methods on a known geometry.
The reference text file is copied next to the output by the caller (the text does not change).
Needs numpy and opencv (installed with `pip install page-dewarp`).
"""
import sys
import numpy as np
import cv2

src, dst, amp = sys.argv[1], sys.argv[2], float(sys.argv[3])
tilt = float(sys.argv[sys.argv.index("--tilt") + 1]) if "--tilt" in sys.argv else 0.0
im = cv2.imread(src, cv2.IMREAD_UNCHANGED)
h, w = im.shape[:2]
yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
arch = 1.0 - ((xx / w - 0.5) * 2.0) ** 2            # 0 at the side edges, 1 in the middle
ysrc = yy + amp * h * arch * (0.6 + 0.4 * yy / h)
out = cv2.remap(im, xx, ysrc, cv2.INTER_CUBIC, borderMode=cv2.BORDER_CONSTANT, borderValue=(255,) * (im.shape[2] if im.ndim == 3 else 1))
if tilt:
    m = cv2.getRotationMatrix2D((w / 2, h / 2), tilt, 1.0)
    out = cv2.warpAffine(out, m, (w, h), flags=cv2.INTER_CUBIC, borderValue=(255,) * (im.shape[2] if im.ndim == 3 else 1))
cv2.imwrite(dst, out, [cv2.IMWRITE_PNG_COMPRESSION, 3])
