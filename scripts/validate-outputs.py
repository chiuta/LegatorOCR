#!/usr/bin/env python3
"""Validate LegatorOCR's structured outputs against the published schemas / validators.

  python3 scripts/validate-outputs.py target/debug/legatorocr IMAGE.png

Needs network (raw.githubusercontent.com, PyPI), lxml and `pip install ocfl-py`. Checks:
  * ALTO 4.4 (altoxml/schema) - the W3C xlink schema is not reachable from every network, so a
    minimal stand-in for its simpleLink attributes is written next to the schema;
  * PAGE XML against the 2024-07-15 PRImA schema, after rewriting our 2019-07-15 namespace
    (the older schema file is not published in the repository any more);
  * the OCFL object written by --ocfl with ocfl-py's validator.
"""
import subprocess, sys, tempfile, urllib.request, pathlib, glob
from lxml import etree

exe, img = sys.argv[1], sys.argv[2]
d = pathlib.Path(tempfile.mkdtemp())
RAW = "https://raw.githubusercontent.com/"
for n, u in {"alto.xsd": RAW + "altoxml/schema/master/v4/alto-4-4.xsd",
             "page.xsd": RAW + "PRImA-Research-Lab/PAGE-XML/master/pagecontent/schema/pagecontent.xsd"}.items():
    (d / n).write_bytes(urllib.request.urlopen(u, timeout=60).read())
(d / "xlink.xsd").write_text('''<?xml version="1.0"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="http://www.w3.org/1999/xlink" xmlns:xlink="http://www.w3.org/1999/xlink">
<xs:attribute name="type" type="xs:string"/><xs:attribute name="href" type="xs:anyURI"/><xs:attribute name="role" type="xs:string"/><xs:attribute name="arcrole" type="xs:string"/>
<xs:attribute name="title" type="xs:string"/><xs:attribute name="show" type="xs:string"/><xs:attribute name="actuate" type="xs:string"/>
<xs:attributeGroup name="simpleLink"><xs:attribute ref="xlink:type" fixed="simple"/><xs:attribute ref="xlink:href"/><xs:attribute ref="xlink:role"/><xs:attribute ref="xlink:arcrole"/><xs:attribute ref="xlink:title"/><xs:attribute ref="xlink:show"/><xs:attribute ref="xlink:actuate"/></xs:attributeGroup></xs:schema>''')
a = d / "alto.xsd"
a.write_text(a.read_text().replace("http://www.loc.gov/standards/xlink/xlink.xsd", "xlink.xsd"))
run = subprocess.run([exe, img, "--ocr", "--dpi", "150", "--pdf", str(d / "o.pdf"), "--alto", str(d / "o.alto.xml"),
                      "--page-xml", str(d / "pages"), "--ocfl", str(d / "obj")], capture_output=True, text=True)
if run.returncode:
    sys.exit(run.stderr)
bad = 0
def check(name, xsd, doc):
    global bad
    s = etree.XMLSchema(etree.parse(str(d / xsd)))
    ok = s.validate(doc)
    print(("ok   " if ok else "FAIL ") + name)
    for e in list(s.error_log)[:5]:
        print("   ", str(e)[:200])
    bad += not ok
check("ALTO 4.4", "alto.xsd", etree.parse(str(d / "o.alto.xml")))
for f in sorted(glob.glob(str(d / "pages" / "*.xml"))):
    t = pathlib.Path(f).read_text().replace("2019-07-15", "2024-07-15").encode()
    check("PAGE " + pathlib.Path(f).name, "page.xsd", etree.fromstring(t).getroottree())
r = subprocess.run(["ocfl-validate.py", str(d / "obj")], capture_output=True, text=True)
print(("ok   " if r.returncode == 0 else "FAIL ") + "OCFL " + (r.stdout + r.stderr).strip().splitlines()[-1])
bad += r.returncode != 0
sys.exit(1 if bad else 0)
