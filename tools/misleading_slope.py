"""Misleading-slope measurement: how much steep ground looks flat under a given sun?
Run on YOUR machine with the real DEM (pip install rasterio numpy scipy):
  python misleading_slope.py selftest
  python misleading_slope.py run DEM.tif [--bbox row0 row1 col0 col1] [--flat-tol 0.10] [--out results.csv]

QUESTION: on the boxwork tile, what share of ground steeper than 15 and 25 degrees has shading within
+/- flat-tol of flat ground, so that it LOOKS flat from shading alone, at each sun elevation and azimuth?

DEFINITION (an assumption, not from a source): looks-flat means
|shade - flat_shade| <= flat_tol * flat_shade, with shade = Lambertian hillshade and flat_shade = sin(sun elevation).
Default flat_tol = 0.10 is arbitrary: report the result at 0.05, 0.10 and 0.20 too. A person's threshold is not measured here.
LIMITS (state them): no cast shadows (they would make some steep ground easier to read), no texture or albedo,
no ambient light, no perspective; a human test (App design v4.1 estimation test) is what shows what people actually see.
STATUS: array code tested on synthetic planes only. The rasterio loader is UNTESTED. Sun elevations come from
mars_sun.py (obliquity unverified); the azimuth sweep is over 8 directions because azimuth is not fixed by the plan.
Do not quote any number from this script until it has run on the real 1 m DEM."""
import sys, csv, argparse, numpy as np

SLOPE_BANDS = (15.0, 25.0)          # degrees, as in the app
SUN_ELEV = (15.0, 30.0, 50.0, 60.0) # degrees; rough Gale values from mars_sun.py (ties to LTST/Ls: see table)
SUN_AZ = tuple(range(0, 360, 45))   # all compass directions (unrealistic at low sun at Gale; see realistic_az)

def realistic_az(el_deg, n=5):
    """Azimuths a Gale sun can have at this elevation over all seasons (morning range sampled, afternoon mirrored).
    Uses mars_sun.py (obliquity 25.19 deg verified on the NASA Mars fact sheet; azimuth formula is standard spherical
    astronomy, not from the Mars24 notes)."""
    import mars_sun as m
    az = []
    for ls in range(0, 360, 10):
        t = m.morning_ltst_for_elevation(ls, el_deg)
        if t: az.append(m.sun_azimuth(ls, t))
    lo, hi = min(az), max(az)
    mor = [lo + (hi - lo) * i / (n - 1) for i in range(n)]
    return tuple(mor + [360.0 - a for a in mor])

def slope_aspect(dem, cell, smooth_m=0.0):
    from scipy.ndimage import gaussian_filter
    d = gaussian_filter(dem, smooth_m / cell) if smooth_m > 0 else dem
    gy, gx = np.gradient(d, cell)
    slope = np.arctan(np.hypot(gx, gy))
    # downhill direction as compass azimuth (0=N, 90=E); rows increase southward so north gradient = -gy
    down_az = np.arctan2(-gx, gy)            # direction of steepest descent, measured from north clockwise
    return slope, down_az

def shade(slope, down_az, sun_el_deg, sun_az_deg):
    """Lambertian: cos(incidence) = sin(h)cos(s) + cos(h)sin(s)cos(sun_az - upslope_az-facing)."""
    h = np.radians(sun_el_deg); a = np.radians(sun_az_deg)
    facing = down_az                          # a slope faces the direction it descends toward
    return np.clip(np.sin(h) * np.cos(slope) + np.cos(h) * np.sin(slope) * np.cos(a - facing), 0, 1)

def misleading_share(slope, down_az, mask, sun_el, sun_az, tol, band_deg):
    steep = mask & (np.degrees(slope) >= band_deg)
    n = int(steep.sum())
    if n == 0:
        return None, 0
    flat = np.sin(np.radians(sun_el))
    looks_flat = np.abs(shade(slope, down_az, sun_el, sun_az) - flat) <= tol * flat
    return float((looks_flat & steep).sum() / n), n

