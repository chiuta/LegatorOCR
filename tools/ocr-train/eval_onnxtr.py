import sys, re, unicodedata, time, os
from collections import Counter
from rapidfuzz.distance import Levenshtein as L
from onnxtr.io import DocumentFile
from onnxtr.models import ocr_predictor
det, reco = sys.argv[1], sys.argv[2]
img, ref = "/tmp/ocr_eval/img", "/tmp/ref3"
PAGES=(4,7,8,13,19,21,27,31,38,45,50,52); GOOD=(4,7,8,19,27,31,38,50)
def norm(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț"); return re.sub(r"\s+"," ",s).strip()
def nd(s): return "".join(c for c in unicodedata.normalize("NFD",s) if not unicodedata.combining(c))
def words(t): return Counter(w for w in re.findall(r"\w+",t.lower()) if len(w)>=3)
m = ocr_predictor(det_arch=det, reco_arch=reco, assume_straight_pages=True)
R={}; t0=time.time()
for p in PAGES:
    doc = DocumentFile.from_images(f"{img}/p{p}.png")
    H = norm(m(doc).render().replace("\n"," "))
    open(f"/tmp/neural_out/ot_{det}_{reco}_p{p}.txt","w").write(H)
    Rf = norm(open(f"{ref}/p{p}.txt",encoding="utf8").read())
    rw=words(Rf); hw=words(H); rwn=words(nd(Rf)); hwn=words(nd(H))
    R[p]=(L.distance(nd(Rf),nd(H)), len(nd(Rf)), sum((rw&hw).values()), sum((rwn&hwn).values()), sum(rw.values()))
    print(p, round(100*R[p][0]/R[p][1],1), flush=True)
for name,ps in (("ALL12",PAGES),("GOOD8",GOOD)):
    e=sum(R[p][0] for p in ps); n=sum(R[p][1] for p in ps); a=sum(R[p][2] for p in ps); b=sum(R[p][4] for p in ps); c=sum(R[p][3] for p in ps)
    print(f"{det}+{reco} {name}: diacritic-blind CER {100*e/n:.2f} %, word recall (exact) {100*a/b:.1f} %, word recall (diacritic-blind) {100*c/b:.1f} %")
print("seconds", round(time.time()-t0))
