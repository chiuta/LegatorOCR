#!/usr/bin/env python3
"""Bridge: a PP-OCR-layout OR a docTR/OnnxTR-layout ONNX detector + recogniser as an external LegatorOCR engine.

No weights are bundled and none are downloaded; you point the bridge at files you obtained yourself
(see README, "ONNX OCR bridge"). Needs only: onnxruntime, numpy, Pillow.

Use:
  export LEGATOROCR_ONNX_DET=/models/det.onnx LEGATOROCR_ONNX_REC=/models/rec.onnx LEGATOROCR_ONNX_DICT=/models/dict.txt
  legatorocr scan.png --pdf out.pdf --ocr --engine tesseract --tesseract scripts/engine-onnx.py

Model families (--onnx-family / LEGATOROCR_ONNX_FAMILY):
  ppocr (default)  PaddleOCR layout: DB detector on the whole page, each detected box is a TEXT LINE, the CTC
                   recogniser reads it and words are split at the space class.
  doctr            docTR / OnnxTR layout (ROADMAP 4.33, e.g. the Romanian-head recognisers of 4.32): the DB detector
                   (1024x1024 input, aspect kept, symmetric black padding, docTR mean/std, logits -> sigmoid) finds
                   WORDS; each word crop goes through the CRNN (32x128, aspect kept, bottom-right black padding, docTR
                   mean/std, crops wider than 8:1 split with overlap and merged), CTC greedy with blank = LAST class.
                   --onnx-dict then holds the vocabulary as ONE string (romanian_head/vocab.txt), not one char per line.
                   Words are grouped into lines and columns by this script (recursive XY cut), so the TSV is in reading order.
                   scripts/get-romanian-doctr-models.sh downloads the models and prints the command line.

Protocol (docs/examples/external-ocr-engine.py): called as `PROGRAM stdin stdout -l LANG --psm N [--dpi D] tsv`
with a PNG on stdin; prints Tesseract TSV (level 5 rows) on stdout. `--version` and `--list-langs` are answered.
The orientation probe (`-l osd`) exits 1, which the CLI treats as "unknown".

Settings (environment variable, or the same as a flag placed anywhere on the command line):
  LEGATOROCR_ONNX_DET        --onnx-det PATH        detector (DB) model; without it the whole page is one line
  LEGATOROCR_ONNX_REC        --onnx-rec PATH        recogniser (CTC) model (required)
  LEGATOROCR_ONNX_DICT       --onnx-dict PATH       one character per line; the CTC blank is class 0, so line i is class i+1
  LEGATOROCR_ONNX_SIZE       --onnx-size N          detector: longest page side, rounded to a multiple of 32 (default 960)
  LEGATOROCR_ONNX_REC_HEIGHT --onnx-rec-height N    recogniser input height, 48 (PP-OCRv3+ default) or 32 (older) (default 48)
  LEGATOROCR_ONNX_THRESH     --onnx-thresh X        detector probability threshold (default 0.3)
  LEGATOROCR_ONNX_UNCLIP     --onnx-unclip X        box growth ratio (default 1.5)
  LEGATOROCR_ONNX_SPACE      --onnx-space 0|1       append a space class after the dictionary, as PaddleOCR does (default 1)
  LEGATOROCR_ONNX_FAMILY     --onnx-family F        ppocr (default) or doctr, see above (doctr ignores --onnx-size and --onnx-rec-height)

Limits, stated plainly: boxes are axis-aligned (no rotated text, no direction classifier); words are split at
the space class using the CTC time step, so word boxes are approximate; confidence is the mean peak
probability of the emitted characters x 100. Accuracy with real PP-OCR models is UNMEASURED.
A model that invents text is not detectable here: keep a person in the loop.
"""
import io
import os
import sys

