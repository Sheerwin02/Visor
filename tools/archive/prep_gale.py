"""VISOR data prep for the Wednesday gate. Run on YOUR machine.
  pip install rasterio numpy scipy matplotlib
  python prep_gale.py stops  localized_pos.csv  stops.csv            # pick 5 PLACES stops in the boxwork
  python prep_gale.py map    DEM.tif localized_pos.csv out.png       # hillshade + rover path + seam check
  python prep_gale.py tilt   DEM.tif localized_pos.csv               # DEM slope vs rover pitch/roll (validation rule)
  python prep_gale.py selftest                                       # tests the array code on synthetic data (no files)
STATUS: array functions tested on synthetic data only. The rasterio loader and the PLACES column matching are UNTESTED;
the PLACES file may use other header names or no header -- the script prints what it found, fix mapping in COLS if needed.
Yaw/pitch/roll sign conventions are UNVERIFIED (read the PLACES format document); 'tilt' compares absolute values only."""
import sys, csv, numpy as np
R_MARS = 3396190.0   # spherical approximation -- assumption
BOX = (4534, 4865)   # boxwork sol range found earlier in PLACES
COLS = {'sol':['sol'], 'lat':['lat'], 'lon':['lon'], 'pitch':['pitch'], 'roll':['roll'], 'yaw':['yaw']}

# ---------- PLACES ----------
def load_places(path):
    rows = list(csv.reader(open(path)))
    head = [h.strip().lower() for h in rows[0]]
    idx = {}
    for k, subs in COLS.items():
        for i, h in enumerate(head):
            if any(h == s or h.startswith(s) or s in h for s in subs) and i not in idx.values():
                idx[k] = i; break
    print("PLACES header:", rows[0]); print("matched columns:", idx)
    if not all(k in idx for k in ('sol', 'lat', 'lon')):
        raise SystemExit("could not match sol/lat/lon columns; edit COLS")
    out = []
    for r in rows[1:]:
        try:
            d = {k: float(r[i]) for k, i in idx.items()}
        except (ValueError, IndexError):
            continue
        out.append(d)
    return out

def pick_stops(rows, n=5):
    box = [r for r in rows if BOX[0] <= r['sol'] <= BOX[1]]
    if len(box) < n: raise SystemExit(f"only {len(box)} boxwork rows")
    ids = np.linspace(0, len(box) - 1, n).astype(int)
    return [box[i] for i in ids]
# NOTE: for the three named targets, the NASA blog places drilling at sols ~4709-4715 (ridge, Nevado Sajama) and the hollow
# (Valle de la Luna) -- check which PLACES rows match and swap them in by hand.

# ---------- terrain maths (tested on synthetic) ----------
def hillshade(dem, cell, az=315., alt=45.):
    gy, gx = np.gradient(dem, cell)
    slope = np.arctan(np.hypot(gx, gy)); aspect = np.arctan2(gx, -gy)
    a, h = np.radians(360 - az + 90), np.radians(alt)
    return np.clip(np.sin(h) * np.cos(slope) + np.cos(h) * np.sin(slope) * np.cos(a - aspect), 0, 1)

def slope_deg(dem, cell, smooth_m=2.0):
    from scipy.ndimage import gaussian_filter
    d = gaussian_filter(dem, smooth_m / cell) if smooth_m > 0 else dem
    gy, gx = np.gradient(d, cell)       # gy: d/d(row) ; rows increase southward
    return np.degrees(np.arctan(np.hypot(gx, gy))), gx, -gy   # gx east, gn north

def seam_check(dem, cell, win=50):
    """Heuristic only: flags straight rows/columns where mean |step| is >4x the median (possible mosaic seam)."""
    res = []
    for ax, name in ((0, 'row'), (1, 'column')):
        step = np.abs(np.diff(dem, axis=ax))
        prof = np.nanmean(step, axis=1 - ax); med = np.nanmedian(prof)
        bad = np.where(prof > 4 * med + 1e-9)[0]
        res.append((name, len(bad), bad[:5].tolist()))
    return res

