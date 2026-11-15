"""Route planner for the VISOR loop (rule fixed before the route was computed).

- corrected slope = slope_3m - measured bias (rover-check signed median, read from data/interim/rover_check), floored at 0
- speed = SPEED_FLAT x exp(-3.5 x tan(corrected slope)); exponential shape from Tobler (1993) hiking function, relative only;
  0.6 m/s flat is the plan's ASSUMPTION (middle of NASA's lunar intrasite range, see SPEED_SOURCE_RANGE_MPS);
  neither is a suited-astronaut or Mars model. Speed scales every cost equally, so it never changes the route, only the times
- a cell is blocked if corrected slope > LIMIT_DEG (ASSUMPTION), unless inside the rover-verified zone
  (discs of VERIFIED_R_M around each localized rover position)
- each leg: minimum walking time over the 8-neighbour 1 m grid (Dijkstra, scipy.sparse.csgraph)
Usage: imported by tools/build_bundle.py;  python tools/route.py selftest
"""
import math
import sys

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import dijkstra

def measured_bias_deg():
    """The rover-check signed median (DEM minus rover tilt), read from its result file; never typed here."""
    import config
    return float(config.rover_check_result()["signed_median_deg"])
SPEED_FLAT = 0.6  # changed from 0.8 after the citation check below; routes are unchanged by speed
CAUTION_DEG = 15.0  # display band only, not a routing rule: the 15 deg slope band from the Apollo slopes abstract (EPSC 2024), as in design v4.1
TOBLER_K = 3.5  # exponent of Tobler's hiking function (Tobler 1993, Fig. II), used as a relative slope penalty only
LIMIT_DEG = 20.0
VERIFIED_R_M = 3.0
# What the planner may choose in the app (each is an ASSUMPTION): flat walking speeds spanning NASA's intrasite range,
# and the slope-limit range around the 20 deg rule.
SPEED_OPTIONS_MPS = (0.4, 0.6, 0.8)
# PUBLISHED (not ours): NASA's lunar operational assumption, "0.4 to 0.8 m/s ... the range assumed for intrasite translations"
# (Norcross et al. 2010, NASA/TP-2010-216115, p. 2, from Apollo film analysis, the Lunar Architecture Team and analog tests;
# site-to-site translations are 1.1-1.4 m/s). Checked in the PDF . The earlier citation "0.8-1.25 m/s" misread the
# example speeds of that report's Table 7. Lunar gravity, not Mars; shown in the app as the source for the default speed.
SPEED_SOURCE_RANGE_MPS = (0.4, 0.8)
LIMIT_RANGE_DEG = (15.0, 25.0)


