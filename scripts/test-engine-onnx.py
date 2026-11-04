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

# ---------------------------------------------------------------------------------------------------
# docTR family (ROADMAP 4.33), SYNTHETIC models again: plumbing, geometry, reading order. No accuracy claim.
import importlib.util
spec = importlib.util.spec_from_file_location("engine_onnx", str(engine))
eo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eo)

# unit: merging two overlapping piece readings, splitting of wide crops, recogniser input padding, letterbox
check(eo.merge_pair("abcde", "cdefgh", 0.5) == "abcdefgh", "merge_pair joins overlapping readings: %r" % eo.merge_pair("abcde", "cdefgh", 0.5))
check(eo.merge_pair("abcdi", "cdefgh", 0.5) == "abcdefgh", "merge_pair tolerates a cut-off outer letter")
pcs, last = eo.split_pieces(Image.new("RGB", (400, 30)))
check(len(pcs) >= 3 and all(p.size == (180, 30) for p in pcs) and 0.0 <= last < 1.0, "wide crop (13:1) is split into %d pieces of 6 x height" % len(pcs))
check(len(eo.split_pieces(Image.new("RGB", (200, 30)))[0]) == 1, "a 6.7:1 crop is not split")
ri = eo.rec_input(Image.new("RGB", (64, 32), (255, 255, 255)))
padv = -eo.DOCTR_REC_MEAN[0] / eo.DOCTR_REC_STD[0]
check(ri.shape == (3, 32, 128) and abs(ri[0, 5, 100] - padv) < 1e-4 and ri[0, 5, 10] > 1.0,
      "recogniser input 3x32x128: image left, black padding right (normalised %.2f), white normalised %.2f" % (padv, ri[0, 5, 10]))
ri2 = eo.rec_input(Image.new("RGB", (256, 32), (255, 255, 255)))
check(abs(ri2[0, 5, 5] - ri[0, 5, 10]) < 1e-4 and abs(ri2[0, 20, 5] - padv) < 1e-4, "a 8:1 crop is scaled to width 128 (height 16) and padded at the bottom")
lb, sc, pl, pt = eo.letterbox(Image.new("RGB", (400, 200), (255, 255, 255)), 64)
check(lb.shape == (64, 64, 3) and pl == 0 and pt == 16 and abs(sc - 0.16) < 1e-9 and lb[0, 0, 0] == 0 and lb[32, 32, 0] == 255,
      "letterbox: aspect kept, symmetric black padding (pad_top %d), scale %.2f" % (pt, sc))

# unit: reading order. Two justified columns (4 lines x 6 words each) and one stray box across the gutter
items = []
for col, cx in enumerate((100, 700)):
    for ln in range(4):
        for wd in range(6):
            items.append(((cx + wd * 60, 200 + ln * 60, cx + wd * 60 + 50, 240 + ln * 60), (col, ln, wd)))
items.append(((450, 500, 760, 540), (1, 9, 9)))  # crosses the gutter, centre in the right column
blocks = eo.xy_cut(items, 20.0)
order_ = [it[1] for b in blocks for ln in eo.doctr_lines(b) for it in ln]
cols = [p[0] for p in order_]
check(cols == sorted(cols), "XY cut: the whole left column is read before the right one (stray box crossing the gutter tolerated)")
check([p[:2] for p in order_[:24]] == [(0, l) for l in range(4) for _ in range(6)], "left column: lines top to bottom, words left to right within a line")
single = [((10 + i * 60, 100, 60 + i * 60, 140), i) for i in range(5)] + [((10 + i * 60, 160, 60 + i * 60, 200), 5 + i) for i in range(5)]
check([p for b in eo.xy_cut(single, 20.0) for ln in eo.doctr_lines(b) for _, p in ln] == list(range(10)), "single column: line by line")

