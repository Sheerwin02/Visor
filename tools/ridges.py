"""Ridge tracing (rule fixed before any trace was made, with amendments 1-4).

Question: can the 25 cm orthophoto show where the boxwork ridges and hollows are, where the 1 m DEM cannot?
Method (automatic, no hand edits; sand-filled hollows are the coherent objects, ridge centre lines can be dark):
  1 data/processed/ortho/ortho_patch.tif snapped to the bundle grid as build_bundle does; luminance = mean of the 3 bands.
  2 Gaussian blur sigma 1 m (4 px); z = (blurred - mean over 30 m) / std over 30 m (box windows).
  3 hollow pixels: z < 0; opening with a 2 m disc; hollow components under 25 m2 removed.
  4 ridge = not hollow; a 1 m cell is hollow if >= 50% of its 16 pixels are hollow.
Check A (primary): 7 stops with NASA parking statements (LABELS below). Class at a stop = majority of 1 m cells whose centre is
  within 2 m (a tie is not a majority: scored as a miss). PASS if >= 6 of 7 correct and Nevado Sajama correct; PARTIAL 5 of 7;
  else FAIL. Context: the score without Valle de la Luna (inferred position).
  Stop position (amendment 4): earliest ROVER-frame PLACES fix with sol inside the plan's sol range, else the latest before it;
  drill pair from stops.json. Per stop: distance to the nearest ridge-hollow edge, "near edge" if < 2 m (amendment 3), and a
  ridge-labelled miss within 3.3 m of ridge pixels lying sunward (within 90 deg of the photo's sun azimuth) is flagged
  "consistent with shadow" (amendment 1). Report only.
Check B: relief = mosaic DEM minus its 31 m median; difference = mean relief on ridge cells minus hollow cells. PASS if > 0 and
  above the 95th percentile of 32 baselines (the class grid shifted 20/30/40/50 m in 8 directions, overlap only).
Context only: hollow share, count, median equivalent diameter; z < -0.25 and z < +0.25 (Check A score, Check B difference);
  Check B on the UA HiRISE DTM registered by the provenance v2 shift and plane.
Outputs: data/interim/ridges/ (result JSON, PNGs, ridges.bin: u8 per cell, row 0 north, 1 ridge, 2 hollow); assets/ridges.json
(summary for the readiness table only). The map layer was dropped after the FAIL, so the cell grid stays local.

Usage: python tools/ridges.py selftest | python tools/ridges.py run
"""
import csv
import json
import math
import os
import sys

import numpy as np
from scipy import ndimage as ndi

import config

R = config.MARS_RADIUS_M
OUT = os.path.join(config.INTERIM, "ridges")
PX = 0.25                      # photo pixel (m)
BLUR_M, WIN_M, OPEN_D_M, MIN_AREA_M2 = 1.0, 30.0, 2.0, 25.0
Z_CUT = 0.0
STOP_R_M = 2.0
SHADOW_M = 3.3
RELIEF_WIN_M = 31
BASE_D = (20, 30, 40, 50)
BASE_DIRS = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)]

# Check A labels (NASA parking statements, quoted; doc rule). plan = (first sol, last sol) of the blog's plan.
LABELS = [
    dict(id="sol4609", label="hollow", plan=(4609, 4610), quote="We are in one of the 'hollows' between the resistant ridges",
         url="https://science.nasa.gov/blog/curiosity-blog-sols-4609-4610-recharged-and-ready-to-roll-onwards"),
    dict(id="sol4627", label="ridge", plan=(4627, 4628), quote="brought the rover from the 'peace sign' ridge intersection to a new ridge site",
         url="https://science.nasa.gov/blog/curiosity-blog-sols-4627-4628-a-ridge-stop-in-the-boxworks/"),
    dict(id="sol4638", label="ridge", plan=(4638, 4640), quote="we are now parked on top of a ridge",
         url="https://science.nasa.gov/blog/curiosity-blog-sols-4638-4640-imaging-extravaganza-atop-a-ridge"),
    dict(id="sol4643", label="hollow", plan=(4643, 4644), quote="Curiosity was successfully parked in the Mojo hollow",
         url="https://science.nasa.gov/blog/curiosity-blog-sols-4641-4648-thinking-outside-and-inside-the-boxwork/"),
    dict(id="sol4645", label="ridge", plan=(4645, 4648), quote="we reached the desired parking location on the ridge south of Mojo",
         url="https://science.nasa.gov/blog/curiosity-blog-sols-4641-4648-thinking-outside-and-inside-the-boxwork/"),
    dict(id="valle_de_la_luna", label="hollow", stop="valle_de_la_luna", quote="a hollow called Valle de la Luna (VL)",
         url="https://www.hou.usra.edu/meetings/lpsc2026/pdf/1716.pdf", inferred=True),
    dict(id="nevado_sajama", label="ridge", stop="nevado_sajama", quote="a ridgetop called Nevado Sajama (NS)",
         url="https://www.hou.usra.edu/meetings/lpsc2026/pdf/1716.pdf", required=True),
]


