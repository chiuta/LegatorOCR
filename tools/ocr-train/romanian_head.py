#!/usr/bin/env python3
"""ROADMAP 4.32: give a FROZEN docTR/OnnxTR CRNN recogniser the Romanian letters (ă ș ț Ă Ș Ț) by
training only a new linear CTC head on its penultimate features (the backbone and the BiLSTM are not
touched), then splicing the head back into a copy of the ONNX graph.

    python3 romanian_head.py feats  ARCH  OUT.npz  [--n 40000] [--fonts train|heldout|all] [--seed 1]
    python3 romanian_head.py train  ARCH  TRAIN.npz HELDOUT.npz  OUT_HEAD.npz [--epochs 6]
    python3 romanian_head.py splice ARCH  HEAD.npz  OUT.onnx OUT.vocab.txt
    python3 romanian_head.py real   DET   ARCH  [MODEL.onnx VOCAB.txt]     # 12 real pages, as 4.31 + CER with diacritics
    python3 romanian_head.py tess                                           # Tesseract ron, same metrics

ARCH = crnn_mobilenet_v3_small | crnn_vgg16_bn.  Needs onnx, onnxruntime, onnxtr, torch (CPU), numpy,
Pillow, rapidfuzz, fontTools, wordfreq.  Environment variables: ROHEAD_WORK (cache dir, default
/tmp/rohead).  The vocabulary of the new model is the docTR 'french' vocabulary + "ășțĂȘȚ" and the CTC
blank stays the LAST class, as onnxtr expects (blank = len(vocab)).

Trained heads (139 KB each) are kept in tools/ocr-train/romanian_head/ (ARCH.npz + vocab.txt); `splice` rebuilds
the ONNX file from the original docTR model (downloaded from the OnnxTR GitHub release). Use in onnxtr:
    reco = onnxtr.models.recognition.crnn_mobilenet_v3_small(model_path="ro.onnx", vocab=open("vocab.txt").read())
    onnxtr.models.ocr_predictor(det_arch="db_mobilenet_v3_large", reco_arch=reco, assume_straight_pages=True)

Everything measured with this script on SYNTHETIC crops is labelled synthetic; the real-page numbers come from
12 pages of one magazine against ABBYY text that was not hand-verified.
"""
import glob, os, sys, re, json, random, unicodedata, time, io
from collections import Counter
import numpy as np

WORK = os.environ.get("ROHEAD_WORK", "/tmp/rohead")
os.makedirs(WORK, exist_ok=True)
NEWCH = "ășțĂȘȚ"
BASE = {"ă": "a", "ș": "s", "ț": "t", "Ă": "A", "Ș": "S", "Ț": "T"}
HELD_FAMILIES = {"Liberation Serif", "Nimbus Roman", "FreeSerif", "P052", "Carlito", "Poppins", "Lora", "DejaVu Sans"}
SKIP_FAMILIES = {"Last Resort High-Efficiency", "DejaVu Math TeX Gyre", "Unifont", "Unifont Sample", "Unifont-JP"}
ARCH_URL = {
    "crnn_mobilenet_v3_small": "https://github.com/felixdittrich92/OnnxTR/releases/download/v0.0.1/crnn_mobilenet_v3_small-bded4d49.onnx",
    "crnn_vgg16_bn": "https://github.com/felixdittrich92/OnnxTR/releases/download/v0.7.1/crnn_vgg16_bn-743599aa.onnx",
}
MEAN = (0.694, 0.695, 0.693)
STD = (0.299, 0.296, 0.301)


def french_vocab():
    from onnxtr.utils import VOCABS
    return VOCABS["french"]


def base_model(arch):
    p = os.path.join(WORK, os.path.basename(ARCH_URL[arch]))
    if not os.path.exists(p):
        import urllib.request
        urllib.request.urlretrieve(ARCH_URL[arch], p)
    return p