def table(dem, cell, bbox=None, tol=0.10, smooth_m=0.0, realistic=True):
    slope, down_az = slope_aspect(dem, cell, smooth_m=smooth_m)
    mask = np.zeros(dem.shape, bool)
    if bbox: mask[bbox[0]:bbox[1], bbox[2]:bbox[3]] = True
    else: mask[:] = True
    rows = []
    for band in SLOPE_BANDS:
        for el in SUN_ELEV:
            shares = []
            for az in (realistic_az(el) if realistic else SUN_AZ):
                s, n = misleading_share(slope, down_az, mask, el, az, tol, band)
                if s is not None: shares.append(s)
            if shares:
                rows.append(dict(band_deg=band, sun_el=el, n_steep_px=n, share_min=min(shares),
                                 share_mean=float(np.mean(shares)), share_max=max(shares), tol=tol))
    return rows

def selftest():
    cell = 1.0
    xs = np.arange(200) * cell
    # plane that descends toward the east at 25 deg
    dem = -np.tan(np.radians(25.0)) * np.tile(xs, (200, 1))
    slope, down_az = slope_aspect(dem, cell, smooth_m=0)
    mid = (slice(20, 180), slice(20, 180))
    assert abs(np.degrees(slope[mid]).mean() - 25.0) < 0.1
    assert abs(np.degrees(down_az[mid]).mean() - 90.0) < 0.5            # descends toward east
    m = np.zeros(dem.shape, bool); m[mid] = True
    # sun due south (az 180): slope runs across the sun direction; shade = sin(h)cos(s)
    el = 30.0
    exp = np.sin(np.radians(el)) * np.cos(np.radians(25.0))
    got = shade(slope, down_az, el, 180.0)[mid].mean()
    assert abs(got - exp) < 1e-3, (got, exp)
    s, n = misleading_share(slope, down_az, m, el, 180.0, 0.10, 15.0)
    assert s == 1.0                       # cos(25 deg)=0.906 -> 9.4% darker than flat: looks flat at tol 0.10
    s, n = misleading_share(slope, down_az, m, el, 180.0, 0.05, 15.0)
    assert s == 0.0                       # tighter tolerance: no longer looks flat
    s, n = misleading_share(slope, down_az, m, el, 90.0, 0.10, 15.0)
    assert s == 0.0                       # sun in the east, slope faces the sun: strongly brighter than flat
    s, n = misleading_share(slope, down_az, m, el, 270.0, 0.10, 15.0)
    assert s == 0.0                       # sun in the west, slope faces away: strongly darker
    print("selftest passed: cross-sun slopes look flat, sun-facing and sun-away slopes do not (synthetic plane).")

def load_dem(path):
    import rasterio
    with rasterio.open(path) as ds:
        dem = ds.read(1).astype(float)
        cell = abs(ds.transform.a)
        nod = ds.nodata
    if nod is not None: dem[dem == nod] = np.nan
    if np.isnan(dem).any():
        print("WARNING: %.1f%% nodata pixels, filled with the median; check seams and gaps first" % (100 * np.isnan(dem).mean()))
        dem = np.where(np.isnan(dem), np.nanmedian(dem), dem)
    return dem, cell

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["selftest", "run"])
    ap.add_argument("dem", nargs="?")
    ap.add_argument("--bbox", nargs=4, type=int)
    ap.add_argument("--flat-tol", type=float, default=0.10)
    ap.add_argument("--out", default="misleading_results.csv")
    ap.add_argument("--smooth", type=float, default=0.0, help="gaussian smoothing in metres; 0 = none (default). 2 m would blur 4-5 m ridges")
    ap.add_argument("--all-azimuths", action="store_true", help="sweep 8 compass directions instead of the realistic Gale range")
    a = ap.parse_args()
    if a.cmd == "selftest":
        selftest(); sys.exit()
    dem, cell = load_dem(a.dem)
    print("DEM %s, cell %.3f m" % (dem.shape, cell))
    allrows = []
    for tol in sorted({0.05, a.flat_tol, 0.20}):
        allrows += table(dem, cell, a.bbox, tol, a.smooth, not a.all_azimuths)
    for r in allrows:
        print("tol %.2f band>=%2d el %2d  steep px %7d  looks-flat share min %.2f mean %.2f max %.2f" %
              (r['tol'], r['band_deg'], r['sun_el'], r['n_steep_px'], r['share_min'], r['share_mean'], r['share_max']))
    with open(a.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(allrows[0].keys())); w.writeheader(); w.writerows(allrows)
    print("wrote", a.out)
