"""Navcam check at Nevado Sajama (sol 4718, site 120, drive 82). Rule written in the doc before any data was downloaded.

Inputs (data/raw/navcam/, git-ignored): left-eye, raw-geometry, full-resolution Navcam RDRs from PDS msl_navcam_processed
(DOI 10.17189/2pw6-a555): XYZ (3D point per pixel, SITE_FRAME 120, +X north +Y east +Z down), RNE (per-pixel range error,
band 1 = downrange, metres), SLP (slope in degrees from the rover-sized surface normal; camera SIS 5.2.1.8.1).
Layout per label: RECORD_BYTES 4096, ^IMAGE at record 14 -> byte offset 53248; IEEE_REAL (big-endian float32), band sequential;
missing XYZ = 0.0. The "L" (linearized) variant is the same stereo solution resampled, so only the raw-geometry set is used.

Steps (as in the rule):
 1. Place points: anchor on the PLACES localized rover position (site 120, drive 82); offset = point minus the telemetry rover
    position in the site frame (label ROVER_COORD_SYSTEM_PARMS). Elevation = PLACES elevation + upward offset.
 2. Height check: points within 5 m of the rover; p90 of |DEM height - Navcam height| > 1 m -> placement unreliable, stop.
 3. Filter: RNE band 1 <= 0.10 m and valid XYZ.
 4. Per 1 m DEM cell: median Navcam slope vs the DEM 3 m slope (raw and bias-corrected); N, median/p90 |d|, signed median, r,
    split by distance 0-5, 5-10, 10-20 m. Descriptive only; < 100 cells = "too little data".
 5. Decimated point cloud (<= 50,000 points) for the approach view.
Usage: python tools/navcam_check.py   (run after tools/build_bundle.py build; writes assets/navcam_4718_points.bin and assets/navcam_check.json)
"""
import csv
import json
import math
import os
import re

import numpy as np

import config
D = config.NAVCAM_DIR
STEM = "NLB_816331121{}_F1200082NCAM00309M1"
OFFSET = 53248
N = 1024
RNE_MAX = 0.10
HEIGHT_P90_MAX = 1.0
NEAR_M = 5.0
MIN_CELLS = 100
MAX_POINTS = 50000


def read_img(kind, bands):
    a = np.fromfile(os.path.join(D, STEM.format(kind) + ".IMG"), dtype=">f4", offset=OFFSET, count=bands * N * N)
    return a.reshape(bands, N, N).astype(float)


def rover_tel():
    lbl = open(os.path.join(D, STEM.format("XYZ") + ".LBL"), encoding="latin-1").read()
    g = re.search(r"GROUP\s*=\s*ROVER_COORD_SYSTEM_PARMS(.*?)END_GROUP", lbl, re.S).group(1)
    assert '"ROVER_NAV_FRAME"' in g and "(120,82," in g.replace(" ", ""), "unexpected rover frame group"
    v = re.search(r"ORIGIN_OFFSET_VECTOR\s*=\s*\(([^)]+)\)", g).group(1)
    return [float(x) for x in v.split(",")]


def q(v, f):
    v = np.sort(np.asarray(v)); return float(v[int(round(f * (len(v) - 1)))])


def stats(d, a, b):
    d = np.asarray(d)
    return dict(n=int(len(d)), median_abs=round(q(np.abs(d), .5), 2), p90_abs=round(q(np.abs(d), .9), 2),
                signed_median=round(q(d, .5), 2), r=round(float(np.corrcoef(a, b)[0, 1]), 3) if len(d) > 2 else None)


