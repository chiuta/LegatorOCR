#!/usr/bin/env python3
"""The user guide must be the one of this build: its text has to name the version in Cargo.toml.
Usage: scripts/check-guide.py [VERSION]   (exit 1 when the guide is stale; rebuild it with
`node docs/guide-src/build.js`)."""
import pathlib, re, subprocess, sys
root = pathlib.Path(__file__).resolve().parent.parent
ver = sys.argv[1] if len(sys.argv) > 1 else re.search(r'^version\s*=\s*"([^"]+)"', (root / "Cargo.toml").read_text(), re.M).group(1)
pdfs = sorted((root / "docs").glob("Ghid-LegatorOCR*.pdf"))
if not pdfs:
    sys.exit("no docs/Ghid-LegatorOCR*.pdf: build them (node docs/guide-src/build.js)")
for pdf in pdfs:
    try:
        text = subprocess.run(["pdftotext", str(pdf), "-"], capture_output=True, text=True, check=True).stdout
    except subprocess.CalledProcessError as e:
        sys.exit(f"cannot read {pdf} ({e.returncode}): is it missing or broken?")
    except FileNotFoundError:  # no poppler: the version is also in the PDF title
        raw = pdf.read_bytes()
        text = ver if (ver.encode() in raw or ver.encode("utf-16-be") in raw or ver.encode("utf-16-be").hex().encode() in raw.lower()) else ""
    if not re.search(re.escape(ver) + r'(?![\w.-])', text):
        sys.exit(f"docs/{pdf.name} does not mention version {ver}: rebuild it (node docs/guide-src/build.js)")
    print(f"{pdf.name} mentions {ver}: ok")
