"""Roadmap 4.22 gate experiment: word-image voting with a per-book lexicon.
Idea: in one book the same word recurs (terms, names, headings). Word crops of all pages are
size-normalised and clustered by normalised correlation; inside a cluster of >= MINN members the
strict majority reading replaces minority readings (ties: no change).
Usage: word_vote.py [WORKDIR]   (engines from EVAL_ENGINE list, default 'tesseract,native')
Runs legatorocr --ocr --json on the 12 pages (cached in WORKDIR), crops words from the processed
page images the CLI writes, and reports CER before/after (reference /tmp/ref3 raw ABBYY text, same
normalisation as tools/ocr-train/eval_real.py) for ALL12 and GOOD8, plus words changed, and
per-change effect (each change applied alone: fixed an error / made an error / neutral)."""
import subprocess, sys, os, re, json, unicodedata
import numpy as np
from PIL import Image
from rapidfuzz.distance import Levenshtein as L

IMG, REF = "/tmp/ocr_eval/img", "/tmp/ref3"
EXE = os.environ.get("LEGATOR", "/home/claude/legatorocr/target/release/legatorocr")
PAGES = (4, 7, 8, 13, 19, 21, 27, 31, 38, 45, 50, 52)
GOOD = (4, 7, 8, 19, 27, 31, 38, 50)
WORK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/wv_work"
GW, GH = 48, 16  # comparison grid

def norm(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return re.sub(r"\s+", " ", s).strip()

_cache = {}
def run(engine, p):
    if (engine, p) in _cache: return _cache[(engine, p)]
    d = f"{WORK}/{engine}/p{p}"
    if not os.path.exists(f"{d}/o.json"):
        os.makedirs(d, exist_ok=True)
        subprocess.run([EXE, f"{IMG}/p{p}.png", "--pdf", f"{d}/x.pdf", "--ocr", "--lang", "ron", "--engine", engine,
                        "--json", f"{d}/o.json", "-o", f"{d}/pages"], capture_output=True)
    j = json.load(open(f"{d}/o.json"))
    img = np.asarray(Image.open(f"{d}/pages/p{p}.png").convert("L")).astype(np.float32)
    _cache[(engine, p)] = (j["pages"][0]["words"], img)
    return _cache[(engine, p)]

def feat(img, w):
    x, y, ww, hh = w["x"], w["y"], w["w"], w["h"]
    c = img[max(0, y):y + hh, max(0, x):x + ww]
    if c.shape[0] < 6 or c.shape[1] < 6: return None
    v = np.asarray(Image.fromarray(c.astype(np.uint8)).resize((GW, GH), Image.BILINEAR)).astype(np.float32).ravel()
    v -= v.mean(); n = np.linalg.norm(v)
    return None if n < 1e-3 else v / n

REFS = {p: norm(open(f"{REF}/p{p}.txt", encoding="utf8").read()) for p in PAGES}

def cer_pages(texts):
    tot = {"all": [0, 0], "good": [0, 0]}; per = {}
    for p in PAGES:
        R = REFS[p]; H = norm(" ".join(texts[p]))
        e = L.distance(R, H); per[p] = 100 * e / len(R)
        for k in ("all",) + (("good",) if p in GOOD else ()):
            tot[k][0] += e; tot[k][1] += len(R)
    return tot["all"][0], tot["good"][0], tot["all"][1], tot["good"][1], per

def experiment(engine, thr, minn, frac, ratio):
    pages = {p: run(engine, p) for p in PAGES}
    items = []  # (page, index, text, vec, aspect)
    for p, (ws, img) in pages.items():
        for i, w in enumerate(ws):
            t = w["text"]
            if len(t) < 3 or not any(ch.isalpha() for ch in t): continue
            v = feat(img, w)
            if v is not None: items.append((p, i, t, v, w["w"] / max(1, w["h"])))
    V = np.stack([it[3] for it in items]); A = np.array([it[4] for it in items])
    # greedy leader clustering: each word joins the best cluster whose leader correlates > thr
    leaders, cl = [], []
    for k, v in enumerate(V):
        best = -1
        if leaders:
            s = V[leaders] @ v
            ok = np.where((s > thr) & (np.abs(np.log(A[leaders] / A[k])) < ratio))[0]
            if len(ok): best = int(ok[np.argmax(s[ok])])
        if best < 0: leaders.append(k); best = len(leaders) - 1
        cl.append(best)
    groups = {}
    for k, c in enumerate(cl): groups.setdefault(c, []).append(k)
    changes = []  # (page, index, old, new)
    for g in groups.values():
        if len(g) < minn: continue
        cnt = {}
        for k in g: cnt[items[k][2]] = cnt.get(items[k][2], 0) + 1
        top = sorted(cnt.items(), key=lambda a: -a[1])
        if len(top) < 2 or top[0][1] == top[1][1] or top[0][1] / len(g) < frac: continue
        maj = top[0][0]
        for k in g:
            if items[k][2] != maj and cnt[items[k][2]] < top[0][1]:
                changes.append((items[k][0], items[k][1], items[k][2], maj))
    base = {p: [w["text"] for w in pages[p][0]] for p in PAGES}
    new = {p: list(v) for p, v in base.items()}
    for p, i, o, n in changes: new[p][i] = n
    b, bg, n_all, n_good, bp = cer_pages(base); a, ag, _, _, ap = cer_pages(new)
    fixed = made = 0
    for p, i, o, n in changes:
        t = list(base[p]); d0 = L.distance(REFS[p], norm(" ".join(t))); t[i] = n
        d1 = L.distance(REFS[p], norm(" ".join(t)))
        fixed += d1 < d0; made += d1 > d0
    worst = max(ap[p] - bp[p] for p in PAGES)
    print(f"{engine:9s} thr={thr} minn={minn} frac={frac}: clusters>={minn}: {sum(len(g)>=minn for g in groups.values())}, "
          f"ALL12 err {b}->{a} ({100*b/n_all:.2f}->{100*a/n_all:.2f}%) GOOD8 err {bg}->{ag} ({100*bg/n_good:.2f}->{100*ag/n_good:.2f}%) "
          f"changed {len(changes)} fixed {fixed} made_error {made} neutral {len(changes)-fixed-made} worst page delta {worst:+.2f}", flush=True)

if __name__ == "__main__":
    for eng in os.environ.get("EVAL_ENGINE", "tesseract,native").split(","):
        for thr in (0.97, 0.94, 0.90, 0.85):
            experiment(eng, thr, 3, 0.5, 0.12)
