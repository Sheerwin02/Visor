import numpy as np
from scipy.ndimage import gaussian_filter
from sim_lib import terrain, noise, H, W
z=terrain(101)
def slope(z,s):
    zs=gaussian_filter(z,s) if s>0 else z
    gy,gx=np.gradient(zs);return np.degrees(np.arctan(np.hypot(gx,gy)))
rows=[]
for name,mk in [('white noise 0.3 m',lambda k:np.random.default_rng(k).normal(size=(H,W))*0.3),
                ('correlated noise 1 m, length 8 m',lambda k:noise(k,1.0,8))]:
    for s in [0,1,2,3,5]:
        ref=slope(z,s)      # slope of the true terrain at that same smoothing
        err=[];keep=[]
        for k in range(10):
            zn=z+mk(k)
            err.append(np.sqrt(np.mean((slope(zn,s)-ref)**2)))
        # signal kept: slope of smoothed truth compared with 1 m-scale truth slope
        base=slope(z,0)
        rows.append((name,s,np.mean(err),np.sqrt(np.mean(ref**2)),np.sqrt(np.mean(base**2))))
print('noise | smoothing sigma (m) | RMS slope error (deg) | RMS true slope at that smoothing (deg) | RMS true slope unsmoothed (deg)')
for r in rows:print('%s | %d | %.2f | %.2f | %.2f'%r)