# ----------------------------------------------------------------------------- graph surgery
def find_head(model):
    """Return (add_node, matmul_node, feature_tensor_name, weight_name, bias_name)."""
    out = model.graph.output[0].name
    add = next(n for n in model.graph.node if out in n.output)
    assert add.op_type == "Add", add.op_type
    mm = next(n for n in model.graph.node if n.output[0] in add.input)
    assert mm.op_type == "MatMul", mm.op_type
    bias = next(i for i in add.input if i != mm.output[0])
    return add, mm, mm.input[0], mm.input[1], bias


def init_arr(model, name):
    import onnx
    from onnx import numpy_helper
    for t in model.graph.initializer:
        if t.name == name:
            return numpy_helper.to_array(t)
    raise KeyError(name)


def feature_session(arch):
    import onnx, onnxruntime as ort
    m = onnx.load(base_model(arch))
    _, _, feat, wn, bn = find_head(m)
    m.graph.output.append(onnx.helper.make_tensor_value_info(feat, onnx.TensorProto.FLOAT, None))
    so = ort.SessionOptions(); so.intra_op_num_threads = 2
    sess = ort.InferenceSession(m.SerializeToString(), so, providers=["CPUExecutionProvider"])
    return sess, init_arr(m, wn), init_arr(m, bn)


# ----------------------------------------------------------------------------- synthetic words
def families():
    import collections
    from fontTools.ttLib import TTFont
    cache = os.path.join(WORK, "fams.json")
    if os.path.exists(cache):
        return json.load(open(cache))
    need = set("ăâîșțĂÂÎȘȚ")
    fs = set()
    for pat in ("/usr/share/fonts/**/*.[to]tf", "/usr/local/lib/python3*/site-packages/**/*.[to]tf"):
        fs |= set(glob.glob(pat, recursive=True))
    fam = collections.defaultdict(list)
    for f in sorted(fs):
        try:
            t = TTFont(f, fontNumber=0, lazy=True); c = t.getBestCmap()
            if c and all(ord(x) in c for x in need):
                fam[t["name"].getBestFamilyName()].append(f)
        except Exception:
            pass
    fam = {k: v for k, v in fam.items() if k not in SKIP_FAMILIES}
    json.dump(fam, open(cache, "w"))
    return fam


def font_list(which):
    fam = families()
    out = []
    for k, v in sorted(fam.items()):
        held = k in HELD_FAMILIES
        if which == "all" or (which == "heldout" and held) or (which == "train" and not held):
            out += [f for f in v if "Variable" not in f]
    return out


def word_pool():
    import wordfreq
    okc = set(french_vocab() + NEWCH)
    ws = [w for w in wordfreq.top_n_list("ro", 80000) if w.isalpha() and len(w) <= 14 and set(w) <= okc]
    return ws


def strip_dia(s):
    return "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c))


def make_label(rnd, pool):
    w = rnd.choice(pool)
    if rnd.random() < 0.55:  # oversample words that contain a diacritic of the target kind
        for _ in range(40):
            if any(c in w for c in "ășț"):
                break
            w = rnd.choice(pool)
    if rnd.random() < 0.04:
        w = strip_dia(w)  # real texts also contain plain a/s/t
    r = rnd.random()
    if r < 0.18:
        w = w[0].upper() + w[1:]
    elif r < 0.26:
        w = w.upper()
    r = rnd.random()
    if r < 0.22:
        w += rnd.choice([",", ".", ";", ":", ")", "!", "?", ".\"", ",\""])
    if rnd.random() < 0.06:
        w = rnd.choice(["(", "'", '"', "-"]) + w
    if rnd.random() < 0.04:
        w = str(rnd.randint(1, 2030))
    return w


