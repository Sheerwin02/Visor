"""Chemistry along the walk: Curiosity's ChemCam (LIBS, Multivariate Oxide Composition tables) and APXS (oxide
concentrations) results for the study sols, placed on the walk grid.

Sources (PDS Geosciences Node, MSL Release 42; copied to data/raw/chem/):
  ChemCam MOC  https://pds-geosciences.wustl.edu/msl/msl-m-chemcam-libs-4_5-rdr-v1/mslccm_1xxx/data/moc/moc_*.csv
               (one row per laser observation point; wt% oxides; no sol column: the sol comes from index/libsindex.tab by
               product id; site and drive are in the product id, e.g. ..._F1172850... = site 117, drive 2850)
  APXS         https://pds-geosciences.wustl.edu/msl/msl-m-apxs-4_5-rdr-v1/mslapx_1xxx/extras/msl_apxs_concentrations_*.csv
               (one row per target integration; sol column; wt% oxides incl. SO3; site and drive in data_products,
               e.g. APB_...ESC47131200082... = sol 4713, site 120, drive 0082)
Placement: a target is put at Curiosity's PLACES localized position for its (site, drive): the rover's position when it
measured, not the target's. ChemCam shoots 2 to 7 m away and APXS touches rock beside the arm, so a target can be a few
metres from its dot. One record per (instrument, target, sol): the mean of its observation points (ChemCam: also the
point count and the highest CaO, since single points can hit a nodule or vein).
Output: assets/chemistry.json. Usage: python tools/chem.py build | selftest
"""
import csv
import glob
import json
import math
import os
import re
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402

RAW = os.path.join(config.RAW, "chem")
CC_OXIDES = ["SiO2", "TiO2", "Al2O3", "FeOT", "MgO", "CaO", "Na2O", "K2O", "MnO"]
APXS_OXIDES = ["Na2O", "MgO", "Al2O3", "SiO2", "P2O5", "SO3", "Cl", "K2O", "CaO", "TiO2", "Cr2O3", "MnO", "FeO"]
R = config.MARS_RADIUS_M


def site_drive_cc(product):
    m = re.search(r"_F(\d{3})(\d{4})", product.upper())
    return (int(m.group(1)), int(m.group(2))) if m else None


def site_drive_apxs(products):
    m = re.search(r"ESC(\d{4})(\d{3})(\d{4})", products.upper())
    return (int(m.group(2)), int(m.group(3))) if m else None


def num(x):
    try:
        v = float(x)
        return v if math.isfinite(v) else None
    except (TypeError, ValueError):
        return None


def sol_index(path):
    """product id (no extension) -> sol, from ChemCam's libsindex.tab (CCS rows)."""
    out = {}
    with open(path, encoding="latin-1") as f:
        for row in csv.reader(f):
            if len(row) > 3 and "CCS" in row[3]:
                try:
                    out[row[3].strip().upper()] = int(row[-1].strip())
                except ValueError:
                    pass
    return out


def read_chemcam(files, sols):
    recs = []
    for p in files:
        with open(p, encoding="latin-1") as f:
            rows = list(csv.reader(f))
        h = next(i for i, r in enumerate(rows) if r and r[0] == "File")
        head = rows[h]
        col = {name: head.index(name) for name in CC_OXIDES}
        dist = head.index("Distance (m)")
        for r in rows[h + 1:]:
            if len(r) < len(head) or not r[0]:
                continue
            pid = r[0].strip().upper().replace(".CSV", "")
            sd, sol = site_drive_cc(pid), sols.get(pid)
            if sd is None or sol is None:
                continue
            recs.append(dict(instrument="ChemCam", target=r[1].strip(), sol=sol, site=sd[0], drive=sd[1], dist_m=num(r[dist]),
                             ox={k: num(r[c]) for k, c in col.items()}))
    return recs


def read_apxs(files):
    recs = []
    for p in files:
        with open(p, encoding="latin-1") as f:
            for r in csv.DictReader(f):
                sd = site_drive_apxs(r.get("data_products", ""))
                sol = num(r.get("sol"))
                if sd is None or sol is None:
                    continue
                recs.append(dict(instrument="APXS", target=r["target"].strip(), sol=int(sol), site=sd[0], drive=sd[1], dist_m=None,
                                 ox={k: num(r.get(k)) for k in APXS_OXIDES}))
    return recs


def aggregate(recs):
    g = defaultdict(list)
    for r in recs:
        g[(r["instrument"], r["target"], r["sol"], r["site"], r["drive"])].append(r)
    out = []
    for (ins, tgt, sol, site, drive), rs in g.items():
        ox = {}
        for k in rs[0]["ox"]:
            v = [r["ox"][k] for r in rs if r["ox"][k] is not None]
            if v:
                ox[k] = round(sum(v) / len(v), 2)
        d = [r["dist_m"] for r in rs if r["dist_m"] is not None]
        ca = [r["ox"].get("CaO") for r in rs if r["ox"].get("CaO") is not None]
        out.append(dict(instrument=ins, target=tgt, sol=sol, site=site, drive=drive, n=len(rs), oxides=ox,
                        cao_max=round(max(ca), 2) if ca else None, dist_m=round(sum(d) / len(d), 2) if d else None))
    return sorted(out, key=lambda x: (x["sol"], x["instrument"], x["target"]))


