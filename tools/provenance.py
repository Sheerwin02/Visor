"""Data readiness, part B: which source is under the patch (rule fixed before any data was read).

Question: over the bundle patch, does the USGS Gale mosaic (heightmap.bin) carry the detail of the independent UA HiRISE DTM
DTEEC_009149_1750_009294_1750_U01, or does it differ (seams, backfill)?
Method (fixed before the run):
  1 Read only the DTM label and the lines covering the bundle grid plus 20 m (HTTP byte ranges). Sample the DTM bilinearly at
    every cell centre of the grid extended by 20 m, with x = R (lon - CENTER_LONGITUDE), y = R lat and the label offsets
    (1-based sample = SAMPLE_PROJECTION_OFFSET + x / MAP_SCALE, line = LINE_PROJECTION_OFFSET - y / MAP_SCALE).
  2 Remove the best-fit plane of (mosaic - UA) over common cells (re-registration offset/tilt is expected, not a finding); then
    pick the integer shift within +-5 m with the best correlation of the 3 m slope maps and apply it.
  3 Slope of each DTM at 3 m (3 x 3 m box, as slope_3m). Cell agrees if |difference| <= 2 deg, partial if <= 3, else disagrees.
  4 CONFIRMED HIRISE if >= 80% (choice) of common cells agree AND the median |difference| is below the 5th percentile of a
    shift baseline (UA shifted a further 20/30/40/50 m in 8 directions); MIXED if 50-80% agree; NOT THIS DTM if < 50%;
    UNTESTED if common coverage < 50%.
  5 Report N, shares, median/p90 |slope difference|, median/p90 |dz| after the plane, plane, shift, baseline table, and a PNG of
    the disagreeing cells for a seam check by eye.
  6 Layer (assets/readiness.bin, u8 per cell, row 0 north): 0 source unknown (no UA data), 1 agrees, 2 partial, 3 disagrees,
    4 rover-checked (within the verified radius of a localized rover position; overrides the others).

v2 (rule fixed after the v1 result, before v2 ran): lines covering the grid plus 120 m; registration by the best
correlation of the 3 m slope maps of the mosaic and the RAW UA DTM over integer shifts within +-100 m in 2 m steps, then 1 m
steps within +-2 m of that; then the plane of (mosaic - shifted UA) is removed and the UA slope recomputed. Best shift on the
edge of the +-100 m search => UNREGISTERED (no layer). Per cell, verdict and baseline as v1. Context only: the same comparison
after a 5 x 5 m box on both DTMs. v2 makes the layer; v1 stays recorded.

Usage:
  python tools/provenance.py selftest
  python tools/provenance.py fetch | run      # v1 (+-5 m): data/interim/provenance/provenance_result.json
  python tools/provenance.py fetch2 | run2    # v2 (+-100 m): provenance_v2_result.json, assets/readiness.bin + readiness.json
Then `python tools/build_bundle.py rehash` and `python tools/validate_bundle.py`.
"""
import json
import math
import os
import re
import sys
import urllib.request

import numpy as np

import config
from build_bundle import slope_deg
from route import verified_mask

R = config.MARS_RADIUS_M
DTM_ID = "DTEEC_009149_1750_009294_1750_U01"
DTM_URL = "https://www.uahirise.org/PDS/DTM/PSP/ORB_009100_009199/PSP_009149_1750_PSP_009294_1750/" + DTM_ID + ".IMG"
DTM_PAGE = "https://www.uahirise.org/dtm/dtm.php?ID=PSP_009149_1750"
CREDIT = "NASA/JPL/University of Arizona (HiRISE DTM " + DTM_ID + ")"
RAW = os.path.join(config.RAW, "hirise_dtm")
OUT = os.path.join(config.INTERIM, "provenance")
MARGIN_M = 20          # rule: grid plus 20 m
SHIFT_MAX = 5          # rule: integer shifts within +-5 m
AGREE, PARTIAL = 2.0, 3.0
CONFIRM_SHARE, MIXED_SHARE, COVER_MIN = 0.80, 0.50, 0.50
BASE_D = (20, 30, 40, 50)
BASE_DIRS = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)]
MARGIN2_M = 120       # v2: grid plus 120 m
SEARCH2_M, STEP2_M, FINE2_M = 100, 2, 2
LABELS = {0: "source unknown", 1: "agrees", 2: "partial", 3: "disagrees", 4: "rover-checked"}