def main():
    m = json.load(open(os.path.join(config.ASSETS, "manifest.json")))
    g = m["grid"]; H, W, cell, zoff = g["height"], g["width"], g["cell_m"], g["z_offset_m"]
    y_south = g["y_north_m"] - H * cell
    z = np.fromfile(os.path.join(config.ASSETS, "heightmap.bin"), "<f4").reshape(H, W) + zoff
    s3 = np.fromfile(os.path.join(config.ASSETS, "slope_3m.bin"), "<f4").reshape(H, W)
    rov = next(r for r in csv.DictReader(open(config.PLACES)) if r["frame"] == "ROVER" and r["site"] == "120" and r["drive"] == "82")
    Nr, Er, Hr = float(rov["northing"]), float(rov["easting"]), float(rov["elevation"])
    xr, yr, zr = rover_tel()
    xyz, rne, slp = read_img("XYZ", 3), read_img("RNE", 3), read_img("SLP", 1)[0]
    valid = ~((xyz[0] == 0) & (xyz[1] == 0) & (xyz[2] == 0)) & np.isfinite(xyz).all(axis=0)
    dn, de, du = xyz[0] - xr, xyz[1] - yr, -(xyz[2] - zr)
    e = (Er + de) - g["x_west_m"]; n = (Nr + dn) - y_south; elev = Hr + du
    dist = np.hypot(de, dn)
    col = np.floor(e / cell).astype(int); row = np.floor((H * cell - n) / cell).astype(int)
    inside = valid & (row >= 0) & (row < H) & (col >= 0) & (col < W)
    out = dict(rule="doc: Navcam check rule", product=STEM.format("XYZ"), pixels_valid=int(valid.sum()),
               rover_places=dict(northing=Nr, easting=Er, elevation=Hr), rover_telemetry_site_frame=[xr, yr, zr])
    near = inside & (dist <= NEAR_M)
    hd = z[row[near], col[near]] - elev[near]
    out["height_check"] = dict(n=int(near.sum()), median_dem_minus_navcam_m=round(q(hd, .5), 3) if near.any() else None,
                               p90_abs_m=round(q(np.abs(hd), .9), 3) if near.any() else None, limit_m=HEIGHT_P90_MAX)
    ok = near.any() and out["height_check"]["p90_abs_m"] <= HEIGHT_P90_MAX
    out["placement"] = "ok" if ok else "UNRELIABLE (height check failed or no near points); no slope comparison"
    good = inside & (rne[0] > 0) & (rne[0] <= RNE_MAX) & np.isfinite(slp)
    out["pixels_after_rne_filter"] = int(good.sum())
    if ok:
        key = row[good] * W + col[good]
        order = np.argsort(key); ks = key[order]; sv = slp[good][order]; dv = dist[good][order]
        uniq, start = np.unique(ks, return_index=True)
        med = np.array([np.median(sv[a:b]) for a, b in zip(start, list(start[1:]) + [len(ks)])])
        cd = np.array([np.median(dv[a:b]) for a, b in zip(start, list(start[1:]) + [len(ks)])])
        r_, c_ = uniq // W, uniq % W
        import route
        bias = route.measured_bias_deg()
        dem = s3[r_, c_].astype(float); corr = np.clip(dem - bias, 0, None)
        if len(uniq) < MIN_CELLS:
            out["comparison"] = f"too little data ({len(uniq)} cells < {MIN_CELLS})"
        else:
            out["comparison"] = dict(cells=int(len(uniq)), dem_raw=stats(dem - med, dem, med), dem_corrected=stats(corr - med, corr, med),
                                     by_distance={f"{a}-{b} m": (stats(dem[(cd >= a) & (cd < b)] - med[(cd >= a) & (cd < b)], dem[(cd >= a) & (cd < b)], med[(cd >= a) & (cd < b)])
                                                                if ((cd >= a) & (cd < b)).sum() > 2 else "n<3") for a, b in ((0, 5), (5, 10), (10, 20))},
                                     note="d = DEM 3 m slope minus median Navcam slope per 1 m cell; descriptive only")
    pts = np.flatnonzero(good.ravel() & inside.ravel())
    if len(pts) > MAX_POINTS:
        pts = pts[np.linspace(0, len(pts) - 1, MAX_POINTS).astype(int)]
    cloud = np.stack([e.ravel()[pts], n.ravel()[pts], elev.ravel()[pts] - zoff, slp.ravel()[pts]], axis=1).astype("<f4")
    cloud.tofile(os.path.join(config.ASSETS, "navcam_4718_points.bin"))
    out["point_cloud"] = dict(file="navcam_4718_points.bin", points=int(len(cloud)), layout="float32 LE x4: e_m, n_m, height above z_offset_m, slope_deg",
                              credit="NASA/JPL-Caltech; Maki, J. (2024), MSL Navcam Processed Data Bundle, NASA PDS, doi:10.17189/2pw6-a555")
    json.dump(out, open(os.path.join(config.ASSETS, "navcam_check.json"), "w"), indent=1)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