OPTS = {
    "det": ("LEGATOROCR_ONNX_DET", "--onnx-det", ""),
    "rec": ("LEGATOROCR_ONNX_REC", "--onnx-rec", ""),
    "dict": ("LEGATOROCR_ONNX_DICT", "--onnx-dict", ""),
    "size": ("LEGATOROCR_ONNX_SIZE", "--onnx-size", "960"),
    "rec_height": ("LEGATOROCR_ONNX_REC_HEIGHT", "--onnx-rec-height", "48"),
    "thresh": ("LEGATOROCR_ONNX_THRESH", "--onnx-thresh", "0.3"),
    "unclip": ("LEGATOROCR_ONNX_UNCLIP", "--onnx-unclip", "1.5"),
    "space": ("LEGATOROCR_ONNX_SPACE", "--onnx-space", "1"),
    "family": ("LEGATOROCR_ONNX_FAMILY", "--onnx-family", "ppocr"),
}
HEADER = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"


def settings(argv):
    """Returns (settings dict, remaining arguments). Flags win over environment variables."""
    s = {k: os.environ.get(v[0], v[2]) for k, v in OPTS.items()}
    rest, i = [], 0
    flags = {v[1]: k for k, v in OPTS.items()}
    while i < len(argv):
        if argv[i] in flags and i + 1 < len(argv):
            s[flags[argv[i]]] = argv[i + 1]
            i += 2
        else:
            rest.append(argv[i])
            i += 1
    return s, rest


def load_dict(path, space):
    with open(path, encoding="utf-8") as f:
        chars = [ln.rstrip("\r\n") for ln in f]
    while chars and chars[-1] == "":  # trailing empty lines; an interior empty line stays a class
        chars.pop()
    if space not in ("0", "false", "no"):
        chars.append(" ")
    return ["<blank>"] + chars  # class 0 = CTC blank


def sessions(s):
    import onnxruntime as ort
    so = ort.SessionOptions()
    so.log_severity_level = 3
    prov = ["CPUExecutionProvider"]
    det = ort.InferenceSession(s["det"], so, providers=prov) if s["det"] else None
    rec = ort.InferenceSession(s["rec"], so, providers=prov)
    return det, rec


def detect(det, img, s):
    """DB text detector. Returns boxes (x0, y0, x1, y1) in image pixels, sorted into reading order."""
    import numpy as np
    W, H = img.size
    side = max(32, int(s["size"]))
    scale = min(1.0, side / max(W, H)) if max(W, H) > side else 1.0
    nw = max(32, int(round(W * scale / 32.0)) * 32)
    nh = max(32, int(round(H * scale / 32.0)) * 32)
    from PIL import Image
    a = np.asarray(img.convert("RGB").resize((nw, nh), Image.BILINEAR), dtype=np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406], np.float32)
    std = np.array([0.229, 0.224, 0.225], np.float32)
    x = ((a - mean) / std).transpose(2, 0, 1)[None].astype(np.float32)
    out = det.run(None, {det.get_inputs()[0].name: x})[0]
    p = np.asarray(out).reshape(out.shape[-2], out.shape[-1])
    mask = p > float(s["thresh"])
    sx, sy = W / p.shape[1], H / p.shape[0]
    thr_area = 4
    boxes = []
    for (x0, y0, x1, y1, area) in components(mask):
        if area < thr_area:
            continue
        w, h = x1 - x0 + 1, y1 - y0 + 1
        d = w * h * float(s["unclip"]) / (2.0 * (w + h))  # DB unclip distance, applied to the bounding box
        bx0 = max(0.0, (x0 - d) * sx)
        by0 = max(0.0, (y0 - d) * sy)
        bx1 = min(float(W), (x1 + 1 + d) * sx)
        by1 = min(float(H), (y1 + 1 + d) * sy)
        if bx1 - bx0 >= 2 and by1 - by0 >= 2:
            boxes.append((int(bx0), int(by0), int(round(bx1)), int(round(by1))))
    return order(boxes)


