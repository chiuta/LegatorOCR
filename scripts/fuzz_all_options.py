#!/usr/bin/env python3
"""Fuzzes EVERY option the command line lists in `--help` (scripts/fuzz_cli.py only knows the old ones).
Each option is run alone, with hostile values, on one small page. Passes when every run ends with exit
status 0, 1 or 2, never with a panic, a signal or a timeout.

  python3 scripts/fuzz_all_options.py [target/release/legatorocr] [--seed N] [--per-option K]
"""
import argparse, os, random, re, subprocess, sys, tempfile, shutil
ap = argparse.ArgumentParser()
ap.add_argument("exe", nargs="?", default="target/release/legatorocr")
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--per-option", type=int, default=6)
ap.add_argument("--timeout", type=int, default=45)
a = ap.parse_args()
exe = os.path.abspath(a.exe)
rnd = random.Random(a.seed)
from PIL import Image, ImageDraw
work = tempfile.mkdtemp(prefix="lgr-fuzzall-")
im = Image.new("L", (700, 900), 235); d = ImageDraw.Draw(im)
for y in range(80, 820, 22):
    for x in range(70, 630, 9):
        if (x // 9) % 3: d.rectangle([x, y, x + 5, y + 9], fill=40)
page = os.path.join(work, "p.png"); im.save(page, dpi=(200, 200))
open(os.path.join(work, "bin.dat"), "wb").write(os.urandom(4096))
open(os.path.join(work, "empty.txt"), "wb").write(b"")
open(os.path.join(work, "utf16.txt"), "wb").write("ăîșț\n".encode("utf-16"))
open(os.path.join(work, "bad.txt"), "wb").write(b"\xff\xfe\x00\xd8abc\n" * 50)
open(os.path.join(work, "big.txt"), "w").write("cuvânt\n" * 400000)
open(os.path.join(work, "pat.txt"), "w").write("\\d\\d\\d-\\d\\d\n(((\n[\n")
os.mkdir(os.path.join(work, "dir"))
help_text = subprocess.run([exe, "--help"], capture_output=True, text=True).stdout
opts = []   # (name, takes_value)
for m in re.finditer(r"^\s+(?:-\w, )?(--[a-z0-9][a-z0-9-]*)(?:[ =]<([^>]+)>)?\s*$", help_text, re.M):
    opts.append((m.group(1), m.group(2)))
SKIP = {"--help", "--version", "--watch", "--serve", "--mcp", "--mcp-server", "--update", "--self-update", "--scan-network", "--listen", "--gui", "--wait", "--interactive"}
nums = ["0", "-1", "1", "0.5", "7", "300", "99999999999", "1e30", "-1e30", "nan", "inf", "", " ", "1,5", "٣", "0x10", "2400"]
texts = ["", "-", "--pdf", "../../x", "a b", "é日本", "x" * 400, "$(id)", "a;b", "%s%n", "\\", "{n}{total}{", "0,0,0,0", "1,2,3", "9999,9999,1,1", "-5,-5,10,10", "#zzz", "2026-13-45"]
paths = [os.path.join(work, n) for n in ("bin.dat", "empty.txt", "utf16.txt", "bad.txt", "big.txt", "pat.txt", "dir", "nope.txt", "p.png")] + ["/dev/null", "/proc/self/mem", work + "/x/y/z/out"]
def vals(name, ph):
    ph = (ph or "").upper()
    if any(k in ph for k in ("FILE", "PATH", "DIR", "FOLDER")) or any(k in name for k in ("-out", "-file", "-dir", "-folder")):
        return paths + texts[:4]
    if any(k in ph for k in ("N", "SECONDS", "DPI", "PERCENT", "MB", "K", "X", "NUM")) and not ph.startswith("MODE"):
        return nums + texts[:6]
    return texts + nums[:6]
bad = 0; runs = 0
for name, ph in opts:
    if name in SKIP: continue
    cases = [[name]] if ph is None else [[name, v] for v in rnd.sample(vals(name, ph), min(a.per_option, len(vals(name, ph))))]
    for extra in cases:
        out = os.path.join(work, "o"); shutil.rmtree(out, ignore_errors=True)
        args = [exe, page, "--pdf", os.path.join(work, "o.pdf"), "--format", "none"] + extra
        runs += 1
        try:
            p = subprocess.run(args, capture_output=True, timeout=a.timeout, stdin=subprocess.DEVNULL, cwd=work, env={**os.environ, "RUST_BACKTRACE": "0"})
            err = p.stderr.decode("utf8", "replace")
            ok = p.returncode in (0, 1, 2) and "panicked" not in err
            why = f"rc={p.returncode}" + (" PANIC" if "panicked" in err else "")
        except subprocess.TimeoutExpired:
            ok, why, err = False, "TIMEOUT", ""
        if not ok:
            bad += 1
            print("FAIL", why, " ".join(repr(x) for x in extra), "\n   ", err.strip()[-300:].replace("\n", "\n    "), flush=True)
shutil.rmtree(work, ignore_errors=True)
print(f"{len(opts)} options found, {runs} runs, {bad} failures")
sys.exit(1 if bad else 0)
