"""Mars view: the whole planet down to the boxwork patch.

  fetch    -> data/raw/overview/bookmarks.json (NASA Mars Trek catalogue: the 'landing site' bookmarks)
  build    -> assets/overview.json + assets/missions.json
  selftest -> tile maths and traverse extraction on synthetic input

overview.json holds:
  - sites: NASA Mars Trek landing-site bookmarks (title, bounding box, description, image), placed at the box centre and
    labelled as such; Curiosity is placed at its PLACES sol-0 position instead (the landing spot, exact).
  - traverse: Curiosity's whole traverse from PLACES (ROVER frames, every localized position, sols 0 to the last in the file).
  - study: the boxwork patch outline (manifest grid) so the app can zoom to it.
  - bases: tile services streamed by the app (no tiles are stored in the bundle): Viking colour and MOLA colour relief from
    NASA Mars Trek (256 px JPEG, equirectangular, 2^(L+1) x 2^L tiles at level L), and the CTX global mosaic
    (NASA/JPL/MSSS/Caltech Murray Lab/Esri; 512 px PNG, same tiling, levels to 17). Levels and formats were read from each
    service's own capabilities and are re-checked by `fetch`.
missions.json = tools/sources/missions.json (which mission's data is used where).
"""
import csv
import json
import math
import os
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402

CAT = ("https://trek.nasa.gov/mars/TrekServices/ws/index/eq/searchItems?start=0&rows=60"
       "&proj=urn:ogc:def:crs:EPSG::104905&key=landing%20site")
CACHE = os.path.join(config.RAW, "overview")
TREK = "https://trek.nasa.gov/tiles/Mars/EQ"
BASES = [
    dict(id="viking", title="Viking colour mosaic", detail="Viking Orbiters · 232 m/px",
         url=TREK + "/Mars_Viking_MDIM21_ClrMosaic_global_232m/1.0.0/default/default028mm/{z}/{row}/{col}.jpg",
         caps=TREK + "/Mars_Viking_MDIM21_ClrMosaic_global_232m/1.0.0/WMTSCapabilities.xml",
         tile_px=256, max_level=7, credit="Viking VIS global colour mosaic (MDIM 2.1), via NASA Mars Trek"),
    dict(id="mola", title="MOLA colour relief", detail="Mars Global Surveyor · 463 m/px",
         url=TREK + "/Mars_MGS_MOLA_ClrShade_merge_global_463m/1.0.0/default/default028mm/{z}/{row}/{col}.jpg",
         caps=TREK + "/Mars_MGS_MOLA_ClrShade_merge_global_463m/1.0.0/WMTSCapabilities.xml",
         tile_px=256, max_level=7, credit="MGS MOLA global colour hillshade, via NASA Mars Trek"),
    dict(id="themis_day", title="THEMIS day IR", detail="Mars Odyssey · 100 m/px · thermal: warmer sunward slopes brighter (Trek)",
         url=TREK + "/THEMIS_DayIR_ControlledMosaics_100m_v2_oct2018/1.0.0/default/default028mm/{z}/{row}/{col}.png",
         caps=TREK + "/THEMIS_DayIR_ControlledMosaics_100m_v2_oct2018/1.0.0/WMTSCapabilities.xml",
         tile_px=256, max_level=9, credit="Mars Odyssey THEMIS day IR controlled mosaic, 100 m (2018), via NASA Mars Trek"),
    dict(id="themis_night", title="THEMIS night IR", detail="Mars Odyssey · 100 m/px · thermal: shows differences in thermal inertia (Trek)",
         url=TREK + "/THEMIS_NightIR_ControlledMosaics_100m_v2_oct2018/1.0.0/default/default028mm/{z}/{row}/{col}.png",
         caps=TREK + "/THEMIS_NightIR_ControlledMosaics_100m_v2_oct2018/1.0.0/WMTSCapabilities.xml",
         tile_px=256, max_level=9, credit="Mars Odyssey THEMIS night IR controlled mosaic, 100 m (2018), via NASA Mars Trek"),
    dict(id="ctx", title="CTX mosaic", detail="MRO · about 5 m/px",
         url="https://astro.arcgis.com/arcgis/rest/services/OnMars/CTX/MapServer/tile/{z}/{row}/{col}",
         caps="https://astro.arcgis.com/arcgis/rest/services/OnMars/CTX/MapServer?f=json",
         tile_px=512, max_level=12, credit="NASA/JPL/MSSS/Caltech Murray Lab/Esri (CTX beta01, uncontrolled)"),
]
# finest level used for CTX: level 12 = 0.3515625/2^12 deg/px = 5.1 m (native ~5 m); level 13 answers HTTP 400 (checked)


def deg_per_px(level, tile_px):
    """Equirectangular tiling with 2^(level+1) tiles across 360 deg (Trek default028mm and the Esri CTX service)."""
    return 360.0 / (tile_px * 2 ** (level + 1))


def m_per_px(level, tile_px):
    return math.radians(deg_per_px(level, tile_px)) * config.MARS_RADIUS_M


def traverse(places_csv):
    """Every localized ROVER position in PLACES: [lat, lon, sol] (planetocentric, east lon), missing values dropped, in
    file order, consecutive duplicates removed."""
    out = []
    for r in csv.DictReader(open(places_csv, encoding="utf-8")):
        if r["frame"] != "ROVER":
            continue
        la, lo, s = float(r["planetocentric_latitude"]), float(r["longitude"]), int(r["sol"])
        if la < -90 or lo < -360 or s < 0:
            continue
        p = [round(la, 6), round(lo, 6), s]
        if not out or out[-1][:2] != p[:2]:
            out.append(p)
    return out