def components(mask, eight=False):
    """Connected components of a boolean array: (x0, y0, x1, y1, pixel count). Run-based labelling,
    4-connected (PP-OCR path) or 8-connected (docTR path, like OpenCV's contour tracing)."""
    import numpy as np
    H, W = mask.shape
    parent = []

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    labels = np.zeros((H, W), np.int32)
    prev = []  # runs of the previous row: (start, end, label)
    runs_all = []
    for y in range(H):
        row = mask[y]
        d = np.diff(np.concatenate(([0], row.view(np.int8), [0])))
        starts, ends = np.flatnonzero(d == 1), np.flatnonzero(d == -1)
        cur = []
        for st, en in zip(starts, ends):
            lab = None
            for (ps, pe, pl) in prev:
                if (ps <= en and st <= pe) if eight else (ps < en and st < pe):
                    if lab is None:
                        lab = find(pl)
                    else:
                        r = find(pl)
                        if r != lab:
                            parent[r] = lab
            if lab is None:
                lab = len(parent)
                parent.append(lab)
            cur.append((int(st), int(en), lab))
            runs_all.append((y, int(st), int(en), lab))
        prev = cur
    acc = {}
    for (y, st, en, lab) in runs_all:
        r = find(lab)
        b = acc.get(r)
        if b is None:
            acc[r] = [st, y, en - 1, y, en - st]
        else:
            b[0] = min(b[0], st)
            b[1] = min(b[1], y)
            b[2] = max(b[2], en - 1)
            b[3] = max(b[3], y)
            b[4] += en - st
    return [tuple(v) for v in acc.values()]


def order(boxes):
    """Reading order: rows by vertical centre (boxes overlapping half their height share a row), then left to right."""
    boxes = sorted(boxes, key=lambda b: (b[1] + b[3]) / 2.0)
    rows = []
    for b in boxes:
        c = (b[1] + b[3]) / 2.0
        for r in rows:
            if abs(c - r["c"]) < 0.5 * min(b[3] - b[1], r["h"]):
                r["b"].append(b)
                break
        else:
            rows.append({"c": c, "h": b[3] - b[1], "b": [b]})
    out = []
    for r in rows:
        out.extend(sorted(r["b"], key=lambda b: b[0]))
    return out


def recognise_crop(rec, crop, chars, s):
    """CTC recogniser on one crop. Returns [(text, conf 0-100, t_start, t_end, T)] words (split at the space class)."""
    import numpy as np
    from PIL import Image
    Hh = int(s["rec_height"])
    ishape = rec.get_inputs()[0].shape  # [N, 3, H, W]; entries may be names/None for dynamic axes
    if isinstance(ishape[2], int) and ishape[2] > 0:
        Hh = ishape[2]
    w, h = crop.size
    nw = max(8, int(round(Hh * w / float(max(h, 1)))))
    fixed_w = ishape[3] if isinstance(ishape[3], int) and ishape[3] > 0 else None
    if fixed_w:
        nw = min(nw, fixed_w)
    a = np.asarray(crop.convert("RGB").resize((nw, Hh), Image.BILINEAR), dtype=np.float32) / 255.0
    a = ((a - 0.5) / 0.5).transpose(2, 0, 1)
    if fixed_w and nw < fixed_w:  # right padding with the (normalised) background value 0
        pad = np.zeros((3, Hh, fixed_w), np.float32)
        pad[:, :, :nw] = a
        a = pad
    out = np.asarray(rec.run(None, {rec.get_inputs()[0].name: a[None].astype(np.float32)})[0])
    p = out.reshape(-1, out.shape[-1]).astype(np.float64)  # (T, C)
    if p.min() < 0 or abs(p.sum(axis=1).mean() - 1.0) > 0.05:  # logits: softmax
        e = np.exp(p - p.max(axis=1, keepdims=True))
        p = e / e.sum(axis=1, keepdims=True)
    T = p.shape[0]
    # the time axis covers the (padded) input width; map back to the real crop width
    used = nw / float(fixed_w) if fixed_w else 1.0
    idx = p.argmax(axis=1)
    conf = p.max(axis=1)
    words, cur, last = [], None, 0
    for t in range(T):
        k = int(idx[t])
        if k == 0 or k == last:
            last = k
            continue
        last = k
        ch = chars[k] if k < len(chars) else ""
        if ch == " ":
            if cur:
                words.append(cur)
                cur = None
            continue
        if cur is None:
            cur = {"text": "", "c": [], "t0": t, "t1": t}
        cur["text"] += ch
        cur["c"].append(float(conf[t]))
        cur["t1"] = t
    if cur:
        words.append(cur)
    res = []
    for wd in words:
        if wd["text"]:
            res.append((wd["text"], 100.0 * sum(wd["c"]) / len(wd["c"]), wd["t0"], wd["t1"], T / used))
    return res


