"""Score engine output (json from the CLI) against a hand transcription of part of a page:
char recall (reference characters matched in order by the alignment), word recall (bag of words)."""
import json,re,sys,unicodedata,collections
from rapidfuzz.distance import Levenshtein as L
def norm(s): return re.sub(r"[\s\-—–]+","",unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț")).lower()
def toks(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț")
    return [t for t in re.findall(r"[^\W\d_]+",s.lower()) if len(t)>=3]
def score(page,engine,tag):
    ref=open(f'/tmp/mobref/{page}.txt').read()
    pg=json.load(open(f'/tmp/mobp/{page}.{tag}.json'))['pages'][0]
    txt=' '.join(w['text'] for w in pg['words'])
    r,h=norm(ref),norm(txt)
    m=sum(op.src_end-op.src_start for op in L.opcodes(r,h) if op.tag=='equal')
    rc=collections.Counter(toks(ref)); hc=collections.Counter(toks(txt))
    wf=sum(min(c,hc[t]) for t,c in rc.items())
    return 100*m/len(r),100*wf/sum(rc.values())
if __name__=="__main__":
    pages=sys.argv[2:]; tag=sys.argv[1]
    cs=ws=0
    for p in pages:
        c,w=score(p,None,tag); cs+=c; ws+=w
        print(f"{p}: char recall {c:.1f}%  word recall {w:.1f}%")
    print(f"mean: char {cs/len(pages):.1f}%  word {ws/len(pages):.1f}%")