# ----------------------------------------------------------------------------------------------- label and grid
def parse_label(text):
    def num(k):
        m = re.search(rf"^\s*{k}\s*=\s*(-?[0-9.]+)", text, re.M)
        if not m:
            raise ValueError(f"label: {k} missing")
        return float(m.group(1))
    return dict(record_bytes=int(num("RECORD_BYTES")), image_record=int(num(r"\^IMAGE")), lines=int(num("LINES")),
                samples=int(num("LINE_SAMPLES")), scale=num("MAP_SCALE"), clon=num("CENTER_LONGITUDE"),
                clat=num("CENTER_LATITUDE"), loff=num("LINE_PROJECTION_OFFSET"), soff=num("SAMPLE_PROJECTION_OFFSET"),
                radius_m=num("A_AXIS_RADIUS") * 1000.0)


def dtm_index(lab, lat, lon):
    """0-based (line, sample) of the DTM pixel-centre grid at planetocentric lat/lon (degrees)."""
    if abs(lab["clat"]) > 1e-9:
        raise ValueError("only CENTER_LATITUDE 0 handled")
    x = lab["radius_m"] * np.radians(lon - lab["clon"])
    y = lab["radius_m"] * np.radians(lat)
    return lab["loff"] - y / lab["scale"] - 1.0, lab["soff"] + x / lab["scale"] - 1.0


def grid_latlon(grid, margin):
    """Cell-centre lat/lon of the bundle grid extended by `margin` cells on every side (row 0 north)."""
    H, W, cell = grid["height"], grid["width"], grid["cell_m"]
    r = np.arange(-margin, H + margin)
    c = np.arange(-margin, W + margin)
    y = grid["y_north_m"] - (r + 0.5) * cell
    x = grid["x_west_m"] + (c + 0.5) * cell
    lat, lon = np.degrees(y / R), np.degrees(x / R)
    return np.meshgrid(lat, lon, indexing="ij")


def bilinear(img, li, si, line0=0):
    """Bilinear sample of img (rows = DTM lines from line0) at fractional indices; NaN outside or next to no-data."""
    li = li - line0
    l0, s0 = np.floor(li).astype(int), np.floor(si).astype(int)
    fl, fs = li - l0, si - s0
    ok = (l0 >= 0) & (s0 >= 0) & (l0 + 1 < img.shape[0]) & (s0 + 1 < img.shape[1])
    l0c, s0c = np.clip(l0, 0, img.shape[0] - 2), np.clip(s0, 0, img.shape[1] - 2)
    a, b = img[l0c, s0c], img[l0c, s0c + 1]
    c, d = img[l0c + 1, s0c], img[l0c + 1, s0c + 1]
    v = (a * (1 - fs) + b * fs) * (1 - fl) + (c * (1 - fs) + d * fs) * fl
    return np.where(ok, v, np.nan)


