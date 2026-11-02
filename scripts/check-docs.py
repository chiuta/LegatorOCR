#!/usr/bin/env python3
"""Check that every long option used in docs/tasks/*.md exists in `legatorocr --help`,
and (with --coverage) report translation coverage of the task recipes.

usage: check-docs.py [path/to/legatorocr] [--coverage]
"""
import glob, os, re, subprocess, sys

args = [a for a in sys.argv[1:] if not a.startswith("--")]
exe = args[0] if args else "target/debug/legatorocr"
docs = sorted(glob.glob("docs/tasks/*.md"))

if "--coverage" in sys.argv:
    base = [d for d in docs if re.search(r"/[^./]+\.md$", d) and not d.endswith("README.md")]
    langs = sorted({m.group(1) for d in docs if (m := re.search(r"\.([a-z]{2})\.md$", d))})
    print(f"{len(base)} English recipes")
    for l in langs:
        done = [b for b in base if os.path.exists(b[:-3] + f".{l}.md")]
        print(f"  {l}: {len(done)}/{len(base)} ({100 * len(done) // max(1, len(base))} %)")
    if not langs:
        print("  no translations yet")
    sys.exit(0)

help_text = subprocess.run([exe, "--help"], capture_output=True, text=True).stdout
known = set(re.findall(r"--[a-z0-9][a-z0-9-]*", help_text))
bad = []
used = 0
for d in docs:
    for block in re.findall(r"```\n(.*?)```", open(d, encoding="utf-8").read(), re.S):
        for flag in re.findall(r"(?<![\w-])(--[a-z0-9][a-z0-9-]*)", block):
            used += 1
            if flag not in known:
                bad.append((d, flag))
for d, f in bad:
    print(f"{d}: unknown option {f}")
print(f"{used} option uses checked in {len(docs)} files, {len(bad)} unknown")
sys.exit(1 if bad else 0)
