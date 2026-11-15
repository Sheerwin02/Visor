"""Build the VISOR asset bundle: the ONLY data the client loads (see the doc's Architecture section).

Inputs (local, git-ignored): data/raw/MSL_Gale_DEM_Mosaic_1m_v3.tif (via `prep dump`), data/processed/ortho/ortho_patch.tif (tools/ortho_crop.py),
data/raw/localized_pos.csv (PLACES), data/interim/rover_check/rover_check_stops.csv and the gate/rover-check result text files.
Output: assets/ (small, meant for the repo):
  manifest.json            grid definition, file list with sha256, sources and credits, measured limits, assumptions
  heightmap.bin            float32 little-endian, row-major, row 0 = NORTH edge, 1 m cells, heights minus z_offset_m (manifest)
  slope_3m.bin             float32 degrees, same grid; DEM smoothed with a 3 x 3 m box filter before the gradient (the gate showed
                           1 m slope is mostly noise); edge cells use one-sided differences
  slope_bands_3m.png       8-bit index on the same grid: 0 = under 15 deg, 1 = 15 to 25, 2 = 25 or more (the app styles them; no colour meaning baked in)
  ortho_0p5m.jpg           the 25 cm photo resampled to 0.5 m (2x2 mean) over exactly the grid extent (fits a 4096 texture)
  ortho_0p25m_n.jpg / _s.jpg  full 25 cm photo split into a north and a south half (each under 4096 px tall)
  rover_path.json          PLACES ROVER positions for sols 4534..4865 in grid coordinates
  rover_check.json         per-stop rover-check labels and values + the run summary
Local coordinates: e_m = metres east of the grid's west edge, n_m = metres north of the grid's south edge; heightmap cell (row, col)
covers e in [col, col+1), n in [H-row-1, H-row). Absolute elevation = stored value + z_offset_m (metres, relative to the MOLA areoid, per PLACES SIS).

  route_ensemble.json / route_usage.bin  route uncertainty ensemble (crates/route-core/examples/ensemble.rs, run AFTER build;
                           build deletes stale copies because they depend on route.json); registered by `rehash`

Usage: python tools/build_bundle.py build [--out assets]      python tools/build_bundle.py selftest
       python tools/build_bundle.py rehash [--out assets]     (add the ensemble files to the manifest after running the ensemble)
"""
import csv
import datetime
import hashlib
import json
import math
import os
import re
import struct
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image
from scipy.ndimage import uniform_filter

import config  # single source of truth for paths and study constants
from config import MARS_RADIUS_M as R, PREP, DEM, ORTHO, PLACES, SOL_FIRST as SOL0, SOL_LAST as SOL1
JPEG_Q = 88


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def slope_deg(z, cell, box_m=3):
    zs = uniform_filter(z, size=int(round(box_m / cell)), mode="nearest")
    gy, gx = np.gradient(zs, cell)
    return np.degrees(np.arctan(np.hypot(gx, gy))).astype(np.float32)


