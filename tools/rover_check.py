"""Rover check (rule fixed before any DEM tilt was computed).

Question: at rover scale, does the 1 m DEM reproduce the tilt Curiosity measured where it parked (sols 4534..4865)?
Method: for each ROVER stop in PLACES, sample the DEM bilinearly every 0.25 m inside a 3.0 m (along heading) x 2.8 m (across)
rectangle centred on the PLACES position, fit a least-squares plane, and compare its tilt with the rover's total tilt
acos(cos pitch * cos roll) (heading-free). Conventions (PLACES SIS, Rankin et al. 2020): pitch + = nose up, roll + = left side up,
yaw = heading clockwise from north.
Outcome: AGREES if median |d| <= 2 and p90 <= 5 deg and the median is below the 5th percentile of the circular-shift medians
(DEM tilts shifted by 5..80 stops along the sol order); PARTIAL if <= 3 / <= 6 and below that percentile; else DOES NOT AGREE;
UNVALIDATED if fewer than 20 stops are usable. Per stop: <= 2 agrees, <= 3 partial, > 3 disagrees.
Context (never changes the outcome): position shifts 0.5/1/2 m in 8 directions, pitch/roll components and a flipped-sign
diagnostic, a 5 m x 5 m footprint, and the DEM smoothed with a 5 x 5 m box filter.

Usage:
  python tools/rover_check.py selftest
  python tools/rover_check.py run data/raw/localized_pos.csv data/raw/MSL_Gale_DEM_Mosaic_1m_v3.tif [--prep target/release/prep] [--out data/interim/rover_check]
Needs numpy and scipy. Reads DEM windows through `prep dump`.
"""
import csv
import math
import os
import re
import struct
import subprocess
import sys

import numpy as np

import config  # single source of truth for paths and study constants
from config import MARS_RADIUS_M as R, SOL_FIRST as SOL0, SOL_LAST as SOL1
STEP = 0.25
FOOT = (3.0, 2.8)
FOOT_CTX = (5.0, 5.0)
HALF_WIN_M = 10.0
SHIFT_MIN, SHIFT_MAX = 5, 80
AGREE, PARTIAL = (2.0, 5.0), (3.0, 6.0)
MIN_STOPS = 20


# ---------- geometry (tested by selftest) ----------
def axes(yaw_deg):
    y = math.radians(yaw_deg)
    fwd = (math.sin(y), math.cos(y))          # (east, north)
    right = (math.cos(y), -math.sin(y))
    left = (-right[0], -right[1])
    return fwd, right, left


def bilinear(z, x, y):
    """z[row, col]; x = col index, y = row index (pixel-centre coordinates). NaN outside or if any corner is NaN."""
    h, w = z.shape
    x0 = np.floor(x).astype(int); y0 = np.floor(y).astype(int)
    fx = x - x0; fy = y - y0
    ok = (x0 >= 0) & (y0 >= 0) & (x0 + 1 < w) & (y0 + 1 < h)
    out = np.full(np.shape(x), np.nan)
    if not ok.any():
        return out
    xa, ya, fxa, fya = x0[ok], y0[ok], fx[ok], fy[ok]
    v = (z[ya, xa] * (1 - fxa) * (1 - fya) + z[ya, xa + 1] * fxa * (1 - fya)
         + z[ya + 1, xa] * (1 - fxa) * fya + z[ya + 1, xa + 1] * fxa * fya)
    out[ok] = v
    return out


def plane_at(z, cell, x_c, y_c, yaw, foot, de0=0.0, dn0=0.0):
    """Fit a plane to the DEM under a footprint centred at array coords (x_c, y_c) (+ offset de0/dn0 metres east/north).
    Returns (tilt, pitch, roll) in degrees, or None if any sample is missing."""
    fwd, right, left = axes(yaw)
    L, W = foot
    us = np.arange(-L / 2 + STEP / 2, L / 2, STEP)
    vs = np.arange(-W / 2 + STEP / 2, W / 2, STEP)
    U, V = np.meshgrid(us, vs)
    de = de0 + U * fwd[0] + V * right[0]
    dn = dn0 + U * fwd[1] + V * right[1]
    zz = bilinear(z, x_c + de / cell, y_c - dn / cell)
    if np.isnan(zz).any():
        return None
    A = np.column_stack([de.ravel(), dn.ravel(), np.ones(de.size)])
    (a, b, _), *_ = np.linalg.lstsq(A, zz.ravel(), rcond=None)
    tilt = math.degrees(math.atan(math.hypot(a, b)))
    pitch = math.degrees(math.atan(a * fwd[0] + b * fwd[1]))
    roll = math.degrees(math.atan(a * left[0] + b * left[1]))
    return tilt, pitch, roll


