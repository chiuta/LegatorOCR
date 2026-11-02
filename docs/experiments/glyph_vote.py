"""Roadmap 4.4 gate experiment: glyph clusters across a book, label voting.
Words from Tesseract tsv are cut into glyph images by vertical projection; words whose cut count
equals their letter count give (glyph image, label) pairs. Glyphs are clustered over all pages;
inside a cluster the majority label replaces minority labels. CER on the tsv text is compared
before and after."""
import subprocess,csv,io,re,unicodedata,sys
import numpy as np
from PIL import Image
from rapidfuzz.distance import Levenshtein as L
PAGES=(4,7,8,13,19,21,27,31,38,45,50,52)
def norm(s):
    s=unicodedata.normalize('NFC',s).replace('ş','ș').replace('ţ','ț'); return re.sub(r'\s+',' ',s).strip()
def tsv(p):
    t=subprocess.run(["tesseract",f"img/p{p}.png","-","-l","ron","tsv"],capture_output=True,text=True).stdout
    return [r for r in csv.DictReader(io.StringIO(t),delimiter='\t',quoting=csv.QUOTE_NONE) if r['level']=='5' and r['text'].strip() and float(r['conf'])>=0]
def segs(ink):
    cols=ink.any(axis=0); out=[];s=None
    for i,c in enumerate(cols):
        if c and s is None: s=i
        if not c and s is not None: out.append((s,i)); s=None
    if s is not None: out.append((s,len(cols)))
    return out
def feat(g,hmed):
    ys=np.where(g.any(axis=1))[0]; xs=np.where(g.any(axis=0))[0]
    return g
def norm_glyph(img):
    h,w=img.shape
    s=16/max(h,w); nh,nw=max(1,round(h*s)),max(1,round(w*s))
    im=Image.fromarray((img*255).astype(np.uint8)).resize((nw,nh),Image.BILINEAR)
    out=np.zeros((16,16),np.float32); y0=(16-nh)//2;x0=(16-nw)//2
    out[y0:y0+nh,x0:x0+nw]=np.asarray(im)/255.
    return out
data=[]  # per word: dict
glyphs=[] # (word index, char index, vec, size feats, label)
words=[]
for p in PAGES:
    im=np.asarray(Image.open(f"img/p{p}.png").convert("L")).astype(np.float32)
    rows=tsv(p)
    hs=sorted(int(r['height']) for r in rows); hmed=hs[len(hs)//2]
    for r in rows:
        x,y,w,h=(int(r[k]) for k in("left","top","width","height"))
        t=r['text']; wi=len(words); words.append([p,t,list(t),float(r['conf'])])
        crop=im[max(0,y):y+h,max(0,x):x+w]
        if crop.size==0 or len(t)<2: continue
        thr=(crop.min()+crop.max())/2
        ink=crop<thr
        if ink.sum()<10: continue
        sg=segs(ink)
        if len(sg)!=len(t): continue
        for ci,(a,b) in enumerate(sg):
            g=ink[:,a:b]
            ys=np.where(g.any(axis=1))[0]
            if len(ys)==0: continue
            gg=g[ys[0]:ys[-1]+1]
            glyphs.append((wi,ci,norm_glyph(gg.astype(np.float32)),np.array([gg.shape[1]/hmed,gg.shape[0]/hmed,(ys[0])/hmed]),t[ci]))
print("words",len(words),"glyphs paired",len(glyphs))
# greedy leader clustering
V=np.stack([g[2].ravel() for g in glyphs]);S=np.stack([g[3] for g in glyphs])
cent=[];cs=[];members=[]
TH=float(sys.argv[1]) if len(sys.argv)>1 else 0.10
MAJ=float(sys.argv[2]) if len(sys.argv)>2 else 0.8
MINN=int(sys.argv[3]) if len(sys.argv)>3 else 5
for i in range(len(glyphs)):
    best=-1;bd=9
    if cent:
        C=np.stack(cent);d=np.abs(C-V[i]).mean(axis=1)+0.5*np.abs(np.stack(cs)-S[i]).sum(axis=1)
        j=int(d.argmin());bd=d[j];best=j
    if bd<TH: members[best].append(i)
    else: cent.append(V[i]);cs.append(S[i]);members.append([i])
print("clusters",len(members),"with >=5:",sum(len(m)>=5 for m in members))
changed=0;good=0;bad=0
ref={p:norm(open(f"ref/p{p}.txt",encoding='utf8').read()) for p in PAGES}
before={p:norm(" ".join(w[1] for w in words if w[0]==p)) for p in PAGES}
for m in members:
    if len(m)<MINN: continue
    labs=[glyphs[i][4] for i in m]
    from collections import Counter
    lab,n=Counter(labs).most_common(1)[0]
    if n/len(m)<MAJ: continue
    for i in m:
        wi,ci,_,_,l=glyphs[i]
        if l!=lab:
            words[wi][2][ci]=lab;changed+=1
after={p:norm(" ".join("".join(w[2]) for w in words if w[0]==p)) for p in PAGES}
b=a=n=0
for p in PAGES:
    b+=L.distance(ref[p],before[p]);a+=L.distance(ref[p],after[p]);n+=len(ref[p])
print("relabelled",changed,"errors before",b,"after",a,"of",n,"chars  CER %.2f -> %.2f"%(100*b/n,100*a/n))