def places_positions(path):
    """(site, drive) -> list of (sol, lat planetocentric, lon east) from PLACES localized_pos.csv."""
    pos = defaultdict(list)
    with open(path, encoding="utf-8") as f:
        for r in csv.DictReader(f):
            try:
                key = (int(float(r["site"])), int(float(r["drive"])))
                lat, lon, sol = float(r["planetocentric_latitude"]), float(r["longitude"]), int(float(r["sol"]))
            except (KeyError, ValueError):
                continue
            if lat > -90 and lon > -900:
                pos[key].append((sol, lat, lon))
    return pos


def place(recs, pos, grid):
    W, H, cell = grid["width"], grid["height"], grid["cell_m"]
    xw, yn = grid["x_west_m"], grid["y_north_m"]
    ys = yn - H * cell
    kept, missing, outside = [], 0, 0
    for r in recs:
        cands = pos.get((r["site"], r["drive"]))
        if not cands:
            missing += 1
            continue
        before = [c for c in cands if c[0] <= r["sol"]]
        sol, lat, lon = max(before) if before else min(cands)
        e, n = R * math.radians(lon) - xw, R * math.radians(lat) - ys
        if not (0 <= e < W * cell and 0 <= n < H * cell):
            outside += 1
            continue
        kept.append(dict(r, e_m=round(e, 2), n_m=round(n, 2), lat=round(lat, 6), lon=round(lon, 6)))
    return kept, missing, outside


def build():
    grid = json.load(open(os.path.join(config.ASSETS, "manifest.json"), encoding="utf-8"))["grid"]
    sols = sol_index(os.path.join(RAW, "libsindex.tab"))
    cc = read_chemcam(sorted(glob.glob(os.path.join(RAW, "moc_*.csv"))), sols)
    ap = read_apxs(sorted(glob.glob(os.path.join(RAW, "msl_apxs_concentrations_*.csv"))))
    recs = [r for r in aggregate(cc + ap) if config.SOL_FIRST <= r["sol"] <= config.SOL_LAST]
    kept, missing, outside = place(recs, places_positions(config.PLACES), grid)
    last = max((r["sol"] for r in cc + ap), default=None)
    doc = dict(
        description="Curiosity ChemCam (LIBS MOC) and APXS oxide results for the study sols, one record per instrument, target and sol "
                    "(mean of its observation points), placed at the rover's PLACES position for that site and drive.",
        placement_note="The dot is where Curiosity stood, not the target: ChemCam targets are 2 to 7 m away, APXS targets beside the arm.",
        units="wt% oxides", last_sol_released=last, sols=[config.SOL_FIRST, config.SOL_LAST],
        counts=dict(records=len(kept), chemcam=sum(r["instrument"] == "ChemCam" for r in kept), apxs=sum(r["instrument"] == "APXS" for r in kept),
                    no_places_position=missing, outside_grid=outside),
        sources=dict(chemcam="https://pds-geosciences.wustl.edu/msl/msl-m-chemcam-libs-4_5-rdr-v1/mslccm_1xxx/data/moc/",
                     apxs="https://pds-geosciences.wustl.edu/msl/msl-m-apxs-4_5-rdr-v1/mslapx_1xxx/extras/",
                     release="MSL PDS Release 42 (ChemCam and APXS through sol 4843)",
                     credit="ChemCam: NASA/JPL-Caltech/LANL/CNES; APXS: NASA/JPL-Caltech/University of Guelph; via the PDS Geosciences Node"),
        records=kept)
    json.dump(doc, open(os.path.join(config.ASSETS, "chemistry.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(json.dumps(doc["counts"]), "last sol released:", last)


def selftest():
    assert site_drive_cc("CL5_807009802CCS_F1172850CCAM06611P3") == (117, 2850)
    assert site_drive_apxs("APB_815912952ESC47131200082_______M1.DAT:APB_x") == (120, 82)
    a = aggregate([dict(instrument="ChemCam", target="t", sol=1, site=1, drive=2, dist_m=3.0, ox={"CaO": 2.0, "SiO2": None}),
                   dict(instrument="ChemCam", target="t", sol=1, site=1, drive=2, dist_m=5.0, ox={"CaO": 30.0, "SiO2": 40.0})])
    assert a[0]["n"] == 2 and a[0]["oxides"] == {"CaO": 16.0, "SiO2": 40.0} and a[0]["cao_max"] == 30.0 and a[0]["dist_m"] == 4.0
    grid = dict(width=10, height=10, cell_m=1.0, x_west_m=R * math.radians(137.0), y_north_m=R * math.radians(-4.0) + 10.0)
    k, miss, out = place([dict(a[0], site=1, drive=2), dict(a[0], site=9, drive=9)],
                         {(1, 2): [(1, -4.0 + math.degrees(3.0 / R), 137.0 + math.degrees(4.0 / R))]}, grid)
    assert miss == 1 and out == 0 and abs(k[0]["e_m"] - 4.0) < 0.01 and abs(k[0]["n_m"] - 3.0) < 0.01, k
    print("chem selftest: PASS")


if __name__ == "__main__":
    {"build": build, "selftest": selftest}.get(sys.argv[1] if len(sys.argv) > 1 else "selftest", selftest)()