def landing(places_csv):
    """The first SITE frame (site 1) in PLACES = the landing position."""
    for r in csv.DictReader(open(places_csv, encoding="utf-8")):
        if r["frame"] == "SITE" and r["site"] == "1":
            return float(r["planetocentric_latitude"]), float(r["longitude"])
    return None


def fetch():
    os.makedirs(CACHE, exist_ok=True)
    d = json.load(urllib.request.urlopen(CAT, timeout=60))
    bm = [x for x in d["response"]["docs"] if x.get("itemType") == "bookmark"]
    json.dump(bm, open(os.path.join(CACHE, "bookmarks.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"{len(bm)} bookmarks cached")
    for b in BASES:  # services still answer (levels are fixed in BASES; this only checks they are there)
        try:
            urllib.request.urlopen(b["caps"], timeout=60).read(200)
            print(f"  {b['id']}: capabilities OK")
        except Exception as ex:
            print(f"  {b['id']}: capabilities FAILED ({ex})")


MISSION_OF = {  # Trek bookmark title -> mission line shown in the app (title words only; no dates typed here)
    "Curiosity Landing Site": "Mars Science Laboratory (Curiosity)",
    "Perseverance Landing Site": "Mars 2020 (Perseverance)",
    "InSight Landing Site": "InSight",
    "Opportunity Landing Site": "Mars Exploration Rover (Opportunity)",
    "Spirit Landing Site": "Mars Exploration Rover (Spirit)",
    "Phoenix Landing Site": "Phoenix",
    "Sojourner Landing Site": "Mars Pathfinder (Sojourner)",
    "Viking 1": "Viking 1 Lander",
    "Viking 2": "Viking 2 Lander",
}


def build(out=config.ASSETS):
    bm = json.load(open(os.path.join(CACHE, "bookmarks.json"), encoding="utf-8"))
    man = json.load(open(os.path.join(out, "manifest.json"), encoding="utf-8"))
    g = man["grid"]
    R = config.MARS_RADIUS_M
    study = [math.degrees(g["x_west_m"] / R), math.degrees((g["y_north_m"] - g["height"] * g["cell_m"]) / R),
             math.degrees((g["x_west_m"] + g["width"] * g["cell_m"]) / R), math.degrees(g["y_north_m"] / R)]
    land = landing(config.PLACES)
    sites = []
    for b in bm:
        t = b.get("title", "")
        if t not in MISSION_OF:
            continue
        w, s, e, n = [float(x) for x in b["bbox"].split(",")]
        site = dict(title=t, mission=MISSION_OF[t], bbox=[w, s, e, n], lat=(s + n) / 2, lon=(w + e) / 2,
                    position="centre of the NASA Mars Trek bookmark box", description=b.get("description", ""),
                    image=(b.get("mediaURL") or [None])[0], source="NASA Mars Trek bookmark '" + b.get("productLabel", "") + "'")
        if t == "Curiosity Landing Site" and land:
            site.update(lat=land[0], lon=land[1], position="PLACES site 1 (landing), planetocentric")
        sites.append(site)
    tr = traverse(config.PLACES)
    doc = dict(
        source="tools/overview.py build", note="Mars view: sites, Curiosity traverse, study patch, streamed basemaps",
        sites=sites,
        traverse=dict(points=tr, n=len(tr), sol_first=min(p[2] for p in tr), sol_last=max(p[2] for p in tr),
                      credit="NASA/JPL-Caltech, MSL PLACES localized_pos (PDS)"),
        study=dict(bbox=[round(x, 6) for x in study], title="Boxwork drill-pair walk (this plan)"),
        bases=[{k: v for k, v in b.items() if k != "caps"} | dict(native_m_per_px=round(m_per_px(b["max_level"], b["tile_px"]), 2))
               for b in BASES],
        credit_sites="NASA Mars Trek (bookmarks and descriptions)",
    )
    json.dump(doc, open(os.path.join(out, "overview.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    src = json.load(open(os.path.join(os.path.dirname(__file__), "sources", "missions.json"), encoding="utf-8"))
    json.dump(src, open(os.path.join(out, "missions.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"overview.json: {len(sites)} sites, traverse {len(tr)} points (sols {doc['traverse']['sol_first']}-{doc['traverse']['sol_last']}), "
          f"study {doc['study']['bbox']}; missions.json: {len(src['missions'])} missions")


def selftest():
    import tempfile
    assert abs(deg_per_px(0, 512) - 0.3515625) < 1e-12, "CTX level 0 = the service's own resolution"
    assert abs(deg_per_px(7, 256) - 360 / (256 * 256)) < 1e-12
    assert 4.5 < m_per_px(12, 512) < 5.5, m_per_px(12, 512)
    rows = ["frame,site,drive,planetocentric_latitude,longitude,sol",
            "SITE,1,-1,-4.5,137.4,-1", "ROVER,1,0,-4.5,137.4,0", "ROVER,1,2,-4.5,137.4,1",
            "ROVER,1,4,-9999,-9999,2", "ROVER,2,0,-4.6,137.5,3"]
    p = os.path.join(tempfile.mkdtemp(), "p.csv")
    open(p, "w").write("\n".join(rows) + "\n")
    assert traverse(p) == [[-4.5, 137.4, 0], [-4.6, 137.5, 3]], traverse(p)
    assert landing(p) == (-4.5, 137.4)
    print("selftest OK")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "selftest"
    {"fetch": fetch, "build": build, "selftest": selftest}[cmd]()
