"""Train the LegatorOCR line recogniser (CNN + 2x BiGRU + CTC) on synthetic text lines and export
it to the LNM1 binary read by crates/legatorocr-nocr. Usage: python train.py OUT_DIR [steps]"""
import sys, os, time, random, struct
import numpy as np
import torch, torch.nn as nn, torch.nn.functional as F
from charset import CHARSET
import synth

NC = len(CHARSET) + 1
C = [16, 32, 64, 64]
HID = 96


class Net(nn.Module):
    def __init__(s):
        super().__init__()
        s.conv = nn.ModuleList()
        s.bn = nn.ModuleList()
        cin = 1
        for c in C:
            s.conv.append(nn.Conv2d(cin, c, 3, padding=1))
            s.bn.append(nn.BatchNorm2d(c))
            cin = c
        s.gru1 = nn.GRU(C[3] * 2, HID, batch_first=True, bidirectional=True)
        s.gru2 = nn.GRU(HID * 2, HID, batch_first=True, bidirectional=True)
        s.fc = nn.Linear(HID * 2, NC)
        s.drop = nn.Dropout(0.15)

    def forward(s, x):  # x: B,1,32,W in 0..1 (1 = ink)
        pools = [(2, 2), (2, 2), (2, 1), (2, 1)]
        for conv, bn, p in zip(s.conv, s.bn, pools):
            x = F.max_pool2d(F.relu(bn(conv(x))), p)
        b, c, h, w = x.shape  # h = 2
        x = x.permute(0, 3, 1, 2).reshape(b, w, c * h)
        x, _ = s.gru1(x)
        x = s.drop(x)
        x, _ = s.gru2(x)
        return s.fc(s.drop(x))  # B,T,NC   (T = W/4)


def encode(t):
    return [CHARSET.index(c) + 1 for c in t]


def batches(seed, bs=32, chunk=320):
    rng = random.Random(seed)
    fonts = synth.load_fonts()
    while True:
        items = [synth.sample(fonts, rng) for _ in range(chunk)]
        items.sort(key=lambda it: it[0].shape[1])
        for i in range(0, chunk, bs):
            grp = items[i:i + bs]
            W = max(a.shape[1] for a, _ in grp)
            W = (W + 3) // 4 * 4
            x = np.zeros((len(grp), 1, synth.H, W), np.float32)
            for j, (a, _) in enumerate(grp):
                x[j, 0, :, :a.shape[1]] = (255 - a) / 255.0
            tg = [encode(t) for _, t in grp]
            yield torch.from_numpy(x), tg, [a.shape[1] // 4 for a, _ in grp], [t for _, t in grp]


class DS(torch.utils.data.IterableDataset):
    def __iter__(self):
        wi = torch.utils.data.get_worker_info()
        return batches(1000 + (wi.id if wi else 0) * 7919 + int(time.time()) % 10000)


def greedy(lp):
    out = []
    for row in lp.argmax(-1).tolist():
        s, prev = [], 0
        for k in row:
            if k != prev and k != 0:
                s.append(CHARSET[k - 1])
            prev = k
        out.append("".join(s))
    return out


def edit(a, b):
    d = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        p, d[0] = d[0], i
        for j, cb in enumerate(b, 1):
            p, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, p + (ca != cb))
    return d[-1]


def validate(net, val):
    net.eval()
    e = n = 0
    with torch.no_grad():
        for x, tg, _, txt in val:
            for o, t in zip(greedy(net(x)), txt):
                e += edit(o, t)
                n += len(t)
    net.train()
    return e / max(n, 1)


def fold(conv, bn):
    s = bn.weight / torch.sqrt(bn.running_var + bn.eps)
    w = conv.weight * s[:, None, None, None]
    b = (conv.bias - bn.running_mean) * s + bn.bias
    return w.detach(), b.detach()


def export(net, path):
    net.eval()
    T = {}
    for i, (conv, bn) in enumerate(zip(net.conv, net.bn)):
        T[f"conv{i}.w"], T[f"conv{i}.b"] = fold(conv, bn)
    for k, g in (("gru1", net.gru1), ("gru2", net.gru2)):
        for suf, name in (("", "f"), ("_reverse", "r")):
            for p in ("weight_ih", "weight_hh", "bias_ih", "bias_hh"):
                T[f"{k}.{name}.{p}"] = getattr(g, f"{p}_l0{suf}").detach()
    T["fc.w"], T["fc.b"] = net.fc.weight.detach(), net.fc.bias.detach()
    with open(path, "wb") as f:
        f.write(b"LNM1")
        cs = CHARSET.encode()
        f.write(struct.pack("<IIII", synth.H, HID, len(C), len(cs)))
        f.write(cs)
        f.write(struct.pack("<I", len(T)))
        for name, t in T.items():
            t = t.contiguous().float()
            nb = name.encode()
            f.write(struct.pack("<I", len(nb)))
            f.write(nb)
            f.write(struct.pack("<I", t.dim()))
            for d in t.shape:
                f.write(struct.pack("<I", d))
            f.write(t.numpy().astype("<f4").tobytes())
    net.train()


if __name__ == "__main__":
    out = sys.argv[1]
    steps = int(sys.argv[2]) if len(sys.argv) > 2 else 60000
    os.makedirs(out, exist_ok=True)
    torch.set_num_threads(int(os.environ.get("TT", "3")))
    net = Net()
    print("params", sum(p.numel() for p in net.parameters()), flush=True)
    opt = torch.optim.AdamW(net.parameters(), 2e-3, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, 3e-3, total_steps=steps, pct_start=0.05)
    ctc = nn.CTCLoss(zero_infinity=True)
    vg = batches(424242)
    val = [next(vg) for _ in range(12)]
    dl = iter(torch.utils.data.DataLoader(DS(), batch_size=None,
                                          num_workers=int(os.environ.get("NW", "2")), prefetch_factor=6))
    t0 = time.time()
    ema = None
    for step in range(1, steps + 1):
        x, tg, _, _ = next(dl)
        lp = net(x).log_softmax(-1)  # B,T,NC
        T = lp.shape[1]
        tgt = torch.tensor([c for t in tg for c in t])
        tl = torch.tensor([len(t) for t in tg])
        loss = ctc(lp.permute(1, 0, 2), tgt, torch.full((x.shape[0],), T, dtype=torch.long), tl)
        opt.zero_grad()
        loss.backward()
        nn.utils.clip_grad_norm_(net.parameters(), 5.0)
        opt.step()
        sched.step()
        ema = loss.item() if ema is None else 0.98 * ema + 0.02 * loss.item()
        if step % 100 == 0:
            print(f"step {step} loss {ema:.3f} {time.time()-t0:.0f}s", flush=True)
        if step % 1000 == 0 or step == steps:
            print(f"== step {step} val CER {validate(net, val):.4f}", flush=True)
            export(net, os.path.join(out, "model.lnm"))
            torch.save(net.state_dict(), os.path.join(out, "ckpt.pt"))
