import subprocess,csv,io,re,unicodedata,statistics as st
from rapidfuzz.distance import Levenshtein as L
def norm(s):
    s=unicodedata.normalize('NFC',s).replace('ş','ș').replace('ţ','ț'); return re.sub(r'\s+',' ',s).strip()
rd=lambda p:open(p,encoding='utf8').read()
rows=[]
for p in (4,7,8,13,19,21,27,31,38,45,50,52):
    t=subprocess.run(["tesseract",f"img/p{p}.png","-","-l","ron","tsv"],capture_output=True,text=True).stdout
    c=[float(r['conf']) for r in csv.DictReader(io.StringIO(t),delimiter='\t',quoting=csv.QUOTE_NONE) if r['level']=='5' and r['text'].strip() and float(r['conf'])>=0]
    R=norm(rd(f'ref/p{p}.txt'));H=norm(rd(f'out_B/p{p}.txt'))
    cer=100*L.distance(R,H)/len(R)
    rows.append((p,st.mean(c),sum(x<70 for x in c)/len(c)*100,cer,len(c)))
    print(p,"meanconf %.1f low%% %.1f CER %.1f words %d"%rows[-1][1:])
import numpy as np
a=np.array([[r[1],r[2],r[3]] for r in rows])
from scipy.stats import spearmanr,pearsonr
for i,n in ((0,"mean conf"),(1,"% words <70")):
    print(n,"spearman %.2f pearson %.2f"%(spearmanr(a[:,i],a[:,2])[0],pearsonr(a[:,i],a[:,2])[0]))
# leave-one-out linear estimate of CER from low%
err=[]
for k in range(len(a)):
    m=np.arange(len(a))!=k
    A=np.vstack([a[m,1],np.ones(m.sum())]).T; w=np.linalg.lstsq(A,a[m,2],rcond=None)[0]
    err.append(abs(w[0]*a[k,1]+w[1]-a[k,2]))
print("LOO mean abs error of CER estimate: %.1f points; baseline (predict mean): %.1f"%(np.mean(err),np.mean(abs(a[:,2]-a[:,2].mean()))))
