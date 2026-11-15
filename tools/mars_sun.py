"""Mars sun position for VISOR (W3 "when to walk").

Mars24 algorithm exactly as printed on the NASA GISS page "Mars24 Sunclock - Algorithm and Worked Examples" (updated
2025-01-07; https://www.giss.nasa.gov/tools/mars24/help/algorithm.html), steps A-1 to D-6, which corrects typos in
Allison & McEwen (2000). Conventions: longitude in degrees WEST, latitude PLANETOGRAPHIC, azimuth clockwise from north.
Tested (selftest) against both GISS worked examples and against the HiRISE label of PSP_009149_1750 (the strip that covers
the VISOR patch). MSL sols: sol N starts when MSD + 137.42/360 = 49269 + N (Mars24 technical notes: the MSL clock starts
at midnight LMST for 137.42 E; K = 49269 from touchdown 2012-08-06 05:14:39 UTC on sol 0, an ASSUMED time).

The old simplified functions (sun_elevation, sun_azimuth, morning_ltst_for_elevation: no 0.25 sin Ls term, planetocentric
latitude) are kept unchanged at the end only for the archived "looks flat" analysis (misleading_slope.py, prep's azimuth
table). Do not use them for W3. Run: python tools/mars_sun.py selftest | table SOL
"""
import math
import sys

# ---------------------------------------------------------------- Mars24 (GISS algorithm page)
PBS_TERMS = [  # B-3: (A_i deg, tau_i Julian years, phi_i deg)
    (0.0071, 2.2353, 49.409), (0.0057, 2.7543, 168.173), (0.0039, 1.1177, 191.837), (0.0037, 15.7866, 21.736),
    (0.0021, 2.1354, 15.704), (0.0020, 2.4694, 95.528), (0.0018, 32.8493, 49.095),
]
MARS_A_KM, MARS_B_KM = 3396.19, 3376.20  # equatorial and polar radius (IAU 2000), for planetographic latitude
MSL_SOL0_K = 49269                      # sol N starts at MSD + 137.42/360 = MSL_SOL0_K + N (derivation in the docstring)
MSL_CLOCK_E = 137.42                    # longitude (E) of the MSL mission clock


def _s(x):
    return math.sin(math.radians(x))


def _c(x):
    return math.cos(math.radians(x))


def tt_minus_utc(jd_ut):
    """A-4 for dates after 1972 in the ranges VISOR uses: TAI-UTC 32 s (2006-2008), 33 s (2009-2012-06-30) ... 37 s from
    2017-01-01 (no leap second since, per the GISS page); TT-UTC = TAI-UTC + 32.184 s."""
    steps = [(2453736.5, 33.0), (2454832.5, 34.0), (2456109.5, 35.0), (2457204.5, 36.0), (2457754.5, 37.0)]
    tai = 32.0
    for jd, v in steps:
        if jd_ut >= jd:
            tai = v
    return tai + 32.184


def from_jd_tt(jd_tt):
    """Steps A-6 to C-5 from the TT Julian date: Ls, EOT, MST, M, R, ... (all degrees / hours as on the page)."""
    dt = jd_tt - 2451545.0                                                   # A-6
    m = 19.3871 + 0.52402073 * dt                                            # B-1
    a_fms = 270.3871 + 0.524038496 * dt                                      # B-2
    pbs = sum(a * _c(0.985626 * dt / tau + phi) for a, tau, phi in PBS_TERMS)  # B-3
    nu_m = ((10.691 + 3.0e-7 * dt) * _s(m) + 0.623 * _s(2 * m) + 0.050 * _s(3 * m) + 0.005 * _s(4 * m)
            + 0.0005 * _s(5 * m) + pbs)                                      # B-4
    ls = (a_fms + nu_m) % 360.0                                              # B-5
    eot = 2.861 * _s(2 * ls) - 0.071 * _s(4 * ls) + 0.002 * _s(6 * ls) - nu_m  # C-1 (degrees)
    msd = (jd_tt - 2451549.5) / 1.0274912517 + 44796.0 - 0.0009626           # C-2 (Mars Sol Date)
    mst = (24.0 * msd) % 24.0
    r = 1.52367934 * (1.00436 - 0.09309 * _c(m) - 0.004336 * _c(2 * m) - 0.00031 * _c(3 * m) - 0.00003 * _c(4 * m))  # D-2
    return dict(dt=dt, m=m % 360.0, a_fms=a_fms % 360.0, pbs=pbs, nu_m=nu_m, ls=ls, eot=eot, msd=msd, mst=mst, r=r)


