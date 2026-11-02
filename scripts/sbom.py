#!/usr/bin/env python3
"""CycloneDX 1.5 SBOM (JSON) built from Cargo.lock alone: every crate with version, source
and checksum (SHA-256 of the .crate file as recorded by cargo). No network, no extra tools.
Usage: scripts/sbom.py [Cargo.lock] > sbom.cdx.json"""
import json, re, sys, datetime, os

lock = sys.argv[1] if len(sys.argv) > 1 else "Cargo.lock"
epoch = os.environ.get("SOURCE_DATE_EPOCH")
ts = (datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc) if epoch
      else datetime.datetime.now(datetime.timezone.utc)).strftime("%Y-%m-%dT%H:%M:%SZ")
comps, ours = [], []
for blk in open(lock, encoding="utf-8").read().split("[[package]]")[1:]:
    f = dict(re.findall(r'^(\w+) = "(.*)"$', blk, re.M))
    name, ver = f.get("name"), f.get("version")
    if not name:
        continue
    c = {"type": "library", "bom-ref": f"{name}@{ver}", "name": name, "version": ver,
         "purl": f"pkg:cargo/{name}@{ver}"}
    if "checksum" in f:
        c["hashes"] = [{"alg": "SHA-256", "content": f["checksum"]}]
    if "source" in f:
        c["externalReferences"] = [{"type": "distribution", "url": f["source"]}]
        comps.append(c)
    else:
        ours.append(c)
root = next((c for c in ours if c["name"] == "legatorocr-cli"), ours[0] if ours else None)
bom = {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
       "metadata": {"timestamp": ts,
                    "tools": {"components": [{"type": "application", "name": "scripts/sbom.py"}]},
                    "component": {**root, "type": "application"} if root else {}},
       "components": sorted(comps + [c for c in ours if c is not root], key=lambda c: c["bom-ref"])}
json.dump(bom, sys.stdout, indent=1, sort_keys=True)
print()
