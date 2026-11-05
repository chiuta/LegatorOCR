"""Turn the JSON saved by a `--verify-pack` page into training lines.

Usage: import_verify.py verified.json [more.json ...] OUT_DIR OUT.tsv [--only-corrected]
Every answered word becomes OUT_DIR/vNNNNN.png (grey, height 32, ink dark on white, like the lines
of dump_lines) and a row "png<TAB>label" in OUT.tsv, the format of train.py's REAL variable.
Words whose label has a character outside the model's charset are skipped. Confirmed words are
words the engine had read correctly although unsure: they are kept (they teach it to be right
about hard words) unless --only-corrected is given. HELD_PAGES / REAL_HELD of train.py do not apply
to these files: keep some pages out of the pack yourself if you want an honest evaluation."""
import base64, io, json, os, sys
from PIL import Image
from charset import CHARSET

args = [a for a in sys.argv[1:] if not a.startswith("--")]
only_corr = "--only-corrected" in sys.argv
*jsons, out_dir, out_tsv = args
os.makedirs(out_dir, exist_ok=True)
cs = set(CHARSET)
n = skipped = 0
with open(out_tsv, "w", encoding="utf8") as f:
    for jf in jsons:
        for it in json.load(open(jf, encoding="utf8")):
            lab = it["text"].strip()
            if not lab or any(c not in cs for c in lab) or (only_corr and not it["corrected"]):
                skipped += 1
                continue
            im = Image.open(io.BytesIO(base64.b64decode(it["jpeg"]))).convert("L")
            w = max(8, round(im.width * 32 / im.height))
            png = os.path.join(out_dir, f"v{n:05d}.png")
            im.resize((w, 32), Image.BILINEAR).save(png)
            f.write(f"{png}\t{lab}\n")
            n += 1
print(f"{n} lines written, {skipped} skipped")
