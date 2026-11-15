"""Browser probe for the scientist tools: the point readout under the cursor (Plan map), the GeoJSON download and
the "out of sight of the exit" Risk item and timeline strip.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_tools.py OUT.png [--gpu]`."""
import base64
import glob
import json
import os
import sys
import tempfile
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
    dl = tempfile.mkdtemp(prefix="visor_dl_")
    call("Browser.setDownloadBehavior", behavior="allow", downloadPath=dl)
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 90)
    time.sleep(2)
    # readout: move the mouse over the map
    for _ in range(3):
        call("Input.dispatchMouseEvent", type="mouseMoved", x=650, y=420)
        time.sleep(1.0)
    r = wait("(() => { const e = document.getElementById('inspect'); return e && e.style.display !== 'none' && e.innerText ? e.innerText.replace(/\\n+/g, ' | ') : null; })()", 15)
    print("readout:", r)
    if not r or "Elevation" not in r or "Rover check" not in r:
        fails.append("no point readout over the map")
    shot("readout")
    # Risk: sight item
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'Risk').click(); 1")
    s = wait("(() => { const h = [...document.querySelectorAll('.risk h4')].find(x => x.textContent.includes('sight')); return h ? h.closest('section').innerText.replace(/\\n+/g, ' | ') : null; })()", 10)
    print("sight:", s, "| strips:", ev("document.querySelectorAll('svg.tl .unseen').length"))
    if not s:
        fails.append("no sight item in Risk")
    shot("risk")
    # GeoJSON download from About
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'About').click(); 1")
    time.sleep(1)
    ev("[...document.querySelectorAll('.ctl-row.share button')].find(b => b.textContent.includes('GeoJSON')).click(); 1")
    f = None
    for _ in range(30):
        f = next(iter(glob.glob(os.path.join(dl, "*.geojson"))), None)
        if f:
            break
        time.sleep(0.5)
    if f:
        g = json.load(open(f, encoding="utf-8"))
        kinds = {}
        for x in g["features"]:
            kinds[x["properties"]["kind"]] = kinds.get(x["properties"]["kind"], 0) + 1
        st = [x for x in g["features"] if x["properties"]["kind"] == "station"][4]
        print("geojson:", kinds, "| station 5:", st["properties"]["name"], st["geometry"]["coordinates"], st["properties"]["bags"])
    else:
        fails.append("no GeoJSON download")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
