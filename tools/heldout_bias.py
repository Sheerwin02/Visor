"""Held-out check of the DEM tilt bias (rule fixed before any held-out
result). Do not change the rule after a result: a new question gets a new rule.

Question: does the bias (median of d = DEM tilt - rover tilt), fitted on some rover stops, improve the DEM tilt on
stops it was not fitted on, and is its sign stable along the traverse?

Rule, as fixed in advance:
1. Folds: stops in file order (sol, site, drive), 4 contiguous blocks of 21, 21, 21, 22; each held out once.
2. Buffer: training stops within B m of any held-out stop are dropped. B = max(3 m, L); L = first lag at which the
   autocorrelation of signed d (stop order) falls below 1/e, times the median distance between consecutive stops.
3. Per fold: b = median signed d of the training stops; held-out error without correction = median |d|, with = median |d - b|.
4. Interval: moving-block bootstrap of the training median, block 8 (primary; 1, 4, 16 context), 10,000 resamples,
   seed 20261112, 95% percentile interval; share of resamples with b > 0.
5. PASS if (A) corrected held-out error < uncorrected, pooled over all held-out stops AND in >= 3 of 4 folds, and
   (B) in all 4 folds b > 0 and its interval excludes 0. PARTIAL if (A) but not (B). FAIL if not (A).
6. Context only: each held-out block's signed median inside its training interval?; 1,000 random 50/50 splits;
   signed median d by thirds of rover tilt; full-sample median with its block-8 interval.

Input: data/interim/rover_check/rover_check_stops.csv (from tools/rover_check.py). Positions: equirectangular sphere
R 3396190 m (as prep locate), only for distances.
Usage: python tools/heldout_bias.py selftest | python tools/heldout_bias.py run [--csv PATH] [--out DIR]
"""
import csv
import json
import math
import os
import sys
from datetime import datetime, timezone

import numpy as np

R_MARS = 3396190.0
FOLDS = (21, 21, 21, 22)
MIN_BUFFER_M = 3.0
BLOCK = 8
CONTEXT_BLOCKS = (1, 4, 16)
RESAMPLES = 10_000
SEED = 20261112
RANDOM_SPLITS = 1000


def positions_m(lat, lon):
    lat, lon = np.radians(lat), np.radians(lon)
    return np.stack([R_MARS * lon, R_MARS * lat], axis=1)


def corr_length(d, xy):
    """Lag (stops) where the autocorrelation of d first drops below 1/e, times the median step (m)."""
    x = d - d.mean()
    den = float((x * x).sum())
    lag = None
    for k in range(1, len(d)):
        if float((x[:-k] * x[k:]).sum()) / den < 1 / math.e:
            lag = k
            break
    if lag is None:
        lag = len(d)
    step = float(np.median(np.linalg.norm(np.diff(xy, axis=0), axis=1)))
    return lag, step, lag * step


def block_boot_median(x, block, n_res, rng):
    """Moving-block bootstrap of the median: blocks of consecutive values, concatenated, cut to len(x)."""
    n = len(x)
    block = min(block, n)
    starts_max = n - block + 1
    k = math.ceil(n / block)
    st = rng.integers(0, starts_max, size=(n_res, k))
    idx = (st[:, :, None] + np.arange(block)[None, None, :]).reshape(n_res, -1)[:, :n]
    return np.median(x[idx], axis=1)


def interval(x, block, rng):
    m = block_boot_median(x, block, RESAMPLES, rng)
    lo, hi = np.percentile(m, [2.5, 97.5])
    return float(lo), float(hi), float((m > 0).mean())


