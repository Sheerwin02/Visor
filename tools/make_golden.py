"""Generate Rust golden fixtures from the Python reference. Run from the repository root: python tools/make_golden.py
Writes crates/terrain-core/tests/fixtures/golden.txt (w h cell, DEM, then per case: el az python_share shade-grid).
Uses smooth_m=0 so the Python and Rust gradients are the same (numpy.gradient edges)."""
import os, sys, numpy as np
sys.path.insert(0, os.path.join(os.path.dirname(__file__)))
import misleading_slope as ms
rng = np.random.default_rng(7)
w = h = 40; cell = 1.0
dem = (np.cumsum(np.cumsum(rng.normal(0, .08, (h, w)), 0), 1)).astype(np.float32).astype(float)
slope, down_az = ms.slope_aspect(dem, cell, smooth_m=0)
out = ["%d %d %r" % (w, h, cell), " ".join("%.7g" % v for v in dem.ravel())]
cases = [(15, 0), (30, 135), (50, 250), (60, 45)]
out.append(str(len(cases)))
mask = np.ones(dem.shape, bool)
for el, az in cases:
    s, n = ms.misleading_share(slope, down_az, mask, el, az, 0.10, 15.0)
    sh = ms.shade(slope, down_az, el, az)
    out.append("%d %d %.7g" % (el, az, -1.0 if s is None else s))
    out.append(" ".join("%.7g" % v for v in sh.ravel()))
p = os.path.join(os.path.dirname(__file__), "..", "crates", "terrain-core", "tests", "fixtures", "golden.txt")
open(p, "w").write("\n".join(out) + "\n"); print("wrote", os.path.abspath(p))
