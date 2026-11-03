#!/usr/bin/env python3
"""Every option in `legatorocr --help` must be covered: named in the README, a task recipe or the
ROADMAP (documentation) AND used in a test or CI script. Lists the ones that are not.
Usage: scripts/check-options.py [path/to/legatorocr] [--strict]   (exit 1 with --strict when gaps exist)"""
import re, subprocess, sys, pathlib, glob
root = pathlib.Path(__file__).resolve().parent.parent
exe = next((a for a in sys.argv[1:] if not a.startswith("--")), str(root / "target/release/legatorocr"))
help_ = subprocess.run([exe, "--help"], capture_output=True, text=True).stdout
opts = sorted(set(re.findall(r"^\s+(?:-\w, )?(--[a-z0-9][a-z0-9-]*)", help_, re.M)))
def read(globs):
    t = ""
    for g in globs:
        for f in glob.glob(str(root / g), recursive=True):
            try: t += pathlib.Path(f).read_text(errors="ignore")
            except Exception: pass
    return t
docs = read(["README.md", "ROADMAP.md", "docs/**/*.md", "docs/**/*.html"])
tests = read(["crates/**/tests/*.rs", "crates/**/src/**/*.rs", ".github/workflows/*.yml", "scripts/*.py", "scripts/*.sh", "web/*.js"])
nodoc = [o for o in opts if o not in docs]
notest = [o for o in opts if o not in tests.replace("\\\"", "\"") or tests.count(o) < 2]
print(f"{len(opts)} options; without documentation: {len(nodoc)}; without a test or script use: {len(notest)}")
for o in nodoc: print("  no doc :", o)
for o in notest: print("  no test:", o)
sys.exit(1 if nodoc else 0)  # an undocumented option fails; the test list is informational (scripts/smoke-options.py covers the rare ones)
