"""Radiation at the stops (conditions): MSL RAD dose rates from the PDS RDR, plus space-weather events from DONKI.

Rule (fixed before any download or value):
- data: MSL RAD RDR (PDS PPI, MSL-M-RAD-3-RDR-V1.0), one file per sol, only the stop sols in stops.json;
- field: [DOSIMETRY_TOTAL_DOSE_B] (silicon, microGray/hour, RAD RDR SIS 4.2.1.6) primary, [DOSIMETRY_TOTAL_DOSE_E] beside it;
- format check: each dose section holds exactly one number, else stop and report (no guessed parse);
- per sol: N, median, p10, p90 of B and E; first/last observation start (LMST); shortest/longest gap between starts; no time
  weighting; QC values reported; solar-event mode (QC bit 00001000) reported separately; non-finite/negative excluded and
  counted; INVALID if a median is outside 1-200 microGray/hour (parse guard; bounds are an assumption);
- shown as context only: no mSv or tissue conversion, no comparison with published averages, never a route limit;
- DONKI: dated CME and SEP events 2025-05-08..2026-04-13, listed only if the record names Mars or MSL among affected targets.

    python tools/rad.py fetch       # RAD files for the stop sols + DONKI (api.nasa.gov, DEMO_KEY, 59-day windows)
    python tools/rad.py build       # assets/radiation.json, then `python tools/build_bundle.py rehash`
    python tools/rad.py selftest
"""
import datetime as dt
import json
import math
import os
import re
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(__file__))
import config  # noqa: E402

RAW = os.path.join(config.RAW, "rad")
OUT_NAME = "radiation.json"
RDR_ROOT = "https://pds-ppi.igpp.ucla.edu/data/MSL-M-RAD-3-RDR-V1.0/"
DOI_NOTE = "MSL RAD RDR, PDS Planetary Plasma Interactions node, MSL-M-RAD-3-RDR-V1.0"
DONKI = "https://api.nasa.gov/DONKI/{kind}?startDate={a}&endDate={b}&api_key=DEMO_KEY"
DONKI_RANGE = (dt.date(2025, 5, 8), dt.date(2026, 4, 13))   # study sols 4534-4865 in Earth dates (conditions feed)
MEDIAN_GUARD = (1.0, 200.0)                                  # microGray/hour; rule: a parse guard, an assumption
SOLAR_EVENT_BIT = 0b00001000
TARGET_RE = re.compile(r"\b(Mars|MSL|Curiosity)\b")


def _get(url, tries=4):
    for k in range(tries):
        try:
            return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "visor-pipeline"}), timeout=120).read()
        except urllib.error.HTTPError as e:
            if e.code != 429 or k == tries - 1:
                raise
            time.sleep(60 * (k + 1))
    raise RuntimeError(url)


def stop_sols():
    st = json.load(open(os.path.join(config.ASSETS, "stops.json"), encoding="utf-8"))["stops"]
    return sorted({int(s["sol"]) for s in st})


def fetch_donki():
    a0, end = DONKI_RANGE
    for kind in ("SEP", "CME"):
        p = os.path.join(RAW, f"donki_{kind.lower()}.json")
        events, a = [], a0
        while a <= end:
            b = min(a + dt.timedelta(days=58), end)
            r = _get(DONKI.format(kind=kind, a=a, b=b))
            events += json.loads(r) if r.strip() else []
            a = b + dt.timedelta(days=1)
        json.dump(events, open(p, "w"))
        json.dump(dict(fetched_utc=dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), source="api.nasa.gov DONKI",
                       range=[str(a0), str(end)]), open(p.replace(".json", ".meta.json"), "w"))
        print(f"DONKI {kind}: {len(events)} events")


def fetch():
    os.makedirs(RAW, exist_ok=True)
    fetch_donki()


