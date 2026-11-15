"""Spiral vs greedy probability-ordered search (same assumptions as searchsim.py: V=0.6 m/s, detection range R, no occlusion,
perfect detection, target ~ Gaussian(sigma) about believed position). Greedy: grid of waypoints spaced 2R; always walk to the unvisited
waypoint with highest (prior mass)/(travel distance). Straight-line travel counts as searching (path sampled at 1 m).
Run: python compare.py"""
import numpy as np
from scipy.spatial import cKDTree
from searchsim import spiral, V
def greedy(sigma,R,total_len):
    sp=2*R; ext=int(np.ceil(3*max(sigma,sp)/sp))
    g=np.arange(-ext,ext+1)*sp; gx,gy=np.meshgrid(g,g); W=np.c_[gx.ravel(),gy.ravel()].astype(float)
    mass=np.exp(-(W**2).sum(1)/(2*sigma**2)); done=np.zeros(len(W),bool)
    pos=np.zeros(2); pts=[pos.copy()]; L=0.0
    while L<total_len and not done.all():
        d=np.hypot(*(W-pos).T); score=np.where(done,-1,mass/(d+sp)); i=score.argmax()
        n=max(1,int(d[i])); seg=pos+(W[i]-pos)*np.linspace(0,1,n+1)[1:,None]
        pts.extend(seg); L+=d[i]; pos=W[i].copy(); done[i]=True
    return np.array(pts)
def first_times(path,T,R):
    hits=cKDTree(path).query_ball_point(T,R); f=np.full(len(T),np.inf)
    for i,h in enumerate(hits):
        if h: f[i]=min(h)/V/60
    return f
if __name__=='__main__':
    rng=np.random.default_rng(0); R=20
    print("R=20 m, V=0.6 m/s (assumed). P(found) in 30/60 min and median time among found, 3000 targets")
    for s in (25,50,100,150):
        T=rng.normal(0,s,(3000,2)); L=V*120*60
        for name,p in (("spiral",spiral(R,L)),("greedy",greedy(s,R,L))):
            f=first_times(p,T,R); ok=np.isfinite(f)
            print(f"sigma={s:4d} {name:6s} 30min {np.mean(f<=30)*100:4.0f}%  60min {np.mean(f<=60)*100:4.0f}%  median(found) {np.median(f[ok]):5.1f} min")
