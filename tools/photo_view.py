"""Which way each station photo looks (compass azimuth and elevation), from the NASA raw-image record of that photo.

Method (derived here, checked on every photo before use):
  view azimuth = PLACES rover heading (yaw, N 0 / E 90) + the record's mast azimuth (rover frame, clockwise from the
  rover's forward axis); view elevation = mast elevation (negative = below the horizon).
Two checks per photo, both must hold or no view is written:
  1. the record's rover attitude quaternion gives the same heading as PLACES (within 0.5 deg);
  2. the azimuth of the record's camera_vector (rover frame) equals the mast azimuth (within 0.5 deg).
Source: NASA raw-images feed https://mars.nasa.gov/api/v1/raw_image_items/ (undocumented API, as tools/fetch_photos.py).

Usage: python tools/photo_view.py fetch   (adds the raw record's pointing fields to tools/sources/stops_source.json)
       python tools/photo_view.py selftest
build_bundle.py rehash calls augment() to write photo.view into assets/stops.json.
"""
import json
import math
import os
import sys
import urllib.request

import config

API = ("https://mars.nasa.gov/api/v1/raw_image_items/?order=sol%20desc&per_page=200&page=0&condition_1=msl:mission"
       "&condition_2={sol}:sol:gte&condition_3={sol}:sol:lte&search=NAV_LEFT_B")
TOL_DEG = 0.5
COMPASS = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"]


def vec(s):
    return [float(v) for v in str(s).strip("()").split(",")]


def quat_heading(q):
    """Heading (deg, 0..360) of the rover's forward axis from an attitude quaternion (s, v1, v2, v3), site frame
    x north, y east, z down (ZYX yaw)."""
    w, x, y, z = q
    return math.degrees(math.atan2(2 * (w * z + x * y), 1 - 2 * (y * y + z * z))) % 360


def ang(a, b):
    """Smallest difference between two angles (deg)."""
    return abs((a - b + 180) % 360 - 180)


def compass(az):
    return COMPASS[int((az % 360) / 22.5 + 0.5) % 16]


def view(pointing, places_yaw):
    """Compass view of one photo, or (None, reason) when a check fails."""
    mast_az, mast_el = float(pointing["mast_az_deg"]), float(pointing["mast_el_deg"])
    qh = quat_heading(pointing["rover_attitude"])
    cv = pointing["camera_vector"]
    cv_az = math.degrees(math.atan2(cv[1], cv[0])) % 360
    d_head, d_cam = ang(qh, places_yaw), ang(cv_az, mast_az)
    if d_head > TOL_DEG:
        return None, f"attitude heading {qh:.2f} vs PLACES yaw {places_yaw:.2f}"
    if d_cam > TOL_DEG:
        return None, f"camera_vector azimuth {cv_az:.2f} vs mast azimuth {mast_az:.2f}"
    az = (places_yaw + mast_az) % 360
    return dict(az_deg=round(az, 1), el_deg=round(mast_el, 1), compass=compass(az),
                check=dict(heading_diff_deg=round(d_head, 3), camera_vs_mast_deg=round(d_cam, 3), tol_deg=TOL_DEG),
                rule="azimuth = PLACES heading + mast azimuth (raw-image record); elevation = mast elevation"), None


def fetch():
    """Add the raw record's pointing fields to each stop photo in stops_source.json (no image download)."""
    src = json.load(open(config.STOPS_SOURCE, encoding="utf-8"))
    for s in src["stops"]:
        p = s.get("photo")
        if not p:
            continue
        items = json.load(urllib.request.urlopen(API.format(sol=p["sol"]), timeout=60))["items"]
        it = next((i for i in items if i.get("imageid") == p["imageid"]), None)
        if it is None:
            print(f"  {s['id']}: {p['imageid']} not in the feed; no pointing")
            continue
        e = it.get("extended") or {}
        p["pointing"] = dict(mast_az_deg=float(e["mast_az"]), mast_el_deg=float(e["mast_el"]), camera_vector=vec(it["camera_vector"]),
                             rover_attitude=vec(it["attitude"]), lmst=e.get("lmst"),
                             source="raw-image record (extended.mast_az / mast_el, camera_vector, attitude)")
        print(f"  {s['id']}: mast az {e['mast_az']} el {e['mast_el']}")
    json.dump(src, open(config.STOPS_SOURCE, "w", encoding="utf-8"), indent=1, ensure_ascii=False)


