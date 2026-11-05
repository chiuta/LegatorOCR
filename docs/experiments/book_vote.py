"""Roadmap 4.54 gate: the 4.22 word-image vote and a per-book lexicon, now over a whole book.
4.22 failed on 12 magazine pages because a strict threshold found almost no repeated words there.
Here: 125 pages of the 1960 textbook (native engine, output of `legatorocr --ocr --json`), words
cropped from the processed pages, clustered by normalised correlation; scored on the 5 pages with a
hand transcription (docs/experiments/mobila1960_*_hand_reference.txt, same scoring as
mobila1960_score.py: character recall by alignment, bag-of-words recall).
Usage: book_vote.py [WORKDIR=/tmp/bk]   (WORKDIR/out/mNNN/{o.json,pages/mNNN.png}, refs in /tmp/mobref)"""
import json, os, re, sys, unicodedata, collections, glob
import numpy as np
from PIL import Image
from rapidfuzz.distance import Levenshtein as L

W = sys.argv[1] if len(sys.argv) > 1 else "/tmp/bk"
EVAL = ("m120", "m200", "m250", "m300", "m350")
GW, GH = 48, 16

def norm(s): return re.sub(r"[\s\-—–]+", "", unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")).lower()
def toks(s):
    s = unicodedata.normalize("NFC", s).replace("ş", "ș").replace("ţ", "ț")
    return [t for t in re.findall(r"[^\W\d_]+", s.lower()) if len(t) >= 3]

REF = {p: open(f"/tmp/mobref/{p}.txt", encoding="utf8").read() for p in EVAL}
def score(texts):
    cs = ws = 0
    for p in EVAL:
        r, h = norm(REF[p]), norm(texts[p])
        m = sum(op.src_end - op.src_start for op in L.opcodes(r, h) if op.tag == "equal")
        rc, hc = collections.Counter(toks(REF[p])), collections.Counter(toks(texts[p]))
        cs += 100 * m / len(r); ws += 100 * sum(min(c, hc[t]) for t, c in rc.items()) / sum(rc.values())
    return cs / len(EVAL), ws / len(EVAL)

pages = {}
for d in sorted(glob.glob(f"{W}/out/m*/o.json")):
    n = d.split("/")[-2]
    if not os.path.exists(f"{W}/out/{n}/pages/{n}.png"): continue
    words = json.load(open(d))["pages"][0]["words"]
    img = np.asarray(Image.open(f"{W}/out/{n}/pages/{n}.png").convert("L")).astype(np.float32)
    pages[n] = (words, img)
print("pages", len(pages), "words", sum(len(w) for w, _ in pages.values()))

def feat(img, w):
    x, y, ww, hh = w["x"], w["y"], w["w"], w["h"]
    c = img[max(0, y):y + hh, max(0, x):x + ww]
    if c.shape[0] < 6 or c.shape[1] < 6: return None
    v = np.asarray(Image.fromarray(c.astype(np.uint8)).resize((GW, GH), Image.BILINEAR)).astype(np.float32).ravel()
    v -= v.mean(); n = np.linalg.norm(v)
    return None if n < 1e-3 else v / n

items = []
for n, (ws, img) in pages.items():
    for i, w in enumerate(ws):
        t = w["text"]
        if len(t) < 3 or not any(c.isalpha() for c in t): continue
        v = feat(img, w)
        if v is not None: items.append((n, i, t, v, w["w"] / max(1, w["h"])))
V = np.stack([it[3] for it in items]); A = np.array([it[4] for it in items])
print("candidate words", len(items))

def base_text():
    return {p: " ".join(w["text"] for w in pages[p][0]) for p in EVAL}
b = score(base_text()); print(f"baseline: char {b[0]:.2f}  word {b[1]:.2f}")

def cluster(thr, ratio=0.12):
    leaders, cl = [], []
    for k in range(len(V)):
        best = -1
        if leaders:
            s = V[leaders] @ V[k]
            ok = np.where((s > thr) & (np.abs(np.log(A[leaders] / A[k])) < ratio))[0]
            if len(ok): best = int(ok[np.argmax(s[ok])])
        if best < 0: leaders.append(k); best = len(leaders) - 1
        cl.append(best)
    g = {}
    for k, c in enumerate(cl): g.setdefault(c, []).append(k)
    return g

def apply_vote(groups, minn, frac):
    new = {n: [w["text"] for w in pages[n][0]] for n in pages}
    changed = 0
    for g in groups.values():
        if len(g) < minn: continue
        cnt = collections.Counter(items[k][2] for k in g)
        top, c = cnt.most_common(1)[0]
        if c / len(g) < frac or list(cnt.values()).count(c) > 1: continue
        for k in g:
            n, i, t = items[k][:3]
            if t != top and n in EVAL: new[n][i] = top; changed += 1
    return {p: " ".join(new[p]) for p in EVAL}, changed

for thr in (0.97, 0.94, 0.90, 0.85):
    g = cluster(thr)
    big = sum(1 for x in g.values() if len(x) >= 3)
    for minn, frac in ((3, 0.5), (5, 0.6), (8, 0.7)):
        t, ch = apply_vote(g, minn, frac)
        s = score(t)
        print(f"vote thr {thr} minn {minn} frac {frac}: clusters>=3 {big}, words changed {ch}, char {s[0]:.2f} ({s[0]-b[0]:+.2f}) word {s[1]:.2f} ({s[1]-b[1]:+.2f})")

# per-book lexicon: a word of an evaluation page that is rare in the book (<2) and within edit distance 1 of a
# word seen >= MINF times elsewhere becomes that word (only when the replacement is unique)
freq = collections.Counter(t.lower() for _, _, t, _, _ in items for t in [re.sub(r"[^\w]", "", t)] if len(t) >= 4)
for minf in (3, 5, 10):
    lex = {w for w, c in freq.items() if c >= minf}
    by_len = collections.defaultdict(list)
    for w in lex: by_len[len(w)].append(w)
    new = {n: [w["text"] for w in pages[n][0]] for n in EVAL}; ch = 0
    for n in EVAL:
        for i, w in enumerate(pages[n][0]):
            core = re.sub(r"[^\w]", "", w["text"]).lower()
            if len(core) < 4 or freq[core] >= 2 or w["text"] != w["text"].strip(): continue
            cand = [x for l in (len(core) - 1, len(core), len(core) + 1) for x in by_len[l] if L.distance(core, x) == 1]
            if len(cand) == 1:
                new[n][i] = re.sub(r"\w+", cand[0], w["text"], count=1); ch += 1
    s = score({p: " ".join(new[p]) for p in EVAL})
    print(f"lexicon minf {minf}: lexicon {len(lex)}, words changed {ch}, char {s[0]:.2f} ({s[0]-b[0]:+.2f}) word {s[1]:.2f} ({s[1]-b[1]:+.2f})")
