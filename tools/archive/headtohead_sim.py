import numpy as np, scipy.sparse as sp
from scipy.sparse.csgraph import dijkstra
from scipy.ndimage import gaussian_filter
from scipy.spatial import cKDTree
rng=np.random.default_rng(7)
H,W=100,160
SLOPE_LIM=np.deg2rad(20)   # assumption
def terrain(seed):
    r=np.random.default_rng(seed)
    seeds=np.c_[r.uniform(-10,W+10,34),r.uniform(-10,H+10,34)]
    yy,xx=np.mgrid[0:H,0:W]
    warp=gaussian_filter(r.normal(size=(2,H,W)),12)*60
    pts=np.c_[(xx+warp[0]).ravel(),(yy+warp[1]).ravel()]
    d,_=cKDTree(seeds).query(pts,k=2)
    edge=((d[:,1]-d[:,0])/2).reshape(H,W)
    amp=r.uniform(0.55,2.05,len(seeds))
    z=1.5*np.exp(-(edge/2.1)**2)
    return z
def noise(seed,sigma=1.0,corr=8):
    n=gaussian_filter(np.random.default_rng(seed).normal(size=(H,W)),corr)
    return n/n.std()*sigma
NB=[(-1,-1),(-1,0),(-1,1),(0,-1),(0,1),(1,-1),(1,0),(1,1)]
def graph(z,mode,pen=None):
    idx=np.arange(H*W).reshape(H,W);rows=[];cols=[];w=[]
    for dy,dx in NB:
        ys=slice(max(0,-dy),H-max(0,dy));xs=slice(max(0,-dx),W-max(0,dx))
        yt=slice(max(0,dy),H-max(0,-dy)) ;xt=slice(max(0,dx),W-max(0,-dx))
        a=idx[ys,xs];b=idx[yt,xt]
        ds=np.hypot(dy,dx);dz=z[yt,xt]-z[ys,xs];g=np.abs(dz)/ds
        sl=np.arctan(g)
        if mode=='short': c=ds*np.ones_like(g)
        else: c=ds*(1+8*g**1.5*np.where(dz>0,1,0.6))
        if pen is not None: c=c+ds*pen[yt,xt]*30
        c=np.where(sl>SLOPE_LIM,np.inf,c)   # impassable on nominal terrain
        ok=np.isfinite(c);rows+=list(a[ok]);cols+=list(b[ok]);w+=list(c[ok])
    return sp.csr_matrix((w,(rows,cols)),shape=(H*W,H*W))
def route(G,s,t):
    dist,pred=dijkstra(G,indices=s,return_predecessors=True)
    if not np.isfinite(dist[t]):return None
    p=[t]
    while p[-1]!=s:p.append(pred[p[-1]])
    return np.array(p[::-1])
def exceed(z,p,ds_cells=True):
    y,x=np.divmod(p,W);dz=np.abs(np.diff(z[y,x]));ds=np.hypot(np.diff(y),np.diff(x))
    return (np.arctan(dz/ds)>SLOPE_LIM)
res={'short':[],'effort':[],'robust':[]};lens={'short':[],'effort':[],'robust':[]}
for ti in range(6):
    z=terrain(100+ti)
    zs=[z+noise(1000*ti+k) for k in range(40)]        # planning noise samples
    # exceedance probability per cell from planning samples
    ex=np.zeros((H,W))
    for zz in zs:
        g=np.hypot(*np.gradient(zz));ex+=(np.arctan(g)>SLOPE_LIM)
    ex/=len(zs)
    Gs=graph(z,'short');Ge=graph(z,'effort');Gr=graph(z,'effort',ex)
    evalz=[z+noise(50000+1000*ti+k) for k in range(60)]  # fresh noise
    for q in range(6):
        s=np.random.default_rng(q+10*ti).integers(0,H*W);t=np.random.default_rng(q+99+10*ti).integers(0,H*W)
        sy,sx=divmod(s,W);ty,tx=divmod(t,W)
        if np.hypot(sy-ty,sx-tx)<60:continue
        ps={'short':route(Gs,s,t),'effort':route(Ge,s,t),'robust':route(Gr,s,t)}
        if any(v is None for v in ps.values()):continue
        for k,p in ps.items():
            fr=np.mean([exceed(zz,p).mean() for zz in evalz])
            res[k].append(fr);y,x=np.divmod(p,W);lens[k].append(np.hypot(np.diff(y),np.diff(x)).sum())
n=len(res['short'])
print('pairs',n)
for k in res:print(k,'mean fraction of steps over 20deg on fresh noisy terrain: %.4f'%np.mean(res[k]),' mean length m: %.1f'%np.mean(lens[k]))
