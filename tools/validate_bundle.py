"""Validate the VISOR asset bundle (assets/) so the client can trust it. Exit code 0 = all checks pass, 1 = failures listed.

Checks:
 - manifest: every listed file exists, byte size and sha256 match
 - heightmap.bin / slope_3m.bin: size = width x height x 4, all values finite; slope within 0..90 deg
 - slope_bands_3m.png and photo textures: dimensions as stated in the manifest, textures at most 4096 px per side
 - stops.json: every stop inside the grid, every fact has an http(s) source and a quote, photo files exist with a credit, walk-back set
 - route.json: all legs possible, polyline inside the grid, passes within 2 m of every stop and returns to the start (acceptance test 2)
 - rover_check.json: labels only from the allowed set
 - no ui_text.json: on-screen words live in app/text/en.json and are checked when the app compiles
Usage: python tools/validate_bundle.py [assets]
"""
import hashlib
import json
import math
import os
import re
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import config  # noqa: E402

LABELS = {"agrees", "partial", "disagrees", "unusable"}
MAX_TEX = 4096


def main(d):
    errs, notes = [], []
    E = errs.append
    m = json.load(open(os.path.join(d, "manifest.json")))
    g = m["grid"]; W, H = g["width"], g["height"]
    for name, v in m["files"].items():
        p = os.path.join(d, name)
        if not os.path.exists(p):
            E(f"missing file: {name}"); continue
        if os.path.getsize(p) != v["bytes"]:
            E(f"size mismatch: {name}")
        h = hashlib.sha256(open(p, "rb").read()).hexdigest()
        if h != v["sha256"]:
            E(f"sha256 mismatch: {name}")
    wb = m["files"].get("walk_back_s.bin")
    if wb is None:
        E("walk_back_s.bin missing from the manifest")
    else:
        a = np.fromfile(os.path.join(d, "walk_back_s.bin"), "<u2")
        if a.size != wb["width"] * wb["height"]:
            E("walk_back_s.bin: size does not match the grid")
        elif (a == 0).sum() < 1 or (a != wb["no_way_back"]).sum() != wb["reachable_cells"]:
            E("walk_back_s.bin: no zero (exit) cell, or reachable count differs from the manifest")
        else:
            notes.append(f"walk_back_s.bin: {wb['reachable_cells']} reachable cells, max {int(a[a != wb['no_way_back']].max())} s")
    # route uncertainty ensemble (optional): must match the current route and grid, and be in the manifest
    ep = os.path.join(d, "route_ensemble.json")
    if os.path.exists(ep):
        ens = json.load(open(ep))
        rt = json.load(open(os.path.join(d, "route.json")))
        if "route_ensemble.json" not in m["files"] or "route_usage.bin" not in m["files"]:
            E("route_ensemble.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not ens.get("zero_noise", {}).get("ok"):
            E("route_ensemble.json: zero-noise check did not pass")
        if ens["inputs"]["published_total_time_s"] != rt["total_time_s"] or ens["inputs"]["bias_deg"] != rt["assumptions"]["bias_deg"]:
            E("route_ensemble.json was computed for another route.json: re-run the ensemble")
        ub = os.path.join(d, "route_usage.bin")
        if not os.path.exists(ub) or os.path.getsize(ub) != m["grid"]["width"] * m["grid"]["height"]:
            E("route_usage.bin missing or not one byte per grid cell")
        else:
            notes.append(f"route_ensemble.json: {len(ens.get('ensembles', []))} ensembles, shown l = {ens.get('shown_correlation_length_m')} m")
    # daily weather (optional, tools/conditions.py): in the manifest, sourced, disclaimer kept, sols in the study range
    cp = os.path.join(d, "conditions.json")
    if os.path.exists(cp):
        cd = json.load(open(cp, encoding="utf-8"))
        if "conditions.json" not in m["files"]:
            E("conditions.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not cd.get("source", {}).get("url") or not cd["source"].get("disclaimer"):
            E("conditions.json: source url or disclaimer missing")
        sols = [r["sol"] for r in cd.get("sols", [])]
        if sols != sorted(set(sols)) or (sols and not (config.SOL_FIRST <= sols[0] and sols[-1] <= config.SOL_LAST)):
            E("conditions.json: sols not unique, sorted and inside the study range")
        if "check" not in cd or "passed" not in cd["check"]:
            E("conditions.json: Ls check result missing")
        notes.append(f"conditions.json: {len(sols)} sols, {len(cd.get('missing_sols', []))} without a record, "
                     f"Ls check {'PASS' if cd['check']['passed'] else 'FAIL (recorded)'}")
    # radiation (optional, tools/rad.py): in the manifest, sourced, every archived sol summarised and inside the rule's guard
    rp = os.path.join(d, "radiation.json")
    if os.path.exists(rp):
        rd = json.load(open(rp, encoding="utf-8"))
        if "radiation.json" not in m["files"]:
            E("radiation.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not rd.get("source", {}).get("url") or not rd.get("context_only"):
            E("radiation.json: source or context-only note missing")
        for s_ in rd.get("sols", []):
            if s_.get("archived") and not s_.get("valid"):
                E(f"radiation.json: sol {s_['sol']} is INVALID under the RAD rule")
        notes.append(f"radiation.json: {sum(1 for s_ in rd.get('sols', []) if s_.get('archived'))} archived stop sols, "
                     f"{len(rd.get('space_weather', {}).get('events', []))} DONKI events")
    # data readiness (optional, tools/provenance.py run2): one class byte per cell, legend and verdict present
    rj = os.path.join(d, "readiness.json")
    if os.path.exists(rj) or os.path.exists(os.path.join(d, "readiness.bin")):
        if not (os.path.exists(rj) and os.path.exists(os.path.join(d, "readiness.bin"))):
            E("readiness.bin and readiness.json must come together")
        else:
            rjd = json.load(open(rj, encoding="utf-8"))
            cls = np.fromfile(os.path.join(d, "readiness.bin"), np.uint8)
            for name in ("readiness.bin", "readiness.json"):
                if name not in m["files"]:
                    E(f"{name} present but not in the manifest: run `python tools/build_bundle.py rehash`")
            if cls.size != W * H:
                E(f"readiness.bin: {cls.size} bytes, expected {W * H}")
            elif cls.max() > 4:
                E("readiness.bin: class above 4")
            if not rjd.get("rule") or not rjd.get("verdict") or sorted(rjd.get("classes", {})) != ["0", "1", "2", "3", "4"]:
                E("readiness.json: rule, verdict or class legend missing")
            notes.append(f"readiness: {rjd.get('verdict')}, classes {np.bincount(cls, minlength=5).tolist()}")
    aj = os.path.join(d, "astronaut.json")
    if os.path.exists(aj):
        am = json.load(open(aj, encoding="utf-8"))
        ab = os.path.getsize(os.path.join(d, "astronaut.bin"))
        for name in ("astronaut.bin", "astronaut.json"):
            if name not in m["files"]:
                E(f"{name} present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not am.get("source", {}).get("credit"):
            E("astronaut.json: credit missing")
        if am.get("format") != 2:
            E("astronaut.json: format 2 (one skinned mesh) expected: run `python tools/astronaut.py build`")
        elif am["idx_offset"] + 4 * am["indices"] > ab or am["indices"] % 3 or am["vertices"] == 0 or len(am.get("joints", [])) != 5:
            E("astronaut: offsets, counts or joints out of range")
        notes.append(f"astronaut: one skinned mesh, {am.get('vertices')} vertices, {len(am.get('joints', []))} joints")
    cj = os.path.join(d, "curiosity.json")
    if os.path.exists(cj):
        cm = json.load(open(cj, encoding="utf-8"))
        cb = os.path.getsize(os.path.join(d, "curiosity.bin"))
        for name in ("curiosity.bin", "curiosity.json"):
            if name not in m["files"]:
                E(f"{name} present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not cm.get("source", {}).get("credit"):
            E("curiosity.json: credit missing")
        if cm["idx_offset"] + 4 * cm["indices"] > cb or cm["indices"] % 3 or not (2.5 < cm["extent_m"]["x"] < 3.2):
            E("curiosity: offsets, counts or width out of range (Curiosity is about 2.8 m wide)")
        notes.append(f"curiosity: {cm['vertices']} vertices, {cm['indices'] // 3} triangles")
    pj = os.path.join(d, "panos.json")
    if os.path.exists(pj):
        pd = json.load(open(pj, encoding="utf-8"))
        ids = {s["id"] for s in json.load(open(os.path.join(d, "stops.json"), encoding="utf-8"))["stops"]}
        if "panos.json" not in m["files"]:
            E("panos.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not pd.get("credit") or not pd.get("doi"):
            E("panos.json without credit or DOI")
        for P in pd.get("panos", []):
            if P["stop"] not in ids or P["file"] not in m["files"]:
                E(f"panorama {P.get('file')}: unknown stop or not in the manifest")
            elif not (P["el_min_deg"] < 0 < P["el_max_deg"] and 0 <= P["start_az_deg"] < 360):
                E(f"panorama {P['file']}: elevation or azimuth out of range")
            else:
                from PIL import Image
                if Image.open(os.path.join(d, P["file"])).size != (P["width"], P["height"]):
                    E(f"panorama {P['file']}: size differs from panos.json")
        notes.append(f"panoramas: {len(pd.get('panos', []))}")
    tj = os.path.join(d, "trek_layers.json")
    if os.path.exists(tj):
        td = json.load(open(tj, encoding="utf-8"))
        if "trek_layers.json" not in m["files"]:
            E("trek_layers.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        for L in td.get("layers", []):
            if not (L.get("m_per_px") and L.get("level") is not None and "data_share" in L):
                E(f"trek layer {L.get('label')}: resolution, level or coverage missing")
            if L.get("file"):
                if L["file"] not in m["files"]:
                    E(f"{L['file']} not in the manifest")
                else:
                    from PIL import Image
                    im = Image.open(os.path.join(d, L["file"]))
                    if im.size != (L["width"], L["height"]) or max(im.size) > 4096:
                        E(f"{L['file']}: size {im.size} vs {L['width']} x {L['height']} (max 4096)")
            if L.get("kind") == "Imagery" and L.get("file") and abs(L.get("offset_vs_usgs_photo", {}).get("east_m", 99)) > 5:
                E(f"{L['label']}: offset from the USGS photo over 5 m; do not show it as a basemap")
        notes.append(f"trek: {len(td.get('layers', []))} layers, {sum(1 for L in td.get('layers', []) if L.get('file'))} basemaps")
    cj = os.path.join(d, "chemistry.json")
    if os.path.exists(cj):
        cd = json.load(open(cj, encoding="utf-8"))
        if "chemistry.json" not in m["files"]:
            E("chemistry.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        W, H = m["grid"]["width"] * m["grid"]["cell_m"], m["grid"]["height"] * m["grid"]["cell_m"]
        for r in cd.get("records", []):
            if not (0 <= r["e_m"] < W and 0 <= r["n_m"] < H) or not r.get("oxides") or r["instrument"] not in ("ChemCam", "APXS"):
                E(f"chemistry.json: bad record {r.get('instrument')} {r.get('target')} sol {r.get('sol')}")
                break
        notes.append(f"chemistry: {len(cd.get('records', []))} records ({cd['counts']['chemcam']} ChemCam, {cd['counts']['apxs']} APXS), last sol {cd.get('last_sol_released')}")
    sj = os.path.join(d, "sun_check.json")
    if os.path.exists(sj):
        sd = json.load(open(sj, encoding="utf-8"))
        if "sun_check.json" not in m["files"]:
            E("sun_check.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if sd.get("outcome") not in ("PASS", "FAIL") or not sd.get("n"):
            E("sun_check.json: outcome or sample count missing")
        notes.append(f"sun check vs Trek: {sd.get('outcome')} (n {sd.get('n')}, max d el {sd.get('max_abs_d_elevation_deg')}, max d az {sd.get('max_abs_d_azimuth_deg')})")
    oj = os.path.join(d, "overview.json")
    if os.path.exists(oj):
        od = json.load(open(oj, encoding="utf-8"))
        if "overview.json" not in m["files"]:
            E("overview.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        for st in od.get("sites", []):
            if not (-90 <= st["lat"] <= 90 and -180 <= st["lon"] <= 360 and st.get("source") and st.get("position")):
                E(f"overview site {st.get('title')}: position or source missing")
        pts = od.get("traverse", {}).get("points", [])
        if not pts or any(not (-90 <= p[0] <= 90) for p in pts):
            E("overview traverse empty or out of range")
        w, s_, e, n = od["study"]["bbox"]
        if not (w < e and s_ < n):
            E("overview study bbox malformed")
        for b in od.get("bases", []):
            if not (b.get("url", "").startswith("https://") and b.get("credit") and b.get("tile_px") in (256, 512)):
                E(f"overview base {b.get('id')}: url, credit or tile size missing")
        notes.append(f"overview: {len(od.get('sites', []))} sites, traverse {len(pts)} points, {len(od.get('bases', []))} basemaps")
    mj = os.path.join(d, "missions.json")
    if os.path.exists(mj):
        md = json.load(open(mj, encoding="utf-8"))
        if "missions.json" not in m["files"]:
            E("missions.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        for ms in md.get("missions", []):
            for u in ms.get("uses", []):
                for f in u.get("files", []):
                    if f not in m["files"]:
                        E(f"missions.json: {ms['short']} / {u['instrument']} names {f}, not in the manifest")
                if not u.get("files") and not u.get("live"):
                    E(f"missions.json: {ms['short']} / {u['instrument']} has neither a bundle file nor a live service")
        notes.append(f"missions: {len(md.get('missions', []))} NASA missions")
    ej = os.path.join(d, "eva_ops.json")
    if os.path.exists(ej):
        ev = json.load(open(ej, encoding="utf-8"))
        if "eva_ops.json" not in m["files"]:
            E("eva_ops.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        for k in ("eva_hours", "egress_ingress_h", "emergency_extension_h", "rescue_limit_min", "pairs", "mars"):
            e_ = ev.get(k, {})
            if not (e_.get("quote") and e_.get("page") and ev.get("sources", {}).get(e_.get("source"), {}).get("url")):
                E(f"eva_ops.json: {k} needs a quote, a page and a source URL")
        if ev.get("eva_hours", {}).get("default") not in ev.get("eva_hours", {}).get("options", []):
            E("eva_ops.json: default EVA length not among the options")
        notes.append(f"eva_ops.json: EVA {ev['eva_hours']['options']} h, rescue {ev['rescue_limit_min']['value']} min")
    rgj = os.path.join(d, "ridges.json")
    if os.path.exists(rgj):
        rg = json.load(open(rgj, encoding="utf-8"))
        if "ridges.json" not in m["files"]:
            E("ridges.json present but not in the manifest: run `python tools/build_bundle.py rehash`")
        if not rg.get("rule") or rg.get("check_a", {}).get("outcome") not in ("PASS", "PARTIAL", "FAIL"):
            E("ridges.json: rule or Check A outcome missing")
        notes.append(f"ridges.json: Check A {rg.get('check_a', {}).get('outcome')}")
    for name in ("heightmap.bin", "slope_3m.bin"):
        a = np.fromfile(os.path.join(d, name), "<f4")
        if a.size != W * H:
            E(f"{name}: {a.size} values, expected {W * H}")
        elif not np.isfinite(a).all():
            E(f"{name}: non-finite values")
        elif name == "slope_3m.bin" and (a.min() < 0 or a.max() > 90):
            E("slope_3m.bin: values outside 0..90")
    for name, v in m["files"].items():
        if name.endswith((".png", ".jpg")) and "width" in v:
            w, h = Image.open(os.path.join(d, name)).size
            if (w, h) != (v["width"], v["height"]):
                E(f"{name}: {w}x{h}, manifest says {v['width']}x{v['height']}")
            if max(w, h) > MAX_TEX and not name.startswith("photos/"):
                E(f"{name}: {w}x{h} exceeds {MAX_TEX} px")
    inside = lambda e, n: 0 <= e < W * g["cell_m"] and 0 <= n < H * g["cell_m"]
    st = json.load(open(os.path.join(d, "stops.json"), encoding="utf-8"))
    ids = {x["id"]: x for x in st["stops"]}
    for x in st["stops"]:
        for k, tk in enumerate(x.get("tasks", [])):
            if tk.get("step") not in ("question", "observe", "image", "sample", "compare") or not tk.get("text"):
                E(f"stop {x['id']} task {k}: step or text missing")
            refs = tk.get("refs") or []
            if not refs:
                E(f"stop {x['id']} task {k}: cites no fact (every station task must cite a sourced fact)")
            if tk.get("step") == "sample" and not (tk.get("bags") and all(isinstance(b, str) and b for b in tk["bags"])):
                E(f"stop {x['id']} task {k}: a sample task needs bag labels")
            for r in refs:
                if r[0] not in ids or not (0 <= r[1] < len(ids[r[0]]["facts"])):
                    E(f"stop {x['id']} task {k}: reference {r} is not a fact in stops.json")
                elif ids[r[0]]["facts"][r[1]].get("inference") and not tk.get("inference"):
                    E(f"stop {x['id']} task {k}: cites an inferred fact but is not marked inference")
    notes.append(f"station tasks: {sum(len(x.get('tasks', [])) for x in st['stops'])}, every one cites a fact")
    for s in st["stops"]:
        if not inside(s["e_m"], s["n_m"]):
            E(f"stop {s['id']} outside the grid")
        for f in s["facts"]:
            if not str(f.get("source", "")).startswith(("http://", "https://")) or not f.get("quote"):
                E(f"stop {s['id']}: fact without an http(s) source and quote")
            if not (f.get("source_title") and re.fullmatch(r"\d{4}-\d{2}-\d{2}", str(f.get("published", "")))):
                E(f"stop {s['id']}: fact without a source title and publication date (YYYY-MM-DD)")
        if s.get("photo"):
            if not os.path.exists(os.path.join(d, s["photo"]["file"])):
                E(f"stop {s['id']}: photo file missing")
            if not s["photo"].get("credit"):
                E(f"stop {s['id']}: photo without credit")
            v = s["photo"].get("view")
            if v is not None and not (0 <= v.get("az_deg", -1) < 360 and -90 <= v.get("el_deg", -99) <= 90 and v.get("compass") and v.get("rule")):
                E(f"stop {s['id']}: photo view direction out of range or without its rule")
        if s.get("walk_back_s") is None:
            E(f"stop {s['id']}: no walk-back time")
    r = json.load(open(os.path.join(d, "route.json")))
    if not r["all_legs_possible"]:
        E("route: not all legs possible")
    poly = np.array(r["polyline_en"])
    if not all(inside(e, n) for e, n in poly):
        E("route polyline leaves the grid")
    for s in st["stops"]:
        dmin = float(np.min(np.hypot(poly[:, 0] - s["e_m"], poly[:, 1] - s["n_m"])))
        if dmin > 2.0:
            E(f"route misses stop {s['id']} by {dmin:.2f} m (limit 2 m)")
    if math.hypot(*(poly[0] - poly[-1])) > 2.0:
        E("route does not return to the start")
    rc = json.load(open(os.path.join(d, "rover_check.json")))
    bad = {x["label"] for x in rc["stops"]} - LABELS
    if bad:
        E(f"rover_check: unknown labels {bad}")
    if os.path.exists(os.path.join(d, "ui_text.json")):
        E("ui_text.json found: the app's words now live in app/text/en.json (checked at compile time by app/build.rs)")
    notes.append(f"{len(m['files'])} manifest files, {len(st['stops'])} stops, route {r['total_length_m']} m")
    for n in notes:
        print("ok:", n)
    for e in errs:
        print("FAIL:", e)
    print("RESULT:", "PASS" if not errs else f"FAIL ({len(errs)})")
    return 0 if not errs else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "assets"))
