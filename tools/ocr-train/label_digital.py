"""Label dumped lines of a digital PDF with its exact text layer (words and spaces from the PDF).
Usage: label_digital.py LINES_DIR OUT.tsv PDF_B PDF_E   (stems b-NN / e-NN = page NN of PDF_B / PDF_E)
   or:  label_digital.py LINES_DIR OUT.tsv k=PDF [k=PDF ...]   (stem k-NN = page NN of that PDF)"""
import json, glob, os, re, sys
from rapidfuzz.distance import Levenshtein as L
from real_labels import fragments, MAXDIFF
from charset import CHARSET
import xml.etree.ElementTree as ET, subprocess
pdfs = dict(a.split("=", 1) for a in sys.argv[3:]) if "=" in sys.argv[3] else {"b": sys.argv[3], "e": sys.argv[4]}
cs = set(CHARSET); kept = tot = 0
def words(pdf, n):
    import real_labels as R
    out = subprocess.run(["pdftotext","-bbox","-f",str(n),"-l",str(n),pdf,"-"],capture_output=True,text=True,check=True).stdout
    out = re.sub(r"[\x00-\x08\x0b\x0c\x0e-\x1f]","",out)
    out = re.sub(r"<!DOCTYPE[^>]*>","",out).replace(' xmlns="http://www.w3.org/1999/xhtml"',"")
    r = []
    for w in ET.fromstring(out).iter("word"):
        t = "".join(R.MAP.get(c,c) for c in (w.text or ""))
        r.append(((float(w.get("xMin"))+float(w.get("xMax")))/2*R.DPI,(float(w.get("yMin"))+float(w.get("yMax")))/2*R.DPI,t))
    return r
with open(sys.argv[2],"w",encoding="utf8") as f:
    for jl in sorted(glob.glob(sys.argv[1]+"/*.jsonl")):
        stem = os.path.basename(jl)[:-6]; k, n = stem.split("-"); n = int(n)
        fr = words(pdfs[k], n)
        for row in map(json.loads, open(jl, encoding="utf8")):
            tot += 1
            x0,x1,y0,y1 = row["x"],row["x"]+row["w"],row["y"],row["y"]+row["h"]
            ins = sorted((fx,t) for fx,fy,t in fr if x0<=fx<=x1 and y0<=fy<=y1)
            lab = " ".join(t for _,t in ins)
            mo = row["model"].replace(" ","")
            if len(lab)<3 or not mo or any(c not in cs for c in lab): continue
            if L.distance(lab.replace(" ",""),mo)/max(len(lab),1) > MAXDIFF: continue
            f.write(f"{sys.argv[1]}/{stem}_{row['i']}.png\t{lab}\n"); kept += 1
print("kept",kept,"of",tot)