def augment(out):
    """Write photo.view (or photo.view_note when a check fails) into out/stops.json from stops_source pointing and the
    PLACES heading in out/rover_check.json. Returns True when stops.json changed."""
    sp, rp = os.path.join(out, "stops.json"), os.path.join(out, "rover_check.json")
    if not (os.path.exists(sp) and os.path.exists(rp) and os.path.exists(config.STOPS_SOURCE)):
        return False
    src = {s["id"]: s for s in json.load(open(config.STOPS_SOURCE, encoding="utf-8"))["stops"]}
    yaw = {(r["sol"], str(r["site"]), str(r["drive"])): r.get("yaw_deg") for r in json.load(open(rp, encoding="utf-8"))["stops"]}
    d = json.load(open(sp, encoding="utf-8"))
    n = 0
    for s in d["stops"]:
        p, pt = s.get("photo"), ((src.get(s["id"]) or {}).get("photo") or {}).get("pointing")
        if not p or not pt:
            continue
        y = yaw.get((s["sol"], str(s["site"]), str(s["drive"])))
        v, why = (None, "no PLACES heading for this stop") if y is None else view(pt, y)
        p.pop("view", None)
        p.pop("view_note", None)
        if v:
            p["view"] = v
            n += 1
        else:
            p["view_note"] = why
    json.dump(d, open(sp, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"  stops.json: photo view direction on {n} of {sum(1 for s in d['stops'] if s.get('photo'))} photos")
    return True


def selftest():
    # identity attitude: heading 0; a 90 deg turn about z (down axis): heading east
    assert quat_heading([1, 0, 0, 0]) == 0
    c = math.cos(math.radians(45))
    assert abs(quat_heading([c, 0, 0, c]) - 90) < 1e-9
    assert ang(359, 1) == 2 and compass(0) == "N" and compass(237.7) == "WSW" and compass(348.8) == "N"
    # rover facing east, mast turned 90 deg right: looking south; camera vector consistent
    pt = dict(mast_az_deg=90.0, mast_el_deg=-20.0, rover_attitude=[c, 0, 0, c], camera_vector=[0.0, 0.94, 0.34])
    v, why = view(pt, 90.0)
    assert why is None and v["az_deg"] == 180.0 and v["compass"] == "S" and v["el_deg"] == -20.0, (v, why)
    # PLACES heading that disagrees with the attitude: no view
    assert view(pt, 120.0)[0] is None
    # camera vector that disagrees with the mast azimuth: no view
    assert view(dict(pt, camera_vector=[1.0, 0.0, 0.0]), 90.0)[0] is None
    # real record (Rio Frio, sol 4634, NLB_808882742EDR_F1181608NCAM00353M_), PLACES yaw -99.242
    real = dict(mast_az_deg=336.986, mast_el_deg=-52.7416, camera_vector=[0.5570892142427597, -0.2361854158765902, 0.7961583113310989],
                rover_attitude=[0.635202, 0.140105, 0.0599985, -0.757158])
    v, why = view(real, -99.242)
    assert why is None and v["az_deg"] == 237.7 and v["compass"] == "WSW", (v, why)
    print("photo_view selftest: PASS")


if __name__ == "__main__":
    {"fetch": fetch, "selftest": selftest}.get(sys.argv[1] if len(sys.argv) > 1 else "selftest", selftest)()
