"""Cross-check of VISOR's sun model (tools/mars_sun.py = Mars24 algorithm; route-core sun.rs is its port) against NASA Mars
Trek's sun-angle service.

The service (found in Trek's own app code, jpl/config/config.json services.sunAngleService and the SunAngle dialog):
  https://trek.nasa.gov/getAzElfromT1/LTD002=Mars,{lat},{lon},{alt_km},{lat},{lon},1,1/Sun/{UTC start}/{UTC end}/{step s}
returns XML samples of the Sun's azimuth and elevation seen from that point on Mars (NAIF ids 499 Mars, 10 Sun: a SPICE
ephemeris computation, independent of the Mars24 formulas). Trek's latitude convention is assumed planetocentric (Trek's
maps are planetocentric); ours is converted to planetographic as mars_sun.py does; the difference is ~0.06 deg here.

RULE (written in this file before the first run; do not change after seeing a result):
  samples every 600 s across each stop sol's daytime (LTST 06:00 to 18:00 by our model), compared only where both models
  put the sun between 5 and 80 deg (azimuth is ill-defined near the zenith); PASS if max |d elevation| <= 0.5 deg and
  max |d azimuth| <= 1.0 deg over all compared samples; else FAIL (reported, not tuned).
Outputs: data/interim/trek_sun/trek_sun_check.json (every sample) and assets/sun_check.json (summary for the app).
Usage: python tools/trek_sun.py run | selftest
"""
import json
import math
import os
import sys
import urllib.request
import xml.etree.ElementTree as ET

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402
import mars_sun as ms  # noqa: E402

URL = "https://trek.nasa.gov/getAzElfromT1/LTD002=Mars,{lat},{lon},0,{lat},{lon},1,1/Sun/{t0}/{t1}/{step}"
STEP_S = 600
EL_RANGE = (5.0, 80.0)
TOL = dict(elevation=0.5, azimuth=1.0)
OUT = os.path.join(config.INTERIM, "trek_sun")


def utc_from_jd(jd):
    """Gregorian UTC 'YYYY-MM-DDTHH:MM:SS' from a Julian date (UT)."""
    z = math.floor(jd + 0.5)
    f = jd + 0.5 - z
    a = z if z < 2299161 else z + 1 + int((z - 1867216.25) / 36524.25) - int(int((z - 1867216.25) / 36524.25) / 4)
    b = a + 1524
    c = int((b - 122.1) / 365.25)
    d = int(365.25 * c)
    e = int((b - d) / 30.6001)
    day = b - d - int(30.6001 * e)
    month = e - 1 if e < 14 else e - 13
    year = c - 4716 if month > 2 else c - 4715
    secs = round(f * 86400)
    if secs >= 86400:  # rounding to the next day is avoided by the caller's 600 s grid; guard anyway
        secs = 86399
    return f"{year:04d}-{month:02d}-{day:02d}T{secs // 3600:02d}:{secs // 60 % 60:02d}:{secs % 60:02d}"


def jd_from_doy(s):
    """'2025-317T01:00:00.000000' (year-day-of-year UTC) to a Julian date (UT)."""
    date, tm = s.split("T")
    y, doy = int(date[:4]), int(date[5:])
    hh, mm, ss = tm.split(":")
    return ms.jd_ut_from_utc(y, 1, 1) + doy - 1 + (int(hh) + int(mm) / 60 + float(ss) / 3600) / 24


def parse(xml_text):
    root = ET.fromstring(xml_text)
    out = []
    for smp in root.iter("sample"):
        out.append((smp.findtext("time"), float(smp.findtext("Az")), float(smp.findtext("El"))))
    return out


def daytime_jd(sol, west_lon, lat_pg):
    """Julian dates (UT) of LTST 06:00 and 18:00 on the sol, by our model."""
    def jd_of(h):
        s = ms.sun_at_sol(sol, h, west_lon, lat_pg)
        jd_tt = ms.jd_tt_from_msd(s["msd"])
        return jd_tt - ms.tt_minus_utc(jd_tt) / 86400.0
    return jd_of(6.0), jd_of(18.0)