# ------------------------------------------------------------------------------------------------
# docTR / OnnxTR family (ROADMAP 4.33). Constants are those of the OnnxTR models; the algorithm is written
# from the published description (DB probability map -> binarise -> contours -> unclip; CRNN + CTC), not copied.
DOCTR_DET_MEAN = (0.798, 0.785, 0.772)
DOCTR_DET_STD = (0.264, 0.2749, 0.287)
DOCTR_REC_MEAN = (0.694, 0.695, 0.693)
DOCTR_REC_STD = (0.299, 0.296, 0.301)
DOCTR_BOX_THRESH = 0.1     # minimal mean probability inside a box
DOCTR_SPLIT_AR = 8.0       # crops wider than this (w/h) are cut into overlapping pieces
DOCTR_SPLIT_TARGET = 6     # piece width = 6 x height
DOCTR_SPLIT_OVERLAP = 0.5


def sigmoid(a):
    import numpy as np
    return 1.0 / (1.0 + np.exp(-np.clip(a, -30.0, 30.0)))


def morph_open3(m):
    """3x3 opening (erosion then dilation) of a uint8 0/1 map; the border counts as background."""
    import numpy as np
    H, W = m.shape
    p = np.pad(m, 1)
    er = np.ones_like(m)
    for dy in range(3):
        for dx in range(3):
            er &= p[dy:dy + H, dx:dx + W]
    p = np.pad(er, 1)
    out = np.zeros_like(m)
    for dy in range(3):
        for dx in range(3):
            out |= p[dy:dy + H, dx:dx + W]
    return out