# ----------------------------------------------------------------------------------------------- the comparison (pure)
def compare(mosaic, ua_ext, margin, cell=1.0):
    """mosaic (H, W) heights; ua_ext (H + 2m, W + 2m) UA heights on the extended grid (NaN = no data). Returns a dict with the
    per-cell slope difference (H, W; NaN where not comparable) and every reported number."""
    H, W = mosaic.shape
    m = margin
    ua_main = ua_ext[m:m + H, m:m + W]
    common = np.isfinite(ua_main)
    if common.sum() < 3:
        return dict(n_common=int(common.sum()), coverage=float(common.mean()), verdict="UNTESTED", diff=np.full((H, W), np.nan))
    # 2a plane of (mosaic - UA) over common cells, applied to the whole extended UA grid
    rr, cc = np.mgrid[0:H, 0:W]
    A = np.c_[np.ones(common.sum()), cc[common], rr[common]]
    coef, *_ = np.linalg.lstsq(A, (mosaic - ua_main)[common], rcond=None)
    re_, ce_ = np.mgrid[-m:H + m, -m:W + m]
    ua_adj = ua_ext + coef[0] + coef[1] * ce_ + coef[2] * re_
    s_m = slope_deg(mosaic, cell)
    # slope_deg's box filter uses running sums, so a NaN would poison its whole row: fill no-data with 0, then mask every cell
    # whose 3 x 3 box, or the boxes its gradient uses, touch no-data
    from scipy.ndimage import maximum_filter, uniform_filter
    valid = np.isfinite(ua_adj)
    s_u = slope_deg(np.where(valid, ua_adj, 0.0), cell).astype(float)
    touched = maximum_filter(uniform_filter((~valid).astype(float), size=3, mode="nearest") > 1e-9, size=3, mode="nearest")
    s_u[touched] = np.nan
    inner = np.zeros((H, W), bool)
    inner[2:-2, 2:-2] = True                                              # mosaic border cells use one-sided gradients

    def at(dx, dy, img=s_u):
        """img on the main grid, sampled dx cells east and dy cells north of each cell (NaN off the extended grid)."""
        out = np.full((H, W), np.nan)
        r0, c0 = m - dy, m + dx
        rs, cs = slice(max(0, -r0), min(H, img.shape[0] - r0)), slice(max(0, -c0), min(W, img.shape[1] - c0))
        out[rs, cs] = img[rs.start + r0:rs.stop + r0, cs.start + c0:cs.stop + c0]
        return out

    # 2b integer shift with the best correlation of the 3 m slope maps
    best = None
    for dy in range(-SHIFT_MAX, SHIFT_MAX + 1):
        for dx in range(-SHIFT_MAX, SHIFT_MAX + 1):
            u = at(dx, dy)
            ok = inner & np.isfinite(u)
            if ok.sum() < 3:
                continue
            r = float(np.corrcoef(s_m[ok], u[ok])[0, 1])
            if best is None or r > best[0]:
                best = (r, dx, dy)
    if best is None:
        return dict(n_common=0, coverage=0.0, verdict="UNTESTED", diff=np.full((H, W), np.nan))
    r_best, dx, dy = best
    u = at(dx, dy)
    ok = inner & np.isfinite(u)
    diff = np.where(ok, s_m - u, np.nan)
    ad = np.abs(diff[ok])
    dz = np.abs(mosaic - at(dx, dy, ua_adj))[ok]
    # baseline: a further 20..50 m in 8 directions
    base = []
    for dist in BASE_D:
        for ux, uy in BASE_DIRS:
            bx, by = dx + int(round(dist * ux / math.hypot(ux, uy))), dy + int(round(dist * uy / math.hypot(ux, uy)))
            ub = at(bx, by)
            okb = inner & np.isfinite(ub)
            if okb.sum() >= 100:
                base.append(dict(dist_m=dist, dir=[ux, uy], n=int(okb.sum()), median_abs_deg=round(float(np.median(np.abs(s_m - ub)[okb])), 3)))
    p5 = float(np.percentile([b["median_abs_deg"] for b in base], 5)) if base else float("nan")
    n = int(ok.sum())
    coverage = n / float(inner.sum())
    sh = dict(agree=float(np.mean(ad <= AGREE)), partial=float(np.mean((ad > AGREE) & (ad <= PARTIAL))), disagree=float(np.mean(ad > PARTIAL)))
    med = float(np.median(ad))
    if coverage < COVER_MIN:
        verdict = "UNTESTED"
    elif sh["agree"] >= CONFIRM_SHARE and med < p5:
        verdict = "CONFIRMED HIRISE"
    elif sh["agree"] >= MIXED_SHARE:
        verdict = "MIXED"
    else:
        verdict = "NOT THIS DTM"
    return dict(verdict=verdict, n_common=n, coverage=round(coverage, 4),
                share=dict((k, round(v, 4)) for k, v in sh.items()),
                median_abs_slope_diff_deg=round(med, 3), p90_abs_slope_diff_deg=round(float(np.percentile(ad, 90)), 3),
                signed_median_slope_diff_deg=round(float(np.median(diff[ok])), 3),
                median_abs_dz_m=round(float(np.median(dz)), 3), p90_abs_dz_m=round(float(np.percentile(dz, 90)), 3),
                plane=dict(offset_m=round(float(coef[0]), 3), per_m_east=float(coef[1]) / cell, per_m_south=float(coef[2]) / cell),
                shift_m=dict(east=dx * cell, north=dy * cell, meaning="UA sampled this far east/north of each mosaic cell"), shift_corr=round(r_best, 4),
                baseline=base, baseline_p5_deg=round(p5, 3), diff=diff)