def sun(jd_tt, west_lon, lat_pg):
    """Full sun geometry at a site: Ls, LTST (h), declination, elevation and azimuth (deg). C-3 to D-6."""
    b = from_jd_tt(jd_tt)
    lmst = (b["mst"] - west_lon / 15.0) % 24.0                               # C-3
    ltst = (lmst + b["eot"] / 15.0) % 24.0                                   # C-4
    lam_s = (15.0 * b["mst"] + b["eot"] + 180.0) % 360.0                     # C-5 (subsolar longitude, west)
    dec = math.degrees(math.asin(0.42565 * _s(b["ls"]))) + 0.25 * _s(b["ls"])  # D-1
    h = west_lon - lam_s                                                     # D-5
    z = math.degrees(math.acos(_s(dec) * _s(lat_pg) + _c(dec) * _c(lat_pg) * _c(h)))
    az = math.degrees(math.atan2(_s(h), _c(lat_pg) * math.tan(math.radians(dec)) - _s(lat_pg) * _c(h))) % 360.0  # D-6
    return dict(b, lmst=lmst, ltst=ltst, lam_s=lam_s, dec=dec, z=z, elevation=90.0 - z, azimuth=az)


def jd_ut_from_utc(year, month, day, hh=0, mm=0, ss=0.0):
    """Julian date (UT) of a Gregorian UTC date (A-1/A-2 via the usual calendar formula)."""
    if month <= 2:
        year, month = year - 1, month + 12
    a = year // 100
    bb = 2 - a + a // 4
    return (math.floor(365.25 * (year + 4716)) + math.floor(30.6001 * (month + 1)) + day + bb - 1524.5
            + (hh + mm / 60.0 + ss / 3600.0) / 24.0)


def sun_utc(jd_ut, west_lon, lat_pg):
    return sun(jd_ut + tt_minus_utc(jd_ut) / 86400.0, west_lon, lat_pg)


def jd_tt_from_msd(msd):
    """Inverse of C-2."""
    return (msd - 44796.0 + 0.0009626) * 1.0274912517 + 2451549.5


def sun_at_sol(sol, ltst_h, west_lon, lat_pg):
    """The sun on MSL sol `sol` when the site's local TRUE solar time is `ltst_h`. Starts from the mean-time guess and
    corrects for the equation of time (two iterations converge far below a second)."""
    msd = MSL_SOL0_K + sol - MSL_CLOCK_E / 360.0          # sol start: midnight LMST at 137.42 E
    # site LMST = clock LMST + (site E - 137.42)/15 h, and site west longitude = 360 - site E
    site_e = 360.0 - west_lon
    target_lmst = ltst_h
    for _ in range(3):
        guess = msd + (target_lmst - (site_e - MSL_CLOCK_E) / 15.0) / 24.0
        s = sun(jd_tt_from_msd(guess), west_lon, lat_pg)
        target_lmst = ltst_h - s["eot"] / 15.0
    return sun(jd_tt_from_msd(msd + (target_lmst - (site_e - MSL_CLOCK_E) / 15.0) / 24.0), west_lon, lat_pg)


def planetographic(lat_pc):
    """Planetocentric to planetographic latitude on the IAU 2000 Mars ellipsoid."""
    return math.degrees(math.atan((MARS_A_KM / MARS_B_KM) ** 2 * math.tan(math.radians(lat_pc))))


# ---------------------------------------------------------------- selftest
def selftest():
    def close(a, b, tol, what):
        assert abs(a - b) <= tol, f"{what}: {a} vs {b} (tol {tol})"

    # GISS example 1: 2000-01-06 00:00:00 UTC, 0 N, 0 W (values as printed on the page)
    jd = jd_ut_from_utc(2000, 1, 6)
    close(jd, 2451549.5, 1e-9, "JD_UT")
    e = sun_utc(jd, 0.0, 0.0)
    for k, v in (("m", 21.74558), ("a_fms", 272.74566), ("pbs", 0.00142), ("nu_m", 4.44193), ("ls", 277.18758), ("eot", -5.18774),
                 ("mst", 23.99425), ("ltst", 23.64840), ("lam_s", 174.72600), ("r", 1.39358), ("azimuth", 191.03905)):
        close(e[k], v, 1e-4 if k != "pbs" else 1e-5, f"example 1 {k}")
    close(e["dec"], -25.22825, 1e-3, "example 1 dec")
    close(e["z"], 154.26182, 1e-3, "example 1 Z")
    # GISS example 2: Spirit, millis 1073137591000, 184.702 W, -14.640 (planetographic, as printed)
    jd = 2440587.5 + 1073137591000 / 8.64e7
    close(jd, 2453008.07397, 1e-5, "example 2 JD_UT")
    e = sun_utc(jd, 184.702, -14.640)
    for k, v in (("m", 66.06858), ("a_fms", 317.09457), ("pbs", 0.01614), ("nu_m", 10.22959), ("ls", 327.32416),
                 ("eot", -12.77553), ("lam_s", 4.70500), ("r", 1.47767), ("azimuth", 179.99383)):
        close(e[k], v, 1e-4 if k != "pbs" else 1e-5, f"example 2 {k}")
    close(min(e["ltst"], 24 - e["ltst"]), 0.0, 1e-3, "example 2 LTST (midnight)")
    close(e["dec"], -13.42065, 1e-3, "example 2 dec")
    close(e["z"], 151.93895, 1e-3, "example 2 Z")
    # Independent check: HiRISE PSP_009149_1750 label (START_TIME 2008-07-09T09:24:54, SOLAR_LONGITUDE 96.272867,
    # LOCAL_TIME 15.48044, INCIDENCE_ANGLE 58.808048 -> sun 31.19 deg high; derived azimuth 303.1 deg) at the
    # observation centre -4.799 N, 137.412 E (uahirise.org). Tolerances cover the label's centre-vs-start difference.
    jd = jd_ut_from_utc(2008, 7, 9, 9, 24, 54)
    h = sun_utc(jd, 360.0 - 137.412, planetographic(-4.799))
    close(h["ls"], 96.272867, 0.01, "HiRISE Ls")
    close(h["ltst"], 15.48044, 0.02, "HiRISE local time")
    close(h["elevation"], 90.0 - 58.808048, 0.5, "HiRISE sun elevation")
    close(h["azimuth"], 303.1, 1.0, "HiRISE sun azimuth")
    # sols: sol_at -> LTST round trip, and the sun is highest near noon
    s = sun_at_sol(4718, 12.0, 222.62, -4.87)
    close(s["ltst"], 12.0, 1e-4, "LTST round trip")
    assert sun_at_sol(4718, 9.0, 222.62, -4.87)["elevation"] < s["elevation"] > sun_at_sol(4718, 15.0, 222.62, -4.87)["elevation"]
    close(planetographic(-4.814633), -4.87, 0.01, "planetographic latitude at the patch")
    print("selftest passed: GISS examples 1 and 2, HiRISE PSP_009149_1750 label, sol/LTST round trip")


