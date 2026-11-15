"""Crop the 25 cm HiRISE colour orthophoto (USGS Astrogeology COG, streamed) to the gate patch plus a margin.

Source: https://asc-pds-services.s3.us-west-2.amazonaws.com/mosaic/Mars/MSL/MSL_Gale_HiRISE-LRGB_78quads_sharp_cog.tif
(3-band uint8, 0.25 m, Equirectangular on the 3396190 m Mars 2000 sphere, central meridian 0: the same projection as the 1 m DEM).
Patch = PLACES ROVER positions sols 4534..4865 + 50 m (tools/gate_patch.py) + MARGIN_M extra.

Writes data/processed/ortho/ortho_patch.tif (GeoTIFF, deflate), data/processed/ortho/ortho_patch_preview.png (1/8 scale) and prints the window.
Usage: python tools/ortho_crop.py data/raw/localized_pos.csv [--margin 10]
Needs rasterio (reads only the needed tiles over HTTP).
"""
import math
import os
import sys

import numpy as np
import rasterio
from rasterio.windows import from_bounds

sys.path.insert(0, os.path.dirname(__file__))
import gate_patch as gp
import config

URL = "https://asc-pds-services.s3.us-west-2.amazonaws.com/mosaic/Mars/MSL/MSL_Gale_HiRISE-LRGB_78quads_sharp_cog.tif"
from config import MARS_RADIUS_M as R  # single source of truth


def main(places, margin):
    rows = gp.load(places)
    lats = [r["lat"] for r in rows]; lons = [r["lon"] for r in rows]
    m = gp.MARGIN_M + margin
    mean_lat = sum(lats) / len(lats)
    dlat = math.degrees(m / R); dlon = math.degrees(m / (R * math.cos(math.radians(mean_lat))))
    north, south = max(lats) + dlat, min(lats) - dlat
    west, east = min(lons) - dlon, max(lons) + dlon
    x0, x1 = R * math.radians(west), R * math.radians(east)
    y0, y1 = R * math.radians(south), R * math.radians(north)
    os.makedirs(os.path.dirname(config.ORTHO), exist_ok=True)
    with rasterio.open(URL) as src:
        win = from_bounds(x0, y0, x1, y1, src.transform).round_offsets().round_lengths()
        data = src.read(window=win)
        prof = src.profile.copy()
        prof.update(driver="GTiff", width=data.shape[2], height=data.shape[1], transform=src.window_transform(win),
                    compress="deflate", tiled=True, blockxsize=512, blockysize=512, photometric="RGB")
        with rasterio.open(config.ORTHO, "w", **prof) as dst:
            dst.write(data)
        print(f"window {win}; size {data.shape[2]} x {data.shape[1]} px; bounds x {x0:.1f}..{x1:.1f} y {y0:.1f}..{y1:.1f}")
        print(f"lat {south:.6f}..{north:.6f} lon {west:.6f}..{east:.6f}; zero (no-data) pixels: {(data.sum(axis=0) == 0).mean()*100:.2f}%")
    prev = data[:, ::8, ::8].transpose(1, 2, 0)
    gp.write_png(os.path.join(os.path.dirname(config.ORTHO), "ortho_patch_preview.png"), np.ascontiguousarray(prev))
    print("wrote data/processed/ortho/ortho_patch.tif and data/processed/ortho/ortho_patch_preview.png")


if __name__ == "__main__":
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    main(a[0], float(a[a.index("--margin") + 1]) if "--margin" in a else 10.0)
