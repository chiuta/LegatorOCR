import re,unicodedata,sys
from rapidfuzz.distance import Levenshtein as L
from wordfreq import top_n_list, zipf_frequency
words=set(w for w in top_n_list('ro',60000))
def norm(s):
    s=unicodedata.normalize('NFC',s).replace('ş','ș').replace('ţ','ț')
    return re.sub(r'\s+',' ',s).strip()
rd=lambda p:open(p,encoding='utf8').read()
by_len={}
for w in words: by_len.setdefault(len(w),[]).append(w)
def cands(w):
    out=[]
    for l in (len(w)-1,len(w),len(w)+1):
        for c in by_len.get(l,[]):
            if abs(len(c)-len(w))<=1 and L.distance(w,c)==1: out.append(c)
    return out
def fix(text,minz):
    def rep(m):
        t=m.group(0)
        lw=t.lower()
        if len(lw)<4 or lw in words or not lw.isalpha(): return t
        c=cands(lw)
        if not c: return t
        c.sort(key=lambda x:-zipf_frequency(x,'ro'))
        best=c[0]
        if zipf_frequency(best,'ro')<minz: return t
        if len(c)>1 and zipf_frequency(c[1],'ro')>=zipf_frequency(best,'ro')-0.3: return t
        r=best.capitalize() if t[0].isupper() and not t.isupper() else (best.upper() if t.isupper() else best)
        return r
    return re.sub(r"[^\W\d_]+",rep,text)
pages=(4,7,8,13,19,21,27,31,38,45,50,52)
for minz in (None,3.5,3.0,2.5):
    d=n=0;ch=0
    for p in pages:
        R=norm(rd(f'ref/p{p}.txt')); H=rd(f'out_A/p{p}.txt')
        if minz is not None:
            H2=fix(H,minz); ch+= (H2!=H)
            H=H2
        H=norm(H); d+=L.distance(R,H); n+=len(R)
    print(minz,d,round(100*d/n,3),'pages changed',ch)