# ----------------------------------------------------------------------------------------------- v2: wide registration
def masked_corr_all(a, ma, b, mb):
    """Pearson correlation of a (H, W) with every same-size window of b (Hb, Wb), over cells valid in both masks, for every
    offset at once (FFT, exact sums). Returns corr[i, j] for a[r, c] vs b[r + i, c + j], and the overlap counts."""
    from scipy.signal import fftconvolve
    def cc(x, y):                                      # sum over r,c of x[r,c] * y[r+i, c+j]
        return fftconvolve(y, x[::-1, ::-1], mode="valid")
    A, Bv = np.where(ma, a, 0.0), np.where(mb, b, 0.0)
    MA, MB = ma.astype(float), mb.astype(float)
    n = np.rint(cc(MA, MB))
    sa, sb = cc(A, MB), cc(MA, Bv)
    saa, sbb, sab = cc(A * A, MB), cc(MA, Bv * Bv), cc(A, Bv)
    with np.errstate(invalid="ignore", divide="ignore"):
        cov = sab - sa * sb / n
        va, vb = saa - sa * sa / n, sbb - sb * sb / n
        r = cov / np.sqrt(va * vb)
    r[(n < 100) | ~np.isfinite(r)] = np.nan
    return r, n


def nan_slope(z, cell=1.0):
    """slope_deg (3 x 3 m box) with no-data kept local (see compare)."""
    from scipy.ndimage import maximum_filter, uniform_filter
    valid = np.isfinite(z)
    s = slope_deg(np.where(valid, z, 0.0), cell).astype(float)
    touched = maximum_filter(uniform_filter((~valid).astype(float), size=3, mode="nearest") > 1e-9, size=3, mode="nearest")
    s[touched] = np.nan
    return s


