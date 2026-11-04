# Tesseract ron: tessdata_fast vs tessdata_best on the 12 real pages (ROADMAP 4.34). Needs the two ron.traineddata in /tmp/td_fast and /tmp/td_best
# (raw.githubusercontent.com/tesseract-ocr/tessdata_{fast,best}/main/ron.traineddata), images in /tmp/ocr_eval/img, references in /tmp/ref3.
import re, unicodedata, subprocess, sys, time
from collections import Counter
from rapidfuzz.distance import Levenshtein as L
img, ref = "/tmp/ocr_eval/img", "/tmp/ref3"
PAGES=(4,7,8,13,19,21,27,31,38,45,50,52); GOOD=(4,7,8,19,27,31,38,50)
def norm(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț"); return re.sub(r"\s+"," ",s).strip()
def nd(s): return "".join(c for c in unicodedata.normalize("NFD",s) if not unicodedata.combining(c))
def words(t): return Counter(w for w in re.findall(r"\w+",t.lower()) if len(w)>=3)
for name,d in (("fast","/tmp/td_fast"),("best","/tmp/td_best")):
    R={}; t0=time.time()
    for p in PAGES:
        H=norm(subprocess.run(["tesseract",f"{img}/p{p}.png","-","--tessdata-dir",d,"-l","ron"],capture_output=True,text=True).stdout)
        Rf=norm(open(f"{ref}/p{p}.txt",encoding="utf8").read())
        rw,hw=words(Rf),words(H)
        R[p]=(L.distance(Rf,H),len(Rf),sum((rw&hw).values()),sum(rw.values()),L.distance(nd(Rf),nd(H)),len(nd(Rf)))
    for g,ps in (("ALL12",PAGES),("GOOD8",GOOD)):
        e=sum(R[p][0] for p in ps); n=sum(R[p][1] for p in ps); a=sum(R[p][2] for p in ps); b=sum(R[p][3] for p in ps); e2=sum(R[p][4] for p in ps); n2=sum(R[p][5] for p in ps)
        print(f"{name} {g}: CER diacritics {100*e/n:.2f} %  blind {100*e2/n2:.2f} %  recall exact {100*a/b:.1f} %  ({time.time()-t0:.0f}s total)")
