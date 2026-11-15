import numpy as np, scipy.sparse as sp
from scipy.sparse.csgraph import dijkstra
from sim_lib import *
import sim_lib
def build(z,mat,fh):
    idx=np.arange(H*W).reshape(H,W);rows=[];cols=[];w=[]
    for dy,dx in NB:
        ys=slice(max(0,-dy),H-max(0,dy));xs=slice(max(0,-dx),W-max(0,dx))
        yt=slice(max(0,dy),H-max(0,-dy));xt=slice(max(0,dx),W-max(0,-dx))
        a=idx[ys,xs];b=idx[yt,xt];ds=np.hypot(dy,dx);dz=z[yt,xt]-z[ys,xs];g=np.abs(dz)/ds
        f=np.where(mat[yt,xt]==0,fh,1.1)   # hollow = fh, ridge = 1.1
        c=ds*(1+8*g**1.5*np.where(dz>0,1,0.6))*f
        c=np.where(np.arctan(g)>SLOPE_LIM,np.inf,c)
        ok=np.isfinite(c);rows+=list(a[ok]);cols+=list(b[ok]);w+=list(c[ok])
    return sp.csr_matrix((w,(rows,cols)),shape=(H*W,H*W))
def cost(z,mat,p,fh):
    y,x=np.divmod(p,W);dy=np.diff(y);dx=np.diff(x);ds=np.hypot(dy,dx);dz=np.diff(z[y,x]);g=np.abs(dz)/ds
    f=np.where(mat[y[1:],x[1:]]==0,fh,1.1)
    return (ds*(1+8*g**1.5*np.where(dz>0,1,0.6))*f).sum()
plan_f=[1.0,1.25,2.1];true_f=[1.0,1.25,2.1]
reg={(a,b):[] for a in plan_f for b in true_f}; hol={a:[] for a in plan_f}; ln={a:[] for a in plan_f}
for ti in range(6):
    z=terrain(100+ti);mat=(z>0.45).astype(int)  # 1 = ridge, 0 = hollow
    Gs={f:build(z,mat,f) for f in plan_f}
    for q in range(6):
        s=np.random.default_rng(q+10*ti).integers(0,H*W);t=np.random.default_rng(q+99+10*ti).integers(0,H*W)
        sy,sx=divmod(s,W);ty,tx=divmod(t,W)
        if np.hypot(sy-ty,sx-tx)<60:continue
        ps={f:route(Gs[f],s,t) for f in plan_f}
        if any(v is None for v in ps.values()):continue
        for tf in true_f:
            best=cost(z,mat,ps[tf],tf)
            for pf in plan_f: reg[(pf,tf)].append(cost(z,mat,ps[pf],tf)/best)
        for pf in plan_f:
            y,x=np.divmod(ps[pf],W);hol[pf].append((mat[y,x]==0).mean());ln[pf].append(np.hypot(np.diff(y),np.diff(x)).sum())
n=len(ln[1.0]);print('pairs',n)
print('Extra cost (percent) when the route was planned with factor P but the true hollow factor is T:')
for tf in true_f:
    print(' true',tf,' '.join('plan%.2f:%.1f%%'%(pf,100*(np.mean(reg[(pf,tf)])-1)) for pf in plan_f))
for pf in plan_f:print('plan hollow factor',pf,'share of route in hollows %.2f'%np.mean(hol[pf]),'length %.1f m'%np.mean(ln[pf]))