def compare_v2(mosaic, ua_ext, margin, cell=1.0, search=SEARCH2_M, step=STEP2_M, fine=FINE2_M, context=True):
    H, W = mosaic.shape
    m = margin
    inner = np.zeros((H, W), bool)
    inner[2:-2, 2:-2] = True
    s_m = slope_deg(mosaic, cell).astype(float)
    s_u_raw = nan_slope(ua_ext, cell)
    corr, _ = masked_corr_all(s_m, inner, np.nan_to_num(s_u_raw), np.isfinite(s_u_raw))
    # corr[i, j] compares mosaic (r, c) with ua_ext (r + i, c + j); i = m - dy, j = m + dx
    def at_c(dx, dy):
        i, j = m - dy, m + dx
        return corr[i, j] if 0 <= i < corr.shape[0] and 0 <= j < corr.shape[1] else np.nan
    k = search // step
    coarse = [(at_c(dx * step, dy * step), dx * step, dy * step) for dy in range(-k, k + 1) for dx in range(-k, k + 1)]
    coarse = [c for c in coarse if np.isfinite(c[0])]
    if not coarse:
        return dict(verdict="UNTESTED", n_common=0, coverage=0.0, diff=np.full((H, W), np.nan))
    r0, cx, cy = max(coarse)
    fine_c = [(at_c(dx, dy), dx, dy) for dy in range(cy - fine, cy + fine + 1) for dx in range(cx - fine, cx + fine + 1)
              if abs(dx) <= search and abs(dy) <= search]
    r_best, dx, dy = max(c for c in fine_c if np.isfinite(c[0]))
    second = max((c for c in coarse if math.hypot(c[1] - dx, c[2] - dy) > 10), default=(float("nan"), 0, 0))
    edge = max(abs(cx), abs(cy)) >= search

    def at(img, ddx, ddy):
        out = np.full((H, W), np.nan)
        r0_, c0_ = m - ddy, m + ddx
        rs, cs = slice(max(0, -r0_), min(H, img.shape[0] - r0_)), slice(max(0, -c0_), min(W, img.shape[1] - c0_))
        if rs.start < rs.stop and cs.start < cs.stop:
            out[rs, cs] = img[rs.start + r0_:rs.stop + r0_, cs.start + c0_:cs.stop + c0_]
        return out

    # plane of (mosaic - shifted UA), applied to the whole extended UA grid, then the UA slope again
    u_h = at(ua_ext, dx, dy)
    common = np.isfinite(u_h)
    rr, cc_ = np.mgrid[0:H, 0:W]
    A = np.c_[np.ones(common.sum()), cc_[common], rr[common]]
    coef, *_ = np.linalg.lstsq(A, (mosaic - u_h)[common], rcond=None)
    re_, ce_ = np.mgrid[-m:H + m, -m:W + m]
    # extended cell (R, C) is compared with mosaic (R - m + dy, C - m - dx): express the plane in mosaic coordinates
    ua_adj = ua_ext + coef[0] + coef[1] * (ce_ - dx) + coef[2] * (re_ + dy)
    s_u = nan_slope(ua_adj, cell)
    u = at(s_u, dx, dy)
    ok = inner & np.isfinite(u)
    diff = np.where(ok, s_m - u, np.nan)
    ad = np.abs(diff[ok])
    dz = np.abs(mosaic - at(ua_adj, dx, dy))[ok]
    base = []
    for dist in BASE_D:
        for ux, uy in BASE_DIRS:
            bx, by = dx + int(round(dist * ux / math.hypot(ux, uy))), dy + int(round(dist * uy / math.hypot(ux, uy)))
            ub = at(s_u, bx, by)
            okb = inner & np.isfinite(ub)
            if okb.sum() >= 100:
                base.append(dict(dist_m=dist, dir=[ux, uy], n=int(okb.sum()), median_abs_deg=round(float(np.median(np.abs(s_m - ub)[okb])), 3)))
    p5 = float(np.percentile([b["median_abs_deg"] for b in base], 5)) if base else float("nan")
    n = int(ok.sum())
    coverage = n / float(inner.sum())
    sh = dict(agree=float(np.mean(ad <= AGREE)), partial=float(np.mean((ad > AGREE) & (ad <= PARTIAL))), disagree=float(np.mean(ad > PARTIAL)))
    med = float(np.median(ad))
    if edge:
        verdict = "UNREGISTERED"
    elif coverage < COVER_MIN:
        verdict = "UNTESTED"
    elif sh["agree"] >= CONFIRM_SHARE and med < p5:
        verdict = "CONFIRMED HIRISE"
    elif sh["agree"] >= MIXED_SHARE:
        verdict = "MIXED"
    else:
        verdict = "NOT THIS DTM"
    res = dict(verdict=verdict, n_common=n, coverage=round(coverage, 4), share=dict((k_, round(v, 4)) for k_, v in sh.items()),
               median_abs_slope_diff_deg=round(med, 3), p90_abs_slope_diff_deg=round(float(np.percentile(ad, 90)), 3),
               signed_median_slope_diff_deg=round(float(np.median(diff[ok])), 3),
               median_abs_dz_m=round(float(np.median(dz)), 3), p90_abs_dz_m=round(float(np.percentile(dz, 90)), 3),
               plane=dict(offset_m=round(float(coef[0]), 3), per_m_east=float(coef[1]) / cell, per_m_south=float(coef[2]) / cell),
               shift_m=dict(east=dx * cell, north=dy * cell, meaning="UA sampled this far east/north of each mosaic cell"),
               shift_corr=round(float(r_best), 4), coarse_best=dict(east=cx, north=cy, corr=round(float(r0), 4)),
               second_peak_beyond_10m=dict(corr=round(float(second[0]), 4), east=second[1], north=second[2]),
               on_search_edge=bool(edge), baseline=base, baseline_p5_deg=round(p5, 3), diff=diff)
    if context:
        from scipy.ndimage import uniform_filter
        ms = uniform_filter(mosaic, size=5, mode="nearest")
        us = np.where(np.isfinite(ua_adj), ua_adj, np.nan)
        valid = np.isfinite(us)
        us_f = uniform_filter(np.where(valid, us, 0.0), size=5, mode="nearest")
        bad = uniform_filter((~valid).astype(float), size=5, mode="nearest") > 1e-9
        us_f[bad] = np.nan
        s_ms, s_us = slope_deg(ms, cell).astype(float), nan_slope(us_f, cell)
        u5 = at(s_us, dx, dy)
        ok5 = inner & np.isfinite(u5)
        a5 = np.abs(s_ms - u5)[ok5]
        res["context_5m_box"] = dict(n=int(ok5.sum()), agree=round(float(np.mean(a5 <= AGREE)), 4),
                                     partial=round(float(np.mean((a5 > AGREE) & (a5 <= PARTIAL))), 4),
                                     median_abs_slope_diff_deg=round(float(np.median(a5)), 3),
                                     p90_abs_slope_diff_deg=round(float(np.percentile(a5, 90)), 3))
    return res