# ----------------------------------------------------------------------------------------------- the trace (pure)
def disc(r_px):
    y, x = np.mgrid[-r_px:r_px + 1, -r_px:r_px + 1]
    return x * x + y * y <= r_px * r_px


def trace(lum, z_cut=Z_CUT, px=PX):
    """lum (4H, 4W) photo luminance -> hollow mask (bool, same shape) and z."""
    b = ndi.gaussian_filter(lum.astype(float), BLUR_M / px, mode="nearest")
    w = int(round(WIN_M / px)) | 1
    m = ndi.uniform_filter(b, w, mode="nearest")
    s = np.sqrt(np.maximum(ndi.uniform_filter(b * b, w, mode="nearest") - m * m, 1e-12))
    z = (b - m) / s
    h = z < z_cut
    h = ndi.binary_opening(h, structure=disc(int(round(OPEN_D_M / 2 / px))))
    lab, n = ndi.label(h)
    if n:
        area = ndi.sum(np.ones_like(lab), lab, index=np.arange(1, n + 1)) * px * px
        keep = np.r_[False, area >= MIN_AREA_M2]
        h = keep[lab]
    return h, z


def to_cells(hollow_px, k=4):
    H, W = hollow_px.shape[0] // k, hollow_px.shape[1] // k
    return hollow_px[:H * k, :W * k].reshape(H, k, W, k).mean(axis=(1, 3)) >= 0.5


def classify_stop(cells_hollow, e, n, H, r_m=STOP_R_M, cell=1.0):
    """Majority class of cells whose centre is within r_m of (e, n). Returns (class or 'tie', hollow cells, total cells)."""
    rr, cc = np.mgrid[0:cells_hollow.shape[0], 0:cells_hollow.shape[1]]
    ce, cn = (cc + 0.5) * cell, (H - rr - 0.5) * cell
    sel = (ce - e) ** 2 + (cn - n) ** 2 <= r_m * r_m
    nh, nt = int(cells_hollow[sel].sum()), int(sel.sum())
    if nt == 0:
        return "none", 0, 0
    if 2 * nh > nt:
        return "hollow", nh, nt
    if 2 * nh < nt:
        return "ridge", nh, nt
    return "tie", nh, nt


def edge_info(hollow_px, e, n, H_px, sun_az_deg, px=PX):
    """Distance (m) from the stop to the nearest pixel of the other class, and whether the nearest RIDGE pixel lies sunward."""
    r, c = int((H_px * px - n) / px), int(e / px)
    r, c = min(max(r, 0), hollow_px.shape[0] - 1), min(max(c, 0), hollow_px.shape[1] - 1)
    here = hollow_px[r, c]
    d, idx = ndi.distance_transform_edt(hollow_px == here, return_indices=True)
    edge_m = float(d[r, c] * px)
    # nearest ridge pixel (for the shadow flag)
    dr, ridx = ndi.distance_transform_edt(hollow_px, return_indices=True)       # distance to nearest ridge (False) pixel
    rr_, rc_ = ridx[0][r, c], ridx[1][r, c]
    ridge_m = float(dr[r, c] * px)
    bearing = (math.degrees(math.atan2(rc_ - c, -(rr_ - r))) + 360) % 360 if ridge_m > 0 else float("nan")
    sunward = ridge_m > 0 and abs((bearing - sun_az_deg + 180) % 360 - 180) <= 90
    return dict(edge_m=round(edge_m, 2), nearest_ridge_m=round(ridge_m, 2),
                nearest_ridge_bearing_deg=None if math.isnan(bearing) else round(bearing, 1), ridge_sunward=bool(sunward))


