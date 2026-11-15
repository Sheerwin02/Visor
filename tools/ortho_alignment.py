"""Photo-to-DEM alignment check (first run as an inline analysis; saved here as a script so it can be reproduced).

Method: resample the 25 cm photo to the 1 m DEM grid (4x4 mean); band-pass its brightness (Gaussian 2 m minus 15 m); fit
brightness = a * dz/deast + b * dz/dsouth + c (unknown sun direction) on the DEM smoothed with a 2 m Gaussian; repeat for DEM
shifts of -8..+8 m in each direction and report R squared at zero shift and at the best shift. A low R squared overall is
expected (colour, not slope, drives most brightness); the check only asks whether a clear offset exists.
Writes data/processed/ortho/alignment.json. Usage: python tools/ortho_alignment.py
"""
import json
import math
import os
import re
import struct
import subprocess
import tempfile

import numpy as np
import rasterio
from scipy.ndimage import gaussian_filter

import config

MAX_SHIFT = 8
EDGE = 20


def main():
    R = config.MARS_RADIUS_M
    with rasterio.open(config.ORTHO) as d:
        img = d.read().astype(float).mean(axis=0); T = d.transform
    H, W = img.shape
    x0, y1 = T.c, T.f
    x1, y0 = x0 + W * 0.25, y1 - H * 0.25
    info = subprocess.run([config.PREP, "info", config.DEM], capture_output=True, text=True, check=True).stdout
    v = [float(s) for s in re.search(r"tiepoint Some\(\[([^\]]+)\]", info).group(1).split(",")]
    X0, Y0 = v[3], v[4]
    lat = lambda y: math.degrees(y / R); lon = lambda x: math.degrees(x / R)
    pad = MAX_SHIFT + 7
    with tempfile.TemporaryDirectory() as td:
        f = os.path.join(td, "dem.f32")
        out = subprocess.run([config.PREP, "dump", config.DEM, f"{lat(y1 + pad):.9f}", f"{lon(x0 - pad):.9f}", f"{lat(y0 - pad):.9f}",
                              f"{lon(x1 + pad):.9f}", f], capture_output=True, text=True, check=True).stdout
        dr0, _, dc0, _ = map(int, re.search(r"rows (\d+)\.\.(\d+) cols (\d+)\.\.(\d+)", out).groups())
        b = open(f, "rb").read()
    w, h = struct.unpack("<II", b[:8])
    z = np.frombuffer(b[12:], "<f4").reshape(h, w).astype(float)
    c_start, r_start = math.ceil(x0 - X0), math.ceil(Y0 - y1)
    ox, oy = int(round((X0 + c_start - x0) / 0.25)), int(round((y1 - (Y0 - r_start)) / 0.25))
    nh, nw = (H - oy) // 4, (W - ox) // 4
    o1 = img[oy:oy + 4 * nh, ox:ox + 4 * nw].reshape(nh, 4, nw, 4).mean(axis=(1, 3))
    I = gaussian_filter(o1, 2.0) - gaussian_filter(o1, 15)
    gy, gx = np.gradient(gaussian_filter(z, 2.0))
    res = []
    for dy in range(-MAX_SHIFT, MAX_SHIFT + 1):
        for dx in range(-MAX_SHIFT, MAX_SHIFT + 1):
            ra, ca = r_start - dr0 + dy, c_start - dc0 + dx
            Gx, Gy = gx[ra:ra + nh, ca:ca + nw], gy[ra:ra + nh, ca:ca + nw]
            m = slice(EDGE, -EDGE)
            y = I[m, m].ravel()
            X = np.column_stack([Gx[m, m].ravel(), Gy[m, m].ravel(), np.ones(y.size)])
            _, rr, *_ = np.linalg.lstsq(X, y, rcond=None)
            res.append((1 - rr[0] / ((y - y.mean()) ** 2).sum(), dx, dy))
    res.sort(reverse=True)
    zero = next(r for r in res if r[1] == 0 and r[2] == 0)[0]
    out = dict(method=__doc__.split("\n\n")[1].replace("\n", " "), grid_m=[nw, nh],
               r2_zero_shift=round(float(zero), 3), r2_best=round(float(res[0][0]), 3), r2_worst=round(float(res[-1][0]), 3),
               best_shift_e_m=res[0][1], best_shift_s_m=res[0][2],
               top5=[dict(e_m=dx, s_m=dy, r2=round(float(r2), 3)) for r2, dx, dy in res[:5]])
    os.makedirs(os.path.dirname(config.ORTHO), exist_ok=True)
    json.dump(out, open(os.path.join(os.path.dirname(config.ORTHO), "alignment.json"), "w"), indent=1)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
