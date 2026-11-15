"""NASA Mars Trek layers for VISOR (Trek is an official challenge resource).

Trek (https://trek.nasa.gov/mars/) serves map tiles over WMTS in the Mars 2000 geographic system (EPSG:104905): a level's
matrix spans 360 deg of longitude in MatrixWidth tiles of 256 px, rows from 90 N. Every tile we used carries
Access-Control-Allow-Origin: *. This tool composites the tiles over the bundle patch OFFLINE into view-only basemaps
(assets/trek_<label>.jpg), so the app does not depend on Trek being reachable, and measures, per layer:
  - finest level and metres per pixel (from WMTSCapabilities.xml);
  - how much of the patch holds data (tile alpha > 0);
  - for the HiRISE mosaic: the offset from our USGS 25 cm photo (FFT cross-correlation of the 1 m grids, +-80 m search);
    reported, NOT applied (a context layer; the measurement map stays the USGS DEM and photo).
Thermal / spectral layers become rows of the data-readiness table with their real size over the patch.
Output: assets/trek_layers.json (+ the jpgs); `python tools/build_bundle.py rehash` registers them.

Usage: python tools/trek.py selftest | build
"""
import io
import json
import math
import os
import re
import sys
import urllib.request

import numpy as np

import config

R = config.MARS_RADIUS_M
BASE = "https://trek.nasa.gov/tiles/Mars/EQ"
CATALOG = "https://trek.nasa.gov/mars/TrekServices/ws/index/eq/searchItems?start=0&rows=50&proj=urn:ogc:def:crs:EPSG::104905&key="
DEEP_LINK = ("https://trek.nasa.gov/mars/#v=0.1&x={lon}&y={lat}&z={z}&p=urn%3Aogc%3Adef%3Acrs%3AEPSG%3A%3A104905&d=&locale=&b=mars"
             "&e={w}%2C{s}%2C{e}%2C{n}&sfz=&w=")
# role: "basemap" (selectable view-only imagery over the patch) or "readiness" (a data-kind row; coarse)
LAYERS = [
    dict(label="curiosity_hirise_mosaic", role="basemap", kind="Imagery", short="HiRISE mosaic", px_per_m=2),
    dict(label="curiosity_ctx_mosaic", role="basemap", kind="Imagery", short="CTX mosaic", px_per_m=1),
    dict(label="THEMIS_NightIR_ControlledMosaics_100m_v2_oct2018", role="basemap", kind="Thermal", short="THEMIS night IR", px_per_m=1),
    dict(label="THEMIS_DayIR_ControlledMosaics_100m_v2_oct2018", role="basemap", kind="Thermal", short="THEMIS day IR", px_per_m=1),
    dict(label="SIM3292_Global_Geology", role="readiness", kind="Geology", short="Global geologic map"),
    dict(label="TES_Thermal_Inertia", role="readiness", kind="Thermal", short="TES thermal inertia"),
    dict(label="TES_Glass_Clay", role="readiness", kind="Spectral", short="TES sheet silicates / high-Si glass"),
    # CRISM datasets (e.g. frt_07_brphyj_mtr3_ll) are listed in the catalogue but are not on this tile service (capabilities
    # 404): left out, named in trek_layers.json "not_tiled".
]
# Checked and left out: Trek's "MRO CTX, Gale Crater Mosaic" (Gale_CTX_BlockAdj_dd) does not line up with the HiRISE
# products over this patch (correlation -0.06 unshifted, best 0.24 at the edge of a +-300 m search, same tile code that places
# Trek's HiRISE mosaic at 0 m); the Curiosity roving-site CTX mosaic does line up and is used instead.
EXCLUDED = ["Gale_CTX_BlockAdj_dd (MRO CTX, Gale Crater Mosaic): does not line up with the HiRISE products here (tools/trek.py)"]
NOT_TILED = ["frt_07_brphyj_mtr3_ll (CRISM Fe-Mg-OH phyllosilicates)", "frt_07_brcarj_mtr3_ll (CRISM Fe-Mg carbonates)"]