def write_bundle(out, z, cell, grid_x_west, grid_y_north, ortho_rgb, rover_rows, check_rows, meta, stops_src=None):
    """Pure core: z (H, W) float heights (absolute, metres); ortho_rgb (4H, 4W, 3) uint8 at cell/4; rover_rows/check_rows: dicts
    with lat/lon. Writes the bundle into `out` and returns the manifest dict."""
    os.makedirs(out, exist_ok=True)
    H, W = z.shape
    assert ortho_rgb.shape[:2] == (4 * H, 4 * W), (ortho_rgb.shape, z.shape)
    if not np.isfinite(z).all():
        raise ValueError("heightmap has nodata/NaN cells: refusing to write (fill rule not defined)")
    z_off = float(math.floor(z.min()))
    files = {}

    def add(name, kind, **desc):
        files[name] = dict(kind=kind, **desc)

    (z - z_off).astype("<f4").tofile(os.path.join(out, "heightmap.bin"))
    add("heightmap.bin", "float32 LE row-major, row 0 north", width=W, height=H, cell_m=cell, units="m above z_offset_m")
    s = slope_deg(z, cell)
    s_grid = s
    s.astype("<f4").tofile(os.path.join(out, "slope_3m.bin"))
    add("slope_3m.bin", "float32 LE row-major, row 0 north", width=W, height=H, units="deg", method="3 x 3 m box filter, then gradient")
    bands = np.zeros_like(s, dtype=np.uint8); bands[s >= 15] = 1; bands[s >= 25] = 2
    Image.fromarray(bands, mode="L").save(os.path.join(out, "slope_bands_3m.png"))
    add("slope_bands_3m.png", "8-bit index PNG", width=W, height=H, values={"0": "<15 deg", "1": "15-25 deg", "2": ">=25 deg"})
    half = ortho_rgb.reshape(2 * H, 2, 2 * W, 2, 3).mean(axis=(1, 3)).round().astype(np.uint8)
    Image.fromarray(half, mode="RGB").save(os.path.join(out, "ortho_0p5m.jpg"), quality=JPEG_Q)
    add("ortho_0p5m.jpg", "JPEG RGB", width=2 * W, height=2 * H, pixel_m=cell / 2, extent="exactly the heightmap grid")
    mid = 2 * H  # split the 0.25 m photo at the middle row of the grid
    Image.fromarray(ortho_rgb[:mid], mode="RGB").save(os.path.join(out, "ortho_0p25m_n.jpg"), quality=JPEG_Q)
    Image.fromarray(ortho_rgb[mid:], mode="RGB").save(os.path.join(out, "ortho_0p25m_s.jpg"), quality=JPEG_Q)
    add("ortho_0p25m_n.jpg", "JPEG RGB", width=4 * W, height=mid, pixel_m=cell / 4, extent=f"north half: grid rows 0..{H // 2}")
    add("ortho_0p25m_s.jpg", "JPEG RGB", width=4 * W, height=4 * H - mid, pixel_m=cell / 4, extent=f"south half: grid rows {H // 2}..{H}")

    y_south = grid_y_north - H * cell

    def local(lat, lon):
        x, y = R * math.radians(lon), R * math.radians(lat)
        e, n = x - grid_x_west, y - y_south
        col, row = int(e // cell), int((grid_y_north - y) // cell)
        elev = float(z[row, col]) if 0 <= row < H and 0 <= col < W else None
        return round(e, 3), round(n, 3), elev

    path = []
    for r in rover_rows:
        e, n, elev = local(r["lat"], r["lon"])
        path.append(dict(sol=r["sol"], site=r["site"], drive=r["drive"], lat=r["lat"], lon=r["lon"], e_m=e, n_m=n,
                         dem_elev_m=elev, places_elev_m=r.get("elev")))
    json.dump(dict(description="Curiosity ROVER positions (PLACES localized_pos), sols %d..%d" % (SOL0, SOL1), points=path),
              open(os.path.join(out, "rover_path.json"), "w"), indent=1)
    add("rover_path.json", "JSON", count=len(path))
    stops = []
    for c in check_rows:
        e, n, _ = local(c["lat"], c["lon"])
        stops.append(dict(c, e_m=e, n_m=n))
    json.dump(dict(description="Rover-check per stop: DEM plane tilt under a 3.0 x 2.8 m footprint vs rover total tilt",
                   labels={"agrees": "|diff| <= 2 deg", "partial": "2 < |diff| <= 3 deg", "disagrees": "|diff| > 3 deg",
                           "not checked": "everywhere off these stops"},
                   summary=meta.get("rover_check_summary"), stops=stops),
              open(os.path.join(out, "rover_check.json"), "w"), indent=1)
    add("rover_check.json", "JSON", count=len(stops))
    if stops_src is not None:
        by_key = {(r["site"], r["drive"]): r for r in rover_rows}
        chk = {(c["site"], c["drive"]): c for c in check_rows}
        out_stops, prev = [], None
        for s in stops_src["stops"]:
            r = by_key.get((s["site"], s["drive"]))
            if r is None:
                raise ValueError(f"stop {s['id']}: site {s['site']} drive {s['drive']} not in PLACES rover rows")
            e, n, elev = local(r["lat"], r["lon"])
            col, row = int(e // cell), int((H - n) // cell)
            c = chk.get((s["site"], s["drive"]))
            leg = None if prev is None else round(math.hypot(e - prev[0], n - prev[1]), 1)
            out_stops.append(dict(id=s["id"], name=s["name"], kind=s["kind"], sol=s["sol"], site=s["site"], drive=s["drive"],
                                  lat=r["lat"], lon=r["lon"], e_m=e, n_m=n, dem_elev_m=elev,
                                  slope_3m_deg=round(float(s_grid[row, col]), 1),
                                  rover_check=None if c is None else dict(label=c["label"], rover_tilt=c["rover_tilt"], dem_tilt=c["dem_tilt"]),
                                  straight_line_from_previous_m=leg, position_note=s["position_note"], facts=s["facts"], tasks=s.get("tasks", []), photo=s["photo"]))
            prev = (e, n)
        import route as rt
        bias = meta.get("bias_deg") if meta.get("bias_deg") is not None else rt.measured_bias_deg()
        plan = rt.plan_loop(s_grid, [(p["e_m"], p["n_m"]) for p in path], out_stops, cell, bias)
        field = plan.pop("walk_back_field")
        NO_WAY = 65535
        fin = np.isfinite(field)
        assert fin.any() and field[fin].max() < NO_WAY, "walk-back seconds must fit in u16"
        np.where(fin, np.round(field), NO_WAY).astype("<u2").tofile(os.path.join(out, "walk_back_s.bin"))
        add("walk_back_s.bin", "uint16 LE row-major, row 0 north", width=W, height=H, units="s", no_way_back=NO_WAY,
            method="Dijkstra from the exit stop on the route's cost graph (same speed, bias, slope limit and rover-verified rule)",
            reachable_cells=int(fin.sum()))
        for x in out_stops:
            x["walk_back_s"] = plan["walk_back_s"][x["id"]]
        json.dump(dict(description=(f"Loop route, rule option d (see the doc): cost = walking time with speed {rt.SPEED_FLAT} m/s x "
                                    f"exp(-{rt.TOBLER_K} tan(slope_3m - {bias} deg)); cells over {rt.LIMIT_DEG} deg corrected slope blocked except "
                                    f"within {rt.VERIFIED_R_M} m of a localized rover position"),
                       assumptions=dict(speed_flat_mps=rt.SPEED_FLAT, tobler_k=rt.TOBLER_K, caution_deg=rt.CAUTION_DEG,
                                        speed_options_mps=list(rt.SPEED_OPTIONS_MPS), limit_range_deg=list(rt.LIMIT_RANGE_DEG),
                                        speed_source_range_mps=list(rt.SPEED_SOURCE_RANGE_MPS), slope_limit_deg=rt.LIMIT_DEG, bias_deg=bias, verified_radius_m=rt.VERIFIED_R_M,
                                        cost_shape="Tobler (1993) hiking-function exponent, Earth walking, relative only"),
                       **plan), open(os.path.join(out, "route.json"), "w"), indent=1)
        add("route.json", "JSON", legs=len(plan["legs"]), all_legs_possible=plan["all_legs_possible"])
        loop = sum(x["straight_line_from_previous_m"] or 0 for x in out_stops)
        loop += math.hypot(out_stops[-1]["e_m"] - out_stops[0]["e_m"], out_stops[-1]["n_m"] - out_stops[0]["n_m"])
        json.dump(dict(description=stops_src["description"], position_rule=stops_src["position_rule"],
                       straight_line_loop_m=round(loop, 1), loop_note="straight lines between stops and back to the first; the walked route is longer",
                       stops=out_stops, context_facts=stops_src.get("context_facts", [])),
                  open(os.path.join(out, "stops.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
        add("stops.json", "JSON", count=len(out_stops))
        for x in out_stops:
            if x["photo"] and os.path.exists(os.path.join(out, x["photo"]["file"])):
                add(x["photo"]["file"], "JPEG (Curiosity raw image)", stop=x["id"], credit=x["photo"]["credit"], source=x["photo"]["url"])
    for name, kind, desc in meta.get("extra_files", []):
        if os.path.exists(os.path.join(out, name)):
            add(name, kind, **desc)
    for name in files:
        p = os.path.join(out, name)
        files[name].update(bytes=os.path.getsize(p), sha256=sha256(p))
    manifest = dict(
        bundle="VISOR asset bundle", version=1, built_utc=meta.get("built_utc"),
        grid=dict(projection="Equirectangular, Mars 2000 sphere radius 3396190 m, central meridian 0, standard parallel 0 (same as the DEM)",
                  x_west_m=grid_x_west, y_north_m=grid_y_north, cell_m=cell, width=W, height=H, z_offset_m=z_off,
                  local_axes="e_m east of west edge, n_m north of south edge; heightmap row 0 is the north edge",
                  lat_range=meta.get("lat_range"), lon_range=meta.get("lon_range")),
        files=files, sources=meta.get("sources"), measured_limits=meta.get("measured_limits"), measured_values=meta.get("measured_values"),
        assumptions=meta.get("assumptions"), unverified=meta.get("unverified"), photo_light=meta.get("photo_light"))
    json.dump(manifest, open(os.path.join(out, "manifest.json"), "w"), indent=1)
    return manifest


def dem_window(x_west, y_north, W, H):
    """Read exactly W x H DEM cells whose NW corner is (x_west, y_north) via `prep dump` (box chosen inside the cells)."""
    lat = lambda y: math.degrees(y / R); lon = lambda x: math.degrees(x / R)
    with tempfile.TemporaryDirectory() as td:
        f = os.path.join(td, "w.f32")
        out = subprocess.run([PREP, "dump", DEM, f"{lat(y_north - 0.5):.10f}", f"{lon(x_west + 0.5):.10f}",
                              f"{lat(y_north - H + 0.5):.10f}", f"{lon(x_west + W - 0.5):.10f}", f], capture_output=True, text=True, check=True).stdout
        r0, r1, c0, c1 = map(int, re.search(r"rows (\d+)\.\.(\d+) cols (\d+)\.\.(\d+)", out).groups())
        b = open(f, "rb").read()
    w, h = struct.unpack("<II", b[:8]); cell = struct.unpack("<f", b[8:12])[0]
    z = np.frombuffer(b[12:], "<f4").reshape(h, w).astype(float)
    nd = np.isclose(z, -32767.0) | ~np.isfinite(z)
    z[nd] = np.nan
    return z, r0, c0, cell


ENSEMBLE_FILES = [
    ("route_ensemble.json", "JSON (crates/route-core/examples/ensemble.rs)", dict(note="route uncertainty ensemble per the doc's Route uncertainty rule")),
    ("route_usage.bin", "uint8 row-major, row 0 north: round(255 x share of ensemble runs whose route crosses the cell)",
     dict(note="the ensemble the app shows; run count and correlation length in route_ensemble.json")),
]

# optional files written by other tools after the bundle (kept across `build`; added to the manifest by `rehash`)
OPTIONAL_FILES = [
    ("conditions.json", "JSON (tools/conditions.py)",
     dict(note="daily REMS weather for the study sols from the NASA/CAB outreach feed; disclaimer and Ls check inside")),
    ("readiness.bin", "uint8 row-major, row 0 north, one byte per grid cell (tools/provenance.py run2)",
     dict(note="data readiness part B v2: 0 source unknown, 1 agrees, 2 partial, 3 disagrees (3 m slope vs the UA HiRISE DTM), "
               "4 rover-checked; rule in the doc, approved", credit="NASA/JPL/University of Arizona (HiRISE DTM DTEEC_009149_1750_009294_1750_U01)")),
    ("readiness.json", "JSON (tools/provenance.py run2)", dict(note="verdict, numbers and legend of readiness.bin")),
    ("astronaut.bin", "LE format 2: one skinned mesh, positions f32x3, normals f32x3, colours u8x4 sRGB, joints u16x4, weights f32x4, indices u32 (tools/astronaut.py build)",
     dict(credit="NASA 3D Resources: Astronaut (Astronaut.glb); no NASA endorsement implied",
          source="https://github.com/nasa/NASA-3D-Resources/tree/master/3D%20Models/Astronaut")),
    ("astronaut.json", "JSON (tools/astronaut.py build)", dict(note="offsets, joint pivots, scale (ASSUMPTION 1.85 m, display only), credit")),
    ("curiosity.bin", "LE: positions f32x3, normals f32x3, colours u8x4 sRGB, indices u32 (tools/curiosity.py build)",
     dict(credit="NASA/JPL-Caltech: NASA Science 3D resources, Curiosity Rover 3D Model; no NASA endorsement implied",
          source="https://science.nasa.gov/resource/curiosity-rover-3d-model/")),
    ("curiosity.json", "JSON (tools/curiosity.py build)", dict(note="offsets, frame (faces +Z, Y up, metres), extent, credit")),
    ("eva_ops.json", "JSON (copied from tools/sources/eva_ops.json)",
     dict(note="EVA length, egress/ingress, emergency hour, 60 min rescue rule, roles; every value quoted with page and URL (NASA ConOps, Moon)")),
    ("ridges.json", "JSON (tools/ridges.py run)", dict(note="ridge tracing result summary for the readiness table; no map layer")),
    ("chemistry.json", "JSON (tools/chem.py build)",
     dict(note="Curiosity ChemCam MOC + APXS oxide results for the study sols, placed at the rover's PLACES position (site, drive)",
          credit="ChemCam: NASA/JPL-Caltech/LANL/CNES; APXS: NASA/JPL-Caltech/University of Guelph; PDS Geosciences Node")),
    ("sun_check.json", "JSON (tools/trek_sun.py run)",
     dict(note="VISOR's sun model (Mars24) vs NASA Mars Trek's SPICE sun-angle service on the six stop sols; rule written before the run",
          credit="NASA Mars Trek sun-angle service")),
    ("overview.json", "JSON (tools/overview.py build)",
     dict(note="Mars view: NASA Mars Trek landing-site bookmarks, Curiosity's whole PLACES traverse, the study patch, streamed basemap services",
          credit="NASA Mars Trek; NASA/JPL-Caltech MSL PLACES")),
    ("missions.json", "JSON (copied from tools/sources/missions.json)", dict(note="which NASA mission's data is used where in VISOR")),
    ("radiation.json", "JSON (tools/rad.py)",
     dict(note="RAD dose rate per stop sol (PDS RDR) and DONKI events naming Mars, per the doc's RAD rule; context only")),
]


HELDOUT_KEYS = ("outcome", "n", "pooled_heldout_median_abs_uncorrected", "pooled_heldout_median_abs_corrected",
                "full_sample_median_deg", "full_sample_ci95_block8", "run_utc")


def heldout_values():
    """The held-out bias check as numbers for measured_values (None when it has not been run)."""
    r = config.heldout_result()
    return {k: r[k] for k in HELDOUT_KEYS} if r else None


def copy_eva_ops(out):
    """The sourced EVA operations values (tools/sources/eva_ops.json) go into the bundle unchanged."""
    src = os.path.join(os.path.dirname(__file__), "sources", "eva_ops.json")
    if os.path.exists(src):
        json.dump(json.load(open(src, encoding="utf-8")), open(os.path.join(out, "eva_ops.json"), "w", encoding="utf-8"),
                  indent=1, ensure_ascii=False)


def augment_rover_check(out):
    """Add each rover-check stop's heading (PLACES yaw, deg clockwise from north) and the footprint used by the rover check
    (tools/rover_check.py FOOT, along x across, m), so Walk mode can draw Curiosity's footprint where and how it parked."""
    import rover_check as rc
    p = os.path.join(out, "rover_check.json")
    if not (os.path.exists(p) and os.path.exists(config.PLACES)):
        return
    d = json.load(open(p, encoding="utf-8"))
    pose = {(r["sol"], r["site"], r["drive"]): r for r in rc.load(config.PLACES)}
    n = 0
    for s in d.get("stops", []):
        r = pose.get((s["sol"], s["site"], s["drive"]))
        if r is not None:
            # PLACES: yaw = heading (N 0, E 90), pitch + = nose up, roll + = left side up (PLACES SIS)
            s["yaw_deg"], s["pitch_deg"], s["roll_deg"] = (round(float(r[k]), 3) for k in ("yaw", "pitch", "roll"))
            n += 1
    d["footprint_m"] = dict(along=rc.FOOT[0], across=rc.FOOT[1], note="rover-check footprint (tools/rover_check.py FOOT), centred on the PLACES position")
    json.dump(d, open(p, "w", encoding="utf-8"), indent=1)
    print(f"  rover_check.json: heading, pitch and roll added to {n} of {len(d.get('stops', []))} stops")


def copy_tasks(out):
    """Station plans (tasks) from tools/sources/stops_source.json into stops.json by stop id (no DEM rebuild needed)."""
    sp = os.path.join(out, "stops.json")
    if not (os.path.exists(sp) and os.path.exists(config.STOPS_SOURCE)):
        return False
    src = {s["id"]: s for s in json.load(open(config.STOPS_SOURCE, encoding="utf-8"))["stops"]}
    d = json.load(open(sp, encoding="utf-8"))
    for s in d["stops"]:
        s["tasks"] = src.get(s["id"], {}).get("tasks", [])
        if src.get(s["id"], {}).get("pair"):
            s["pair"] = src[s["id"]]["pair"]
        else:
            s.pop("pair", None)
    srcdoc = json.load(open(config.STOPS_SOURCE, encoding="utf-8"))
    d["tasks_note"] = srcdoc.get("tasks_note", "")
    d["bags_note"] = srcdoc.get("bags_note", "")
    json.dump(d, open(sp, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"  stops.json: {sum(len(s['tasks']) for s in d['stops'])} station tasks")
    return True


def rehash(out):
    """Add (or refresh) the ensemble outputs and optional files in manifest.json, with size and sha256, and the held-out
    bias result in measured_values. Nothing else changes."""
    mp = os.path.join(out, "manifest.json")
    m = json.load(open(mp))
    copy_eva_ops(out)
    augment_rover_check(out)
    import photo_view
    tasks_done = copy_tasks(out)
    if (photo_view.augment(out) or tasks_done) and "stops.json" in m["files"]:  # changed in place: refresh its size and hash
        sp = os.path.join(out, "stops.json")
        m["files"]["stops.json"].update(bytes=os.path.getsize(sp), sha256=sha256(sp))
    if "rover_check.json" in m["files"]:                         # changed in place above: refresh its size and hash
        rp = os.path.join(out, "rover_check.json")
        m["files"]["rover_check.json"].update(bytes=os.path.getsize(rp), sha256=sha256(rp))
    # NASA Mars Trek context layers (tools/trek.py): the json and every basemap image it lists
    tj = os.path.join(out, "trek_layers.json")
    if os.path.exists(tj):
        td = json.load(open(tj, encoding="utf-8"))
        m["files"]["trek_layers.json"] = dict(kind="JSON (tools/trek.py build)", note="NASA Mars Trek layers: resolution, coverage and alignment over the patch; view only",
                                              bytes=os.path.getsize(tj), sha256=sha256(tj))
        for L in td.get("layers", []):
            if L.get("file") and os.path.exists(os.path.join(out, L["file"])):
                fp = os.path.join(out, L["file"])
                m["files"][L["file"]] = dict(kind="JPEG RGB (tools/trek.py, composited from Mars Trek tiles)", credit="NASA Mars Trek; " + str(L.get("title")),
                                             width=L.get("width"), height=L.get("height"), bytes=os.path.getsize(fp), sha256=sha256(fp))
        print(f"  trek_layers.json + {sum(1 for L in td.get('layers', []) if L.get('file'))} basemaps added to the manifest")
    # Curiosity's Navcam 360-degree mosaics at the stops (tools/navcam_pano.py): the json and every panorama it lists
    pj = os.path.join(out, "panos.json")
    if os.path.exists(pj):
        pd = json.load(open(pj, encoding="utf-8"))
        m["files"]["panos.json"] = dict(kind="JSON (tools/navcam_pano.py build)", note="Navcam 360-degree mosaic per stop: product, projection, stretch",
                                        bytes=os.path.getsize(pj), sha256=sha256(pj))
        for P in pd.get("panos", []):
            fp = os.path.join(out, P["file"])
            if os.path.exists(fp):
                m["files"][P["file"]] = dict(kind="JPEG grey, cylindrical 360 deg, azimuth clockwise from north (tools/navcam_pano.py)",
                                             credit=pd["credit"], stop=P["stop"], width=P["width"], height=P["height"],
                                             bytes=os.path.getsize(fp), sha256=sha256(fp))
        print(f"  panos.json + {len(pd.get('panos', []))} panoramas added to the manifest")
    hv = heldout_values()
    if hv:
        m.setdefault("measured_values", {})["heldout_bias"] = hv
        print(f"  measured_values.heldout_bias: {hv['outcome']} (run {hv['run_utc']})")
    for name, kind, desc in ENSEMBLE_FILES + OPTIONAL_FILES:
        p = os.path.join(out, name)
        if os.path.exists(p):
            m["files"][name] = dict(kind=kind, **desc, bytes=os.path.getsize(p), sha256=sha256(p))
            print(f"  {name}  {os.path.getsize(p) / 1e6:.2f} MB  added to the manifest")
        else:
            m["files"].pop(name, None)
    json.dump(m, open(mp, "w"), indent=1)


def photo_light():
    """The light baked into the orthophoto (HiRISE label, tools/sources/photo_light.json): sun elevation = 90 - incidence;
    azimuth from north = SUB_SOLAR_AZIMUTH - NORTH_AZIMUTH (HiRISE data dictionary convention)."""
    src = json.load(open(os.path.join(os.path.dirname(__file__), "sources", "photo_light.json"), encoding="utf-8"))
    return dict(src, sun_elevation_deg=round(90.0 - src["incidence_angle"], 2),
                sun_azimuth_deg=round((src["sub_solar_azimuth"] - src["north_azimuth"]) % 360.0, 1))


def build(out):
    for name, _, _ in ENSEMBLE_FILES:  # they depend on route.json: never keep a stale ensemble
        if os.path.exists(os.path.join(out, name)):
            os.remove(os.path.join(out, name))
            print(f"removed stale {name}: re-run the ensemble, then `rehash`")
    gr, rcr = config.gate_result(), config.rover_check_result()
    oa = json.load(open(os.path.join(config.PROCESSED, 'ortho', 'alignment.json'), encoding='utf-8'))
    import rasterio
    sys.path.insert(0, os.path.dirname(__file__))
    import rover_check as rc
    info = subprocess.run([PREP, "info", DEM], capture_output=True, text=True, check=True).stdout
    tie = [float(v) for v in re.search(r"tiepoint Some\(\[([^\]]+)\]", info).group(1).split(",")]
    X0, Y0 = tie[3], tie[4]
    with rasterio.open(ORTHO) as d:
        img = d.read().transpose(1, 2, 0); T = d.transform
    ox0, oy1 = T.c, T.f
    ox1, oy0 = ox0 + img.shape[1] * 0.25, oy1 - img.shape[0] * 0.25
    c_start, c_end = math.ceil(ox0 - X0), math.floor(ox1 - X0)          # DEM cells fully inside the photo
    r_start, r_end = math.ceil(Y0 - oy1), math.floor(Y0 - oy0)
    W, H = c_end - c_start, r_end - r_start
    xw, yn = X0 + c_start, Y0 - r_start
    z, r0, c0, cell = dem_window(xw, yn, W, H)
    assert (r0, c0) == (r_start, c_start) and z.shape == (H, W), ((r0, c0), (r_start, c_start), z.shape, (H, W))
    # The DEM's cell edges sit at fractional metres while the photo's sit on whole metres, so snap to the nearest photo pixel
    # (residual <= 0.125 m, recorded in the manifest; small next to the ~2 m photo-DEM alignment limit).
    fx, fy = (xw - ox0) / 0.25, (oy1 - yn) / 0.25
    px, py = int(round(fx)), int(round(fy))
    snap_e_m, snap_s_m = round((fx - px) * 0.25, 4), round((fy - py) * 0.25, 4)
    ortho = np.ascontiguousarray(img[py:py + 4 * H, px:px + 4 * W])
    rover = [dict(r, elev=None) for r in rc.load(PLACES)]
    elev = {}
    with open(PLACES, newline="") as f:
        for r in csv.DictReader(f):
            if r["frame"] == "ROVER" and SOL0 <= int(r["sol"]) <= SOL1:
                elev[(r["site"], r["drive"])] = float(r["elevation"])
    for r in rover:
        r["elev"] = elev.get((r["site"], r["drive"]))
    checks = []
    with open(os.path.join(config.ROVER_CHECK_DIR, "rover_check_stops.csv"), newline="") as f:
        for c in csv.DictReader(f):
            checks.append(dict(sol=int(c["sol"]), site=c["site"], drive=c["drive"], lat=float(c["lat"]), lon=float(c["lon"]),
                               rover_tilt=float(c["rover_tilt"]), dem_tilt=float(c["dem_tilt"]) if c["dem_tilt"] else None,
                               abs_diff=float(c["abs_diff"]) if c["abs_diff"] else None, label=c["label"]))
    lat = lambda y: math.degrees(y / R); lon = lambda x: math.degrees(x / R)
    meta = dict(
        built_utc=datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        lat_range=[round(lat(yn - H), 6), round(lat(yn), 6)], lon_range=[round(lon(xw), 6), round(lon(xw + W), 6)],
        rover_check_summary=open(os.path.join(config.ROVER_CHECK_DIR, "rover_check_summary.txt"), encoding="utf-8").read().splitlines()[:9],
        bias_deg=rcr["signed_median_deg"],
        sources=[
            dict(name="MSL Gale Merged DEM 1 m v3", credit="Calef III, F. J., and Parker, T. (2016), PDS Annex, USGS",
                 url="https://planetarymaps.usgs.gov/mosaic/Mars/MSL/MSL_Gale_DEM_Mosaic_1m_v3.tif"),
            dict(name="MSL Gale HiRISE LRGB colour orthophoto mosaic (25 cm COG)", credit="USGS Astrogeology; HiRISE (NASA/JPL/University of Arizona)",
                 url="https://asc-pds-services.s3.us-west-2.amazonaws.com/mosaic/Mars/MSL/MSL_Gale_HiRISE-LRGB_78quads_sharp_cog.tif"),
            dict(name="MSL PLACES localized_pos", credit="NASA/JPL-Caltech, MSL PLACES (PDS)",
                 url="https://planetarydata.jpl.nasa.gov/img/data/msl/msl_places/data_localizations/localized_pos.csv")],
        photo_light=photo_light(),
        # the same results as numbers, for the app's panels (strings above are for people reading the manifest)
        measured_values=dict(
            gate={k: gr[k] for k in ("decision", "patch_share_pct", "reference_share_pct", "needed_pct", "run_utc")},
            rover_check={k: rcr[k] for k in ("outcome", "n", "median_abs_deg", "p90_abs_deg", "signed_median_deg", "below_chance",
                                             "shift_baseline_p5_deg", "per_stop", "run_utc")},
            **({"heldout_bias": heldout_values()} if heldout_values() else {})),
        measured_limits=dict(
            gate_1m=(f"{gr['decision']}: at native 1 m, {gr['patch_share_pct']}% of the patch is >=15 deg vs {gr['reference_share_pct']}% "
                     f"of rover-gentle reference ground (run {gr['run_utc']})"),
            rover_check=(f"{rcr['outcome']}: median |DEM tilt - rover tilt| {rcr['median_abs_deg']} deg, p90 {rcr['p90_abs_deg']}, "
                         f"signed median {rcr['signed_median_deg']:+} deg (DEM steeper if positive), N {rcr['n']}; "
                         f"{'beats' if rcr['below_chance'] else 'does not beat'} the shift baseline (run {rcr['run_utc']})"),
            ortho_alignment=(f"photo vs DEM best shift {oa['best_shift_e_m']} m east, {oa['best_shift_s_m']} m south; R2 {oa['r2_zero_shift']} at zero "
                             f"shift vs {oa['r2_best']} best; taken as aligned within about 2 m, no correction applied"),
            ortho_grid_snap=f"photo snapped to the nearest 0.25 m pixel: residual {snap_e_m} m east, {snap_s_m} m south of the DEM cell edge"),
        assumptions=["slope bands and slope_3m use a 3 x 3 m box filter (choice informed by the gate; not a measured optimum)",
                     "route slope limit, walking speed, slope penalty and caution band are assumptions (route.json 'assumptions')"],
        unverified=["which cells come from HiRISE vs the single CTX backfill DEM", "orthophoto licence/reuse wording (credit as listed)"],
        extra_files=[("navcam_4718_points.bin", "float32 LE x4 per point: e_m, n_m, height above z_offset_m, slope_deg (tools/navcam_check.py)",
                      dict(credit="NASA/JPL-Caltech; Maki, J. (2024), MSL Navcam Processed Data Bundle, NASA PDS, doi:10.17189/2pw6-a555", stop="nevado_sajama")),
                     ("navcam_check.json", "JSON (tools/navcam_check.py)", dict(note="height check and slope comparison per the doc's Navcam rule"))])
    stops_src = json.load(open(config.STOPS_SOURCE, encoding="utf-8"))
    m = write_bundle(out, z, cell, xw, yn, ortho, rover, checks, meta, stops_src)
    total = sum(v["bytes"] for v in m["files"].values())
    print(f"bundle written to {out}/: grid {W} x {H} m, z_offset {m['grid']['z_offset_m']} m, {len(m['files'])} files + manifest, {total/1e6:.1f} MB")
    for k, v in m["files"].items():
        print(f"  {k:22s} {v['bytes']/1e6:6.2f} MB  {v['kind']}")


def selftest():
    H, W, cell = 40, 30, 1.0
    t = math.tan(math.radians(20))
    z = np.tile(np.arange(W) * t, (H, 1)) - 3600.0              # rises east at 20 deg
    ortho = (np.random.default_rng(1).integers(0, 255, (4 * H, 4 * W, 3))).astype(np.uint8)
    xw, yn = R * math.radians(137.38), R * math.radians(-4.80)
    lat_c = math.degrees((yn - 10.5) / R); lon_c = math.degrees((xw + 5.5) / R)
    rover = [dict(sol=4600, site="1", drive="2", lat=lat_c, lon=lon_c, elev=-3590.0)]
    checks = [dict(sol=4600, site="1", drive="2", lat=lat_c, lon=lon_c, rover_tilt=8.0, dem_tilt=9.0, abs_diff=1.0, label="agrees")]
    with tempfile.TemporaryDirectory() as td:
        src = dict(description="t", position_rule="t", stops=[dict(id="a", name="A", kind="ridge", site="1", drive="2", sol=4600,
                   position_note="t", facts=[dict(text="f", source="u", quote="q")], photo=None)])
        m = write_bundle(td, z, cell, xw, yn, ortho, rover, checks, dict(built_utc="test", bias_deg=3.0), src)
        st = json.load(open(os.path.join(td, "stops.json"), encoding="utf-8"))["stops"][0]
        assert abs(st["slope_3m_deg"] - 20.0) < 0.1 and st["rover_check"]["label"] == "agrees", st
        hm = np.fromfile(os.path.join(td, "heightmap.bin"), "<f4").reshape(H, W) + m["grid"]["z_offset_m"]
        assert np.allclose(hm, z, atol=1e-3), "heightmap round trip"
        s = np.fromfile(os.path.join(td, "slope_3m.bin"), "<f4").reshape(H, W)
        assert abs(float(np.median(s[5:-5, 5:-5])) - 20.0) < 0.01, "interior slope of a 20 deg plane"
        b = np.array(Image.open(os.path.join(td, "slope_bands_3m.png")))
        assert (b[5:-5, 5:-5] == 1).all(), "20 deg is band 1"
        for name, v in m["files"].items():
            assert sha256(os.path.join(td, name)) == v["sha256"], name
        p = json.load(open(os.path.join(td, "rover_path.json")))["points"][0]
        assert abs(p["e_m"] - 5.5) < 1e-3 and abs(p["n_m"] - (H - 10.5)) < 1e-3, p
        assert abs(p["dem_elev_m"] - z[10, 5]) < 1e-9, "point falls in cell row 10, col 5"
        n = np.array(Image.open(os.path.join(td, "ortho_0p25m_n.jpg"))); s_ = np.array(Image.open(os.path.join(td, "ortho_0p25m_s.jpg")))
        assert n.shape[0] + s_.shape[0] == 4 * H and n.shape[1] == 4 * W
        zbad = z.copy(); zbad[3, 3] = np.nan
        try:
            write_bundle(td, zbad, cell, xw, yn, ortho, rover, checks, {})
            raise AssertionError("NaN heightmap must be refused")
        except ValueError:
            pass
    print("selftest passed: heightmap round trip, 3 m slope of a 20 deg plane, bands, hashes, local coordinates, photo split, NaN refusal (synthetic)")


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a or a[0] not in ("build", "selftest", "rehash"):
        sys.exit(__doc__)
    if a[0] == "selftest":
        selftest()
    elif a[0] == "rehash":
        rehash(a[a.index("--out") + 1] if "--out" in a else "assets")
    else:
        build(a[a.index("--out") + 1] if "--out" in a else "assets")