# e2e: a docTR-layout detector (sigmoid(6*(R-B-1.5)) on the normalised input: text = yellow on white, the black padding gives 0)
DETS = {"mean": eo.DOCTR_DET_MEAN, "std": eo.DOCTR_DET_STD}
xd = H.make_tensor_value_info("input", TP.FLOAT, ["b", 3, 1024, 1024])
yd = H.make_tensor_value_info("logits", TP.FLOAT, ["b", 1, 1024, 1024])
save([H.make_node("Slice", ["input", "s0", "e0", "ax"], ["r"]),
      H.make_node("Slice", ["input", "s2", "e2", "ax"], ["bl"]),
      H.make_node("Sub", ["r", "bl"], ["df"]),
      H.make_node("Sub", ["df", "c"], ["df2"]),
      H.make_node("Mul", ["df2", "k"], ["logits"])], [xd], [yd],
     [NH.from_array(np.array([0], np.int64), "s0"), NH.from_array(np.array([1], np.int64), "e0"),
      NH.from_array(np.array([2], np.int64), "s2"), NH.from_array(np.array([3], np.int64), "e2"),
      NH.from_array(np.array([1], np.int64), "ax"),
      NH.from_array(np.array(1.5, np.float32), "c"), NH.from_array(np.array(6.0, np.float32), "k")], "ddet.onnx")
# recogniser: fixed output "Bună ăț" for every crop; vocabulary as ONE string, blank = last class
VOC = "BunățĂ0"
WORD = "Bună"
TT = 32
SEQ2 = list(WORD)
steps = []
for ch in SEQ2:
    steps += [ch, None]
steps = steps + [None] * (TT - len(steps))
lg2 = np.full((1, TT, len(VOC) + 1), 0.0, np.float32)
for t, ch in enumerate(steps):
    lg2[0, t, len(VOC) if ch is None else VOC.index(ch)] = 12.0
xr2 = H.make_tensor_value_info("input", TP.FLOAT, ["b", 3, 32, 128])
yr2 = H.make_tensor_value_info("logits", TP.FLOAT, ["b", TT, len(VOC) + 1])
save([H.make_node("ReduceSum", ["input", "axs"], ["s"], keepdims=0),
      H.make_node("Mul", ["s", "zero"], ["s0"]),
      H.make_node("Reshape", ["s0", "shp"], ["s1"]),
      H.make_node("Add", ["lg", "s1"], ["logits"])], [xr2], [yr2],
     [NH.from_array(np.array(0.0, np.float32), "zero"), NH.from_array(np.array([-1, 1, 1], np.int64), "shp"),
      NH.from_array(np.array([1, 2, 3], np.int64), "axs"),
      NH.from_array(lg2, "lg")], "drec.onnx")
(d / "vocab.txt").write_text(VOC, encoding="utf-8")
# page: white, 2 columns x 3 lines of 4 yellow word blocks each (so the order is only decidable with the column cut)
W2, H2 = 1240, 1754
pg = Image.new("RGB", (W2, H2), (255, 255, 255))
dg = ImageDraw.Draw(pg)
WB = []
for ci, cx in enumerate((100, 700)):
    for ln in range(3):
        for wd in range(4):
            b = (cx + wd * 130, 300 + ln * 90, cx + wd * 130 + 110, 300 + ln * 90 + 40)
            WB.append((ci, ln, wd, b))
            dg.rectangle([b[0], b[1], b[2] - 1, b[3] - 1], fill=(255, 255, 0))
pg.save(d / "page2.png")
envd = {k: v for k, v in os.environ.items() if not k.startswith("LEGATOROCR_ONNX")}
envd.update(LEGATOROCR_ONNX_FAMILY="doctr", LEGATOROCR_ONNX_DET=str(d / "ddet.onnx"), LEGATOROCR_ONNX_REC=str(d / "drec.onnx"),
            LEGATOROCR_ONNX_DICT=str(d / "vocab.txt"), LEGATOROCR_ONNX_UNCLIP="0")