def render_word(text, fontpath, rnd, nprng):
    from PIL import Image, ImageDraw, ImageFont, ImageFilter
    size = rnd.choice([28, 34, 40, 48, 56, 64])
    font = ImageFont.truetype(fontpath, size)
    l, t, r, b = font.getbbox(text)
    asc, desc = font.getmetrics()
    padx = rnd.randint(1, 10) if rnd.random() < 0.8 else rnd.randint(10, 24)
    if rnd.random() < 0.5:   # box around the ink
        pady = rnd.randint(0, 8)
        top, bot = t - pady, b + pady
    else:                    # box around the whole line box
        top, bot = -rnd.randint(0, 6), asc + desc + rnd.randint(0, 6)
    W, H = (r - l) + 2 * padx, max(8, bot - top)
    bgc = np.array([rnd.randint(205, 252), rnd.randint(195, 248), rnd.randint(170, 242)])
    ink = np.array([rnd.randint(0, 70), rnd.randint(0, 60), rnd.randint(0, 55)])
    mask = Image.new("L", (W, H), 0)
    ImageDraw.Draw(mask).text((padx - l, -top), text, font=font, fill=255)
    ang = rnd.uniform(-2.0, 2.0)
    if abs(ang) > 0.2:
        mask = mask.rotate(ang, resample=Image.BICUBIC, expand=False)
    # simulate a lower scan resolution before the recogniser's own resize
    sc = rnd.choice([1.0, 1.0, 0.7, 0.5, 0.4])
    if sc < 1.0:
        mask = mask.resize((max(4, int(W * sc)), max(4, int(H * sc))), Image.BILINEAR)
    sig = rnd.uniform(0.0, 1.1) * (1 if sc == 1.0 else 0.6)
    if sig > 0.1:
        mask = mask.filter(ImageFilter.GaussianBlur(sig))
    m = np.asarray(mask, dtype=np.float32)[..., None] / 255.0
    if rnd.random() < 0.35:  # ink spread / thinning
        k = rnd.choice([0.8, 0.9, 1.15, 1.3])
        m = np.clip(m * k, 0, 1)
    img = bgc[None, None, :] * (1 - m) + ink[None, None, :] * m
    # paper grain and uneven light
    img = img * (0.93 + 0.14 * nprng.random(img.shape[:2])[..., None] * rnd.random())
    img = img + nprng.normal(0, rnd.uniform(0, 12), img.shape)
    img = np.clip(img, 0, 255).astype(np.uint8)
    if rnd.random() < 0.3:
        from PIL import Image as I
        buf = io.BytesIO(); I.fromarray(img).save(buf, "JPEG", quality=rnd.choice([30, 50, 70]))
        img = np.asarray(I.open(buf).convert("RGB"))
    return img


def _work(args):
    seed, n, which = args
    rnd = random.Random(seed); nprng = np.random.default_rng(seed)
    fonts = font_list(which); pool = word_pool()
    imgs, labels = [], []
    while len(imgs) < n:
        lab = make_label(rnd, pool)
        try:
            im = render_word(lab, rnd.choice(fonts), rnd, nprng)
        except Exception:
            continue
        if im.shape[1] / im.shape[0] > 8:
            continue
        imgs.append(im); labels.append(lab)
    return imgs, labels


