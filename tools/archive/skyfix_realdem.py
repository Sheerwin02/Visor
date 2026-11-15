"""Gate test on the REAL Gale DEM (no photos needed). Run on your own machine.
  pip install rasterio numpy scipy
  python skyfix_realdem.py MSL_Gale_DEM_Mosaic_1m_v3.tif stops.csv
stops.csv lines: name,lat,lon   (planetocentric deg, from PLACES localized_pos.csv; boxwork ~ -4.80,137.385)
For each stop: skyline std (deg), then 40 noisy trials of the heading-unknown match.
SELF-CONSISTENT test: observed and candidate skylines come from the SAME DEM, so this measures
DISTINCTIVENESS of the skyline, not accuracy on real photos (that is step 2 in CLAUDE.md).
STATUS: run_core() tested on synthetic arrays only; load() (rasterio) NOT tested -- fix any API errors."""
import sys, csv, numpy as np
from scipy.ndimage import zoom
import skyfix as s

R = 3396190.0            # Mars radius (m), spherical approximation -- assumption
RMIN, HALF = 300.0, 2000.0   # far-horizon only; crop half-width (m)

def load(path, lat, lon):
    import rasterio
    from rasterio.windows import Window
    with rasterio.open(path) as ds:
        cell = np.radians(abs(ds.transform.a)) * R      # metres per pixel (equirectangular, degrees/pixel)
        r0, c0 = ds.index(lon, lat)
        n = int(HALF / cell)
        a = ds.read(1, window=Window(c0 - n, r0 - n, 2 * n, 2 * n), boundless=True,
                    fill_value=np.nan).astype('float64')
        nd = ds.nodata
    if nd is not None:
        a[a == nd] = np.nan
    return a, cell

def run_core(dem, cell, name, trials=40, sky_noise=0.15, radius=240., spacing=30.):
    gaps = np.isnan(dem).mean()
    if gaps > 0.02:
        print(f"{name:20s} skipped: {gaps*100:.0f}% nodata in crop"); return None
    dem = np.where(np.isnan(dem), np.nanmedian(dem), dem)
    dem = zoom(dem, cell / 8.0, order=1)                  # 8 m grid, as in the prototype
    s.CELL = 8.0; s.N = dem.shape[0]; s.RMIN = RMIN
    cx = cy = s.N * 8.0 / 2
    sky = s.skyline(dem, cx, cy)[0]
    rng = np.random.default_rng(0); errs = []
    for _ in range(trials):
        obs = np.roll(sky + rng.normal(0, sky_noise, s.NAZ), -int(rng.integers(s.NAZ)))
        th = rng.uniform(0, 2 * np.pi); r = rng.uniform(0, radius * .7)
        px, py = cx + r * np.cos(th), cy + r * np.sin(th)
        g = np.arange(-radius, radius + 1, spacing); gx, gy = np.meshgrid(px + g, py + g)
        m = (gx - px) ** 2 + (gy - py) ** 2 <= radius ** 2; gx, gy = gx[m], gy[m]
        sc = s.match(obs, s.skyline(dem, gx, gy), s.NAZ)
        i = sc.argmin(); errs.append(np.hypot(gx[i] - cx, gy[i] - cy))
    e = np.array(errs)
    print(f"{name:20s} skyline_std={sky.std():5.2f} deg  median_err={np.median(e):6.1f} m  within60m={np.mean(e<=60)*100:4.0f}%")
    return sky.std(), float(np.mean(e <= 60))

if __name__ == '__main__':
    res = []
    for row in csv.reader(open(sys.argv[2])):
        if row and not row[0].startswith('#'):
            dem, cell = load(sys.argv[1], float(row[1]), float(row[2]))
            res.append(run_core(dem, cell, row[0]))
    ok = sum(1 for r in res if r and r[0] >= 0.4 and r[1] >= 0.8)
    print(f"\n{ok}/{len(res)} stops pass. GATE: go if >= 3 of 5 pass (thresholds from the synthetic test, not from Gale).")
