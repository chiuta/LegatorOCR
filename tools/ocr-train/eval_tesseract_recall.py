import re, unicodedata, subprocess
from collections import Counter
from rapidfuzz.distance import Levenshtein as L
img, ref = "/tmp/ocr_eval/img", "/tmp/ref3"
PAGES=(4,7,8,13,19,21,27,31,38,45,50,52); GOOD=(4,7,8,19,27,31,38,50)
def norm(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț"); return re.sub(r"\s+"," ",s).strip()
def nd(s): return "".join(c for c in unicodedata.normalize("NFD",s) if not unicodedata.combining(c))
def words(t): return Counter(w for w in re.findall(r"\w+",t.lower()) if len(w)>=3)
R={}
for p in PAGES:
    H=norm(subprocess.run(["tesseract",f"{img}/p{p}.png","-","-l","ron"],capture_output=True,text=True).stdout)
    Rf=norm(open(f"{ref}/p{p}.txt",encoding="utf8").read())
    rw=words(Rf); hw=words(H); rwn=words(nd(Rf)); hwn=words(nd(H))
    R[p]=(L.distance(nd(Rf),nd(H)),len(nd(Rf)),sum((rw&hw).values()),sum((rwn&hwn).values()),sum(rw.values()))
for name,ps in (("ALL12",PAGES),("GOOD8",GOOD)):
    e=sum(R[p][0] for p in ps); n=sum(R[p][1] for p in ps); a=sum(R[p][2] for p in ps); b=sum(R[p][4] for p in ps); c=sum(R[p][3] for p in ps)
    print(f"tesseract-ron {name}: diacritic-blind CER {100*e/n:.2f} %, word recall (exact) {100*a/b:.1f} %, (diacritic-blind) {100*c/b:.1f} %")