def table(sol, west_lon=None, lat_pg=None):
    """Sun by whole LTST hour on a sol, at the VISOR exit stop (from assets/stops.json + PLACES planetodetic latitude)."""
    if west_lon is None:
        import json, os
        st = json.load(open(os.path.join(os.path.dirname(__file__), "..", "assets", "stops.json"), encoding="utf-8"))["stops"][0]
        west_lon, lat_pg = 360.0 - st["lon"], planetographic(st["lat"])
    print(f"sol {sol}, site {west_lon:.3f} W, {lat_pg:.4f} planetographic")
    for h in range(6, 19):
        s = sun_at_sol(sol, float(h), west_lon, lat_pg)
        print(f"  LTST {h:02d}:00  Ls {s['ls']:6.2f}  elevation {s['elevation']:6.2f}  azimuth {s['azimuth']:6.1f}")


# ---------------------------------------------------------------- legacy (archived looks-flat analysis only)
LAT = -4.8      # Gale, degrees (planetocentric approximation; legacy)
OBLIQ = 25.19   # degrees; NASA Mars fact sheet (legacy formula, no 0.25 sin Ls term)


def sun_elevation(ls_deg, ltst_hours, lat_deg=LAT, obliq_deg=OBLIQ):
    """LEGACY (superseded by sun_at_sol): simplified declination, planetocentric latitude."""
    dec = math.asin(math.sin(math.radians(obliq_deg)) * math.sin(math.radians(ls_deg)))
    h = math.radians((ltst_hours - 12.0) * 15.0)
    la = math.radians(lat_deg)
    s = math.sin(la) * math.sin(dec) + math.cos(la) * math.cos(dec) * math.cos(h)
    return math.degrees(math.asin(s))


def sun_azimuth(ls_deg, ltst_hours, lat_deg=LAT, obliq_deg=OBLIQ):
    """LEGACY (superseded by sun_at_sol)."""
    dec = math.asin(math.sin(math.radians(obliq_deg)) * math.sin(math.radians(ls_deg)))
    h = math.radians((ltst_hours - 12.0) * 15.0)
    la = math.radians(lat_deg)
    y = -math.cos(dec) * math.sin(h)
    x = math.sin(dec) * math.cos(la) - math.cos(dec) * math.sin(la) * math.cos(h)
    return math.degrees(math.atan2(y, x)) % 360.0


def morning_ltst_for_elevation(ls_deg, el_deg, lat_deg=LAT):
    """LEGACY (superseded): morning LTST at which the sun reaches el_deg, by bisection."""
    lo, hi = 5.0, 12.0
    if sun_elevation(ls_deg, hi, lat_deg) < el_deg:
        return None
    if sun_elevation(ls_deg, lo, lat_deg) > el_deg:
        return lo
    for _ in range(60):
        mid = 0.5 * (lo + hi)
        if sun_elevation(ls_deg, mid, lat_deg) < el_deg:
            lo = mid
        else:
            hi = mid
    return 0.5 * (lo + hi)


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[:1] == ["table"]:
        table(int(a[1]) if len(a) > 1 else 4718)
    else:
        selftest()
