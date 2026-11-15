"""Daily weather at Curiosity for the study sols (conditions): REMS values from the NASA/CAB Mars weather feed.

Why the feed and not the PDS tables: the PDS Atmospheres REMS bundle (doi:10.17189/vwva-ms24) reaches sol 4843, but its
per-sol tables are 12-21 MB each and their columns are not yet read here. The feed gives one summary per sol and is meant
for outreach; its disclaimer travels with the data and is shown in the app. Values are copied, never edited or filled.

Checks (rule fixed before the first run): the feed's Ls (whole degrees) must agree with tools/mars_sun.py within
1.0 deg on every study sol, and the feed's Earth date (the Earth day at the sol's local midday) is compared with ours.
Result of the first run: FAIL (max 1.294 deg). Post-hoc explanation, not a test: the feed's Ls is the whole-degree
floor of Ls at the START of the sol (matches ours exactly on 321 of 323 sols). The FAIL is recorded and the bundle built
anyway (temperatures and pressure do not depend on Ls); the check and the explanation travel in conditions.json.

    python tools/conditions.py fetch      # download the feed into data/raw/conditions/ (with fetch time and sha256)
    python tools/conditions.py build      # write assets/conditions.json, then `python tools/build_bundle.py rehash`
    python tools/conditions.py selftest
"""
import datetime as dt
import hashlib
import json
import math
import os
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402
import mars_sun  # noqa: E402

FEED_URL = "https://mars.nasa.gov/rss/api/?feed=weather&category=msl&feedtype=json"
RAW_DIR = os.path.join(config.RAW, "conditions")
RAW = os.path.join(RAW_DIR, "msl_weather.json")
META = os.path.join(RAW_DIR, "msl_weather.meta.json")
OUT_NAME = "conditions.json"
LS_TOLERANCE_DEG = 1.0
PDS_REMS = dict(name="MSL REMS bundle, PDS Atmospheres node", doi="10.17189/vwva-ms24",
                url="https://atmos.nmsu.edu/PDS/data/PDS4/msl_rems_bundle/", last_sol=4843)
# site for the sun checks: the exit stop of assets/stops.json (as the app's sun model); the selftest uses Gale's round values
SELFTEST_SITE = (137.4, -4.8)


