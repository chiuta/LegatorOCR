import numpy as np, sys, random
from PIL import Image
sys.path.insert(0,'.')
# DCT score per tile, replicating the Rust feature on arbitrary luma tiles (quant table of the file)
def dct_mat():
    M=np.zeros((8,8))
    for k in range(8):
        for n in range(8):
            M[k,n]=(np.sqrt(1/8) if k==0 else 0.5)*np.cos((2*n+1)*k*np.pi/16)
    return M
M=dct_mat()
modes=[(0,1),(1,0),(1,1),(0,2),(2,0)]
def score(luma,q):
    h,w=luma.shape; bh,bw=h//8,w//8
    B=luma[:bh*8,:bw*8].astype(np.float64).reshape(bh,8,bw,8).transpose(0,2,1,3)-128
    C=np.einsum('ku,abuv,lv->abkl',M,B,M)   # C[a,b,k(row freq),l(col freq)]
    sc=[]
    K=16
    for (u,v) in modes:
        c=np.round(C[:,:,v,u]/q[v*8+u]).astype(int).ravel()
        hist=np.bincount(np.clip(c,-K,K)+K,minlength=2*K+1)
        best=0
        for side in (1,-1):
            ks=[k for k in range(2,15) if hist[side*k+K]>=30]
            if len(ks)<8: continue
            y=np.log([hist[side*k+K] for k in ks]); x=np.array(ks)/14.0
            res=y-np.polyval(np.polyfit(x,y,2),x)
            f=np.arange(1,41)/40*0.5
            P=np.abs(np.exp(-2j*np.pi*np.outer(f,ks))@res)**2
            best=max(best,P.max()/P.mean()*np.sqrt((res**2).mean()))
        sc.append(best)
    return np.mean(sc)
