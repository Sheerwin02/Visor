"""Gate: does the 1 m Gale DEM show ridge-scale slopes above its own noise? (rule fixed in advance
before any slope result; revised from the first proposal after review.)

PATCH: bounding box of Curiosity ROVER positions in PLACES for sols 4534..4865, plus a fixed 50 m margin
  ("rover-track patch": it may include non-boxwork terrain).
STEP 0, MAPPING CHECK: at 3 stops (first, middle, last) the DEM value at the located pixel must be within 1.0 m of the PLACES
  elevation (PLACES takes its elevation from this same DEM, per the PLACES SIS). Any miss -> INCONCLUSIVE, nothing else runs.
NOISE REFERENCE: rover-footprint boxes (3 m x 3 m) at the ROVER stops in these sols where |pitch| < 5 and |roll| < 5 deg
  (chosen from rover tilt only). The rover says that ground is gentle at its own scale, so 1 m pixels >= 15 deg inside it are
  DEM noise or small rocks. Reference share = max(observed share, 3 / N) (rule of three for a small sample).
DECISION (native 1 m, no smoothing):
  PASS         patch share >= 15 deg is >= 3 x reference share AND >= 1%
  FAIL         otherwise
  INCONCLUSIVE mapping check fails, OR reference has < 100 valid pixels, OR the seam veto applies:
               a shaded-relief image of the patch with steep pixels marked (written to --png) shows the steep pixels mostly
               along straight lines (DTM seams) rather than the ridge network. That veto is a visual judgement: report who made it.
Smoothed results (sigma 1, 2, 3 m) and the 25 deg band are reported as context only. Why no smoothing in the decision:
  synthetic 3 m wide x 1 m tall ridges (about rover width, per NASA) vanish at 1 m Gaussian smoothing, and the DEM is already
  smoothed by stereo matching (posts about 4x the image pixel, UAHiRISE).

Usage:
  python tools/gate_patch.py data/raw/localized_pos.csv                       # boxes only
  python tools/gate_patch.py data/raw/localized_pos.csv --run DEM.tif [--prep target/release/prep] [--png out.png]
Needs numpy for the image (stdlib otherwise). Margins in metres on the DEM's own sphere, radius 3396190 m.
"""
import csv
import math
import os
import struct
import subprocess
import sys
import zlib

import config  # single source of truth for paths and study constants
from config import MARS_RADIUS_M as R, SOL_FIRST as SOL0, SOL_LAST as SOL1
MARGIN_M = 50.0
REF_HALF_M = 1.5          # rover-footprint box, 3 m x 3 m (rover about 10 ft x 9 ft, NASA)
LEVEL_DEG = 5.0
MAP_TOL_M = 1.0
MIN_REF_PX = 100
MIN_SHARE, MIN_RATIO = 0.01, 3.0


def box(lat, lon, half_m, mean_lat):
    dlat = math.degrees(half_m / R)
    dlon = math.degrees(half_m / (R * math.cos(math.radians(mean_lat))))
    return lat + dlat, lon - dlon, lat - dlat, lon + dlon  # N, W, S, E


def load(path):
    rows = []
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            if r["frame"] != "ROVER":
                continue
            sol = int(r["sol"])
            lat, lon = float(r["planetocentric_latitude"]), float(r["longitude"])
            if not (SOL0 <= sol <= SOL1) or lat == -9999 or lon == -9999:
                continue
            rows.append(dict(sol=sol, lat=lat, lon=lon, elev=float(r["elevation"]),
                             pitch=float(r["pitch"]), roll=float(r["roll"])))
    if not rows:
        sys.exit("no ROVER rows in the sol range: check the file")
    return rows


def prep_out(prep, *args):
    return subprocess.run([prep, *args], capture_output=True, text=True, check=True).stdout


def slopebox(prep, dem, b, label):
    out = prep_out(prep, "slopebox", dem, *[f"{v:.7f}" for v in b], "--label", label)
    res = {}
    for line in out.splitlines():
        if line.startswith("RESULT,"):
            _, lab, sigma, n, excl, ge15, ge25, *_ = line.split(",")
            res[float(sigma)] = dict(n=int(n), excl=int(excl), ge15=int(ge15), ge25=int(ge25))
    return out, res


def write_png(path, rgb):
    h, w, _ = rgb.shape
    raw = b"".join(b"\x00" + rgb[y].tobytes() for y in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))
    open(path, "wb").write(png)


def seam_image(prep, dem, patch, png_path, dump_path):
    import numpy as np
    prep_out(prep, "dump", dem, *[f"{v:.7f}" for v in patch], dump_path)
    buf = open(dump_path, "rb").read()
    w, h = struct.unpack("<II", buf[:8])
    cell = struct.unpack("<f", buf[8:12])[0]
    z = np.frombuffer(buf[12:], dtype="<f4").reshape(h, w).astype(float)
    z = np.where(np.isnan(z), np.nanmean(z), z)
    gy, gx = np.gradient(z, cell)                      # gy = dz/dsouth (rows run south)
    slope = np.degrees(np.arctan(np.hypot(gx, gy)))
    az, el = math.radians(315), math.radians(45)       # display hillshade, sun from the north-west
    nx, ny, nz = -gx, gy, np.ones_like(z)              # normal with x east, y north
    n = np.sqrt(nx ** 2 + ny ** 2 + nz ** 2)
    sx, sy, sz = math.sin(az) * math.cos(el), math.cos(az) * math.cos(el), math.sin(el)
    shade = np.clip((nx * sx + ny * sy + nz * sz) / n, 0, 1)
    g = (60 + 195 * shade).astype(np.uint8)
    rgb = np.stack([g, g, g], axis=-1)
    rgb[slope >= 15.0] = [220, 40, 40]
    write_png(png_path, np.ascontiguousarray(rgb))
    return w, h