def rover_tilt(pitch, roll):
    return math.degrees(math.acos(math.cos(math.radians(pitch)) * math.cos(math.radians(roll))))


def q(v, f):
    v = sorted(v)
    return v[round(f * (len(v) - 1))]


def outcome(med, p90, below_chance, n):
    if n < MIN_STOPS:
        return "UNVALIDATED"
    if med <= AGREE[0] and p90 <= AGREE[1] and below_chance:
        return "AGREES"
    if med <= PARTIAL[0] and p90 <= PARTIAL[1] and below_chance:
        return "PARTIAL"
    return "DOES NOT AGREE"


def selftest():
    cell = 1.0
    t = math.tan(math.radians(10.0))
    h = w = 41
    cols = np.arange(w)
    z = np.tile(cols * t, (h, 1)).astype(float)          # rises to the east by 10 deg
    xc = yc = 20.0
    for yaw, want_p, want_r in ((90, 10, 0), (270, -10, 0), (0, 0, -10), (180, 0, 10)):
        tl, p, r = plane_at(z, cell, xc, yc, yaw, FOOT)
        assert abs(tl - 10) < 1e-6 and abs(p - want_p) < 1e-6 and abs(r - want_r) < 1e-6, (yaw, tl, p, r)
    zn = np.tile((np.arange(h)[::-1] * t)[:, None], (1, w))  # rises to the north (row index runs south)
    tl, p, r = plane_at(zn, cell, xc, yc, 0, FOOT)
    assert abs(p - 10) < 1e-6 and abs(r) < 1e-6, (tl, p, r)
    assert abs(rover_tilt(6, 8) - math.degrees(math.acos(math.cos(math.radians(6)) * math.cos(math.radians(8))))) < 1e-12
    zz = z.copy(); zz[20, 20] = np.nan
    assert plane_at(zz, cell, xc, yc, 90, FOOT) is None
    assert outcome(1.9, 4.9, True, 85) == "AGREES" and outcome(2.5, 5.5, True, 85) == "PARTIAL"
    assert outcome(1.0, 2.0, False, 85) == "DOES NOT AGREE" and outcome(1.0, 2.0, True, 10) == "UNVALIDATED"
    print("selftest passed: plane tilt/pitch/roll signs (east and north planes, 4 headings), nodata, outcome rule (synthetic only)")


# ---------- data access ----------
def load(path):
    rows = []
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            if r["frame"] != "ROVER" or not (SOL0 <= int(r["sol"]) <= SOL1):
                continue
            rows.append(dict(sol=int(r["sol"]), site=r["site"], drive=r["drive"],
                             lat=float(r["planetocentric_latitude"]), lon=float(r["longitude"]),
                             pitch=float(r["pitch"]), roll=float(r["roll"]), yaw=float(r["yaw"])))
    return rows


def tiepoint(prep, dem):
    out = subprocess.run([prep, "info", dem], capture_output=True, text=True, check=True).stdout
    m = re.search(r"model tiepoint Some\(\[([^\]]+)\]\)", out)
    v = [float(x) for x in m.group(1).split(",")]
    m2 = re.search(r"pixel scale Some\(([0-9.]+)\)", out)
    return v[3], v[4], float(m2.group(1))