rd = subprocess.run([sys.executable, str(engine), "stdin", "stdout", "tsv"], input=(d / "page2.png").read_bytes(), capture_output=True, env=envd, timeout=300)
check(rd.returncode == 0, "doctr family exit 0 (stderr: %s)" % rd.stderr.decode()[-300:])
rowsd = [l.split("\t") for l in rd.stdout.decode("utf-8").splitlines()[1:]]
check(len(rowsd) == len(WB) and all(x[11] == WORD for x in rowsd), "doctr: %d word blocks -> %d rows, text %r (blank = last class, non-ASCII ok)" % (len(WB), len(rowsd), WORD))
if len(rowsd) == len(WB):
    # reading order: column 0 (3 lines x 4 words) then column 1; line numbers 1..6; words left to right
    exp = [b for (ci, ln, wd, b) in sorted(WB, key=lambda t: (t[0], t[1], t[2]))]
    got = [tuple(int(v) for v in x[6:10]) for x in rowsd]
    tol = 6
    okpos = all(abs(g[0] - e[0]) <= tol and abs(g[1] - e[1]) <= tol and abs(g[0] + g[2] - e[2]) <= tol and abs(g[1] + g[3] - e[3]) <= tol for g, e in zip(got, exp))
    check(okpos, "doctr: boxes (1024 letterbox mapped back to page pixels, unclip 0) in column-then-line reading order, within %d px" % tol)
    check([int(x[4]) for x in rowsd] == [1] * 4 + [2] * 4 + [3] * 4 + [4] * 4 + [5] * 4 + [6] * 4, "doctr: line numbers group the words of one line")
    check([int(x[5]) for x in rowsd[:4]] == [1, 2, 3, 4], "doctr: word numbers restart per line")
# a vocabulary of the wrong size is a clear error, not garbage
(d / "badvocab.txt").write_text("Bun", encoding="utf-8")
rb = subprocess.run([sys.executable, str(engine), "stdin", "stdout", "tsv"], input=(d / "page2.png").read_bytes(), capture_output=True,
                    env=dict(envd, LEGATOROCR_ONNX_DICT=str(d / "badvocab.txt")), timeout=300)
check(rb.returncode == 3 and b"vocabulary" in rb.stderr, "doctr: wrong vocabulary size gives a clear error")
# the flag works like the variable; an unknown family is refused; ppocr stays the default
envn = {k: v for k, v in envd.items() if k != "LEGATOROCR_ONNX_FAMILY"}
rf = subprocess.run([sys.executable, str(engine), "--onnx-family", "doctr", "stdin", "stdout", "tsv"], input=(d / "page2.png").read_bytes(), capture_output=True, env=envn, timeout=300)
check(rf.returncode == 0 and rf.stdout == rd.stdout, "--onnx-family doctr gives the same as LEGATOROCR_ONNX_FAMILY=doctr")
rx = subprocess.run([sys.executable, str(engine), "--onnx-family", "nope", "stdin", "stdout", "tsv"], input=b"x", capture_output=True, env=envn, timeout=60)
check(rx.returncode == 2 and b"family" in rx.stderr, "unknown family is refused")

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
if len(sys.argv) > 1:  # docTR family through the CLI: words and line structure arrive in the JSON
    cmd2 = [sys.argv[1], str(d / "page2.png"), "--dpi", "150", "--ocr", "--lang", "ron", "--engine", "tesseract", "--tesseract", str(engine),
            "--json", str(d / "o2.json"), "--no-deskew", "--no-auto-orient", "--split", "off", "--format", "none", "--out-dir", str(d / "out2")]
    c2 = subprocess.run(cmd2, capture_output=True, text=True, env=envd, timeout=600)
    check(c2.returncode == 0, "CLI with the doctr family exit 0 (%s)" % (c2.stderr or "")[-200:])
    if c2.returncode == 0:
        w2 = json.load(open(d / "o2.json"))["pages"][0]["words"]
        check(len(w2) == len(WB) and all(w["text"] == WORD for w in w2) and len(set(w["line"] for w in w2)) == 6,
              "CLI doctr: %d words in %d lines" % (len(w2), len(set(w["line"] for w in w2))))
print("engine-onnx synthetic test: %s" % ("FAILED %d" % len(bad) if bad else "all ok (plumbing only, accuracy not measured)"))
sys.exit(1 if bad else 0)
