#!/usr/bin/env python3
"""SYNTHETIC plumbing test of scripts/engine-onnx.py. It builds two tiny ONNX models with the `onnx` package:
  detector   : probability = sigmoid(-4 * mean(normalised RGB)), i.e. 1 on dark pixels, 0 on light ones;
  recogniser : ignores the image and returns the CTC probabilities of a fixed string ("Hello lumea ăț").
It proves: input/output wiring, DB-style box finding and the unclip growth, coordinates after the resize to a
multiple of 32 and back, CTC greedy decoding (blank, repeated letters, space class, non-ASCII), word boxes
from time steps, TSV format, and (when a legatorocr binary is given) the whole path through
`legatorocr --engine tesseract --tesseract scripts/engine-onnx.py`.
It does NOT measure recognition quality: that needs real PP-OCR models, which were not available.
Usage: scripts/test-engine-onnx.py [path/to/legatorocr]   (exit 0 = ok, 77 = skipped: onnx/onnxruntime missing)"""
import json, os, pathlib, subprocess, sys, tempfile
try:
    import numpy as np
    import onnx
    from onnx import TensorProto as TP, helper as H, numpy_helper as NH
    import onnxruntime  # noqa: F401
    from PIL import Image, ImageDraw
except Exception as e:  # pragma: no cover
    print("skip engine-onnx (needs onnx, onnxruntime, numpy, Pillow: %s)" % e)
    sys.exit(77)

root = pathlib.Path(__file__).resolve().parent.parent
engine = root / "scripts" / "engine-onnx.py"
d = pathlib.Path(tempfile.mkdtemp())
DICT = list("Helo umaăț")            # class i+1; the space class is appended by the bridge (SPACE=1)
SEQ = list("Hel") + [None] + list("lo") + [None, " ", None] + list("lumea") + [" "] + list("ăț")
T = 40
SEQ = SEQ + [None] * (T - len(SEQ))
chars = ["<blank>"] + DICT + [" "]


def save(nodes, inputs, outputs, inits, name):
    g = H.make_graph(nodes, name, inputs, outputs, inits)
    m = H.make_model(g, opset_imports=[H.make_opsetid("", 13)])
    m.ir_version = 8
    onnx.checker.check_model(m)
    onnx.save(m, str(d / name))


# detector
x = H.make_tensor_value_info("x", TP.FLOAT, [1, 3, "h", "w"])
y = H.make_tensor_value_info("y", TP.FLOAT, [1, 1, "h", "w"])
save([H.make_node("ReduceMean", ["x"], ["m"], axes=[1], keepdims=1),
      H.make_node("Mul", ["m", "k"], ["z"]),
      H.make_node("Sigmoid", ["z"], ["y"])], [x], [y], [NH.from_array(np.array(-4.0, np.float32), "k")], "det.onnx")
# recogniser
C = len(chars)
logit = np.full((1, T, C), 0.0, np.float32)
for t, ch in enumerate(SEQ):
    logit[0, t, 0 if ch is None else chars.index(ch)] = 12.0
xr = H.make_tensor_value_info("x", TP.FLOAT, [1, 3, 48, "w"])
yr = H.make_tensor_value_info("y", TP.FLOAT, [1, T, C])
save([H.make_node("ReduceSum", ["x"], ["s"], keepdims=0),
      H.make_node("Mul", ["s", "zero"], ["s0"]),
      H.make_node("Add", ["lg", "s0"], ["l2"]),
      H.make_node("Softmax", ["l2"], ["y"], axis=2)], [xr], [yr],
     [NH.from_array(np.array(0.0, np.float32), "zero"), NH.from_array(logit, "lg")], "rec.onnx")
(d / "dict.txt").write_text("\n".join(DICT) + "\n", encoding="utf-8")

# page: white, two dark blocks at known places
W, Hh = 1240, 1754
BLOCKS = [(140, 200, 700, 260), (140, 400, 900, 470)]
im = Image.new("RGB", (W, Hh), (255, 255, 255))
dr = ImageDraw.Draw(im)
for b in BLOCKS:
    dr.rectangle([b[0], b[1], b[2] - 1, b[3] - 1], fill=(0, 0, 0))
im.save(d / "page.png")

env = dict(os.environ, LEGATOROCR_ONNX_DET=str(d / "det.onnx"), LEGATOROCR_ONNX_REC=str(d / "rec.onnx"),
           LEGATOROCR_ONNX_DICT=str(d / "dict.txt"),
           LEGATOROCR_ONNX_UNCLIP="0")  # the synthetic detector marks the whole block, not a shrunk kernel: no growth needed
bad = []


def check(cond, msg):
    print(("ok   " if cond else "FAIL ") + msg)
    if not cond:
        bad.append(msg)


# expected words and their time-step spans
spans, cur = [], None
last = None
for t, ch in enumerate(SEQ):
    if ch is None or ch == last:
        last = ch
        continue
    last = ch
    if ch == " ":
        cur = None
        continue
    if cur is None:
        cur = [t, t, ""]
        spans.append(cur)
    cur[1] = t
    cur[2] += ch
TEXTS = [s[2] for s in spans]
check(TEXTS == ["Hello", "lumea", "ăț"], "test sequence decodes as designed: %r" % TEXTS)

r = subprocess.run([sys.executable, str(engine), "stdin", "stdout", "-l", "ron", "--psm", "3", "tsv"],
                   input=(d / "page.png").read_bytes(), capture_output=True, env=env, timeout=300)
