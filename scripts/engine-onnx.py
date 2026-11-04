#!/usr/bin/env python3
"""Bridge: any PP-OCR-layout ONNX detector + recogniser as an external LegatorOCR engine.

No weights are bundled and none are downloaded; you point the bridge at files you obtained yourself
(see README, "ONNX OCR bridge"). Needs only: onnxruntime, numpy, Pillow.

Use:
  export LEGATOROCR_ONNX_DET=/models/det.onnx LEGATOROCR_ONNX_REC=/models/rec.onnx LEGATOROCR_ONNX_DICT=/models/dict.txt
  legatorocr scan.png --pdf out.pdf --ocr --engine tesseract --tesseract scripts/engine-onnx.py

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


def components(mask):
    """4-connected components of a boolean array: (x0, y0, x1, y1, pixel count). Run-based labelling."""
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
                if ps < en and st < pe:
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


def run(png, s):
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
        print("engine-onnx 1.0 (PP-OCR layout ONNX bridge)")
        return 0
    if "--list-langs" in rest:
        print("List of available languages (1):\nron")
        return 0
    if "osd" in rest:
        sys.stderr.write("orientation detection is not supported by the ONNX bridge\n")
        return 1
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