def parse_rdr(text):
    """Observations of one RAD RDR file: start (LMST 'SOL H:MM:SS'), duration (s), QC, B and E dose (None if no section).
    Raises ValueError when a dose section does not hold exactly one number (the rule's format check)."""
    obs = []
    for block in re.split(r"(?m)^\[OBSERVATION: *\d+\]\s*$", text)[1:]:
        def field(k):
            m = re.search(rf"(?m)^{k}=\"?([^\"\n]*)\"?\s*$", block)
            return m.group(1).strip() if m else None

        def dose(tag):
            m = re.search(rf"(?ms)^\[{tag}: *\d+\]\s*$(.*?)(?=^\[|\Z)", block)
            if not m:
                return None
            nums = re.findall(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?|nan|NaN|inf", m.group(1))
            if len(nums) != 1:
                raise ValueError(f"{tag}: {len(nums)} numbers in one section (expected 1)")
            return float(nums[0])
        obs.append(dict(start_mars=field("START_OBS_MARS"), start_utc=field("START_OBS_UTC"),
                        duration_s=float(field("OBS_DURATION")) if field("OBS_DURATION") else None,
                        qc=field("QC"), b=dose("DOSIMETRY_TOTAL_DOSE_B"), e=dose("DOSIMETRY_TOTAL_DOSE_E")))
    return obs


def _lmst_s(s):
    """'4618 0:40:06' -> seconds since sol 0 start (Mars seconds of LMST)."""
    sol, hms = s.split()
    h, m, x = (float(v) for v in hms.split(":"))
    return int(sol) * 88775.244 + (h * 3600 + m * 60 + x) * 88775.244 / 86400.0


def _pct(v, q):
    v = sorted(v)
    if not v:
        return None
    k = (len(v) - 1) * q
    lo, hi = math.floor(k), math.ceil(k)
    return v[lo] + (v[hi] - v[lo]) * (k - lo)


def summarize(obs):
    def good(x):
        return x is not None and math.isfinite(x) and x >= 0.0
    dose_obs = [o for o in obs if o["b"] is not None or o["e"] is not None]
    solar = [o for o in dose_obs if o["qc"] and re.fullmatch(r"[01]{8}", o["qc"]) and int(o["qc"], 2) & SOLAR_EVENT_BIT]
    normal = [o for o in dose_obs if o not in solar]
    out = dict(n_observations=len(obs), n_with_dose=len(dose_obs), n_solar_event_mode=len(solar),
               qc_values=sorted({o["qc"] for o in obs if o["qc"] is not None}))
    for k in ("b", "e"):
        vals = [o[k] for o in normal if good(o[k])]
        out[f"{k}_excluded"] = sum(1 for o in normal if o[k] is not None and not good(o[k]))
        out[f"{k}_n"] = len(vals)
        out[f"{k}_median"], out[f"{k}_p10"], out[f"{k}_p90"] = (None if not vals else round(_pct(vals, q), 4) for q in (0.5, 0.1, 0.9))
        out[f"{k}_solar_event_median"] = round(_pct([o[k] for o in solar if good(o[k])], 0.5), 4) if solar else None
    starts = sorted(_lmst_s(o["start_mars"]) for o in dose_obs if o["start_mars"])
    gaps = [b - a for a, b in zip(starts, starts[1:])]
    out["first_start_lmst"] = min((o["start_mars"] for o in dose_obs), key=_lmst_s, default=None)
    out["last_start_lmst"] = max((o["start_mars"] for o in dose_obs), key=_lmst_s, default=None)
    out["gap_min_s"], out["gap_max_s"] = (round(min(gaps)), round(max(gaps))) if gaps else (None, None)
    out["valid"] = all(out[f"{k}_median"] is not None and MEDIAN_GUARD[0] <= out[f"{k}_median"] <= MEDIAN_GUARD[1] for k in ("b", "e"))
    return out


def donki_events():
    """CME/SEP records naming Mars, MSL or Curiosity among affected targets (rule). Returns (events, note)."""
    events, missing = [], []
    for kind in ("SEP", "CME"):
        p = os.path.join(RAW, f"donki_{kind.lower()}.json")
        if not os.path.exists(p):
            missing.append(kind)
            continue
        for x in json.load(open(p)):
            if kind == "SEP":
                targets = " ".join(i.get("displayName", "") for i in x.get("instruments") or [])
                when, link, eid = x.get("eventTime"), x.get("link"), x.get("sepID")
            else:
                targets = " ".join(imp.get("location", "") for an in x.get("cmeAnalyses") or [] for en in an.get("enlilList") or []
                                   for imp in en.get("impactList") or [])
                when, link, eid = x.get("startTime"), x.get("link"), x.get("activityID")
            if TARGET_RE.search(targets):
                arrivals = sorted({imp.get("arrivalTime") for an in x.get("cmeAnalyses") or [] for en in an.get("enlilList") or []
                                   for imp in en.get("impactList") or [] if TARGET_RE.search(imp.get("location", ""))} - {None})
                events.append(dict(type=kind, id=eid, start_utc=when, link=link, mars_arrival_modelled_utc=arrivals or None))
    events.sort(key=lambda e: e["start_utc"] or "")
    note = f"space weather list not available ({', '.join(missing)} not fetched)" if missing else None
    return events, note


def build(out=config.ASSETS):
    sols = stop_sols()
    per = []
    for sol in sols:
        files = sorted(f for f in os.listdir(RAW) if re.fullmatch(rf"RAD_RDR_\d{{4}}_\d{{3}}_\d\d_\d\d_{sol}_V\d+\.TXT", f))
        if not files:
            per.append(dict(sol=sol, archived=False))
            print(f"sol {sol}: no archived value")
            continue
        obs = []
        for f in files:
            obs += parse_rdr(open(os.path.join(RAW, f), encoding="latin-1").read())
        s = dict(sol=sol, archived=True, files=files, **summarize(obs))
        per.append(s)
        print(f"sol {sol}: N {s['b_n']} (of {s['n_observations']} obs; {s['n_solar_event_mode']} solar-event mode; "
              f"excluded B {s['b_excluded']} E {s['e_excluded']}); B median {s['b_median']} (p10 {s['b_p10']}, p90 {s['b_p90']}) "
              f"E median {s['e_median']} (p10 {s['e_p10']}, p90 {s['e_p90']}) uGy/h; starts {s['first_start_lmst']} .. "
              f"{s['last_start_lmst']}; gaps {s['gap_min_s']}-{s['gap_max_s']} s; QC {s['qc_values']}; {'VALID' if s['valid'] else 'INVALID'}")
    events, note = donki_events()
    print(f"DONKI events naming Mars/MSL/Curiosity: {len(events)}" + (f" ({note})" if note else ""))
    doc = dict(rule="Proposal tab, 'RAD rule' (approved, before any value)",
               source=dict(name=DOI_NOTE, url=RDR_ROOT, sis=RDR_ROOT + "DOCUMENT/RAD_RDR_SIS.HTM",
                           credit="NASA/JPL-Caltech/SwRI (MSL RAD)"),
               units="microGray/hour (RAD RDR SIS 4.2.1.6); B = silicon detector, E = plastic scintillator",
               context_only="no conversion to mSv or tissue dose; never a route limit",
               sols=per, space_weather=dict(source="NASA DONKI via api.nasa.gov", range=[str(d) for d in DONKI_RANGE],
                                            rule="CME/SEP records naming Mars, MSL or Curiosity among affected targets",
                                            events=events, note=note))
    p = os.path.join(out, OUT_NAME)
    json.dump(doc, open(p, "w", encoding="utf-8"), indent=1)
    print(f"written {p}; now run `python tools/build_bundle.py rehash`")


def selftest():
    def obs(n, mars, b, e, qc="00000000", extra=""):
        return (f"[OBSERVATION: {n:02d}]\nSTART_OBS_MARS=\"{mars}\"\nSTART_OBS_UTC=\"2025-214 13:01:37\"\nOBS_DURATION=961\nQC=\"{qc}\"\n"
                f"[COUNTERS: {n:02d}]\n 1 2 3\n" + (f"[DOSIMETRY_TOTAL_DOSE_B: {n:02d}]\n\n{b}\n\n[DOSIMETRY_TOTAL_DOSE_E: {n:02d}]\n\n{e}\n{extra}\n"
                                                    if b is not None else ""))
    text = "[FILE]\nSTART_SOL=4618\n" + obs(0, "4618 0:40:06", 7.0, 6.0) + obs(1, "4618 1:00:00", None, None) \
        + obs(2, "4618 1:20:00", 9.0, 8.0) + obs(3, "4618 2:00:00", -1.0, 7.0) + obs(4, "4618 3:00:00", 50.0, 40.0, qc="00001000")
    o = parse_rdr(text)
    assert len(o) == 5 and o[1]["b"] is None and o[0]["b"] == 7.0 and o[4]["qc"] == "00001000", o
    s = summarize(o)
    assert s["n_with_dose"] == 4 and s["n_solar_event_mode"] == 1 and s["b_excluded"] == 1, s
    assert s["b_n"] == 2 and s["b_median"] == 8.0 and s["e_n"] == 3 and s["e_median"] == 7.0, s
    assert s["b_solar_event_median"] == 50.0 and s["valid"], s
    assert s["first_start_lmst"] == "4618 0:40:06" and s["last_start_lmst"] == "4618 3:00:00"
    try:
        parse_rdr(obs(0, "4618 0:40:06", "7.0 8.0", 6.0))
        raise AssertionError("two numbers in one dose section must fail the format check")
    except ValueError:
        pass
    bad = summarize(parse_rdr(obs(0, "4618 0:40:06", 500.0, 6.0)))
    assert not bad["valid"], "a median outside the guard is INVALID"
    assert TARGET_RE.search("Mars") and not TARGET_RE.search("Marshall"), "whole-word target match"
    print("rad selftest OK")


if __name__ == "__main__":
    {"fetch": fetch, "build": build, "selftest": selftest}[sys.argv[1] if len(sys.argv) > 1 else "selftest"]()