def tilt_compare(dem, cell, r0c0s, yaws, pitches, rolls, smooth_m=2.0, yaw_conv='north_cw'):
    """Along/across slope at rover positions vs |pitch|,|roll|. yaw_conv unverified: 'north_cw' or 'east_ccw'."""
    _, gx, gn = slope_deg(dem, cell, smooth_m)
    out = []
    for (r, c), yaw, p, ro in zip(r0c0s, yaws, pitches, rolls):
        y = np.radians(yaw)
        h = (np.sin(y), np.cos(y)) if yaw_conv == 'north_cw' else (np.cos(y), np.sin(y))   # (east, north)
        along = np.degrees(np.arctan(gx[r, c] * h[0] + gn[r, c] * h[1]))
        across = np.degrees(np.arctan(gx[r, c] * h[1] - gn[r, c] * h[0]))
        out.append((abs(along) - abs(p), abs(across) - abs(ro)))
    e = np.abs(np.array(out)); return np.median(e, axis=0), np.percentile(e, 90, axis=0)

# ---------- DEM access ----------
class RasterioSource:
    def __init__(self, path):
        import rasterio; self.ds = rasterio.open(path)
        self.cell = np.radians(abs(self.ds.transform.a)) * R_MARS   # assumes degrees/pixel
    def rc(self, lat, lon): return self.ds.index(lon, lat)
    def window(self, lat, lon, half_m):
        from rasterio.windows import Window
        r0, c0 = self.rc(lat, lon); n = int(half_m / self.cell)
        a = self.ds.read(1, window=Window(c0 - n, r0 - n, 2 * n, 2 * n), boundless=True, fill_value=np.nan).astype('float64')
        if self.ds.nodata is not None: a[a == self.ds.nodata] = np.nan
        return a, (r0 - n, c0 - n)

class ArraySource:   # for the self-test
    def __init__(self, arr, cell, lat0, lon0): self.arr, self.cell, self.lat0, self.lon0 = arr, cell, lat0, lon0
    def rc(self, lat, lon):
        d = np.degrees(self.cell / R_MARS)
        return int(round((self.lat0 - lat) / d)), int(round((lon - self.lon0) / d))
    def window(self, lat, lon, half_m):
        r0, c0 = self.rc(lat, lon); n = int(half_m / self.cell)
        return self.arr[r0 - n:r0 + n, c0 - n:c0 + n].copy(), (r0 - n, c0 - n)

def make_map(src, rows, out, half=2500.):
    import matplotlib; matplotlib.use('Agg'); import matplotlib.pyplot as plt
    box = [r for r in rows if BOX[0] <= r['sol'] <= BOX[1]]
    lat = np.mean([r['lat'] for r in box]); lon = np.mean([r['lon'] for r in box])
    dem, (ro, co) = src.window(lat, lon, half)
    print(f"nodata fraction in map window: {np.isnan(dem).mean()*100:.1f}%; seam heuristic:", seam_check(np.where(np.isnan(dem), np.nanmedian(dem), dem), src.cell))
    hs = hillshade(np.where(np.isnan(dem), np.nanmedian(dem), dem), src.cell)
    fig, ax = plt.subplots(figsize=(10, 10)); ax.imshow(hs, cmap='gray')
    pts = np.array([src.rc(r['lat'], r['lon']) for r in box]) - [ro, co]
    ax.plot(pts[:, 1], pts[:, 0], 'r.-', ms=3, lw=.8); ax.set_title("DEM hillshade + PLACES path (check seams, blur near the path)")
    fig.savefig(out, dpi=110); print("wrote", out)