def run():
    st = json.load(open(os.path.join(config.ASSETS, "stops.json"), encoding="utf-8"))
    lat_pc, lon_e = st["stops"][0]["lat"], st["stops"][0]["lon"]
    west, lat_pg = 360.0 - lon_e, ms.planetographic(lat_pc)
    os.makedirs(OUT, exist_ok=True)
    rows = []
    for sol in sorted({s["sol"] for s in st["stops"]}):
        j0, j1 = daytime_jd(sol, west, lat_pg)
        j0 = math.floor(j0 * 144) / 144  # whole 600 s
        url = URL.format(lat=f"{lat_pc:.6f}", lon=f"{lon_e:.6f}", t0=utc_from_jd(j0), t1=utc_from_jd(j1), step=STEP_S)
        req = urllib.request.Request(url, headers={"User-Agent": "VISOR (Space Apps 2026 entry)"})
        text = urllib.request.urlopen(req, timeout=120).read().decode("latin-1")
        open(os.path.join(OUT, f"trek_sun_sol{sol}.xml"), "w", encoding="utf-8").write(text)
        for t, az, el in parse(text):
            ours = ms.sun_utc(jd_from_doy(t), west, lat_pg)
            rows.append(dict(sol=sol, utc=t, trek_el=el, trek_az=az, ours_el=round(ours["elevation"], 4), ours_az=round(ours["azimuth"], 4)))
        print(f"sol {sol}: {sum(1 for r in rows if r['sol'] == sol)} samples")
    used = [r for r in rows if EL_RANGE[0] <= r["trek_el"] <= EL_RANGE[1] and EL_RANGE[0] <= r["ours_el"] <= EL_RANGE[1]]
    de = [abs(r["trek_el"] - r["ours_el"]) for r in used]
    da = [abs((r["trek_az"] - r["ours_az"] + 180) % 360 - 180) for r in used]
    ok = bool(used) and max(de) <= TOL["elevation"] and max(da) <= TOL["azimuth"]
    med = lambda v: sorted(v)[len(v) // 2]
    summary = dict(outcome="PASS" if ok else "FAIL", n=len(used), n_all=len(rows), sols=sorted({r["sol"] for r in rows}),
                   max_abs_d_elevation_deg=round(max(de), 4) if de else None, median_abs_d_elevation_deg=round(med(de), 4) if de else None,
                   max_abs_d_azimuth_deg=round(max(da), 4) if da else None, median_abs_d_azimuth_deg=round(med(da), 4) if da else None,
                   rule=dict(step_s=STEP_S, elevation_range_deg=list(EL_RANGE), tolerance_deg=TOL, written="in tools/trek_sun.py before the first run"),
                   source=dict(name="NASA Mars Trek sun-angle service (SPICE; NAIF 499 Mars, 10 Sun)", url="https://trek.nasa.gov/mars/",
                               endpoint="https://trek.nasa.gov/getAzElfromT1"),
                   site=dict(lat_planetocentric=lat_pc, lon_east=lon_e, note="Trek latitude assumed planetocentric; ours planetographic"))
    json.dump(dict(summary=summary, samples=rows), open(os.path.join(OUT, "trek_sun_check.json"), "w"), indent=1)
    json.dump(summary, open(os.path.join(config.ASSETS, "sun_check.json"), "w"), indent=1)
    print(json.dumps(summary, indent=1))


def selftest():
    assert utc_from_jd(ms.jd_ut_from_utc(2025, 11, 13, 1, 0, 0)) == "2025-11-13T01:00:00"
    assert abs(jd_from_doy("2025-317T01:00:00.000000") - ms.jd_ut_from_utc(2025, 11, 13, 1)) < 1e-9, "day 317 of 2025 = Nov 13"
    x = parse('<MPG><ltd><samples><sample><time>2025-317T00:00:00.000000</time><Az id="0">273.76</Az><El id="0">-1.70</El></sample></samples></ltd></MPG>')
    assert x == [("2025-317T00:00:00.000000", 273.76, -1.70)], x
    print("trek_sun selftest: PASS")


if __name__ == "__main__":
    {"run": run, "selftest": selftest}.get(sys.argv[1] if len(sys.argv) > 1 else "selftest", selftest)()