check(r.returncode == 0, "engine exit 0 (stderr: %s)" % r.stderr.decode()[-200:])
rows = [l.split("\t") for l in r.stdout.decode("utf-8").splitlines()[1:]]
check(len(rows) == 6, "2 lines x 3 words = 6 rows, got %d" % len(rows))
check([x[11] for x in rows] == TEXTS * 2, "texts (incl. non-ASCII) per line")
check(all(float(x[10]) > 95 for x in rows), "confidence from CTC peak probability > 95")
check([x[4] for x in rows] == ["1"] * 3 + ["2"] * 3, "line numbers group words per detected box")
# boxes: detector works on a page resized to 960 long side and 32-multiples; allow the unclip growth + 2 cells
for li, b in enumerate(BLOCKS):
    got = [tuple(int(v) for v in x[6:10]) for x in rows[li * 3:(li + 1) * 3]]
    tol = 3 * W / 960.0 + 2
    ly0, ly1 = got[0][1], got[0][1] + got[0][3]
    check(abs(ly0 - b[1]) <= tol and abs(ly1 - b[3]) <= tol,
          "line %d vertical extent %d..%d vs block %d..%d" % (li + 1, ly0, ly1, b[1], b[3]))
    x0 = got[0][0]
    xend = got[-1][0] + got[-1][2]
    want_end = b[0] + (b[2] - b[0]) * (spans[2][1] + 1) / T   # the last emitted step ends at this fraction of the box
    tolx = tol
    check(abs(x0 - b[0]) <= tolx and abs(xend - want_end) <= tolx, "line %d horizontal extent %d..%d vs expected %d..%d" % (li + 1, x0, xend, b[0], int(want_end)))
    # word split follows the time axis: first word starts at the box start; words are ordered and disjoint
    check(got[0][0] <= got[1][0] and got[0][0] + got[0][2] <= got[1][0] + 1 and got[1][0] + got[1][2] <= got[2][0] + 1,
          "line %d words ordered left to right without overlap" % (li + 1))
# a word's relative position inside the block matches its time steps (this fails if the box mapping is wrong)
b = BLOCKS[1]; bw = rows[3][6:10]
first = [int(v) for v in rows[3][6:10]]
lastw = [int(v) for v in rows[5][6:10]]
span = (lastw[0] + lastw[2]) - first[0]
want = (spans[2][1] + 1 - spans[0][0]) / T
check(abs(span / ((b[2] - b[0])) - want) < 0.05, "word span fraction of the line follows the CTC time axis")

env_g = dict(env, LEGATOROCR_ONNX_UNCLIP="1.5")
rg = subprocess.run([sys.executable, str(engine), "stdin", "stdout", "tsv"], input=(d / "page.png").read_bytes(), capture_output=True, env=env_g, timeout=300)
rowg = [l.split("\t") for l in rg.stdout.decode("utf-8").splitlines()[1:]]
check(rg.returncode == 0 and int(rowg[0][9]) > int(rows[0][9]), "unclip 1.5 grows the box (line height %s > %s)" % (rowg[0][9], rows[0][9]))

# flags instead of environment variables, and the probes
env2 = {k: v for k, v in os.environ.items() if not k.startswith("LEGATOROCR_ONNX")}
r2 = subprocess.run([sys.executable, str(engine), "--onnx-det", str(d / "det.onnx"), "--onnx-rec", str(d / "rec.onnx"),
                     "--onnx-dict", str(d / "dict.txt"), "--onnx-unclip", "0", "stdin", "stdout", "tsv"], input=(d / "page.png").read_bytes(),
                    capture_output=True, env=env2, timeout=300)
check(r2.returncode == 0 and r2.stdout == r.stdout, "flags give the same result as environment variables")
r3 = subprocess.run([sys.executable, str(engine), "--version"], capture_output=True, env=env2)
check(r3.returncode == 0 and b"engine-onnx" in r3.stdout, "--version")
r4 = subprocess.run([sys.executable, str(engine), "stdin", "stdout", "tsv"], input=b"x", capture_output=True, env=env2)
check(r4.returncode != 0 and b"LEGATOROCR_ONNX_REC" in r4.stderr, "missing model path gives a clear error")

# end to end through the CLI
if len(sys.argv) > 1:
    exe = sys.argv[1]
    cmd = [exe, str(d / "page.png"), "--pdf", str(d / "o.pdf"), "--dpi", "150", "--ocr", "--lang", "ron", "--engine", "tesseract",
           "--tesseract", str(engine), "--json", str(d / "o.json"), "--no-deskew", "--no-auto-orient"]
    c = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=600)
    check(c.returncode == 0, "legatorocr --engine tesseract --tesseract engine-onnx.py exit 0 (%s)" % (c.stderr or "")[-200:])
    if c.returncode == 0:
        pg = json.load(open(d / "o.json"))["pages"][0]
        ws = pg["words"]
        check([w["text"] for w in ws] == TEXTS * 2, "CLI words: %r" % [w["text"] for w in ws])
        ys = sorted(set(w["y"] for w in ws))
        check(len(ys) == 2, "CLI: two lines")
        if len(ys) == 2:
            # the CLI crops the page to its content first, so absolute y moves; the distance between the lines must not
            check(abs((ys[1] - ys[0]) - (BLOCKS[1][1] - BLOCKS[0][1])) < 12,
                  "CLI: distance between lines %d vs %d in the source (coordinates survive the pipeline)" % (ys[1] - ys[0], BLOCKS[1][1] - BLOCKS[0][1]))
print("engine-onnx synthetic test: %s" % ("FAILED %d" % len(bad) if bad else "all ok (plumbing only, accuracy not measured)"))
sys.exit(1 if bad else 0)
