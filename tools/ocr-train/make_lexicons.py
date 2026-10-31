"""Build crates/legatorocr-nocr/lexicons/<lang>.txt.gz from the `wordfreq` package's lists
(data licence: CC BY-SA 4.0, see crates/legatorocr-nocr/lexicons/README.md)."""
import gzip, os, sys
from wordfreq import top_n_list
from charset import CHARSET
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
ok = set(CHARSET)
for l in ["ro", "en", "fr", "it", "es", "pt", "de", "hu"]:
    ws, seen = [], set()
    for w in top_n_list(l, 60000):
        w = w.lower()
        if len(w) >= 2 and w.isalpha() and all(c in ok for c in w) and w not in seen:
            seen.add(w)
            ws.append(w)
        if len(ws) >= 40000:
            break
    with gzip.open(f"{out}/{l}.txt.gz", "wt", encoding="utf-8", compresslevel=9) as f:
        f.write("\n".join(ws) + "\n")
    print(l, len(ws))
