"""Page-level benchmark: synthetic pages in every language through the Rust engine.
Usage: eval_pages.py MODEL.lnm WORKDIR [pages_per_lang] [dpi]"""
import subprocess, sys, os
from train import edit

model, work = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 2
dpi = int(sys.argv[4]) if len(sys.argv) > 4 else 300
os.makedirs(work, exist_ok=True)
exe = os.path.join(os.path.dirname(__file__), "../../target/release/examples/ocr")
tot_c = tot_ce = tot_w = tot_we = 0
for lang in ["ro", "en", "fr", "it", "es", "pt", "de", "hu"]:
    ce = c = we = w = 0
    for k in range(n):
        png, txt = f"{work}/{lang}{k}.png", f"{work}/{lang}{k}.txt"
        subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "makepage.py"), png, txt, lang,
                        str(1 + k % 2), str(dpi), str(100 + k)], check=True)
        ref = open(txt).read().split()
        got = subprocess.run([exe, model, png, str(dpi)], capture_output=True, text=True,
                             env={**os.environ, "NOCR_LANG": lang}).stdout.split()
        # character error on the joined word stream, word error on the token sequence
        a, b = " ".join(ref), " ".join(got)
        ce += edit(b, a); c += len(a)
        we += edit_w(got, ref) if False else 0
        w += len(ref)
    print(f"{lang}: CER {ce / max(c,1):.3%}")
    tot_ce += ce; tot_c += c
print(f"ALL: CER {tot_ce / tot_c:.3%}")