def fetch():
    os.makedirs(RAW_DIR, exist_ok=True)
    req = urllib.request.Request(FEED_URL, headers={"User-Agent": "visor-pipeline"})
    data = urllib.request.urlopen(req, timeout=60).read()
    json.loads(data)  # must parse
    open(RAW, "wb").write(data)
    meta = dict(url=FEED_URL, fetched_utc=dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
    json.dump(meta, open(META, "w"), indent=1)
    print(f"feed saved: {RAW} ({len(data) / 1e6:.2f} MB, fetched {meta['fetched_utc']})")


def _num(v):
    """Feed values are strings; '--' or empty means not available (kept as None, never filled)."""
    if v in (None, "", "--"):
        return None
    try:
        x = float(v)
    except ValueError:
        return None
    return int(x) if x.is_integer() else x


def _text(v):
    return None if v in (None, "", "--") else str(v)


def exit_site(out=config.ASSETS):
    st = json.load(open(os.path.join(out, "stops.json"), encoding="utf-8"))["stops"][0]
    return st["lon"], st["lat"]


def ours(sol, site=SELFTEST_SITE):
    """Our Ls (deg) at local true noon, and the UTC date of that moment, from mars_sun (Mars24). site = (east lon, lat pc)."""
    s = mars_sun.sun_at_sol(sol, 12.0, 360.0 - site[0], mars_sun.planetographic(site[1]))
    jd_tt = mars_sun.jd_tt_from_msd(s["msd"])
    jd_ut = jd_tt - mars_sun.tt_minus_utc(jd_tt) / 86400.0
    date = (dt.datetime(2000, 1, 1, 12, tzinfo=dt.timezone.utc) + dt.timedelta(days=jd_ut - 2451545.0)).date().isoformat()
    return s["ls"], date


def ls_diff(a, b):
    return abs((a - b + 180.0) % 360.0 - 180.0)


def build_doc(feed, meta, first=config.SOL_FIRST, last=config.SOL_LAST, site=SELFTEST_SITE):
    recs = {}
    for r in feed["soles"]:
        try:
            s = int(r["sol"])
        except (KeyError, ValueError):
            continue
        if first <= s <= last:
            recs[s] = r
    rows, worst, date_same = [], 0.0, 0
    for s in sorted(recs):
        r = recs[s]
        ls_feed = _num(r.get("ls"))
        ls_ours, date_ours = ours(s, site)
        if ls_feed is not None:
            worst = max(worst, ls_diff(ls_feed, ls_ours))
        date_same += r.get("terrestrial_date") == date_ours
        rows.append(dict(sol=s, date=_text(r.get("terrestrial_date")), ls=ls_feed, season=_text(r.get("season")),
                         air_min_c=_num(r.get("min_temp")), air_max_c=_num(r.get("max_temp")),
                         ground_min_c=_num(r.get("min_gts_temp")), ground_max_c=_num(r.get("max_gts_temp")),
                         pressure_pa=_num(r.get("pressure")), uv_index=_text(r.get("local_uv_irradiance_index")),
                         sky=_text(r.get("atmo_opacity")), sunrise=_text(r.get("sunrise")), sunset=_text(r.get("sunset"))))
    missing = [s for s in range(first, last + 1) if s not in recs]
    check = dict(rule=f"feed Ls within {LS_TOLERANCE_DEG} deg of tools/mars_sun.py at local noon on every sol",
                 n=len(rows), ls_max_abs_diff_deg=round(worst, 3), passed=worst <= LS_TOLERANCE_DEG,
                 date_same_as_ours=date_same)
    sod = [r for r in recs.values() if _num(r.get("ls")) is not None]
    same = sum(_num(r["ls"]) == math.floor(mars_sun.sun_at_sol(int(r["sol"]), 0.0, 360.0 - site[0],
                                                               mars_sun.planetographic(site[1]))["ls"]) for r in sod)
    check["post_hoc"] = dict(note="explanation found after the result, not a pre-set test: the feed's Ls looks like the "
                                  "whole-degree floor of Ls at the start of the sol (local midnight)",
                             floor_at_sol_start_exact=same, n=len(sod))
    disc = " ".join(feed.get("descriptions", {}).get("disclaimer_en", "").split())
    return dict(
        source=dict(name="Mars weather at Curiosity (REMS), Centro de Astrobiologia (CAB) / NASA", url=FEED_URL,
                    fetched_utc=meta.get("fetched_utc"), raw_sha256=meta.get("sha256"), disclaimer=disc),
        units=dict(air_c="deg C (the feed does not state its temperature unit; the values only make sense as deg C)",
                   ground_c="deg C (as above)", pressure_pa="Pa (feed description gives pressure in pascals)"),
        science_archive=PDS_REMS,
        sols=rows, missing_sols=missing, check=check)


def build(out=config.ASSETS):
    feed = json.load(open(RAW, encoding="utf-8"))
    meta = json.load(open(META)) if os.path.exists(META) else {}
    doc = build_doc(feed, meta, site=exit_site(out))
    c = doc["check"]
    print(f"sols {config.SOL_FIRST}-{config.SOL_LAST}: {c['n']} with a record, {len(doc['missing_sols'])} without "
          f"({doc['missing_sols'][:12]}{' ...' if len(doc['missing_sols']) > 12 else ''})")
    print(f"Ls check: max |feed - ours| {c['ls_max_abs_diff_deg']} deg (tolerance {LS_TOLERANCE_DEG}) -> {'PASS' if c['passed'] else 'FAIL'}")
    print(f"Earth date same as ours on {c['date_same_as_ours']} of {c['n']} sols")
    if not c["passed"]:
        print("Ls check FAILED as written; recorded in conditions.json with the post-hoc explanation")
    p = os.path.join(out, OUT_NAME)
    json.dump(doc, open(p, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"written {p} ({os.path.getsize(p) / 1e3:.0f} kB); now run `python tools/build_bundle.py rehash`")


def selftest():
    # a synthetic feed: one good sol, one with '--' values, one outside the range; our own Ls is used as the feed's
    ls0, d0 = ours(4600)
    feed = dict(descriptions=dict(disclaimer_en="  outreach \n only "), soles=[
        dict(sol="4600", ls=str(round(ls0)), terrestrial_date=d0, min_temp="-80", max_temp="-20", min_gts_temp="-90",
             max_gts_temp="5", pressure="800", local_uv_irradiance_index="Moderate", atmo_opacity="Sunny", season="Month 3",
             sunrise="05:57", sunset="17:39"),
        dict(sol="4601", ls="--", terrestrial_date="x", min_temp="--", max_temp="", pressure="--"),
        dict(sol="9999", ls="1")])
    doc = build_doc(feed, {}, 4600, 4602)
    assert [r["sol"] for r in doc["sols"]] == [4600, 4601] and doc["missing_sols"] == [4602], doc
    a, b = doc["sols"]
    assert a["air_min_c"] == -80 and a["pressure_pa"] == 800 and a["uv_index"] == "Moderate"
    assert b["air_min_c"] is None and b["air_max_c"] is None and b["ls"] is None, "missing stays missing"
    assert doc["check"]["passed"] and doc["check"]["date_same_as_ours"] == 1
    assert doc["source"]["disclaimer"] == "outreach only"
    assert ls_diff(359.6, 0.2) < 0.61 and ls_diff(10, 12) == 2
    feed["soles"][0]["ls"] = str(round(ls0 + 5))
    assert not build_doc(feed, {}, 4600, 4602)["check"]["passed"], "a 5 deg Ls error must fail"
    assert not math.isnan(ls0)
    print("conditions selftest OK")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "selftest"
    {"fetch": fetch, "build": build, "selftest": selftest}[cmd]()
