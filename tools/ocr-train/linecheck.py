"""Cross-check: render synthetic lines, save PNGs + torch's greedy output, so the Rust engine's
result on the same pictures can be compared. Usage: linecheck.py CKPT OUTDIR N"""
import sys, random, os, json
import numpy as np, torch
from PIL import Image
import train, synth
ck, out, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
os.makedirs(out, exist_ok=True)
net = train.Net(); net.load_state_dict(torch.load(ck)); net.eval()
fonts = synth.load_fonts(); rng = random.Random(777)
res = []
for i in range(n):
    a, t = synth.sample(fonts, rng)
    Image.fromarray(a.astype(np.uint8)).save(f"{out}/l{i}.png")
    W = (a.shape[1] + 3) // 4 * 4
    x = np.zeros((1, 1, 32, W), np.float32); x[0, 0, :, :a.shape[1]] = (255 - a) / 255.0
    with torch.no_grad():
        p = train.greedy(net(torch.from_numpy(x)))[0]
    res.append({"file": f"l{i}.png", "truth": t, "torch": p})
json.dump(res, open(f"{out}/ref.json", "w"), ensure_ascii=False, indent=1)