def dem_value(prep, dem, lat, lon):
    out = prep_out(prep, "locate", dem, f"{lat:.9f}", f"{lon:.9f}")
    for line in out.splitlines():
        if line.startswith("DEM value at that pixel"):
            return float(line.split("pixel")[1].split("(")[0].strip())
    return None


def main(path, dem=None, prep=config.PREP, png=os.path.join(config.GATE_DIR, "gate_patch.png")):
    rows = load(path)
    lats = [r["lat"] for r in rows]
    mean_lat = sum(lats) / len(lats)
    dlat = math.degrees(MARGIN_M / R)
    dlon = math.degrees(MARGIN_M / (R * math.cos(math.radians(mean_lat))))
    patch = (max(lats) + dlat, min(r["lon"] for r in rows) - dlon, min(lats) - dlat, max(r["lon"] for r in rows) + dlon)
    level = [r for r in rows if abs(r["pitch"]) < LEVEL_DEG and abs(r["roll"]) < LEVEL_DEG]
    checks = (rows[0], rows[len(rows) // 2], rows[-1])
    print(f"ROVER positions, sols {min(r['sol'] for r in rows)}..{max(r['sol'] for r in rows)}: N = {len(rows)}")
    print(f"patch (margin {MARGIN_M:.0f} m): lat {patch[2]:.6f} .. {patch[0]:.6f}, lon {patch[1]:.6f} .. {patch[3]:.6f}")
    print(f"noise reference: {len(level)} stops with |pitch|,|roll| < {LEVEL_DEG} deg, {2*REF_HALF_M:.0f} m footprint boxes")
    if dem is None:
        return
    print(f"\nSTEP 0, mapping check (DEM value vs PLACES elevation, tolerance {MAP_TOL_M} m):")
    ok_map = True
    for r in checks:
        val = dem_value(prep, dem, r["lat"], r["lon"])
        diff = None if val is None else val - r["elev"]
        good = diff is not None and abs(diff) <= MAP_TOL_M
        ok_map &= good
        print(f"  sol {r['sol']}: DEM {val}  PLACES {r['elev']:.2f}  diff {None if diff is None else round(diff, 3)}  {'ok' if good else 'MISS'}")
    if not ok_map:
        print("DECISION: INCONCLUSIVE (mapping check failed; nothing else run)")
        return
    out, pres = slopebox(prep, dem, patch, "patch")
    print("\n" + out)
    ref = {}
    for r in level:
        _, rr = slopebox(prep, dem, box(r["lat"], r["lon"], REF_HALF_M, mean_lat), f"ref_sol{r['sol']}")
        for s, v in rr.items():
            t = ref.setdefault(s, dict(n=0, ge15=0, ge25=0))
            for k in t:
                t[k] += v[k]
    print("sigma_m  patch>=15%  ref>=15%  patch>=25%  ref>=25%  ref_px   (smoothed rows and 25 deg: context only)")
    for s in sorted(pres):
        p, q = pres[s], ref[s]
        print(f"{s:7.1f}  {100*p['ge15']/p['n']:9.3f}  {100*q['ge15']/max(q['n'],1):8.3f}  {100*p['ge25']/p['n']:10.3f}  {100*q['ge25']/max(q['n'],1):8.3f}  {q['n']:6d}")
    p, q = pres[0.0], ref[0.0]
    ps = p["ge15"] / p["n"]
    qobs = q["ge15"] / max(q["n"], 1)
    qs = max(qobs, 3.0 / max(q["n"], 1))
    w, h = seam_image(prep, dem, patch, png, png + ".f32")
    print(f"\nseam-veto image: {png} ({w} x {h} px; red = slope >= 15 deg at native 1 m). Look at it before accepting a PASS.")
    if q["n"] < MIN_REF_PX:
        verdict = f"INCONCLUSIVE (reference has {q['n']} valid pixels < {MIN_REF_PX})"
    elif ps >= MIN_SHARE and ps >= MIN_RATIO * qs:
        verdict = "PASS, subject to the seam veto (visual check of the image)"
    else:
        verdict = "FAIL"
    print(f"DECISION (native 1 m): {verdict}\n  patch {100*ps:.3f}% ({p['ge15']}/{p['n']} px); reference observed {100*qobs:.3f}% "
          f"({q['ge15']}/{q['n']} px), used {100*qs:.3f}% (max of observed and 3/N); needed >= {100*max(MIN_SHARE, MIN_RATIO*qs):.3f}%")
    import datetime
    import json
    os.makedirs(config.GATE_DIR, exist_ok=True)
    json.dump(dict(run_utc=datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                   rule="doc: Decision rule (approved)", decision=verdict.split(",")[0],
                   patch_share_pct=round(100 * ps, 3), patch_steep_px=p["ge15"], patch_px=p["n"],
                   reference_share_pct=round(100 * qobs, 3), reference_steep_px=q["ge15"], reference_px=q["n"],
                   needed_pct=round(100 * max(MIN_SHARE, MIN_RATIO * qs), 3),
                   by_smoothing={str(s_): dict(patch_pct=round(100 * pres[s_]["ge15"] / pres[s_]["n"], 3),
                                               reference_pct=round(100 * ref[s_]["ge15"] / max(ref[s_]["n"], 1), 3)) for s_ in sorted(pres)}),
              open(config.GATE_RESULT, "w"), indent=1)


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    dem = a[a.index("--run") + 1] if "--run" in a else None
    prep = a[a.index("--prep") + 1] if "--prep" in a else config.PREP
    png = a[a.index("--png") + 1] if "--png" in a else os.path.join(config.GATE_DIR, "gate_patch.png")
    main(a[0], dem, prep, png)
