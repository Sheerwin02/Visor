"""How long does it take to find a small target when your position error is sigma?
Assumptions (labelled, not measured): walking speed V m/s, detection range R m (distance at which the crew recognises
the target), target offset from believed position ~ 2D Gaussian(sigma). Expanding-square spiral from the believed position,
track spacing 2R (sweep width W=2R, US Coast Guard Theory of Search). No terrain occlusion, no fatigue, perfect detection inside R.
Run: python searchsim.py"""
import numpy as np
from scipy.spatial import cKDTree
V = 0.6   # m/s  assumption (same as the proposal's walking-speed assumption)
def spiral(R, total_len):
    # expanding square spiral with leg increments of 2R, 1 m steps
    pts=[(0.0,0.0)]; x=y=0.0; d=2*R; dirs=[(1,0),(0,1),(-1,0),(0,-1)]; k=0; leg=d; L=0.0
    while L<total_len:
        dx,dy=dirs[k%4]
        n=int(round(leg))
        for _ in range(n):
            x+=dx; y+=dy; pts.append((x,y)); L+=1
            if L>=total_len: break
        k+=1
        if k%2==0: leg+=d
    return np.array(pts)
def run(sigma,R,minutes=(15,30,60,120),n=4000,seed=0):
    rng=np.random.default_rng(seed)
    T=rng.normal(0,sigma,(n,2))
    path=spiral(R,V*max(minutes)*60)
    tree=cKDTree(path)
    first=np.full(n,np.inf)
    hits=tree.query_ball_point(T,R)
    for i,h in enumerate(hits):
        if h: first[i]=min(h)/V/60.0   # minutes
    out={m: np.mean(first<=m) for m in minutes}
    med=np.median(first[np.isfinite(first)]) if np.isfinite(first).any() else np.nan
    t90=np.percentile(first,90) if np.mean(np.isfinite(first))>=0.9 else np.inf
    return out,med,t90
if __name__=='__main__':
    print("V=%.1f m/s (assumed). P(found) within 15/30/60/120 min; median time of those found; 90th-percentile time (inf = not reached in 120 min)\n"%V)
    for R in (10,20,40):
        for s in (10,25,50,100,150):
            out,med,t90=run(s,R)
            print(f"R={R:3d} m  sigma={s:4d} m | "+"  ".join(f"{m}min {out[m]*100:4.0f}%" for m in out)+f" | median {med:5.1f} min | p90 {t90:6.1f} min",flush=True)
