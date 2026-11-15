"""Curiosity's own 360-degree view at each station (Walk "look around"): PDS Navcam cylindrical mosaics, one per stop.

Rule (fixed before looking at any image, no hand-picking):
- data: PDS MSL Navcam mosaic bundle MSLMOS_1XXX (doi:10.17189/dk0p-1d15), product
  N_R000_<sol>_ILT<site>CYL_S_<drive>_UNCORM1 for each stop's PLACES sol / site / drive in assets/stops.json;
  a stop without that exact product gets no panorama (never a nearby one);
- projection from the label only: CYLINDRICAL, azimuth = START_AZIMUTH + sample / MAP_RESOLUTION, clockwise, in SITE_FRAME
  (x north: checked on the six station photos, tools/photo_view.py, where the site-frame attitude heading equals the PLACES
  heading); elevation = (ZERO_ELEVATION_LINE - line) / MAP_RESOLUTION; checks: 360 deg across, elevation limits within
  ELEV_TOL_DEG of MSL:MAXIMUM/MINIMUM_ELEVATION, image size = RECORD_BYTES x (FILE_RECORDS - 2 header records);
  AMENDMENT (after the first build, before any image was looked at): the tolerance was 1 pixel; the first label differed by
  1-3 pixels (0.05-0.14 deg: the keywords appear to give the data extent, not the image edges), so it is now ELEV_TOL_DEG and
  each panorama records its differences (el_label_diff_deg);
- display: 16-bit radiance -> 8 bit by a linear stretch between the 0.5th and 99.5th percentile of non-zero pixels (0 =
  no data, kept black), resized to PANO_W wide (aspect kept), JPEG; a display choice, not a measurement.

    python tools/navcam_pano.py fetch      # labels + images into data/raw/navcam_mosaic (~32 MB each)
    python tools/navcam_pano.py build      # assets/pano_<stop>.jpg + assets/panos.json, then build_bundle.py rehash
    python tools/navcam_pano.py selftest
"""
import json
import os
import re
import sys
import urllib.request

import numpy as np

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402

RAW = os.path.join(config.RAW, "navcam_mosaic")
ROOT = "https://planetarydata.jpl.nasa.gov/img/data/msl/MSLMOS_1XXX/DATA/SOL{sol:05d}/"
DOI = "10.17189/dk0p-1d15"
CREDIT = "NASA/JPL-Caltech; MSL Navcam mosaic products, PDS (doi:10.17189/dk0p-1d15)"
OUT_NAME = "panos.json"
PANO_W = 4096
ELEV_TOL_DEG = 0.25   # amended, see the rule
STRETCH = (0.5, 99.5)


def product(stop):
    return f"N_R000_{stop['sol']}_ILT{int(stop['site'])}CYL_S_{int(stop['drive']):04d}_UNCORM1"


def label_values(text):
    """PDS3 label keywords (first occurrence; numbers parsed, units dropped)."""
    out = {}
    for m in re.finditer(r"^\s*([A-Z_:\^]+)\s*=\s*(.+)$", text, re.M):
        k, v = m.group(1), m.group(2).strip()
        if k in out:
            continue
        n = re.match(r"^\(?\s*(-?[0-9.]+(?:[eE][-+]?[0-9]+)?)", v)
        out[k] = float(n.group(1)) if n else v.strip('"')
    return out


def geometry(lab):
    """Projection checks from the label; raises on anything the rule does not expect."""
    if lab.get("MAP_PROJECTION_TYPE") != "CYLINDRICAL":
        raise ValueError(f"projection {lab.get('MAP_PROJECTION_TYPE')}")
    if lab.get("POSITIVE_AZIMUTH_DIRECTION") != "CLOCKWISE" or lab.get("REFERENCE_COORD_SYSTEM_NAME") != "SITE_FRAME":
        raise ValueError("azimuth not clockwise in SITE_FRAME")
    res, w, h = lab["MAP_RESOLUTION"], int(lab["LINE_SAMPLES"]), int(lab["LINES"])
    a0, a1 = lab["START_AZIMUTH"], lab["STOP_AZIMUTH"]
    if abs((a1 - a0) - 360.0) > 1e-6 or abs(w / res - 360.0) > 1.0 / res:
        raise ValueError(f"not 360 deg across: {a0}..{a1}, {w} px at {res} px/deg")
    z = lab["ZERO_ELEVATION_LINE"]
    el_max, el_min = z / res, (z - h) / res
    d_max, d_min = el_max - lab["MSL:MAXIMUM_ELEVATION"], el_min - lab["MSL:MINIMUM_ELEVATION"]
    if abs(d_max) > ELEV_TOL_DEG or abs(d_min) > ELEV_TOL_DEG:
        raise ValueError(f"elevation limits {el_min:.3f}..{el_max:.3f} vs label {lab['MSL:MINIMUM_ELEVATION']}..{lab['MSL:MAXIMUM_ELEVATION']}")
    if int(lab["RECORD_BYTES"]) != 2 * w or int(lab["FILE_RECORDS"]) != h + 2 or int(lab["SAMPLE_BITS"]) != 16:
        raise ValueError("record layout not 16-bit rows after 2 header records")
    return dict(width=w, height=h, px_per_deg=res, start_az_deg=a0, zero_elevation_line=z, el_max_deg=round(el_max, 3),
                el_min_deg=round(el_min, 3), offset=2 * int(lab["RECORD_BYTES"]), el_label_diff_deg=[round(d_min, 3), round(d_max, 3)])


def read_image(path, g):
    dt = ">i2" if True else "<i2"   # SAMPLE_TYPE MSB_INTEGER (checked in build)
    a = np.fromfile(path, dtype=dt, count=g["width"] * g["height"], offset=g["offset"])
    return a.reshape(g["height"], g["width"]).astype(np.float32)