def window(prep, dem, lat, lon, tmp):
    dlat = math.degrees(HALF_WIN_M / R)
    dlon = math.degrees(HALF_WIN_M / (R * math.cos(math.radians(lat))))
    out = subprocess.run([prep, "dump", dem, f"{lat + dlat:.9f}", f"{lon - dlon:.9f}", f"{lat - dlat:.9f}", f"{lon + dlon:.9f}", tmp],
                         capture_output=True, text=True, check=True).stdout
    m = re.search(r"rows (\d+)\.\.(\d+) cols (\d+)\.\.(\d+)", out)
    r0, c0 = int(m.group(1)), int(m.group(3))
    buf = open(tmp, "rb").read()
    w, h = struct.unpack("<II", buf[:8])
    cell = struct.unpack("<f", buf[8:12])[0]
    z = np.frombuffer(buf[12:], dtype="<f4").reshape(h, w).astype(float)
    return z, r0, c0, cell


def run(places, dem, prep, outdir):
    from scipy.ndimage import uniform_filter
    os.makedirs(outdir, exist_ok=True)
    X0, Y0, scale = tiepoint(prep, dem)
    rows = load(places)
    tmp = os.path.join(outdir, "_win.f32")
    res = []
    dirs = [(math.sin(math.radians(a)), math.cos(math.radians(a))) for a in range(0, 360, 45)]
    for r in rows:
        z, r0, c0, cell = window(prep, dem, r["lat"], r["lon"], tmp)
        col = (R * math.radians(r["lon"]) - X0) / scale
        row = (Y0 - R * math.radians(r["lat"])) / scale
        xc, yc = col - c0 - 0.5, row - r0 - 0.5       # pixel-centre array coordinates
        main = plane_at(z, cell, xc, yc, r["yaw"], FOOT)
        ctx5 = plane_at(z, cell, xc, yc, r["yaw"], FOOT_CTX)
        zs = uniform_filter(z, size=5, mode="nearest")
        sm = plane_at(zs, cell, xc, yc, r["yaw"], FOOT)
        shifts = {d: [plane_at(z, cell, xc, yc, r["yaw"], FOOT, d * e, d * n) for e, n in dirs] for d in (0.5, 1.0, 2.0)}
        res.append(dict(r, rt=rover_tilt(r["pitch"], r["roll"]), main=main, ctx5=ctx5, sm=sm, shifts=shifts))
    os.remove(tmp)
    use = [x for x in res if x["main"] is not None]
    n = len(use)
    lines = []
    P = lines.append
    P(f"Rover check (rule fixed in advance). Stops: {len(res)}, usable: {n}")
    if n == 0:
        P("UNVALIDATED: no usable stops")
        print("\n".join(lines)); return
    rt = np.array([x["rt"] for x in use]); dt = np.array([x["main"][0] for x in use])
    d = dt - rt; ad = np.abs(d)
    med, p90 = q(ad, .5), q(ad, .9)
    shift_meds = [q(np.abs(np.roll(dt, k) - rt), .5) for k in range(SHIFT_MIN, min(SHIFT_MAX, n - SHIFT_MIN) + 1)]
    p5 = q(shift_meds, .05)
    below = med < p5
    r_corr = float(np.corrcoef(dt, rt)[0, 1])
    P(f"PRIMARY (total tilt, 3.0 x 2.8 m footprint): N {n}  median |d| {med:.2f}  p90 |d| {p90:.2f}  signed median {q(d, .5):+.2f}  "
      f"mean signed {d.mean():+.2f}  r {r_corr:.3f}")
    P(f"  rover tilt median {q(rt, .5):.2f}; DEM tilt median {q(dt, .5):.2f}")
    P(f"  chance baseline: {len(shift_meds)} circular shifts ({SHIFT_MIN}..{SHIFT_MIN + len(shift_meds) - 1} stops): median of shifted medians "
      f"{q(shift_meds, .5):.2f}, 5th pct {p5:.2f}; observed below 5th pct: {below}")
    P(f"  flat-map baseline (predicts 0 tilt): median |d| {q(rt, .5):.2f}")
    verdict = outcome(med, p90, below, n)
    P(f"OUTCOME: {verdict}")
    per = {"agrees": int((ad <= 2).sum()), "partial": int(((ad > 2) & (ad <= 3)).sum()), "disagrees": int((ad > 3).sum())}
    P(f"  per stop: agrees (<=2) {per['agrees']}, partial (2-3] {per['partial']}, disagrees (>3) {per['disagrees']}")
    P("CONTEXT (does not change the outcome):")
    for dist in (0.5, 1.0, 2.0):
        vals = [abs(s[0] - x["rt"]) for x in use for s in x["shifts"][dist] if s is not None]
        P(f"  position shift {dist} m (8 directions pooled): median |d| {q(vals, .5):.2f}  p90 {q(vals, .9):.2f}")
    for name, key in (("5 x 5 m footprint", "ctx5"), ("5 x 5 m box-smoothed DEM", "sm")):
        vv = [abs(x[key][0] - x["rt"]) for x in use if x[key] is not None]
        P(f"  {name}: N {len(vv)} median |d| {q(vv, .5):.2f}  p90 {q(vv, .9):.2f}")
    dp = np.array([x["main"][1] - x["pitch"] for x in use]); dr = np.array([x["main"][2] - x["roll"] for x in use])
    fp = np.array([-x["main"][1] - x["pitch"] for x in use]); fr = np.array([-x["main"][2] - x["roll"] for x in use])
    P(f"  components: median |d pitch| {q(np.abs(dp), .5):.2f}, |d roll| {q(np.abs(dr), .5):.2f}; "
      f"flipped signs: |d pitch| {q(np.abs(fp), .5):.2f}, |d roll| {q(np.abs(fr), .5):.2f}")
    if q(np.abs(fp), .5) <= 0.5 * q(np.abs(dp), .5) or q(np.abs(fr), .5) <= 0.5 * q(np.abs(dr), .5):
        P("  SIGN DIAGNOSTIC: a flipped convention halves a component error -> components reported as a convention problem (inconclusive)")
    else:
        P("  sign diagnostic: documented convention not beaten by half by a flipped variant")
    txt = "\n".join(lines)
    print(txt)
    open(os.path.join(outdir, "rover_check_summary.txt"), "w", encoding="utf-8").write(txt + "\n")
    import datetime
    import json
    json.dump(dict(run_utc=datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                   rule="doc: Rover-check rule (approved)", outcome=verdict, n=n, stops=len(res),
                   median_abs_deg=round(float(med), 2), p90_abs_deg=round(float(p90), 2), signed_median_deg=round(float(q(d, .5)), 2),
                   mean_signed_deg=round(float(d.mean()), 2), r=round(r_corr, 3), shift_baseline_p5_deg=round(float(p5), 2),
                   below_chance=bool(below), per_stop=per),
              open(os.path.join(outdir, "rover_check_result.json"), "w"), indent=1)
    with open(os.path.join(outdir, "rover_check_stops.csv"), "w", newline="", encoding="utf-8") as f:
        wr = csv.writer(f)
        wr.writerow(["sol", "site", "drive", "lat", "lon", "yaw", "rover_pitch", "rover_roll", "rover_tilt", "dem_tilt", "dem_pitch", "dem_roll", "abs_diff", "label"])
        for x in res:
            if x["main"] is None:
                wr.writerow([x["sol"], x["site"], x["drive"], x["lat"], x["lon"], x["yaw"], x["pitch"], x["roll"], f"{x['rt']:.3f}", "", "", "", "", "unusable"])
                continue
            a = abs(x["main"][0] - x["rt"])
            lab = "agrees" if a <= 2 else ("partial" if a <= 3 else "disagrees")
            wr.writerow([x["sol"], x["site"], x["drive"], x["lat"], x["lon"], x["yaw"], x["pitch"], x["roll"], f"{x['rt']:.3f}",
                         f"{x['main'][0]:.3f}", f"{x['main'][1]:.3f}", f"{x['main'][2]:.3f}", f"{a:.3f}", lab])


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a or a[0] not in ("selftest", "run"):
        sys.exit(__doc__)
    if a[0] == "selftest":
        selftest()
    else:
        prep = a[a.index("--prep") + 1] if "--prep" in a else config.PREP
        out = a[a.index("--out") + 1] if "--out" in a else config.ROVER_CHECK_DIR
        run(a[1], a[2], prep, out)
