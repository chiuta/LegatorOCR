import os,sys,numpy as np,torch
import train as T
from train import *
held=load_real(os.environ["REAL"],held=True)
def pg(p):
    import re;return re.match(r"([be]-\d+|p\d+)",os.path.basename(p)).group(0)
paths=[l.split("\t")[0] for l in open(os.environ["REAL"],encoding="utf8")]
for m in sys.argv[1:]:
    net=Net(); load_lnm(net,m); net.eval()
    groups={"digital":[], "magazine":[]}
    held.sort(key=lambda it: it[0].shape[1])
    tot={}
    for a,t in held: pass
    for name,sel in (("all",lambda t:True),):
        rv=[]
        for i in range(0,len(held),16):
            grp=held[i:i+16];W=(max(a.shape[1] for a,_ in grp)+3)//4*4
            x=np.zeros((len(grp),1,synth.H,W),np.float32)
            for j,(a,_) in enumerate(grp): x[j,0,:,:a.shape[1]]=(255-a)/255.0
            rv.append((torch.from_numpy(x),None,None,[t for _,t in grp]))
        print(m,name,len(held),"lines CER %.4f"%validate(net,rv))