def classes(diff, verified):
    """Layer classes per cell (see the module docstring)."""
    out = np.zeros(diff.shape, np.uint8)
    a = np.abs(diff)
    fin = np.isfinite(diff)
    out[fin & (a <= AGREE)] = 1
    out[fin & (a > AGREE) & (a <= PARTIAL)] = 2
    out[fin & (a > PARTIAL)] = 3
    out[verified] = 4
    return out


# ----------------------------------------------------------------------------------------------- fetch / run
def http_range(url, a, b):
    req = urllib.request.Request(url, headers={"Range": f"bytes={a}-{b}"})
    with urllib.request.urlopen(req, timeout=300) as r:
        if r.status != 206:
            raise RuntimeError(f"server ignored the byte range (HTTP {r.status})")
        return r.read()


def needed_lines(lab, grid, margin=MARGIN_M):
    lat, lon = grid_latlon(grid, margin)
    li, si = dtm_index(lab, lat, lon)
    return max(0, int(np.floor(li.min())) - 2), min(lab["lines"] - 1, int(np.ceil(li.max())) + 2)


def fetch(margin=MARGIN_M, tag=""):
    os.makedirs(RAW, exist_ok=True)
    head = http_range(DTM_URL, 0, 65535)
    text = head.decode("latin-1")
    text = text[:text.find("END\r\n") + 5] if "END\r\n" in text else text
    lab = parse_label(text)
    open(os.path.join(RAW, DTM_ID + ".LBL.txt"), "w", encoding="latin-1").write(text)
    grid = json.load(open(os.path.join(config.ASSETS, "manifest.json")))["grid"]
    l0, l1 = needed_lines(lab, grid, margin)
    rb = lab["record_bytes"]
    start = (lab["image_record"] - 1) * rb + l0 * rb
    data = http_range(DTM_URL, start, start + (l1 - l0 + 1) * rb - 1)
    if len(data) != (l1 - l0 + 1) * rb:
        raise RuntimeError(f"short read: {len(data)} bytes")
    open(os.path.join(RAW, f"{DTM_ID}_lines_{l0}_{l1}.f32"), "wb").write(data)
    json.dump(dict(dtm=DTM_ID, url=DTM_URL, line0=l0, line1=l1, bytes=len(data)), open(os.path.join(RAW, f"fetch{tag}.json"), "w"), indent=1)
    print(f"label + DTM lines {l0}..{l1} ({len(data) / 1e6:.1f} MB) -> {RAW}")


def load_ua(grid, margin=MARGIN_M, tag=""):
    lab = parse_label(open(os.path.join(RAW, DTM_ID + ".LBL.txt"), encoding="latin-1").read())
    f = json.load(open(os.path.join(RAW, f"fetch{tag}.json")))
    l0, l1 = f["line0"], f["line1"]
    img = np.fromfile(os.path.join(RAW, f"{DTM_ID}_lines_{l0}_{l1}.f32"), "<f4").reshape(l1 - l0 + 1, lab["samples"]).astype(float)
    img[(img < -1e30) | ~np.isfinite(img)] = np.nan                           # MISSING_CONSTANT 16#FF7FFFFB#
    lat, lon = grid_latlon(grid, margin)
    li, si = dtm_index(lab, lat, lon)
    return bilinear(img, li, si, line0=l0), lab


