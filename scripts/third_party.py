#!/usr/bin/env python3
"""Regenerate THIRD_PARTY_LICENSES.md from Cargo.lock (all platforms' dependencies)."""
import json, subprocess, sys
m = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"]))
ws = set(m["workspace_members"])
rows = sorted({(p["name"], p["version"], p.get("license") or "see crate", p.get("repository") or "")
               for p in m["packages"] if p["id"] not in ws})
out = ["# Third-party software\n",
       "LegatorOCR itself is MIT-licensed (see `LICENSE`). It is built from the Rust crates below, each under",
       "its own licence (the choice in an `OR` expression is yours). The graphical interface also embeds the",
       "fonts shipped with `epaint_default_fonts` (Ubuntu Font Licence, SIL OFL, MIT/Apache).\n",
       "Optical character recognition runs the separately installed **Tesseract** program (Apache-2.0);",
       "nothing from Tesseract is linked or bundled.\n",
       "| Crate | Version | Licence |", "|---|---|---|"]
out += [f"| {n} | {v} | {l} |" for n, v, l, _ in rows]
open("THIRD_PARTY_LICENSES.md", "w").write("\n".join(out) + "\n")
print(len(rows), "crates", file=sys.stderr)