def check_a(cells_hollow, hollow_px, stops, sun_az):
    H = cells_hollow.shape[0]
    rows = []
    for s in stops:
        cls, nh, nt = classify_stop(cells_hollow, s["e_m"], s["n_m"], H)
        ok = cls == s["label"]
        ei = edge_info(hollow_px, s["e_m"], s["n_m"], hollow_px.shape[0], sun_az)
        shadow = (not ok) and s["label"] == "ridge" and ei["nearest_ridge_m"] <= SHADOW_M and ei["ridge_sunward"]
        rows.append(dict(s, traced=cls, hollow_cells=nh, cells=nt, correct=ok, near_edge=ei["edge_m"] < STOP_R_M,
                         consistent_with_shadow=bool(shadow), **ei))
    n_ok = sum(r["correct"] for r in rows)
    req = all(r["correct"] for r in rows if r.get("required"))
    outcome = "PASS" if n_ok >= 6 and req else ("PARTIAL" if n_ok >= 5 else "FAIL")
    no_inf = [r for r in rows if not r.get("inferred")]
    return dict(outcome=outcome, correct=n_ok, n=len(rows), required_correct=req,
                context_without_inferred=f"{sum(r['correct'] for r in no_inf)} of {len(no_inf)}", stops=rows)


def relief(z, win=RELIEF_WIN_M):
    return z - ndi.median_filter(z, size=win, mode="nearest")


def check_b(rel, cells_hollow, valid=None):
    H, W = rel.shape
    valid = np.isfinite(rel) if valid is None else valid & np.isfinite(rel)

    def diff(hm, ok):
        r_, h_ = ok & ~hm, ok & hm
        return float(rel[r_].mean() - rel[h_].mean()) if r_.any() and h_.any() else float("nan")

    d0 = diff(cells_hollow, valid)
    base = []
    for dist in BASE_D:
        for ux, uy in BASE_DIRS:
            dx, dy = int(round(dist * ux / math.hypot(ux, uy))), int(round(dist * uy / math.hypot(ux, uy)))
            sh = np.zeros_like(cells_hollow)
            ok = np.zeros_like(valid)
            r0, c0 = max(0, dy), max(0, -dx)                    # shifted grid: class at (r, c) taken from (r - dy, c + dx)
            rs, cs = slice(max(0, dy), min(H, H + dy)), slice(max(0, -dx), min(W, W - dx))
            src_r, src_c = slice(rs.start - dy, rs.stop - dy), slice(cs.start + dx, cs.stop + dx)
            sh[rs, cs] = cells_hollow[src_r, src_c]
            ok[rs, cs] = valid[rs, cs]
            base.append(dict(dist_m=dist, dir=[ux, uy], diff_m=round(diff(sh, ok), 4)))
    p95 = float(np.nanpercentile([b["diff_m"] for b in base], 95))
    return dict(outcome="PASS" if d0 > 0 and d0 > p95 else "FAIL", ridge_minus_hollow_m=round(d0, 4), baseline_p95_m=round(p95, 4),
                baseline=base)


# ----------------------------------------------------------------------------------------------- inputs
def load_photo(g):
    import rasterio
    with rasterio.open(os.path.join(config.PROCESSED, "ortho", "ortho_patch.tif")) as d:
        img = d.read().transpose(1, 2, 0)
        T = d.transform
    ox0, oy1 = T.c, T.f
    px, py = int(round((g["x_west_m"] - ox0) / PX)), int(round((oy1 - g["y_north_m"]) / PX))
    H, W = g["height"], g["width"]
    rgb = img[py:py + 4 * H, px:px + 4 * W]
    assert rgb.shape[:2] == (4 * H, 4 * W), rgb.shape
    return rgb.astype(float).mean(axis=2)


