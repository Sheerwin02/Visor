"""Browser probe for NASA Mars Trek layers: the Imagery group lists the basemaps, choosing the Trek HiRISE mosaic loads it
(no error line), the Risk tab's data readiness shows Mars Trek rows, and the About tab carries an "Open in NASA Mars Trek" link.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_trek.py OUT.png`."""
import base64
import os
import sys
import time

sys.stdout.reconfigure(encoding="utf-8")
sys.path.insert(0, os.path.dirname(__file__))
import browser_smoke as b  # noqa: E402


def probe(call, out):
    ev = lambda js: call("Runtime.evaluate", expression=js, returnByValue=True).get("result", {}).get("value")
    call("Emulation.setDeviceMetricsOverride", width=1440, height=900, deviceScaleFactor=1, mobile=False)
    shot = lambda n: open(out.replace(".png", f"_{n}.png"), "wb").write(base64.b64decode(call("Page.captureScreenshot", format="png")["data"]))
    wait = lambda js, t=60: next((v for _ in range(int(t / 0.4)) if (v := ev(js)) or time.sleep(0.4)), None)
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    names = wait("(() => { const r = [...document.querySelectorAll('.basemaps .basemap')]; return r.length > 1 ? r.map(x => x.innerText.split(String.fromCharCode(10)).join(' | ')) : null; })()", 90)
    print("basemaps:", names)
    if not names:
        return ["no basemap choices"]
    ev("[...document.querySelectorAll('.basemaps .basemap')][1].click(); 1")
    on = wait("(() => { const r = document.querySelectorAll('.basemaps .basemap')[1]; return r && r.getAttribute('aria-checked') === 'true' && !r.innerText.includes('\u2026') ? 'loaded' : null; })()", 60)
    err = ev("(document.querySelector('.basemaps .warn') || {}).innerText")
    print("Trek HiRISE chosen:", on, "| error:", err)
    if on != "loaded" or err:
        fails.append("Trek basemap did not load")
    time.sleep(2)
    shot("hirise")
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'Risk').click(); [...document.querySelectorAll('details')].forEach(d => d.open = true); 1")
    rows = wait("(() => { [...document.querySelectorAll('details')].forEach(d => d.open = true); const r = [...document.querySelectorAll('.rrow-c')].filter(x => x.innerText.includes('Mars Trek')); return r.length ? r.map(x => x.innerText.split(String.fromCharCode(10)).join(' | ')) : null; })()", 30)
    print("Mars Trek readiness rows:")
    for r in rows or []:
        print("  ", r)
    if not rows:
        fails.append("no Mars Trek rows in the readiness table")
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'About').click(); 1")
    link = wait("(() => { const a = [...document.querySelectorAll('#inspector a')].find(x => x.href.startsWith('https://trek.nasa.gov/mars/#')); return a ? a.href : null; })()", 20)
    print("About link:", link)
    if not link:
        fails.append("no Mars Trek link in About")
    ev("[...document.querySelectorAll('.basemaps .basemap')][0].click(); 1")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True))