def to_8bit(a):
    valid = a > 0
    if not valid.any():
        raise ValueError("no valid pixels")
    lo, hi = np.percentile(a[valid], STRETCH)
    out = np.clip((a - lo) / max(hi - lo, 1e-6), 0, 1) * 255
    out[~valid] = 0
    return out.astype(np.uint8), (float(lo), float(hi)), float(valid.mean())


def stops():
    return json.load(open(os.path.join(config.ASSETS, "stops.json"), encoding="utf-8"))["stops"]


def fetch():
    os.makedirs(RAW, exist_ok=True)
    for s in stops():
        p = product(s)
        for ext in ("LBL", "IMG"):
            dst = os.path.join(RAW, f"{p}.{ext}")
            if os.path.exists(dst):
                continue
            url = ROOT.format(sol=s["sol"]) + f"{p}.{ext}"
            try:
                urllib.request.urlretrieve(url, dst)
                print(f"  {s['id']}: {p}.{ext} {os.path.getsize(dst) / 1e6:.1f} MB")
            except Exception as e:  # noqa: BLE001
                print(f"  {s['id']}: {p}.{ext} not available ({e}); no panorama for this stop")
                break


def build():
    from PIL import Image
    out = dict(rule=__doc__.split("Rule")[1].split("python tools")[0].strip(), doi=DOI, credit=CREDIT, panos=[])
    for s in stops():
        p = product(s)
        lp, ip = os.path.join(RAW, p + ".LBL"), os.path.join(RAW, p + ".IMG")
        if not (os.path.exists(lp) and os.path.exists(ip)):
            print(f"  {s['id']}: no {p}; skipped")
            continue
        lab = label_values(open(lp, encoding="latin-1").read())
        if lab.get("SAMPLE_TYPE") != "MSB_INTEGER":
            raise ValueError(f"{p}: sample type {lab.get('SAMPLE_TYPE')}")
        if int(lab.get("PLANET_DAY_NUMBER", s["sol"])) != s["sol"] or int(lab.get("COORDINATE_SYSTEM_INDEX", -1)) != int(s["site"]):
            raise ValueError(f"{p}: sol/site in the label differ from the stop")
        g = geometry(lab)
        img8, (lo, hi), cover = to_8bit(read_image(ip, g))
        h_out = round(g["height"] * PANO_W / g["width"])
        name = f"pano_{s['id']}.jpg"
        Image.fromarray(img8).resize((PANO_W, h_out), Image.LANCZOS).save(os.path.join(config.ASSETS, name), quality=85, optimize=True)
        out["panos"].append(dict(stop=s["id"], sol=s["sol"], site=s["site"], drive=s["drive"], product=p, file=name,
                                 width=PANO_W, height=h_out, start_az_deg=g["start_az_deg"], el_max_deg=g["el_max_deg"],
                                 el_min_deg=g["el_min_deg"], el_label_diff_deg=g["el_label_diff_deg"], source_px=[g["width"], g["height"]], px_per_deg_source=g["px_per_deg"],
                                 instrument=lab.get("INSTRUMENT_NAME"), start_time=lab.get("START_TIME"), coverage=round(cover, 3),
                                 stretch=dict(percentiles=list(STRETCH), radiance_dn=[round(lo, 1), round(hi, 1)]),
                                 url=ROOT.format(sol=s["sol"]) + p + ".IMG"))
        print(f"  {s['id']}: {name} {PANO_W}x{h_out}, elev {g['el_min_deg']}..{g['el_max_deg']}, coverage {cover:.1%}")
    json.dump(out, open(os.path.join(config.ASSETS, OUT_NAME), "w", encoding="utf-8"), indent=1)
    print(f"  {OUT_NAME}: {len(out['panos'])} of {len(stops())} stops")


def selftest():
    lab = label_values('''MAP_PROJECTION_TYPE = CYLINDRICAL
  MAP_RESOLUTION = (10.0 <pixel/degree>,
  MSL:MAXIMUM_ELEVATION = 20.0 <deg>
  MSL:MINIMUM_ELEVATION = -70.0 <deg>
  POSITIVE_AZIMUTH_DIRECTION = CLOCKWISE
  REFERENCE_COORD_SYSTEM_NAME = "SITE_FRAME"
  START_AZIMUTH = 0.0 <deg>
  STOP_AZIMUTH = 360.0 <deg>
  ZERO_ELEVATION_LINE = 200.0
  LINES = 900
  LINE_SAMPLES = 3600
  SAMPLE_BITS = 16
  RECORD_BYTES = 7200
  FILE_RECORDS = 902''')
    g = geometry(lab)
    assert g["el_max_deg"] == 20.0 and g["el_min_deg"] == -70.0 and g["offset"] == 14400, g
    for bad in (dict(MAP_PROJECTION_TYPE="POLAR"), dict(STOP_AZIMUTH=180.0), dict(ZERO_ELEVATION_LINE=210.0), dict(FILE_RECORDS=901.0)):
        try:
            geometry({**lab, **bad})
            raise AssertionError(f"accepted {bad}")
        except ValueError:
            pass
    a = np.zeros((4, 4), np.float32)
    a[1:, :] = np.arange(12).reshape(3, 4) + 100
    b, (lo, hi), cover = to_8bit(a)
    assert b[0].max() == 0 and b.max() == 255 and cover == 0.75, (b, cover)
    assert product(dict(sol=4711, site="120", drive="82")) == "N_R000_4711_ILT120CYL_S_0082_UNCORM1"
    print("navcam_pano selftest: PASS")


if __name__ == "__main__":
    {"fetch": fetch, "build": build, "selftest": selftest}.get(sys.argv[1] if len(sys.argv) > 1 else "selftest", selftest)()