def stop_positions(g):
    H = g["height"]
    stops_doc = {s["id"]: s for s in json.load(open(os.path.join(config.ASSETS, "stops.json"), encoding="utf-8"))["stops"]}
    fixes = []
    with open(config.PLACES, newline="") as f:
        for r in csv.DictReader(f):
            if r["frame"] != "ROVER":
                continue
            try:
                fixes.append((int(r["sol"]), r["site"], r["drive"], float(r["planetocentric_latitude"]), float(r["longitude"])))
            except ValueError:
                continue
    fixes.sort(key=lambda x: (x[0], int(x[1]), int(x[2])))
    out = []
    for L in LABELS:
        s = dict(L)
        if "stop" in L:
            st = stops_doc[L["stop"]]
            s.update(e_m=st["e_m"], n_m=st["n_m"], fix=dict(source="stops.json", sol=st.get("sol")))
        else:
            a, b = L["plan"]
            inside = [x for x in fixes if a <= x[0] <= b]
            fx = inside[0] if inside else [x for x in fixes if x[0] < a][-1]
            sol, site, drive, lat, lon = fx
            s.update(e_m=R * math.radians(lon) - g["x_west_m"], n_m=R * math.radians(lat) - (g["y_north_m"] - H * g["cell_m"]),
                     fix=dict(source="PLACES", sol=sol, site=site, drive=drive, rule="inside the plan" if inside else "latest before"))
        out.append(s)
    return out


# ----------------------------------------------------------------------------------------------- run
def run():
    man = json.load(open(os.path.join(config.ASSETS, "manifest.json")))
    g = man["grid"]
    H, W = g["height"], g["width"]
    sun_az = man["photo_light"]["sun_azimuth_deg"]
    lum = load_photo(g)
    hollow_px, z = trace(lum)
    cells = to_cells(hollow_px)
    stops = stop_positions(g)
    a = check_a(cells, hollow_px, stops, sun_az)
    dem = np.fromfile(os.path.join(config.ASSETS, "heightmap.bin"), "<f4").reshape(H, W).astype(float)
    b = check_b(relief(dem), cells)
    # context
    lab, n = ndi.label(hollow_px)
    areas = ndi.sum(np.ones_like(lab), lab, index=np.arange(1, n + 1)) * PX * PX if n else np.array([])
    ctx = dict(hollow_share=round(float(cells.mean()), 4), hollow_count=int(n),
               hollow_equiv_diameter_median_m=round(float(np.median(2 * np.sqrt(areas / math.pi))), 2) if n else None)
    for zc in (-0.25, 0.25):
        hp, _ = trace(lum, z_cut=zc)
        cz = to_cells(hp)
        ctx[f"z_cut_{zc:+}"] = dict(check_a=f"{check_a(cz, hp, stops, sun_az)['correct']} of 7", check_b_diff_m=check_b(relief(dem), cz)["ridge_minus_hollow_m"])
    try:
        import provenance as pv
        v2 = json.load(open(os.path.join(pv.OUT, "provenance_v2_result.json")))
        ua_ext, _ = pv.load_ua(g, pv.MARGIN2_M, "_v2")
        dx, dy, M = int(v2["shift_m"]["east"]), int(v2["shift_m"]["north"]), pv.MARGIN2_M
        ua = np.full((H, W), np.nan)
        r0, c0 = M - dy, M + dx
        ua[:, :] = ua_ext[r0:r0 + H, c0:c0 + W]
        ctx["check_b_on_ua_dtm"] = {k: v for k, v in check_b(relief(np.nan_to_num(ua, nan=np.nanmean(ua))), cells, np.isfinite(ua)).items() if k != "baseline"}
    except Exception as ex:                                    # context only
        ctx["check_b_on_ua_dtm"] = f"not computed: {ex}"
    import datetime
    res = dict(run_utc=datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
               rule="doc: Ridge tracing rule (approved by the author, amendments 1-4)",
               check_a=a, check_b=b, context=ctx, photo_sun_azimuth_deg=sun_az)
    os.makedirs(OUT, exist_ok=True)
    json.dump(res, open(os.path.join(OUT, "ridges_result.json"), "w"), indent=1)
    cls = np.where(cells, 2, 1).astype(np.uint8)
    cls.tofile(os.path.join(OUT, "ridges.bin"))
    json.dump(dict(rule=res["rule"], run_utc=res["run_utc"], layer="not shown: dropped by the author after the FAIL", method="25 cm photo: darker-than-local hollows (sigma 1 m, 30 m window, z < 0, 2 m opening, >= 25 m2)",
                   check_a=dict(outcome=a["outcome"], correct=a["correct"], n=a["n"]), check_b=dict(outcome=b["outcome"], ridge_minus_hollow_m=b["ridge_minus_hollow_m"]),
                   source="MSL Gale HiRISE LRGB orthophoto mosaic (USGS); HiRISE NASA/JPL/University of Arizona"),
              open(os.path.join(config.ASSETS, "ridges.json"), "w"), indent=1)
    try:
        from PIL import Image
        base = np.clip(lum, 0, 255).astype(np.uint8)
        rgb = np.stack([base] * 3, -1)
        edge = hollow_px ^ ndi.binary_erosion(hollow_px, disc(2))
        rgb[edge] = (255, 140, 0)
        for s in a["stops"]:
            r, c = int((4 * H * PX - s["n_m"]) / PX), int(s["e_m"] / PX)
            col = (0, 220, 0) if s["correct"] else (255, 0, 0)
            rgb[max(0, r - 6):r + 7, max(0, c - 6):c + 7] = col
        Image.fromarray(rgb).save(os.path.join(OUT, "ridges_trace.png"))
    except ImportError:
        pass
    print(json.dumps({k: v for k, v in res.items() if k != "check_b"} | dict(check_b={k: v for k, v in b.items() if k != "baseline"}), indent=1, default=str))


