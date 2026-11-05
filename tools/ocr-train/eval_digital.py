"""CER on held-out pages of the two digital PDFs. Usage: eval_digital.py MODEL.lnm|tess [IMGDIR=deg|.]   (env HELD)"""
import subprocess, sys, os, re, unicodedata, json, tempfile
from concurrent.futures import ThreadPoolExecutor
from rapidfuzz.distance import Levenshtein as L
D="/mnt/user-data/attachments/4b0a78e0-55ff-59dc-a74b-132a0e477577"
PDF={"b":subprocess.check_output(f"ls {D}/*Bro_ura_Jocuri*",shell=True,text=True).strip(),"e":subprocess.check_output(f"ls {D}/*Exercitii*",shell=True,text=True).strip()}
model=sys.argv[1]; img=sys.argv[2] if len(sys.argv)>2 else "/tmp/ot/deg"; ext="jpg" if img.endswith("deg") else "png"
exe="/home/claude/legatorocr/target/release/legatorocr"
pages=open("/tmp/ot/held.txt").read().strip().split(",")
def norm(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț").replace("Ş","Ș").replace("Ţ","Ț").replace("­","")
    return re.sub(r"\s+"," ",s).strip()
def one(p):
    k,n=p.split("-")
    ref=norm(subprocess.run(["pdftotext","-raw","-f",str(int(n)),"-l",str(int(n)),PDF[k],"-"],capture_output=True,text=True).stdout)
    with tempfile.TemporaryDirectory() as d:
        if model=="tess":
            args=["--engine","tesseract"]; env=os.environ
        else:
            args=["--engine","native"]; env={**os.environ,"LEGATOROCR_NATIVE_MODEL":os.path.abspath(model)}
        subprocess.run([exe,f"{img}/{p}.{ext}","--pdf",f"{d}/x.pdf","--ocr","--lang","ron",*args,"--json",f"{d}/o.json","-o",f"{d}/pg"],capture_output=True,env=env)
        j=json.load(open(f"{d}/o.json"))
    H=norm(" ".join(w["text"] for pg in j["pages"] for w in pg["words"]))
    R2,H2=ref.replace(" ",""),H.replace(" ","")
    return p,L.distance(ref,H),len(ref),L.distance(R2,H2),len(R2)
with ThreadPoolExecutor(2) as ex: rs=list(ex.map(one,pages))
for r in rs: print(r[0],f"{100*r[1]/max(r[2],1):.2f}",f"{100*r[3]/max(r[4],1):.2f}")
a=sum(r[1] for r in rs);b=sum(r[2] for r in rs);c=sum(r[3] for r in rs);d=sum(r[4] for r in rs)
print(f"TOTAL CER {100*a/b:.2f}%  space-blind {100*c/d:.2f}%")
