import json,re,sys,unicodedata,collections
def toks(s):
    s=unicodedata.normalize("NFC",s).replace("ş","ș").replace("ţ","ț")
    return [t for t in re.findall(r"[^\W\d_]+",s.lower()) if len(t)>=3]
ref=collections.Counter(toks(open('/tmp/mobref/m200.txt').read()))
for e in sys.argv[1:]:
    pg=json.load(open(f'/tmp/mobp/m200.{e}.json'))['pages'][0]
    hyp=collections.Counter(toks(' '.join(w['text'] for w in pg['words'])))
    found=sum(min(c,hyp[t]) for t,c in ref.items())
    print(e,'word recall %.1f%% (%d/%d)'%(100*found/sum(ref.values()),found,sum(ref.values())))