def selftest():
    rng = np.random.default_rng(1); n = 700; cell = 1.0
    from scipy.ndimage import gaussian_filter
    dem = gaussian_filter(rng.standard_normal((n, n)), 15) * 60
    # known plane tilt: add 10 deg slope toward the east to test tilt_compare
    dem += np.tan(np.radians(10)) * np.arange(n)[None, :] * cell
    d = np.degrees(cell / R_MARS); lat0, lon0 = -4.0, 137.0
    src = ArraySource(dem, cell, lat0, lon0)
    pts = [(lat0 - 350 * d + k * 0.0003, lon0 + 350 * d + k * 0.0003) for k in range(10)]
    rows = [dict(sol=4534 + 30 * k, lat=a, lon=b, pitch=0., roll=0., yaw=90.) for k, (a, b) in enumerate(pts)]
    st = pick_stops(rows, 5); assert len(st) == 5
    hs = hillshade(dem, cell); assert 0 <= hs.min() and hs.max() <= 1
    s, gx, gn = slope_deg(dem, cell, 0); assert abs(np.median(gx) - np.tan(np.radians(10))) < 0.15, np.median(gx)
    # step seam: add 30 m step down a column; heuristic must flag it
    d2 = dem.copy(); d2[:, 400:] += 30; sc = seam_check(d2, cell); assert sc[1][1] >= 1 and 399 in sc[1][2], sc
    assert seam_check(dem, cell)[1][1] <= 3
    rc = [src.rc(r['lat'], r['lon']) for r in rows]; rc = [(a, b) for a, b in rc]
    # facing east (yaw 90, north_cw): along-slope ~10 deg; claim pitch 10 -> small median error; wrong pitch 0 -> ~10
    med, p90 = tilt_compare(dem, cell, rc, [90] * 10, [10] * 10, [0] * 10, 2.0)
    med0, _ = tilt_compare(dem, cell, rc, [90] * 10, [0] * 10, [0] * 10, 2.0)
    assert med[0] < 3 and med0[0] > 6, (med, med0)
    make_map(src, rows, '/tmp/selftest_map.png', half=200.)
    print("selftest OK: array code runs and recovers a 10 degree tilt, flags a seam step. Loader and PLACES parsing untested.")

if __name__ == '__main__':
    cmd = sys.argv[1] if len(sys.argv) > 1 else ''
    if cmd == 'selftest': selftest()
    elif cmd == 'stops':
        rows = load_places(sys.argv[2]); st = pick_stops(rows)
        with open(sys.argv[3], 'w') as f:
            for r in st: f.write(f"sol{int(r['sol'])},{r['lat']},{r['lon']}\n")
        print("wrote", sys.argv[3], "- add the ridge (Nevado Sajama, ~sol 4709-4715) and hollow (Valle de la Luna) rows by hand")
    elif cmd == 'map':
        make_map(RasterioSource(sys.argv[2]), load_places(sys.argv[3]), sys.argv[4])
    elif cmd == 'tilt':
        src = RasterioSource(sys.argv[2]); rows = [r for r in load_places(sys.argv[3]) if BOX[0] <= r['sol'] <= BOX[1] and all(k in r for k in ('pitch', 'roll', 'yaw'))]
        for smooth in (2.0, 5.0):
            for conv in ('north_cw', 'east_ccw'):
                es = []
                for r in rows:
                    dem, (ro, co) = src.window(r['lat'], r['lon'], 40.)
                    if np.isnan(dem).mean() > 0.02: continue
                    dem = np.where(np.isnan(dem), np.nanmedian(dem), dem); k = dem.shape[0] // 2
                    es.append((tilt_compare(dem, src.cell, [(k, k)], [r['yaw']], [r['pitch']], [r['roll']], smooth, conv)[0]))
                if es:
                    e = np.array(es); print(f"smooth {smooth} m yaw {conv}: N={len(e)} median |along-pitch| {np.median(e[:,0]):.1f} deg, |across-roll| {np.median(e[:,1]):.1f} deg")
        print("Report both smoothings; the yaw convention that matches is unverified -- read the PLACES format doc. Slope agreement at rover scale only.")
    else: print(__doc__)
