"""Browser probe for the chemistry layer: the rail row turns the ChemCam/APXS dots on, the point readout names the
nearest target, and a station's "Measured nearby" table shows oxide values.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_chem.py OUT.png [--gpu]`."""
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
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 90)
    time.sleep(2)
    row = "[...document.querySelectorAll('#layers button.layer')].find(b => b.textContent.trim().startsWith('Chemistry'))"
    print("rail row:", ev(f"({row} || {{}}).innerText"), "| pressed before:", ev(f"({row} || {{getAttribute: () => null}}).getAttribute('aria-checked')"))
    ev(f"{row}.click(); 1")
    on = wait(f"{row}.getAttribute('aria-checked') === 'true' ? 1 : null", 15)
    print("layer on:", bool(on))
    if not on:
        fails.append("chemistry layer did not turn on")
    time.sleep(2)
    shot("map")
    xy = ev("(() => { const l = [...document.querySelectorAll('.flag-label')].find(x => x.innerText.includes('Valle')); const r = l.getBoundingClientRect(); return [r.left + r.width / 2, r.bottom + 6]; })()")
    print("hover at:", xy)
    for _ in range(3):
        call("Input.dispatchMouseEvent", type="mouseMoved", x=xy[0], y=xy[1])
        time.sleep(1)
    r = wait("(() => { const e = document.getElementById('inspect'); return e && e.innerText.includes('Chemistry') ? e.innerText.split(String.fromCharCode(10)).join(' | ') : null; })()", 10)
    print("readout:", r)
    if not r:
        fails.append("readout has no chemistry line")
    ev("[...document.querySelectorAll('table.stations .sn')].find(b => b.textContent.includes('Valle')).click(); 1")
    t = wait("(() => { const t = document.querySelector('table.chem'); return t ? t.innerText.split(String.fromCharCode(10)).join(' / ') : null; })()", 15)
    print("Valle de la Luna, measured nearby:", (t or "")[:500])
    if not t:
        fails.append("no chemistry table on the station")
    time.sleep(1)
    shot("station")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
