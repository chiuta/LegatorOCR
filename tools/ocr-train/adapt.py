"""Adapt an LNM1 model to the lines of ONE book by training only the top of the network.

Usage: adapt.py IN.lnm LINES.tsv OUT.lnm [LAYERS] [EPOCHS]
LAYERS = fc (output layer only), gru2 (second recurrent layer + output) or gru (both recurrent layers + output).
The lines below the trained part are run once and their features cached, so an epoch is cheap.
LINES.tsv: png path <TAB> label (from agree_labels.py or real_labels.py).
"""
import sys, time, os
import numpy as np, torch, torch.nn as nn
from PIL import Image
import train as T
from charset import CHARSET

inp, tsv, out = sys.argv[1:4]
layers = sys.argv[4] if len(sys.argv) > 4 else "fc"
epochs = int(sys.argv[5]) if len(sys.argv) > 5 else 30
torch.set_num_threads(int(os.environ.get("TT", "2")))
net = T.Net()
T.load_lnm(net, inp)
net.eval()
rows = []
for ln in open(tsv, encoding="utf8"):
    png, lab = ln.rstrip("\n").split("\t", 1)
    a = np.asarray(Image.open(png).convert("L"))
    W = (a.shape[1] + 3) // 4 * 4
    x = np.zeros((1, 1, a.shape[0], W), np.float32)
    x[0, 0, :, :a.shape[1]] = (255 - a) / 255.0
    rows.append((torch.from_numpy(x), T.encode(lab), lab))

pools = [(2, 2), (2, 2), (2, 1), (2, 1)]
def low(x):
    for conv, bn, p in zip(net.conv, net.bn, pools):
        x = torch.nn.functional.max_pool2d(torch.relu(bn(conv(x))), p)
    b, c, h, w = x.shape
    x = x.permute(0, 3, 1, 2).reshape(b, w, c * h)
    if layers != "gru":
        x, _ = net.gru1(x)
    if layers == "fc":
        x, _ = net.gru2(x)
    return x
with torch.no_grad():
    feats = [low(x) for x, _, _ in rows]
def top(f):
    if layers == "gru":
        f, _ = net.gru1(f)
    if layers in ("gru", "gru2"):
        f, _ = net.gru2(f)
    return net.fc(f)
params = list(net.fc.parameters())
if layers in ("gru2", "gru"): params += list(net.gru2.parameters())
if layers == "gru": params += list(net.gru1.parameters())
for p in net.parameters(): p.requires_grad_(False)
for p in params: p.requires_grad_(True)
opt = torch.optim.Adam(params, 3e-4)
ctc = nn.CTCLoss(zero_infinity=True)
order = list(range(len(rows)))
t0 = time.time()
for ep in range(epochs):
    np.random.shuffle(order)
    tot = 0
    for i in range(0, len(order), 16):
        idx = order[i:i + 16]
        # pad features to a common length
        L = max(feats[j].shape[1] for j in idx)
        F = torch.zeros(len(idx), L, feats[idx[0]].shape[2])
        for k, j in enumerate(idx): F[k, :feats[j].shape[1]] = feats[j][0]
        lp = top(F).log_softmax(-1).permute(1, 0, 2)
        tg = torch.tensor([c for j in idx for c in rows[j][1]])
        loss = ctc(lp, tg, torch.tensor([feats[j].shape[1] for j in idx]), torch.tensor([len(rows[j][1]) for j in idx]))
        opt.zero_grad(); loss.backward(); opt.step(); tot += loss.item()
    print(f"epoch {ep + 1} loss {tot / (len(order) / 16):.4f}  {time.time() - t0:.0f}s", flush=True)
T.export(net, out)
print("wrote", out)