def run(v2=False):
    man = json.load(open(os.path.join(config.ASSETS, "manifest.json")))
    g = man["grid"]
    H, W = g["height"], g["width"]
    mosaic = np.fromfile(os.path.join(config.ASSETS, "heightmap.bin"), "<f4").reshape(H, W).astype(float) + g["z_offset_m"]
    if v2:
        ua_ext, lab = load_ua(g, MARGIN2_M, "_v2")
        res = compare_v2(mosaic, ua_ext, MARGIN2_M, g["cell_m"])
    else:
        ua_ext, lab = load_ua(g)
        res = compare(mosaic, ua_ext, MARGIN_M, g["cell_m"])
    diff = res.pop("diff")
    path = json.load(open(os.path.join(config.ASSETS, "rover_path.json")))["points"]
    ver = verified_mask(H, W, [(p["e_m"], p["n_m"]) for p in path], g["cell_m"])
    cls = classes(diff, ver)
    os.makedirs(OUT, exist_ok=True)
    import datetime
    res.update(run_utc=datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
               rule=("doc: Data readiness part B v2, wide registration (approved by the author)" if v2 else
                     "doc: Data readiness rule (approved by the author)"), dtm=DTM_ID, dtm_url=DTM_URL, dtm_page=DTM_PAGE,
               credit=CREDIT, label=lab, class_counts={LABELS[k]: int((cls == k).sum()) for k in LABELS})
    tag = "_v2" if v2 else ""
    if v2 and res["verdict"] == "UNREGISTERED":
        cls[:] = 0                                                              # rule: no cell layer
        cls[ver] = 4
        res["class_counts"] = {LABELS[k]: int((cls == k).sum()) for k in LABELS}
    json.dump(res, open(os.path.join(OUT, f"provenance{tag}_result.json"), "w"), indent=1)
    # seam check picture: disagree white, partial grey, agree dark, no data dark red
    try:
        from PIL import Image
        rgb = np.zeros((H, W, 3), np.uint8)
        rgb[cls == 0] = (90, 20, 20)
        rgb[cls == 1] = (30, 30, 30)
        rgb[cls == 2] = (140, 140, 140)
        rgb[cls == 3] = (255, 255, 255)
        rgb[cls == 4] = (40, 120, 200)
        Image.fromarray(rgb).save(os.path.join(OUT, f"provenance{tag}_cells.png"))
        d = np.nan_to_num(np.clip(diff, -10, 10), nan=0.0)
        Image.fromarray(((d + 10) / 20 * 255).astype(np.uint8)).save(os.path.join(OUT, f"provenance{tag}_slope_diff.png"))
    except ImportError:
        print("Pillow missing: no PNG")
    if not v2:                                                                 # the layer comes from v2 only
        print(json.dumps({k: v for k, v in res.items() if k not in ("baseline", "label")}, indent=1))
        return
    cls.tofile(os.path.join(config.ASSETS, "readiness.bin"))
    json.dump(dict(rule=res["rule"], run_utc=res["run_utc"], verdict=res["verdict"], width=W, height=H, cell_m=g["cell_m"],
                   encoding="uint8 row-major, row 0 north", classes={str(k): v for k, v in LABELS.items()},
                   thresholds_deg=dict(agree=AGREE, partial=PARTIAL), comparison=dict(dtm=DTM_ID, url=DTM_PAGE, credit=CREDIT),
                   result={k: res[k] for k in ("n_common", "coverage", "share", "median_abs_slope_diff_deg", "p90_abs_slope_diff_deg",
                                               "median_abs_dz_m", "p90_abs_dz_m", "shift_m", "baseline_p5_deg")},
                   replaces="v1 (+-5 m search): NOT THIS DTM, recorded in the doc",
                   class_counts=res["class_counts"]),
              open(os.path.join(config.ASSETS, "readiness.json"), "w"), indent=1)
    print(json.dumps({k: v for k, v in res.items() if k not in ("baseline", "label")}, indent=1))
    print("baseline medians:", [b["median_abs_deg"] for b in res["baseline"]])


