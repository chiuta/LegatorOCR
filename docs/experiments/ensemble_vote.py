import json,re,unicodedata,sys
from rapidfuzz.distance import Levenshtein as L
def norm(s):
    s=unicodedata.normalize('NFC',s).replace('ş','ș').replace('ţ','ț')
    return re.sub(r'\s+',' ',s).strip()
rd=lambda p:open(p,encoding='utf8').read()
def words(p):
    j=json.load(open(p))['pages'][0]; return j['words']
def iou(a,b):
    x0=max(a['x'],b['x']);y0=max(a['y'],b['y']);x1=min(a['x']+a['w'],b['x']+b['w']);y1=min(a['y']+a['h'],b['y']+b['h'])
    i=max(0,x1-x0)*max(0,y1-y0); u=a['w']*a['h']+b['w']*b['h']-i
    return i/u if u else 0
def text(ws):
    # join in tesseract line order
    out=[];cur=None
    for w in ws:
        if cur is not None and w['line']!=cur: out.append('\n')
        out.append(w['text']);out.append(' ');cur=w['line']
    return ''.join(out)
def combine(t,n,margin,mode):
    res=[]
    for w in t:
        best=None;bi=0
        for v in n:
            i=iou(w,v)
            if i>bi: bi=i;best=v
        w2=dict(w)
        if best is not None and bi>0.4 and best['text']!=w['text']:
            if mode=='conf' and best['conf']>w['conf']+margin: w2['text']=best['text']
        res.append(w2)
    return res
pages=(4,7,8,13,19,21,27,31,38,45,50,52)
def score(f):
    d=n=0
    for p in pages:
        R=norm(rd(f'/tmp/ocr_eval/ref/p{p}.txt'));H=norm(f(p));d+=L.distance(R,H);n+=len(R)
    return d,round(100*d/n,2)
print('tess only',score(lambda p:text(words(f'/tmp/ocr_eval/ens/p{p}.tess.json'))))
print('native only',score(lambda p:text(words(f'/tmp/ocr_eval/ens/p{p}.nat.json'))))
for m in (0,5,10,20,30):
    print('vote margin',m,score(lambda p:text(combine(words(f'/tmp/ocr_eval/ens/p{p}.tess.json'),words(f'/tmp/ocr_eval/ens/p{p}.nat.json'),m,'conf'))))