def cmd_feats(arch, out, n=40000, which="train", seed=1):
    from multiprocessing import Pool
    from onnxtr.models.preprocessor import PreProcessor
    sess, W, B = feature_session(arch)
    pre = PreProcessor((32, 128), batch_size=256, mean=MEAN, std=STD, preserve_aspect_ratio=True)
    chunks = [(seed * 1000 + i, 500, which) for i in range(n // 500)]
    feats, labels = [], []
    t0 = time.time()
    with Pool(2) as pool:
        for k, (imgs, labs) in enumerate(pool.imap(_work, chunks)):
            off = 0
            for batch in pre(imgs):
                o = sess.run(None, {"input": np.ascontiguousarray(batch.transpose(0, 3, 1, 2))})
                feats.append(o[1].astype(np.float16)); labels += labs[off:off + len(batch)]; off += len(batch)
            if k % 10 == 0:
                print(f"{which} {len(labels)}/{n} {time.time()-t0:.0f}s", flush=True)
    F = np.concatenate(feats)
    np.savez_compressed(out, F=F, labels=np.array(labels))
    print("saved", out, F.shape)


# ----------------------------------------------------------------------------- training
def cmd_train(arch, trainf, heldf, out, epochs=6):
    import torch
    from rapidfuzz.distance import Levenshtein as L
    torch.set_num_threads(2); torch.manual_seed(0)
    V = french_vocab() + NEWCH
    blank = len(V)
    sess, W0, B0 = feature_session(arch)
    nold = len(french_vocab())
    # init: old rows from the original weights, new rows copied from their base letter, blank last
    Wn = np.zeros((W0.shape[0], blank + 1), np.float32); Bn = np.zeros(blank + 1, np.float32)
    Wn[:, :nold] = W0[:, :nold]; Bn[:nold] = B0[:nold]
    Wn[:, blank] = W0[:, nold]; Bn[blank] = B0[nold]
    for i, c in enumerate(NEWCH):
        j = V.index(BASE[c]); Wn[:, nold + i] = W0[:, j]; Bn[nold + i] = B0[j] - 1.0
    tr = np.load(trainf); he = np.load(heldf)
    okc = set(V)
    def keep(d):  # drop samples whose label has a character outside the vocabulary (a few typographic quotes)
        L = [str(x) for x in d["labels"]]; ix = [i for i, x in enumerate(L) if set(x) <= okc]
        return torch.from_numpy(d["F"][ix]), [L[i] for i in ix]
    Ftr, Ltr = keep(tr); Fhe, Lhe = keep(he)
    W = torch.nn.Parameter(torch.from_numpy(Wn)); B = torch.nn.Parameter(torch.from_numpy(Bn))
    opt = torch.optim.Adam([W, B], lr=2e-3)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=2e-3, total_steps=epochs * (len(Ltr) // 256 + 1))
    ctc = torch.nn.CTCLoss(blank=blank, zero_infinity=True)
    enc = lambda s: [V.index(c) for c in s]

    def decode(F):
        with torch.no_grad():
            lg = (F.float() @ W + B).argmax(-1).numpy()
        out = []
        for seq in lg:
            prev, s = -1, []
            for k in seq:
                if k != prev and k != blank:
                    s.append(V[k])
                prev = k
            out.append("".join(s))
        return out

    def synth_report(tag):
        hyp = decode(Fhe); res = {}
        ok = sum(h == r for h, r in zip(hyp, Lhe)); res["word_exact"] = ok / len(Lhe)
        per = {c: [0, 0] for c in NEWCH}
        for h, r in zip(hyp, Lhe):
            ops = L.opcodes(r, h)
            for op in ops:
                if op.tag == "equal":
                    for i in range(op.src_start, op.src_end):
                        if r[i] in per:
                            per[r[i]][0] += 1
            for c in per:
                per[c][1] += r.count(c)
        res["diacritic_char_recall"] = {c: (a / b if b else None) for c, (a, b) in per.items()}
        tot = sum(a for a, b in per.values()); den = sum(b for a, b in per.values())
        res["diacritic_char_recall_all"] = tot / max(1, den)
        print(tag, json.dumps(res, ensure_ascii=False), flush=True)
        return res

    synth_report("heldout-fonts, before training (new rows = copies of base letters)")
    N = len(Ltr)
    for ep in range(epochs):
        perm = torch.randperm(N); tl = 0
        for s in range(0, N, 256):
            idx = perm[s:s + 256]
            x = Ftr[idx].float()
            lg = (x @ W + B).log_softmax(-1).transpose(0, 1)
            tg = [enc(Ltr[i]) for i in idx.tolist()]
            tgt = torch.tensor([c for t in tg for c in t]); tl_ = torch.tensor([len(t) for t in tg])
            il = torch.full((len(idx),), lg.shape[0], dtype=torch.long)
            loss = ctc(lg, tgt, il, tl_)
            opt.zero_grad(); loss.backward(); opt.step(); sched.step(); tl += loss.item()
        print(f"epoch {ep} train ctc {tl/(N/256):.4f}", flush=True)
        res = synth_report(f"heldout-fonts epoch {ep}")
    np.savez(out, W=W.detach().numpy(), B=B.detach().numpy(), vocab=V, synth=json.dumps(res, ensure_ascii=False))
    print("saved", out)


def cmd_splice(arch, head, outonnx, outvocab):
    import onnx
    from onnx import numpy_helper
    h = np.load(head)
    m = onnx.load(base_model(arch))
    add, mm, feat, wn, bn = find_head(m)
    for t in m.graph.initializer:
        if t.name == wn:
            t.CopyFrom(numpy_helper.from_array(h["W"].astype(np.float32), wn))
        elif t.name == bn:
            t.CopyFrom(numpy_helper.from_array(h["B"].astype(np.float32), bn))
    d = m.graph.output[0].type.tensor_type.shape.dim
    d[-1].dim_value = h["W"].shape[1]
    onnx.checker.check_model(m)
    onnx.save(m, outonnx)
    open(outvocab, "w", encoding="utf8").write(str(h["vocab"]))
    print("spliced", outonnx, os.path.getsize(outonnx), "bytes; vocab", len(str(h["vocab"])), "+ blank")


# ----------------------------------------------------------------------------- real-page evaluation (same as 4.31 + CER with diacritics)
IMG, REF = "/tmp/ocr_eval/img", "/tmp/ref3"
PAGES = (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52); GOOD = (4, 7, 8, 19, 27, 31, 38, 50)


def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()


def nd(s):
    return "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c))


def words(t):
    return Counter(w for w in re.findall(r"\w+", t.lower()) if len(w) >= 3)


def score(pairs):
    """pairs: {page: (ref, hyp)} -> prints and returns metrics for ALL12 and GOOD8."""
    from rapidfuzz.distance import Levenshtein as L
    R = {}
    for p, (Rf, H) in pairs.items():
        rw, hw, rwn, hwn = words(Rf), words(H), words(nd(Rf)), words(nd(H))
        R[p] = (L.distance(nd(Rf), nd(H)), L.distance(Rf, H), len(Rf), len(nd(Rf)),
                sum((rw & hw).values()), sum((rwn & hwn).values()), sum(rw.values()))
    res = {}
    for name, ps in (("ALL12", PAGES), ("GOOD8", GOOD)):
        s = [sum(R[p][i] for p in ps) for i in range(7)]
        res[name] = dict(cer_blind=100 * s[0] / s[3], cer_diacritics=100 * s[1] / s[2],
                         recall_exact=100 * s[4] / s[6], recall_blind=100 * s[5] / s[6])
        print(name, " ".join(f"{k} {v:.2f}" for k, v in res[name].items()))
    return res


def cmd_eval(det, arch, model=None, vocab=None):
    from onnxtr.io import DocumentFile
    from onnxtr.models import ocr_predictor, recognition
    if model:
        reco = getattr(recognition, arch)(model_path=model, vocab=open(vocab, encoding="utf8").read())
    else:
        reco = arch
    m = ocr_predictor(det_arch=det, reco_arch=reco, assume_straight_pages=True)
    pairs = {}; t0 = time.time()
    tag = "new" if model else "base"
    for p in PAGES:
        H = norm(m(DocumentFile.from_images(f"{IMG}/p{p}.png")).render().replace("\n", " "))
        open(f"{WORK}/{tag}_{det}_{arch}_p{p}.txt", "w").write(H)
        pairs[p] = (norm(open(f"{REF}/p{p}.txt", encoding="utf8").read()), H)
    print(f"{det}+{arch} [{tag}] seconds {time.time()-t0:.0f}")
    return score(pairs)


def cmd_tess():
    import subprocess
    pairs = {}
    for p in PAGES:
        H = norm(subprocess.run(["tesseract", f"{IMG}/p{p}.png", "-", "-l", "ron"], capture_output=True, text=True).stdout)
        pairs[p] = (norm(open(f"{REF}/p{p}.txt", encoding="utf8").read()), H)
    print("tesseract ron")
    return score(pairs)


if __name__ == "__main__":
    a = sys.argv[1:]
    opt = lambda k, d: type(d)(a[a.index(k) + 1]) if k in a else d
    if a[0] == "feats":
        cmd_feats(a[1], a[2], opt("--n", 40000), opt("--fonts", "train"), opt("--seed", 1))
    elif a[0] == "train":
        cmd_train(a[1], a[2], a[3], a[4], opt("--epochs", 6))
    elif a[0] == "splice":
        cmd_splice(*a[1:5])
    elif a[0] == "real":
        cmd_eval(a[1], a[2], *(a[3:5] if len(a) >= 5 else (None, None)))
    elif a[0] == "tess":
        cmd_tess()
    else:
        print(__doc__)