# ----------------------------------------------------------------------------------------------- selftest (synthetic)
def selftest():
    rng = np.random.default_rng(3)
    H, W, m = 120, 90, MARGIN_M
    from scipy.ndimage import gaussian_filter
    big = gaussian_filter(rng.normal(0, 1, (H + 2 * m + 20, W + 2 * m + 20)), 2.0) * 12.0      # rough synthetic terrain
    mosaic = big[m + 10:m + 10 + H, m + 10:m + 10 + W].copy()
    # UA grid holds each feature 3 cells west and 2 cells north of the mosaic's (so it must be sampled at -3 east, -2 north),
    # with an offset, a tilt and small noise
    sx, sy = 3, 2
    ua = big[10 - sy:10 - sy + H + 2 * m, 10 + sx:10 + sx + W + 2 * m].copy()
    rr, cc = np.mgrid[-m:H + m, -m:W + m]
    ua = ua - 4.0 + 0.002 * cc - 0.001 * rr + rng.normal(0, 0.01, ua.shape)
    r = compare(mosaic, ua, m)
    assert r["verdict"] == "CONFIRMED HIRISE", r["verdict"]
    assert (r["shift_m"]["east"], r["shift_m"]["north"]) == (-sx, -sy), r["shift_m"]
    assert r["share"]["agree"] > 0.95 and r["median_abs_dz_m"] < 0.1, r
    # unrelated terrain: not this DTM
    other = gaussian_filter(rng.normal(0, 1, ua.shape), 2.0) * 12.0
    assert compare(mosaic, other, m)["verdict"] == "NOT THIS DTM"
    # mostly missing UA data: untested
    holed = ua.copy()
    holed[:, : (W + 2 * m) * 3 // 4] = np.nan
    assert compare(mosaic, holed, m)["verdict"] == "UNTESTED"
    # v2: a 37 m east / 21 m south offset (beyond v1's +-5 m) is found exactly; an offset beyond the search is UNREGISTERED
    M2 = 60
    big2 = gaussian_filter(rng.normal(0, 1, (H + 2 * M2 + 80, W + 2 * M2 + 80)), 2.0) * 12.0
    mos2 = big2[M2 + 40:M2 + 40 + H, M2 + 40:M2 + 40 + W].copy()
    ex, ny = 37, -21                                    # mosaic (r, c) == UA ext (r + M2 - ny, c + M2 + ex)
    ua2 = big2[40 + ny:40 + ny + H + 2 * M2, 40 - ex:40 - ex + W + 2 * M2].copy()
    r2, c2 = np.mgrid[-M2:H + M2, -M2:W + M2]
    ua2 = ua2 + 3.0 + 0.01 * c2 + rng.normal(0, 0.01, ua2.shape)
    v = compare_v2(mosaic=mos2, ua_ext=ua2, margin=M2, search=50, step=2, fine=2, context=False)
    assert (v["shift_m"]["east"], v["shift_m"]["north"]) == (ex, ny), v["shift_m"]
    assert v["verdict"] == "CONFIRMED HIRISE" and v["share"]["agree"] > 0.95, (v["verdict"], v["share"])
    ex3 = 23                                            # just past a 20 m search: the best shift lands on its edge
    ua3 = big2[40:40 + H + 2 * M2, 40 - ex3:40 - ex3 + W + 2 * M2].copy()
    assert compare_v2(mosaic=mos2, ua_ext=ua3, margin=M2, search=20, step=2, fine=2, context=False)["verdict"] == "UNREGISTERED"
    # one no-data pixel masks only its 5 x 5 neighbourhood (box + gradient), not its row
    one = ua.copy()
    one[m + 60, m + 45] = np.nan
    d1 = compare(mosaic, one, m)["diff"]
    border = H * W - (H - 4) * (W - 4)
    assert int(np.isnan(d1).sum()) == border + 25, int(np.isnan(d1).sum())
    # classes: rover-checked overrides, NaN = unknown
    d = np.array([[0.5, 2.5, 9.0, np.nan]])
    assert classes(d, np.array([[False, False, False, False]])).tolist() == [[1, 2, 3, 0]]
    assert classes(d, np.array([[False, True, False, False]])).tolist() == [[1, 4, 3, 0]]
    # label convention: the west edge of the first DTM sample falls on index -0.5
    lab = dict(clat=0.0, clon=137.41, radius_m=3396190.0, scale=1.0117467945795, loff=-273961.5, soff=3319.5)
    _, s = dtm_index(lab, np.array(-4.8), np.array(137.35334875))
    assert abs(s - (-0.5)) < 0.01, s
    print("provenance selftest: PASS")


if __name__ == "__main__":
    a = sys.argv[1:]
    {"selftest": selftest, "fetch": fetch, "run": run, "fetch2": lambda: fetch(MARGIN2_M, "_v2"), "run2": lambda: run(True)}.get(a[0] if a else "", lambda: print(__doc__))()
