#!/usr/bin/env python3
"""Command-line fuzzing of the `legatorocr` binary: random flag combinations with hostile
values (NaN, huge, negative, unicode, empty, option-like strings) against small, corrupt
and multi-page inputs. Passes when every run ends with exit status 0, 1 or 2, never with
a panic message, a signal (abort/segfault/OOM) or a timeout.

  scripts/fuzz_cli.py [--bin target/release/legatorocr] [--runs 300] [--seed 1]
"""
import argparse, os, random, subprocess, sys, tempfile, shutil

ap = argparse.ArgumentParser()
ap.add_argument("--bin", default="target/release/legatorocr")
ap.add_argument("--runs", type=int, default=300)
ap.add_argument("--seed", type=int, default=1)
a = ap.parse_args()
rnd = random.Random(a.seed)

try:
    from PIL import Image, ImageDraw
except ImportError:
    sys.exit("needs Pillow to make test inputs")

work = tempfile.mkdtemp(prefix="lgr-clifuzz-")
def mk(name, w, h, text=True):
    im = Image.new("RGB", (w, h), (238, 232, 215)); d = ImageDraw.Draw(im)
    if text:
        for y in range(h // 8, h - h // 8, 14):
            for x in range(w // 8, w - w // 8, 6):
                if (x // 6) % 2: d.rectangle([x, y, x + 3, y + 6], fill=(25, 25, 25))
    p = os.path.join(work, name); im.save(p, dpi=(300, 300)); return p
good = [mk("a.png", 600, 800), mk("b.png", 900, 500), mk("c.png", 40, 40), mk("blank.png", 500, 500, False)]
tif = os.path.join(work, "m.tif")
Image.open(good[0]).convert("L").save(tif, save_all=True, append_images=[Image.open(good[1]).convert("L")])
open(os.path.join(work, "bad.png"), "wb").write(b"\x89PNG\r\n\x1a\n" + os.urandom(200))
open(os.path.join(work, "empty.png"), "wb").write(b"")
inputs = good + [tif, tif + "#2", tif + "#9", os.path.join(work, "bad.png"), os.path.join(work, "empty.png"), os.path.join(work, "nope.png"), work]

nums = ["0", "-1", "1", "0.5", "7", "300", "99999999999", "1e30", "-1e30", "nan", "inf", "-inf", "", " ", "1,5", "٣", "0x10", "2400", "36", "35", "200"]
words = ["", "auto", "bogus", "--pdf", "-", "ron+eng", "../x", "a b", "é日本", "x" * 300, "$(id)", "a;b"]
opts = {
    "--mode": ["color", "gray", "bw", "mixed", "hybrid", "x"], "--split": ["auto", "off", "middle", "x"],
    "--threshold": ["sauvola", "wolf", "niblack", "nick", "bradley", "otsu", "x"], "--align": ["center", "top-left", "bottom-right", "x-y", ""],
    "--dpi": nums, "--margin": nums, "--sauvola-k": nums, "--skew": nums, "--max-skew": nums, "--rotate": nums,
    "--output-dpi": nums, "--hybrid-dpi": nums, "--picture-sensitivity": nums, "--pdf-quality": nums,
    "--page-size": ["a4", "a5", "letter", "100x200", "0x0", "-5x10", "1e9x1e9", "axb", "", "5000x5000", "10x10"],
    "--psm": nums, "--min-conf": nums, "--lang": words, "--format": ["png", "tiff", "none", "x"],
}
flags = ["--no-deskew", "--no-despeckle", "--normalize", "--dewarp", "--uniform-size", "--skip-blank", "--keep-going", "--no-auto-orient"]
bad = 0
for i in range(a.runs):
    args = [a.bin]
    for _ in range(rnd.randint(1, 4)): args.append(rnd.choice(inputs))
    for k in rnd.sample(list(opts), rnd.randint(0, 6)):
        args += [k, rnd.choice(opts[k])]
    args += rnd.sample(flags, rnd.randint(0, 4))
    if rnd.random() < 0.8: args += ["--pdf", os.path.join(work, f"o{i % 5}.pdf")]
    if rnd.random() < 0.4: args += ["--ocr"] if rnd.random() < 0.3 else []
    if "--format" not in args: args += ["--format", rnd.choice(["none", "png", "tiff"]), "-o", os.path.join(work, "out")]
    rnd.shuffle(args[1:]) if False else None
    try:
        p = subprocess.run(args, capture_output=True, timeout=90, env={**os.environ, "RUST_BACKTRACE": "0"})
        err = p.stderr.decode("utf8", "replace")
        ok = p.returncode in (0, 1, 2) and "panicked" not in err
        why = f"rc={p.returncode}" + (" PANIC" if "panicked" in err else "")
    except subprocess.TimeoutExpired:
        ok, why, err = False, "TIMEOUT", ""
    if not ok:
        bad += 1
        print(f"[{i}] FAIL {why}: {' '.join(map(repr, args[1:]))}\n{err[-400:]}")
shutil.rmtree(work, ignore_errors=True)
print(f"{a.runs} runs, {bad} failures")
sys.exit(1 if bad else 0)