def letterbox(img, side):
    """Resize keeping the aspect ratio to fit side x side, pad symmetrically with black (0).
    Returns (uint8 array side x side x 3, scale, pad_left, pad_top)."""
    import numpy as np
    from PIL import Image
    W, H = img.size
    if H / float(W) > 1.0:
        nh, nw = side, max(int(side / (H / float(W))), 1)
    else:
        nh, nw = max(int(side * (H / float(W))), 1), side
    a = np.asarray(img.convert("RGB").resize((nw, nh), Image.BILINEAR), dtype=np.uint8)
    pl, pt = -(-(side - nw) // 2), -(-(side - nh) // 2)
    out = np.zeros((side, side, 3), np.uint8)
    out[pt:pt + nh, pl:pl + nw] = a
    return out, nw / float(W), pl, pt


def doctr_detect(det, img, s):
    """DB word detector (docTR layout). Returns boxes (x0, y0, x1, y1) in image pixels (unordered)."""
    import numpy as np
    W, H = img.size
    ishape = det.get_inputs()[0].shape
    side = ishape[2] if isinstance(ishape[2], int) and ishape[2] > 0 else 1024
    a, scale, pl, pt = letterbox(img, side)
    x = (a.astype(np.float32) / 255.0 - np.array(DOCTR_DET_MEAN, np.float32)) / np.array(DOCTR_DET_STD, np.float32)
    x = np.ascontiguousarray(x.transpose(2, 0, 1)[None], dtype=np.float32)
    out = np.asarray(det.run(None, {det.get_inputs()[0].name: x})[0])
    p = sigmoid(out.reshape(out.shape[-2], out.shape[-1]).astype(np.float32))
    bitmap = morph_open3((p >= float(s["thresh"])).astype(np.uint8))
    ph, pw = p.shape
    unclip = float(s["unclip"])
    boxes = []
    for (x0, y0, x1, y1, area) in components(bitmap.astype(bool), eight=True):
        if x1 - x0 < 2 or y1 - y0 < 2:
            continue
        w, h = x1 - x0 + 1, y1 - y0 + 1
        if float(p[max(0, y0):min(ph - 1, y1) + 1, max(0, x0):min(pw - 1, x1) + 1].mean()) < DOCTR_BOX_THRESH:
            continue
        d = w * h * unclip / (2.0 * (w + h))
        bx0, bx1 = max(0.0, x0 - d), min(float(pw), x0 + w + d)
        by0, by1 = max(0.0, y0 - d), min(float(ph), y0 + h + d)
        # back to page pixels: undo the padding and the scale
        px0, px1 = (bx0 - pl) / scale, (bx1 - pl) / scale
        py0, py1 = (by0 - pt) / scale, (by1 - pt) / scale
        px0, px1 = max(0.0, px0), min(float(W), px1)
        py0, py1 = max(0.0, py0), min(float(H), py1)
        if px1 - px0 >= 2 and py1 - py0 >= 2:
            boxes.append((int(round(px0)), int(round(py0)), int(round(px1)), int(round(py1))))
    return boxes


def split_pieces(crop):
    """Crops wider than DOCTR_SPLIT_AR x height are cut into overlapping pieces. Returns (pieces, overlap ratio of the last)."""
    import math
    w, h = crop.size
    if w / float(h) <= DOCTR_SPLIT_AR:
        return [crop], 0.0
    sw = max(1, math.ceil(h * DOCTR_SPLIT_TARGET))
    ov = max(0, math.floor(sw * DOCTR_SPLIT_OVERLAP))
    if w <= sw:
        return [crop], 0.0
    starts = list(range(0, w - sw + 1, sw - ov))
    if starts[-1] + sw < w:
        starts.append(w - sw)
    last = ((starts[-2] + sw) - starts[-1]) / float(sw) if len(starts) > 1 else 0.0
    return [crop.crop((st, 0, st + sw, h)) for st in starts], last


def merge_pair(a, b, ratio):
    """Join the readings of two neighbouring pieces whose ends show the same image part. The outer letter of each side may
    be cut off and is ignored when looking for the overlap; the overlap with the fewest mismatches wins, ties go to the
    one nearest the expected length."""
    if min(len(a), len(b)) <= 1:
        return a + b
    ac, bc = a[:-1], b[1:]
    mx = min(len(ac), len(bc))
    exp = round(len(b) * ratio) - 3

    def miss(k):
        return sum(1 for u, v in zip(ac[-k:], bc[:k]) if u != v)

    perfect = [k for k in range(1, mx + 1) if miss(k) == 0]
    if perfect:
        k = min(perfect, key=lambda k: abs((k - 1) - exp))
        return ac + bc[k:]
    if exp < -1:
        return a + b
    if exp < 0:
        return ac + bc
    k = min(range(1, mx + 1), key=lambda k: miss(k) + abs((k - 1) - exp))
    return ac + bc[k:]


def rec_input(piece):
    """One crop -> (3, 32, 128) float32: aspect kept, black padding right/bottom, docTR mean/std."""
    import numpy as np
    from PIL import Image
    sh, sw = 32, 128
    w, h = piece.size
    if h / float(w) > sh / float(sw):
        nh, nw = sh, max(int(sh / (h / float(w))), 1)
    else:
        nh, nw = max(int(sw * (h / float(w))), 1), sw
    a = np.asarray(piece.convert("RGB").resize((nw, nh), Image.BILINEAR), dtype=np.uint8)
    canvas = np.zeros((sh, sw, 3), np.uint8)
    canvas[:nh, :nw] = a
    x = (canvas.astype(np.float32) / 255.0 - np.array(DOCTR_REC_MEAN, np.float32)) / np.array(DOCTR_REC_STD, np.float32)
    return x.transpose(2, 0, 1)


def doctr_read(rec, crops, vocab, batch=64):
    """CTC greedy reading of word crops (blank = last class). Returns [(text, conf 0-100)]."""
    import numpy as np
    items, plan = [], []
    for c in crops:
        pieces, last = split_pieces(c)
        plan.append((len(items), len(items) + len(pieces), last))
        items += [rec_input(q) for q in pieces]
    texts, confs = [], []
    blank = len(vocab)
    for i in range(0, len(items), batch):
        x = np.ascontiguousarray(np.stack(items[i:i + batch]), dtype=np.float32)
        lg = np.asarray(rec.run(None, {rec.get_inputs()[0].name: x})[0])
        if lg.shape[-1] != blank + 1:
            raise ValueError("recogniser has %d classes but the vocabulary has %d characters (+ blank): wrong --onnx-dict for this model"
                             % (lg.shape[-1], len(vocab)))
        e = np.exp(lg - lg.max(axis=-1, keepdims=True))
        pr = e / e.sum(axis=-1, keepdims=True)
        idx, pk = pr.argmax(-1), pr.max(-1)
        for n in range(lg.shape[0]):
            out, cf, prev = [], [], -1
            for t in range(lg.shape[1]):
                k = int(idx[n, t])
                if k != prev and k != blank:
                    out.append(vocab[k])
                    cf.append(float(pk[n, t]))
                prev = k
            texts.append("".join(out))
            confs.append(100.0 * sum(cf) / len(cf) if cf else 0.0)
    res = []
    for (a, b, last) in plan:
        if b - a == 1:
            res.append((texts[a], confs[a]))
        else:
            t = texts[a]
            for j in range(a + 1, b):
                t = merge_pair(t, texts[j], last if j == b - 1 else DOCTR_SPLIT_OVERLAP)
            res.append((t, sum(confs[a:b]) / (b - a)))
    return res


def xy_cut(items, gap_x):
    """Reading order for word boxes by recursive XY cut. items: [(box, payload)]. Returns blocks (lists of items) in order:
    split at the widest empty vertical band (at least gap_x wide) first, else at the tallest empty horizontal band; a set
    that cannot be cut is one block (its lines are formed by the caller)."""
    if len(items) <= 1:
        return [items]
    if len(items) >= 20:  # vertical gutter that a few stray boxes may cross (noise, a long word): low-coverage band
        import numpy as np
        x0 = min(it[0][0] for it in items)
        x1 = max(it[0][2] for it in items)
        cov = np.zeros(x1 - x0 + 2, np.int32)
        for it in items:
            cov[it[0][0] - x0] += 1
            cov[it[0][2] - x0] -= 1
        cov = np.cumsum(cov)[:-1]
        low = cov <= max(1, int(0.01 * len(items)))
        best, bs, i = 0, 0, 0
        while i < len(low):
            if low[i]:
                j = i
                while j < len(low) and low[j]:
                    j += 1
                if j - i > best and i > 0 and j < len(low):
                    best, bs = j - i, i
                i = j
            else:
                i += 1
        if best >= gap_x:
            mid = x0 + bs + best / 2.0
            left = [it for it in items if (it[0][0] + it[0][2]) / 2.0 < mid]
            right = [it for it in items if (it[0][0] + it[0][2]) / 2.0 >= mid]
            if left and right:
                return xy_cut(left, gap_x) + xy_cut(right, gap_x)
    for axis in (0, 1):
        lo, hi = (0, 2) if axis == 0 else (1, 3)
        srt = sorted(items, key=lambda it: it[0][lo])
        edge, best, cut = srt[0][0][hi], 0, None
        for i in range(1, len(srt)):
            gap = srt[i][0][lo] - edge
            if gap > best and (axis == 1 or gap >= gap_x):
                best, cut = gap, i
            edge = max(edge, srt[i][0][hi])
        if cut is not None and best > 0:
            return xy_cut(srt[:cut], gap_x) + xy_cut(srt[cut:], gap_x)
    return [items]


def doctr_lines(items):
    """items [(box, payload)] of one block -> lines (lists of items), top to bottom, each left to right."""
    rows = []
    for it in sorted(items, key=lambda it: it[0][1] + it[0][3]):
        b = it[0]
        c = (b[1] + b[3]) / 2.0
        for r in rows:
            if abs(c - r["c"]) < 0.5 * min(b[3] - b[1], r["h"]):
                r["b"].append(it)
                r["c"] += (c - r["c"]) / len(r["b"])
                break
        else:
            rows.append({"c": c, "h": b[3] - b[1], "b": [it]})
    rows.sort(key=lambda r: r["c"])
    return [sorted(r["b"], key=lambda it: it[0][0]) for r in rows]


def run_doctr(png, s):
    from PIL import Image
    img = Image.open(io.BytesIO(png))
    img.load()
    img = img.convert("RGB")
    if not s["det"]:
        raise ValueError("the doctr family needs a detector: set LEGATOROCR_ONNX_DET")
    with open(s["dict"], encoding="utf-8") as f:
        vocab = f.read().rstrip("\r\n")
    det, rec = sessions(s)
    boxes = doctr_detect(det, img, s)
    if not boxes:
        return []
    W, H = img.size
    crops = [img.crop((max(0, b[0]), max(0, b[1]), min(W, b[2] + 1), min(H, b[3] + 1))) for b in boxes]
    texts = doctr_read(rec, crops, vocab)
    items = [(b, t) for b, t in zip(boxes, texts) if t[0].strip()]
    if not items:
        return []
    hs = sorted(b[3] - b[1] for b, _ in items)
    gap_x = max(8.0, 0.8 * hs[len(hs) // 2])
    rows, ln = [], 0
    for blk in xy_cut(items, gap_x):
        for line in doctr_lines(blk):
            ln += 1
            for wn, (b, (text, conf)) in enumerate(line, 1):
                rows.append((ln, wn, b[0], b[1], max(1, b[2] - b[0]), max(1, b[3] - b[1]), conf, text))
    return rows


def run(png, s):
    if s["family"] == "doctr":
        return run_doctr(png, s)

    from PIL import Image
    img = Image.open(io.BytesIO(png))
    img.load()
    chars = load_dict(s["dict"], s["space"])
    det, rec = sessions(s)
    boxes = detect(det, img, s) if det is not None else [(0, 0, img.size[0], img.size[1])]
    rows = []
    for ln, (x0, y0, x1, y1) in enumerate(boxes, 1):
        words = recognise_crop(rec, img.crop((x0, y0, x1, y1)), chars, s)
        bw = x1 - x0
        for wn, (text, conf, t0, t1, T) in enumerate(words, 1):
            # a character occupies [t, t+1) of T steps; the word spans its first to last emitted step
            wx0 = x0 + int(bw * t0 / T)
            wx1 = x0 + int(round(bw * (t1 + 1) / T))
            rows.append((ln, wn, wx0, y0, max(1, wx1 - wx0), y1 - y0, conf, text))
    return rows


def main(argv):
    s, rest = settings(argv)
    if "--version" in rest:
        print("engine-onnx 1.0 (PP-OCR and docTR layout ONNX bridge)")
        return 0
    if "--list-langs" in rest:
        print("List of available languages (1):\nron")
        return 0
    if "osd" in rest:
        sys.stderr.write("orientation detection is not supported by the ONNX bridge\n")
        return 1
    if s["family"] not in ("ppocr", "doctr"):
        sys.stderr.write("engine-onnx: --onnx-family must be ppocr or doctr, not %r\n" % s["family"])
        return 2
    if not s["rec"] or not s["dict"]:
        sys.stderr.write("engine-onnx: set LEGATOROCR_ONNX_REC and LEGATOROCR_ONNX_DICT (and LEGATOROCR_ONNX_DET); see README\n")
        return 2
    try:
        rows = run(sys.stdin.buffer.read(), s)
    except Exception as e:  # report, do not hide: the CLI shows stderr
        sys.stderr.write("engine-onnx: %s: %s\n" % (type(e).__name__, e))
        return 3
    out = [HEADER]
    for ln, wn, x, y, w, h, c, t in rows:
        t = t.replace("\t", " ").replace("\n", " ")
        out.append("5\t1\t1\t1\t%d\t%d\t%d\t%d\t%d\t%d\t%.1f\t%s" % (ln, wn, x, y, w, h, c, t))
    sys.stdout.buffer.write(("\n".join(out) + "\n").encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
