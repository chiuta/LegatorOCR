#!/usr/bin/env python3
"""Sync version stamps from the single source of truth (Cargo.toml, or `version_from`
in .version-sync.json) into the published page, app.json, CITATION.cff and any
configured text patterns. Standard library only.

  python3 -I scripts/sync-version.py          # rewrite what is out of date
  python3 -I scripts/sync-version.py --check  # exit 1 if anything is out of date (no writes)
"""
import datetime, glob, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CFG = json.loads((ROOT / ".version-sync.json").read_text(encoding="utf-8"))
CHECK = "--check" in sys.argv
TODAY = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
pending = {}  # path -> new text
notes = []


def read(p):
    return pending.get(p) if p in pending else p.read_text(encoding="utf-8")


def put(p, text, why):
    if text != read(p):
        pending[p] = text
        notes.append(f"{p.relative_to(ROOT)}: {why}")


def source_version():
    vf = CFG.get("version_from")
    if vf:
        text, rx = (ROOT / vf["file"]).read_text(encoding="utf-8"), vf["regex"]
    else:
        text, rx = (ROOT / "Cargo.toml").read_text(encoding="utf-8"), r'^version\s*=\s*"([^"]+)"'
    m = re.search(rx, text, re.M)
    if not m:
        sys.exit("sync-version: cannot read the source version")
    return m.group(1)


def sub_group(text, rx, value, flags=re.M):
    """Replace capture group 1 of every match of rx with value."""
    out, last = [], 0
    for m in re.finditer(rx, text, flags):
        out.append(text[last:m.start(1)] + value)
        last = m.end(1)
    out.append(text[last:])
    return "".join(out)


new = source_version()
pages = ROOT / CFG.get("pages", ".")
appjson = pages / "app.json"
old = None
if appjson.exists():
    old = json.loads(appjson.read_text(encoding="utf-8")).get("version")
changed_version = old != new

# 1. <meta name="application-version"> / last-updated in the published pages
for rel in CFG.get("html", ["index.html"]):
    p = pages / rel
    s = read(p)
    m = re.search(r'<meta name="application-version" content="([^"]*)">', s)
    if m and m.group(1) == new:
        continue
    if m:
        s = s.replace(m.group(0), f'<meta name="application-version" content="{new}">')
        s = re.sub(r'(<meta name="last-updated" content=")[^"]*(">)', r"\g<1>" + TODAY + r"\g<2>", s, count=1)
    else:
        tags = f'<meta name="application-version" content="{new}">\n<meta name="last-updated" content="{TODAY}">\n'
        e = re.search(r"<!--fff:kit:end[^>]*-->\n?", s) or re.search(r"<meta charset[^>]*>\n?", s)
        if not e:
            sys.exit(f"sync-version: no place for the meta tags in {p}")
        pos = e.end()
        s = s[:pos] + ("" if s[pos - 1] == "\n" else "\n") + tags + s[pos:]
    put(p, s, f"application-version {new}")

# 2. app.json
slug = CFG.get("slug") or ROOT.name.lower()
data = {}
if appjson.exists():
    data = json.loads(appjson.read_text(encoding="utf-8"))
if data.get("version") != new:
    data.update({"name": data.get("name") or CFG.get("name") or ROOT.name, "slug": slug,
                 "version": new, "updated": TODAY,
                 "repository": data.get("repository") or CFG.get("repository"),
                 "url": data.get("url") or f"https://{slug}.chiuta.ro/"})
    pending[appjson] = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    notes.append(f"{appjson.relative_to(ROOT)}: version {new}")

# 3. CITATION.cff
cff = ROOT / "CITATION.cff"
if cff.exists():
    s = sub_group(read(cff), r'^version:\s*"?([^"\n]+)"?', new)
    if s != read(cff):
        s = sub_group(s, r"^date-released:\s*(\S+)", TODAY)
    put(cff, s, f"version {new}")

# 4. configured text patterns (capture group 1 = the version)
for item in CFG.get("patterns", []):
    for f in sorted(glob.glob(str(ROOT / item["file"]))):
        p = pathlib.Path(f)
        put(p, sub_group(read(p), item["regex"], new), f"text version {new}")
# dates next to a version are refreshed only when the version actually changed
if changed_version:
    for item in CFG.get("date_patterns", []):
        for f in sorted(glob.glob(str(ROOT / item["file"]))):
            p = pathlib.Path(f)
            put(p, sub_group(read(p), item["regex"], TODAY), f"date {TODAY}")

if CHECK:
    if notes:
        print("version stamps out of date (source version %s):" % new)
        print("\n".join("  " + n for n in notes))
        sys.exit(1)
    print("version stamps in sync:", new)
else:
    for p, t in pending.items():
        p.write_text(t, encoding="utf-8")
    print(("updated to " + new + ":\n  " + "\n  ".join(notes)) if notes else "already in sync: " + new)
