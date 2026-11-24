#!/usr/bin/env python3
"""Every option of `legatorocr --help` must be named in each of the eight READMEs.
Usage: scripts/check-readme-options.py [path/to/legatorocr]   (exit 1 when an option is missing somewhere)"""
import re, subprocess, sys, pathlib
root = pathlib.Path(__file__).resolve().parent.parent
exe = next((a for a in sys.argv[1:] if not a.startswith("--")), str(root / "target/release/legatorocr"))
h = subprocess.run([exe, "--help"], capture_output=True, text=True).stdout
opts = sorted(set(re.findall(r"^\s+(?:-\w, )?(--[a-z0-9][a-z0-9-]*)", h, re.M)))
bad = 0
for f in ["README.md"] + [f"README.{l}.md" for l in "ro de es fr hu it pt".split()]:
    t = (root / f).read_text(encoding="utf8")
    miss = [o for o in opts if not re.search(re.escape(o) + r"(?![a-z0-9-])", t)]
    print(f"{f:14s} {len(opts) - len(miss)}/{len(opts)}", miss if miss else "")
    bad += len(miss)
sys.exit(1 if bad else 0)