def get(url, timeout=120):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "visor-pipeline"}), timeout=timeout) as r:
        return r.read()


def levels(cap_xml):
    """[(level, matrix_width, matrix_height)] from WMTSCapabilities.xml (levels are not listed in order)."""
    out = []
    for b in re.findall(r"<TileMatrix>(.*?)</TileMatrix>", cap_xml, re.S):
        g = lambda t: re.search(rf"<{t}>\s*([^<]+?)\s*</{t}>", b).group(1)
        out.append((int(g("ows:Identifier")), int(float(g("MatrixWidth"))), int(float(g("MatrixHeight")))))
    return sorted(out)


def deg_per_tile(matrix_width):
    return 360.0 / matrix_width


def m_per_px(matrix_width):
    return deg_per_tile(matrix_width) / 256.0 * math.pi / 180.0 * R


def tile_range(bbox, matrix_width):
    """Tile columns / rows covering bbox (lon_w, lat_s, lon_e, lat_n)."""
    d = deg_per_tile(matrix_width)
    w, s, e, n = bbox
    return int((w + 180) // d), int((e + 180) // d), int((90 - n) // d), int((90 - s) // d)


def global_px(lon, lat, matrix_width):
    d = deg_per_tile(matrix_width)
    return (lon + 180.0) / d * 256.0, (90.0 - lat) / d * 256.0


def sample_bilinear(mosaic, x, y):
    """mosaic (H, W, C) float; x, y arrays of pixel-centre coordinates (pixel i covers i..i+1)."""
    x = np.clip(x - 0.5, 0, mosaic.shape[1] - 1.001)
    y = np.clip(y - 0.5, 0, mosaic.shape[0] - 1.001)
    x0, y0 = np.floor(x).astype(int), np.floor(y).astype(int)
    fx, fy = (x - x0)[..., None], (y - y0)[..., None]
    a, b = mosaic[y0, x0], mosaic[y0, x0 + 1]
    c, d = mosaic[y0 + 1, x0], mosaic[y0 + 1, x0 + 1]
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


def decodes(b):
    """A real tile: non-empty and a decodable image (Trek answers missing tiles with 404 or an empty 200)."""
    from PIL import Image
    if not b:
        return False
    try:
        Image.open(io.BytesIO(b)).verify()
        return True
    except Exception:
        return False


def has_data(b):
    """The tile holds data (some pixel with alpha > 127), not only transparent padding."""
    from PIL import Image
    return decodes(b) and bool((np.asarray(Image.open(io.BytesIO(b)).convert("RGBA"))[..., 3] > 127).any())


def to_rgba(img):
    from PIL import Image
    return np.asarray(img.convert("RGBA"), float)


def composite(label, level, mw, grid, px_per_m, cache):
    """Fetch the tiles over the grid and resample them onto it: returns (rgba (H*k, W*k, 4), data share)."""
    from PIL import Image
    W, H, xw, yn = grid["width"], grid["height"], grid["x_west_m"], grid["y_north_m"]
    bbox = (math.degrees(xw / R), math.degrees((yn - H) / R), math.degrees((xw + W) / R), math.degrees(yn / R))
    c0, c1, r0, r1 = tile_range(bbox, mw)
    mosaic = np.zeros(((r1 - r0 + 1) * 256, (c1 - c0 + 1) * 256, 4))
    for r in range(r0, r1 + 1):
        for c in range(c0, c1 + 1):
            p = os.path.join(cache, f"{label}_{level}_{r}_{c}.png")
            if not os.path.exists(p):
                try:
                    b = get(f"{BASE}/{label}/1.0.0/default/default028mm/{level}/{r}/{c}.png")
                except Exception as ex:
                    print(f"    tile {level}/{r}/{c}: {ex}")
                    continue
                if not decodes(b):
                    print(f"    tile {level}/{r}/{c}: empty or not an image")
                    continue
                open(p, "wb").write(b)
            mosaic[(r - r0) * 256:(r - r0 + 1) * 256, (c - c0) * 256:(c - c0 + 1) * 256] = to_rgba(Image.open(p))
    k = px_per_m
    ox, oy = np.meshgrid((np.arange(W * k) + 0.5) / k, (np.arange(H * k) + 0.5) / k)
    lon = np.degrees((xw + ox) / R)
    lat = np.degrees((yn - oy) / R)
    gx, gy = global_px(lon, lat, mw)
    out = sample_bilinear(mosaic, gx - c0 * 256, gy - r0 * 256)
    share = float((out[..., 3] > 127).mean())
    return out, share, (c1 - c0 + 1) * (r1 - r0 + 1)


def fft_shift(a, b, max_shift):
    """Integer shift (dx east, dy north, in cells) maximising the normalised cross-correlation of b moved onto a."""
    a = (a - a.mean()) / (a.std() + 1e-9)
    b = (b - b.mean()) / (b.std() + 1e-9)
    F = np.fft.fft2(a) * np.conj(np.fft.fft2(b))
    cc = np.real(np.fft.ifft2(F)) / a.size
    H, W = a.shape
    best = (-2.0, 0, 0)
    for dy in range(-max_shift, max_shift + 1):
        for dx in range(-max_shift, max_shift + 1):
            v = cc[dy % H, dx % W]
            if v > best[0]:
                best = (float(v), dx, dy)
    return best


def build():
    from PIL import Image
    man = json.load(open(os.path.join(config.ASSETS, "manifest.json")))
    g = man["grid"]
    cache = os.path.join(config.RAW, "trek")
    os.makedirs(cache, exist_ok=True)
    W, H = g["width"], g["height"]
    bbox = (math.degrees(g["x_west_m"] / R), math.degrees((g["y_north_m"] - H) / R),
            math.degrees((g["x_west_m"] + W) / R), math.degrees(g["y_north_m"] / R))
    out_layers = []
    for L in LAYERS:
        lab = L["label"]
        try:
            cap = get(f"{BASE}/{lab}/1.0.0/WMTSCapabilities.xml").decode("utf-8", "replace")
        except Exception as ex:
            print(f"{lab}: capabilities failed ({ex}); skipped")
            continue
        lv = levels(cap)
        # the finest listed level is not always tiled everywhere: use the finest level that has a tile over the patch
        z, mw = lv[-1][0], lv[-1][1]
        for zz, mww, _ in reversed(lv):
            c0, _, r0, _ = tile_range(bbox, mww)
            try:
                if has_data(get(f"{BASE}/{lab}/1.0.0/default/default028mm/{zz}/{r0}/{c0}.png")):
                    z, mw = zz, mww
                    break
            except Exception:
                continue
        meta = {}
        try:
            cat = json.loads(get(CATALOG + lab).decode("utf-8"))["response"]["docs"]
            meta = next((d for d in cat if d.get("productLabel") == lab), {})
        except Exception:
            pass
        entry = dict(label=lab, role=L["role"], kind=L["kind"], short=L["short"], title=meta.get("title", L["short"]),
                     instrument=meta.get("instrument"), uuid=meta.get("item_UUID"), level=z, m_per_px=round(m_per_px(mw), 3),
                     patch_px=[round(W / m_per_px(mw), 1), round(H / m_per_px(mw), 1)])
        k = L.get("px_per_m", 1)
        rgba, share, ntiles = composite(lab, z, mw, g, k, cache)
        entry.update(data_share=round(share, 4), tiles=ntiles)
        if L["role"] == "basemap":
            name = f"trek_{lab}.jpg"
            rgb = np.clip(rgba[..., :3], 0, 255).astype(np.uint8)
            Image.fromarray(rgb, "RGB").save(os.path.join(config.ASSETS, name), quality=88)
            entry.update(file=name, width=W * k, height=H * k)
        if L["kind"] == "Imagery":
            # offset vs our USGS 25 cm photo, both on the 1 m grid (luminance)
            usgs = np.asarray(Image.open(os.path.join(config.ASSETS, "ortho_0p5m.jpg")).convert("L"), float)
            usgs = usgs.reshape(H, 2, W, 2).mean(axis=(1, 3))
            trek = rgba[..., :3].mean(axis=2).reshape(H, k, W, k).mean(axis=(1, 3))
            if entry["m_per_px"] > 2:                      # compare at the coarse layer's own scale
                from scipy.ndimage import uniform_filter
                usgs = uniform_filter(usgs, int(round(entry["m_per_px"] / 2)) | 1)
            r, dx, dy = fft_shift(usgs, trek, 80)
            r0 = float(np.corrcoef(usgs.ravel(), trek.ravel())[0, 1])
            entry.update(offset_vs_usgs_photo=dict(east_m=dx, north_m=-dy, corr_best=round(r, 3), corr_unshifted=round(r0, 3),
                                                   search_m=80, note="reported, not applied; positive east = Trek content sits east of ours"))
        print(f"{lab}: level {z}, {entry['m_per_px']} m/px, patch {entry['patch_px'][0]} x {entry['patch_px'][1]} px, "
              f"data {share:.1%}, {ntiles} tiles" + (f", offset {entry.get('offset_vs_usgs_photo')}" if 'offset_vs_usgs_photo' in entry else ""))
        out_layers.append(entry)
    centre = ((bbox[0] + bbox[2]) / 2, (bbox[1] + bbox[3]) / 2)
    doc = dict(source=dict(name="NASA Mars Trek (JPL Solar System Treks)", url="https://trek.nasa.gov/mars/", credit="NASA/JPL-Caltech Mars Trek; products credited to their instrument teams",
                           service=f"{BASE}/<label>/1.0.0/default/default028mm/<level>/<row>/<col>.png (WMTS, EPSG:104905)"),
               note="view-only context layers composited offline by tools/trek.py; measurements use the USGS 1 m DEM and 25 cm photo",
               patch_bbox_deg=[round(v, 6) for v in bbox], not_tiled=NOT_TILED, excluded=EXCLUDED, deep_link=DEEP_LINK, deep_link_zoom=16, centre_deg=[round(v, 6) for v in centre],
               layers=out_layers)
    json.dump(doc, open(os.path.join(config.ASSETS, "trek_layers.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print("assets/trek_layers.json written; run `python tools/build_bundle.py rehash`")


def selftest():
    # tile math: level matrix of 2^z * 5 columns (Trek's default028mm starts at 5 x 3), 360 deg wide
    assert abs(deg_per_tile(5) - 72.0) < 1e-12
    c0, c1, r0, r1 = tile_range((137.378, -4.815, 137.388, -4.796), 5 * 2 ** 13)
    d = 360.0 / (5 * 2 ** 13)
    assert c0 == int((137.378 + 180) // d) and r0 == int((90 + 4.796) // d) and c1 >= c0 and r1 >= r0
    x, y = global_px(-180.0, 90.0, 10)
    assert x == 0.0 and y == 0.0
    # bilinear on a ramp is exact
    m = np.zeros((4, 4, 1))
    m[..., 0] = np.arange(4)[None, :]
    v = sample_bilinear(m, np.array([1.5, 2.0]), np.array([1.5, 1.5]))
    assert np.allclose(v[:, 0], [1.0, 1.5]), v
    # the FFT search recovers a known shift
    rng = np.random.default_rng(2)
    a = rng.normal(size=(64, 64))
    b = np.roll(np.roll(a, -3, axis=1), 5, axis=0)       # b is a moved 3 west and 5 down (south)
    r, dx, dy = fft_shift(a, b, 8)
    assert (dx, dy) == (3, -5) and r > 0.99, (dx, dy, r)
    print("trek selftest: PASS")


if __name__ == "__main__":
    a = sys.argv[1:]
    {"selftest": selftest, "build": build}.get(a[0] if a else "", lambda: print(__doc__))()