def to_cell(e, n, H, cell=1.0):
    return int((H * cell - n) // cell), int(e // cell)          # (row, col); row 0 = north


def verified_mask(H, W, rover_en, cell=1.0, r_m=VERIFIED_R_M):
    m = np.zeros((H, W), bool)
    rr, cc = np.mgrid[0:H, 0:W]
    ce, cn = (cc + 0.5) * cell, (H - rr - 0.5) * cell             # cell-centre coordinates
    for e, n in rover_en:
        r0, c0 = to_cell(e, n, H, cell)
        k = int(math.ceil(r_m / cell)) + 1
        a, b = max(0, r0 - k), min(H, r0 + k + 1)
        c, d = max(0, c0 - k), min(W, c0 + k + 1)
        m[a:b, c:d] |= (ce[a:b, c:d] - e) ** 2 + (cn[a:b, c:d] - n) ** 2 <= r_m ** 2
    return m


def build_graph(slope3m, verified, cell=1.0, bias_deg=0.0):
    H, W = slope3m.shape
    corr = np.clip(slope3m - bias_deg, 0, None)
    speed = SPEED_FLAT * np.exp(-TOBLER_K * np.tan(np.radians(corr)))
    blocked = (corr > LIMIT_DEG) & ~verified
    inv = 1.0 / speed
    idx = np.arange(H * W).reshape(H, W)
    rows, cols, wts = [], [], []
    for dr, dc in ((0, 1), (1, 0), (1, 1), (1, -1)):
        a = idx[0:H - dr, max(0, -dc):W - max(0, dc)]                # each cell paired with its (dr, dc) neighbour
        b = idx[dr:H, max(0, dc):W - max(0, -dc)]
        step = cell * math.hypot(dr, dc)
        ok = ~(blocked.ravel()[a.ravel()] | blocked.ravel()[b.ravel()])
        w = step * 0.5 * (inv.ravel()[a.ravel()] + inv.ravel()[b.ravel()])
        rows += [a.ravel()[ok], b.ravel()[ok]]; cols += [b.ravel()[ok], a.ravel()[ok]]; wts += [w[ok], w[ok]]
    g = coo_matrix((np.concatenate(wts), (np.concatenate(rows), np.concatenate(cols))), shape=(H * W, H * W)).tocsr()
    return g, corr, blocked


def path_from(pred, src, dst):
    out, k = [], dst
    while k != src:
        if k < 0:
            return None
        out.append(k); k = pred[k]
    out.append(src)
    return out[::-1]


def plan_loop(slope3m, rover_en, stops, cell=1.0, bias_deg=None):
    """stops: list of dicts with id, e_m, n_m (first = exit). Returns dict with legs, per-stop walk-back and the polyline."""
    H, W = slope3m.shape
    verified = verified_mask(H, W, rover_en, cell)
    if bias_deg is None:
        bias_deg = measured_bias_deg()
    g, corr, blocked = build_graph(slope3m, verified, cell, bias_deg)
    nodes = [to_cell(s["e_m"], s["n_m"], H, cell) for s in stops]
    nid = [r * W + c for r, c in nodes]
    order = list(range(len(stops))) + [0]
    legs, poly = [], []
    for i in range(len(order) - 1):
        a, b = nid[order[i]], nid[order[i + 1]]
        dist, pred = dijkstra(g, indices=a, return_predecessors=True)
        p = None if not np.isfinite(dist[b]) else path_from(pred, a, b)
        if p is None:
            legs.append(dict(frm=stops[order[i]]["id"], to=stops[order[i + 1]]["id"], possible=False))
            continue
        rc = [(k // W, k % W) for k in p]
        length = sum(cell * math.hypot(r2 - r1, c2 - c1) for (r1, c1), (r2, c2) in zip(rc, rc[1:]))
        cs = [float(corr[r, c]) for r, c in rc]
        legs.append(dict(frm=stops[order[i]]["id"], to=stops[order[i + 1]]["id"], possible=True, length_m=round(length, 1),
                         time_s=round(float(dist[b]), 0), max_corrected_slope_deg=round(max(cs), 1),
                         share_in_rover_verified=round(float(np.mean([verified[r, c] for r, c in rc])), 3),
                         cells_over_limit_inside_verified=int(sum(1 for r, c in rc if corr[r, c] > LIMIT_DEG))))
        pts = [[round((c + 0.5) * cell, 2), round((H - r - 0.5) * cell, 2)] for r, c in rc]
        legs[-1]["poly_start"] = 0 if not poly else len(poly) - 1   # index of this leg's first point in polyline_en
        poly += pts if not poly else pts[1:]
        legs[-1]["poly_end"] = len(poly) - 1
    back, _ = dijkstra(g, indices=nid[0], return_predecessors=True)
    walk_back = {s["id"]: (round(float(back[k]), 0) if np.isfinite(back[k]) else None) for s, k in zip(stops, nid)}
    # time back to the exit from EVERY cell under the same rules (inf = no way back without breaking the slope rule)
    walk_back_field = back.reshape(H, W)
    return dict(legs=legs, polyline_en=poly, walk_back_s=walk_back, walk_back_field=walk_back_field,
                blocked_cells=int(blocked.sum()), verified_cells=int(verified.sum()),
                total_length_m=round(sum(l.get("length_m", 0) for l in legs), 1),
                total_time_s=round(sum(l.get("time_s", 0) for l in legs), 0),
                all_legs_possible=all(l["possible"] for l in legs))


def selftest():
    H, W = 60, 60
    s = np.full((H, W), 5.0)
    s[:, 30] = 40.0                        # a steep wall across the whole grid at column 30
    stops = [dict(id="a", e_m=10.5, n_m=30.5), dict(id="b", e_m=50.5, n_m=30.5)]
    r = plan_loop(s, [], stops, bias_deg=3.0)
    assert not r["all_legs_possible"], "wall must block without a verified zone"
    r = plan_loop(s, [(30.5, 30.5)], stops, bias_deg=3.0)
    assert r["all_legs_possible"], "verified zone must open the wall"
    leg = r["legs"][0]
    assert leg["cells_over_limit_inside_verified"] >= 1 and 40 <= leg["length_m"] <= 45, leg
    flat = plan_loop(np.full((H, W), 3.0), [], stops, bias_deg=3.0)  # corrected slope 0 everywhere (synthetic bias)
    assert abs(flat["legs"][0]["time_s"] - 40 / SPEED_FLAT) < 1, flat["legs"][0]
    assert flat["walk_back_s"]["b"] == flat["legs"][1]["time_s"]
    f = flat["walk_back_field"]; ra, ca = to_cell(10.5, 30.5, H)
    assert f[ra, ca] == 0 and np.isfinite(f).all(), "field: zero at the exit, finite on open ground"
    walled = plan_loop(s, [], stops, bias_deg=3.0)["walk_back_field"]
    assert not np.isfinite(walled[:, 31:]).any(), "field: no way back across the wall without a verified zone"
    lg = flat["legs"][0]
    assert flat["polyline_en"][lg["poly_start"]] == [10.5, 30.5] and flat["polyline_en"][lg["poly_end"]] == [50.5, 30.5], lg
    print("selftest passed: wall blocks, rover-verified zone opens it, flat time = distance / SPEED_FLAT, walk-back + field + leg indices (synthetic)")


if __name__ == "__main__":
    if sys.argv[1:] == ["selftest"]:
        selftest()
    else:
        sys.exit(__doc__)