# ----------------------------------------------------------------------------------------------- selftest (synthetic)
def selftest():
    rng = np.random.default_rng(5)
    H, W = 120, 100                                       # cells (1 m); photo at 4 px per cell
    # boxwork-like: 5 m ridges every 18 m in x and y, hollows darker; plus noise
    yy, xx = np.mgrid[0:4 * H, 0:4 * W] * PX
    ridge = (np.mod(xx, 18.0) < 5.0) | (np.mod(yy, 18.0) < 5.0)
    lum = np.where(ridge, 150.0, 110.0) + rng.normal(0, 12, ridge.shape)
    hp, _ = trace(lum)
    cells = to_cells(hp)
    truth = to_cells(~ridge)
    assert np.mean(cells == truth) > 0.9, np.mean(cells == truth)
    # stops: centre of hollows and ridges (e, n in metres, n from the south edge)
    stops = []
    for i, (e, n_from_top, lab) in enumerate([(11.5, 11.5, "hollow"), (29.5, 47.5, "hollow"), (65.5, 83.5, "hollow"),
                                              (2.5, 30.0, "ridge"), (38.5, 60.0, "ridge"), (56.5, 100.0, "ridge"), (74.5, 20.0, "ridge")]):
        stops.append(dict(id=f"s{i}", label=lab, e_m=e, n_m=H - n_from_top, required=(i == 6)))
    a = check_a(cells, hp, stops, 303.0)
    assert a["outcome"] == "PASS" and a["correct"] == 7, [(s["id"], s["traced"]) for s in a["stops"]]
    # labels swapped: FAIL
    sw = [dict(s, label="ridge" if s["label"] == "hollow" else "hollow") for s in stops]
    assert check_a(cells, hp, sw, 303.0)["outcome"] == "FAIL"
    # required stop wrong but 6 of 7 right: not PASS
    st2 = [dict(s) for s in stops]
    st2[6]["label"] = "hollow"
    assert check_a(cells, hp, st2, 303.0)["outcome"] == "PARTIAL"
    # tie is a miss
    ch = np.zeros((4, 4), bool)
    ch[:, :2] = True
    assert classify_stop(ch, 2.0, 2.0, 4, r_m=1.0)[0] == "tie"
    # check B: ridges 1.5 m higher -> PASS with the right size; no relation -> FAIL
    zt = np.where(to_cells(ridge), 1.5, 0.0) + 0.002 * np.arange(W)[None, :] * 30 + rng.normal(0, 0.2, (H, W))
    b = check_b(relief(zt), cells)
    assert b["outcome"] == "PASS" and 0.8 < b["ridge_minus_hollow_m"] < 1.7, b["ridge_minus_hollow_m"]
    assert check_b(relief(rng.normal(0, 1, (H, W))), cells)["outcome"] == "FAIL"
    # shadow flag: a ridge-labelled stop just east of a ridge (ridge sunward at azimuth 270) is flagged when missed
    hm = np.zeros((40, 40), bool)
    hm[:, 20:] = True                                     # west half ridge (False), east half hollow (True)
    cm = to_cells(hm)
    r = check_a(cm, hm, [dict(id="x", label="ridge", e_m=6.5, n_m=5.0)], 270.0)["stops"][0]
    assert not r["correct"] and r["consistent_with_shadow"], r
    print("ridges selftest: PASS")


if __name__ == "__main__":
    a = sys.argv[1:]
    {"selftest": selftest, "run": run}.get(a[0] if a else "", lambda: print(__doc__))()
