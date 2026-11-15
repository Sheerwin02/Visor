"""Synthetic boxwork-like terrain for SENSITIVITY tests only. Not Gale. Not a result.
Dimensions used (sources opened): ridges average 4-5 m wide (Siebach et al., LPSC 2013, method not stated); ridges up to ~5 m across
and hollows 10-20 m in diameter (Siebach et al., GSA 2025 abstract); hollows usually 0.2-0.5 m deep, steep walls up to 3.5 m (LPSC 2013).
Ridge HEIGHT/slope is NOT given by those sources, so relief is a swept parameter, and the cosine ridge profile is my assumption.
Run: python synth_boxwork.py"""
import numpy as np, sys, os
from scipy.spatial import cKDTree
sys.path.insert(0, os.path.dirname(__file__))
import misleading_slope as ms

def make(size_m=300.0, step_m=0.25, hollow_d=15.0, ridge_w=4.5, relief=1.0, seed=0):
    rng = np.random.default_rng(seed)
    pitch = hollow_d + ridge_w
    n = int(size_m / pitch) + 2
    gx, gy = np.meshgrid(np.arange(n) * pitch, np.arange(n) * pitch)
    seeds = np.c_[gx.ravel() + rng.uniform(-.3, .3, gx.size) * pitch, gy.ravel() + rng.uniform(-.3, .3, gx.size) * pitch]
    m = int(size_m / step_m)
    xs = (np.arange(m) + .5) * step_m
    X, Y = np.meshgrid(xs, xs)
    d, _ = cKDTree(seeds).query(np.c_[X.ravel(), Y.ravel()], k=2)
    e = ((d[:, 1] - d[:, 0]) / 2).reshape(m, m)        # distance to the nearest cell edge (ridge centre line)
    e0 = ridge_w / 2                                     # ridge base half-width
    prof = np.where(e < e0, 0.5 * (1 + np.cos(np.pi * e / e0)), 0.0)
    return relief * prof, step_m

def block_mean(z, k):
    h, w = (z.shape[0] // k) * k, (z.shape[1] // k) * k
    return z[:h, :w].reshape(h // k, k, w // k, k).mean(axis=(1, 3))

def share(dem, cell, el, band=15.0, tol=0.10, smooth=0.0):
    slope, da = ms.slope_aspect(dem, cell, smooth_m=smooth)
    mask = np.ones(dem.shape, bool)
    v = [ms.misleading_share(slope, da, mask, el, az, tol, band)[0] for az in ms.realistic_az(el)]
    v = [x for x in v if x is not None]
    return (min(v), float(np.mean(v)), max(v)) if v else None

if __name__ == "__main__":
    out = []
    P = lambda *a: (print(*a), out.append(" ".join(str(x) for x in a)))
    P("SYNTHETIC SENSITIVITY TEST. Terrain is invented from published widths; ridge relief is swept. Not a Gale result.")
    P("\n[1] How much of the ground is >=15 deg at 1 m, by ridge relief (profile: cosine, base width 4.5 m)")
    for H in (0.3, 0.5, 1.0, 2.0):
        z, st = make(relief=H); z1 = block_mean(z, int(1 / st))
        sl, _ = ms.slope_aspect(z1, 1.0, smooth_m=0)
        P("relief %.1f m: share >=15 deg %.3f, >=25 deg %.3f, max slope %.0f deg" % (H, (np.degrees(sl) >= 15).mean(), (np.degrees(sl) >= 25).mean(), np.degrees(sl).max()))
    P("\n[2] Resolution: same terrain (relief 1.0 m) sampled at coarser cells (block mean), slope at that cell size")
    z, st = make(relief=1.0)
    for cell in (0.25, 1.0, 2.0, 5.0):
        zc = block_mean(z, int(round(cell / st))) if cell > st else z
        sl, _ = ms.slope_aspect(zc, cell, smooth_m=0)
        P("cell %.2f m: share >=15 deg %.3f, max slope %.0f deg" % (cell, (np.degrees(sl) >= 15).mean(), np.degrees(sl).max()))
    P("\n[3] Smoothing: 1 m cell, relief 1.0 m, share >=15 deg after gaussian smoothing")
    z1 = block_mean(z, 4)
    for sm in (0, 1, 2):
        sl, _ = ms.slope_aspect(z1, 1.0, smooth_m=sm)
        P("smooth %d m: share >=15 deg %.3f, max slope %.0f deg" % (sm, (np.degrees(sl) >= 15).mean(), np.degrees(sl).max()))
    P("\n[4] Looks-flat share of ground >=15 deg (tol 0.10, realistic Gale azimuths, no shadows), 1 m cell, by relief and sun elevation: min / mean / max over azimuths")
    for H in (0.5, 1.0, 2.0):
        z, st = make(relief=H); z1 = block_mean(z, 4)
        for el in (15, 30, 50, 60):
            r = share(z1, 1.0, el)
            P("relief %.1f m, sun %2d deg: %s" % (H, el, "none steep" if r is None else "%.2f / %.2f / %.2f" % r))
    open(os.path.join(os.path.dirname(__file__), "synth_boxwork_results.txt"), "w").write("\n".join(out) + "\n")