def evaluate(d, xy, tilt=None):
    n = len(d)
    assert sum(FOLDS) == n, f"rule expects {sum(FOLDS)} stops, got {n}"
    lag, step, L = corr_length(d, xy)
    B = max(MIN_BUFFER_M, L)
    rng = np.random.default_rng(SEED)
    edges = np.cumsum((0,) + FOLDS)
    folds, held_unc, held_cor = [], [], []
    for f in range(len(FOLDS)):
        test = np.arange(edges[f], edges[f + 1])
        dist = np.min(np.linalg.norm(xy[:, None, :] - xy[None, test, :], axis=2), axis=1)
        train = np.array([i for i in range(n) if i not in set(test) and dist[i] > B])
        b = float(np.median(d[train]))
        lo, hi, pos = interval(d[train], BLOCK, rng)
        ctx = {str(l): interval(d[train], l, rng)[:2] for l in CONTEXT_BLOCKS}
        unc, cor = np.abs(d[test]), np.abs(d[test] - b)
        held_unc += list(unc); held_cor += list(cor)
        hm = float(np.median(d[test]))
        folds.append(dict(fold=f + 1, test_stops=len(test), train_stops=len(train), dropped_by_buffer=n - len(test) - len(train),
                          b_deg=round(b, 3), ci95_block8=[round(lo, 3), round(hi, 3)], share_b_pos=round(pos, 4),
                          ci95_context={k: [round(v[0], 3), round(v[1], 3)] for k, v in ctx.items()},
                          heldout_median_abs_uncorrected=round(float(np.median(unc)), 3),
                          heldout_median_abs_corrected=round(float(np.median(cor)), 3),
                          improved=bool(np.median(cor) < np.median(unc)),
                          heldout_signed_median=round(hm, 3), heldout_median_inside_train_ci=bool(lo <= hm <= hi)))
    pooled_unc, pooled_cor = float(np.median(held_unc)), float(np.median(held_cor))
    a = pooled_cor < pooled_unc and sum(f["improved"] for f in folds) >= 3
    bb = all(f["b_deg"] > 0 and f["ci95_block8"][0] > 0 for f in folds)
    outcome = "PASS" if a and bb else ("PARTIAL" if a else "FAIL")
    # context: random 50/50 splits
    gains = []
    for _ in range(RANDOM_SPLITS):
        p = rng.permutation(n); tr, te = p[: n // 2], p[n // 2:]
        b = np.median(d[tr])
        gains.append(float(np.median(np.abs(d[te])) - np.median(np.abs(d[te] - b))))
    lo, hi, pos = interval(d, BLOCK, rng)
    res = dict(outcome=outcome, criterion_A=bool(a), criterion_B=bool(bb), n=n,
               corr_lag_stops=lag, median_step_m=round(step, 3), L_m=round(L, 3), buffer_m=round(B, 3),
               pooled_heldout_median_abs_uncorrected=round(pooled_unc, 3), pooled_heldout_median_abs_corrected=round(pooled_cor, 3),
               blocked_pooled_gain_deg=round(pooled_unc - pooled_cor, 3),
               random_split_gain_deg_median=round(float(np.median(gains)), 3),
               random_split_share_improved=round(float(np.mean(np.array(gains) > 0)), 4),
               full_sample_median_deg=round(float(np.median(d)), 3), full_sample_ci95_block8=[round(lo, 3), round(hi, 3)],
               full_sample_share_b_pos=round(pos, 4), folds=folds)
    if tilt is not None:
        order = np.argsort(tilt, kind="stable"); thirds = np.array_split(order, 3)
        res["by_rover_tilt_thirds"] = [dict(rover_tilt_range=[round(float(tilt[t].min()), 2), round(float(tilt[t].max()), 2)],
                                            n=len(t), signed_median_d=round(float(np.median(d[t])), 3)) for t in thirds]
    return res


def selftest():
    rng = np.random.default_rng(1)
    n = sum(FOLDS)
    xy = np.cumsum(np.full((n, 2), [3.0, 1.0]), axis=0)          # stops 3.16 m apart along a line
    e = np.zeros(n)
    for i in range(1, n):                                          # AR(1) noise, some autocorrelation
        e[i] = 0.5 * e[i - 1] + rng.normal(0, 1.5)
    r = evaluate(3.0 + e, xy, tilt=rng.uniform(0, 20, n))
    assert r["outcome"] == "PASS", r["outcome"]
    assert r["buffer_m"] >= MIN_BUFFER_M and all(f["dropped_by_buffer"] >= 0 for f in r["folds"])
    r0 = evaluate(e - np.median(e), xy)
    assert r0["outcome"] != "PASS", "zero bias must not pass"
    flip = np.where(np.arange(n) < n // 2, 3.0, -3.0) + 0.3 * e    # bias sign flips half way: must not pass
    assert evaluate(flip, xy)["outcome"] != "PASS"
    m = block_boot_median(np.arange(10.0), 4, 100, np.random.default_rng(0))
    assert m.shape == (100,) and m.min() >= 0 and m.max() <= 9
    print("selftest passed: constant bias -> PASS, zero bias and sign flip -> not PASS, block bootstrap shape (synthetic)")


def run(csv_path, out):
    rows = list(csv.DictReader(open(csv_path, encoding="utf-8")))
    d = np.array([float(r["dem_tilt"]) - float(r["rover_tilt"]) for r in rows])
    xy = positions_m(np.array([float(r["lat"]) for r in rows]), np.array([float(r["lon"]) for r in rows]))
    tilt = np.array([float(r["rover_tilt"]) for r in rows])
    res = evaluate(d, xy, tilt)
    res = dict(run_utc=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
               rule="doc: Held-out bias rule (approved)", input=os.path.relpath(csv_path), **res)
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "heldout_bias_result.json"), "w", encoding="utf-8") as f:
        json.dump(res, f, indent=1)
    print(json.dumps(res, indent=1))


if __name__ == "__main__":
    sys.path.insert(0, os.path.dirname(__file__))
    if sys.argv[1:2] == ["selftest"]:
        selftest()
    elif sys.argv[1:2] == ["run"]:
        import config
        a = sys.argv[2:]
        csv_path = a[a.index("--csv") + 1] if "--csv" in a else os.path.join(config.ROVER_CHECK_DIR, "rover_check_stops.csv")
        out = a[a.index("--out") + 1] if "--out" in a else os.path.join(config.INTERIM, "heldout_bias")
        run(csv_path, out)
    else:
        print(__doc__)
