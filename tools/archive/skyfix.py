"""VISOR Fix prototype: GPS-free localization by matching an observed skyline to DEM skylines.
Heading is UNKNOWN (no compass). Synthetic terrain only; validates method behaviour, not Gale.
Usage: python skyfix.py  (prints a results table)"""
import numpy as np, sys
from scipy.ndimage import map_coordinates, gaussian_filter

N = 500; CELL = 8.0            # 4 km x 4 km at 8 m
NAZ = 180                      # 2 deg azimuth steps
EYE = 1.7
RMAX = 1800.0
RMIN = 16.0
BETA = 2.4
DIST = np.arange(16.0, RMAX, 8.0)

def fractal(rms, rng, beta=None):
    beta = BETA if beta is None else beta
    f = np.fft.fftfreq(N)
    k = np.sqrt(f[:, None]**2 + f[None, :]**2); k[0, 0] = 1
    amp = k**(-beta/2); amp[0, 0] = 0
    z = np.fft.ifft2(amp*np.exp(2j*np.pi*rng.random((N, N)))).real
    return z/z.std()*rms

def skyline(dem, x, y):
    """elevation angle (deg) of horizon at azimuths, from (x,y) in metres. 1 obs or array of obs."""
    x = np.atleast_1d(x); y = np.atleast_1d(y)
    DIST = np.arange(RMIN, RMAX, 8.0)
    az = np.arange(NAZ)*2*np.pi/NAZ
    px = (x[:, None, None] + DIST[None, None, :]*np.sin(az)[None, :, None])/CELL
    py = (y[:, None, None] + DIST[None, None, :]*np.cos(az)[None, :, None])/CELL
    z = map_coordinates(dem, [py.ravel(), px.ravel()], order=1, mode='nearest').reshape(px.shape)
    z0 = map_coordinates(dem, [y/CELL, x/CELL], order=1)[:, None, None] + EYE
    ang = np.degrees(np.arctan2(z - z0, DIST[None, None, :]))
    return ang.max(axis=2)       # (n_obs, NAZ)

def match(obs, cands, fov_bins):
    """obs: (NAZ,) possibly partial (fov_bins wide window, unknown heading). cands (M,NAZ).
    returns score (M,) = min over heading shifts of RMS diff (deg)."""
    w = obs[:fov_bins]
    best = np.full(len(cands), np.inf)
    for s in range(NAZ):
        seg = np.roll(cands, -s, axis=1)[:, :fov_bins]
        # remove mean offset? no: absolute elevation angle is observable (camera pitch known from IMU)
        d = np.sqrt(((seg - w)**2).mean(axis=1))
        best = np.minimum(best, d)
    return best

def trial(rng, relief, dem_err, dem_corr_m, sky_noise, fov_deg, radius=240., spacing=30.):
    truth = fractal(relief, rng)
    err = gaussian_filter(rng.standard_normal((N, N)), dem_corr_m/CELL) if dem_corr_m > 0 else rng.standard_normal((N, N))
    err = err/err.std()*dem_err
    dem = truth + err
    cx, cy = N*CELL/2 + rng.uniform(-300, 300, 2)
    obs_full = skyline(truth, cx, cy)[0] + rng.normal(0, sky_noise, NAZ)
    relief_deg = skyline(truth, cx, cy)[0].std()
    # candidate grid around a *wrong* prior centre (prior error ~ up to radius)
    th = rng.uniform(0, 2*np.pi); r = rng.uniform(0, radius*0.7)
    px0, py0 = cx + r*np.cos(th), cy + r*np.sin(th)
    g = np.arange(-radius, radius+1, spacing)
    gx, gy = np.meshgrid(px0+g, py0+g); m = (gx-px0)**2+(gy-py0)**2 <= radius**2
    gx, gy = gx[m], gy[m]
    cands = skyline(dem, gx, gy)
    fov_bins = int(fov_deg/360*NAZ)
    start = rng.integers(NAZ)       # unknown heading: random window of obs
    obs = np.roll(obs_full, -start)[:fov_bins]
    sc = match(obs, cands, fov_bins)
    i = sc.argmin()
    e = np.hypot(gx[i]-cx, gy[i]-cy)
    # ambiguity: spread of near-best candidates
    near = sc <= sc.min()*1.15 + 0.02
    spread = np.sqrt(((gx[near]-gx[i])**2 + (gy[near]-gy[i])**2).max()) if near.sum() > 1 else 0
    amb = np.hypot(gx[near]-gx[i], gy[near]-gy[i]).max()
    # baseline: prior centre error (no localization)
    return dict(err=e, relief_deg=relief_deg, amb=amb, prior=np.hypot(px0-cx, py0-cy), best=sc.min())

def sweep(label, n=40, seed=1, **kw):
    rng = np.random.default_rng(seed)
    R = [trial(rng, **kw) for _ in range(n)]
    e = np.array([r['err'] for r in R]); p = np.array([r['prior'] for r in R])
    a = np.array([r['amb'] for r in R]); rd = np.mean([r['relief_deg'] for r in R])
    print(f"{label:34s} skyline_std={rd:5.2f}deg  median_err={np.median(e):6.1f} m  p90={np.percentile(e,90):6.1f} m  "
          f"within60m={np.mean(e<=60)*100:4.0f}%  better_than_prior={np.mean(e<p)*100:4.0f}%  median_ambig={np.median(a):6.1f} m", flush=True)
    return R

if __name__ == '__main__':
    base = dict(relief=40., dem_err=1.0, dem_corr_m=8., sky_noise=0.15, fov_deg=360.)
    print("candidate search radius 240 m, spacing 30 m; truth hit if error <= ~30 m grid limit\n")
    for rel in (5., 15., 40., 100.):
        sweep(f"relief rms {rel:.0f} m (FOV360)", **{**base, 'relief': rel})
    print()
    for fov in (360., 180., 90.):
        sweep(f"FOV {fov:.0f} deg (relief 40)", **{**base, 'fov_deg': fov})
    print()
    for de, dc in ((0.3, 8.), (1.0, 8.), (3.0, 8.), (10.0, 8.)):
        sweep(f"DEM err {de} m corr {dc} m", **{**base, 'dem_err': de, 'dem_corr_m': dc})
    print()
    for sn in (0.05, 0.15, 0.5, 1.0):
        sweep(f"skyline noise {sn} deg", **{**base, 'sky_noise': sn})
