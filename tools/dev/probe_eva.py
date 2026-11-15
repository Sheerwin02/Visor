"""Browser probe for the whole-EVA plan: the Brief opens with the EVA plan; the EVA lane shows egress, walking
legs, stations, ingress and the reserve on a time axis; a stop card shows its station plan; in Walk, seeking into a station
shows the station panel with its plan and time left.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_eva.py OUT.png [--gpu]`."""
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
    first = wait("(() => { const g = document.querySelector('.budget'); return g && g.title.includes('stations') ? g.title : null; })()", 90)
    print("plan go line:", first)
    if not first:
        fails.append("Plan does not open with the EVA go line")
    blocks = wait("(() => { const g = [...document.querySelectorAll('svg.tl g.eva-b')]; return g.some(x => x.className.baseVal.includes('eva-egress')) ? g.map(x => x.className.baseVal.replace('eva-b eva-', '') + ':' + x.querySelector('title').textContent).join(' | ') : null; })()", 30)
    print("EVA lane:", blocks)
    kinds = (blocks or "")
    if not all(k in kinds for k in ("egress:", "station:", "walk:", "ingress:")) or not ev("!!document.querySelector('svg.tl .eva-reserve')"):
        fails.append("EVA lane misses egress, station, walk, ingress or the reserve")
    print("ticks:", ev("[...document.querySelectorAll('svg.tl text.tick')].map(t => t.textContent).slice(-8).join(' ')"))
    shot("plan")
    # a station block opens its stop card with the plan
    ev("[...document.querySelectorAll('svg.tl g.eva-station')][3].dispatchEvent(new MouseEvent('click', {bubbles: true})); 1")
    plan = wait("(() => { const d = document.querySelector('.card dl.plan'); return d ? document.querySelector('#card-title').innerText + ' | ' + d.innerText.replace(/\\n+/g, ' / ') : null; })()", 20)
    print("card plan:", (plan or "")[:400])
    if not plan:
        fails.append("station block did not open a card with a plan")
    time.sleep(1)
    shot("card")
    # Walk: seek into the Valle de la Luna station, third person
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk")
    time.sleep(2)
    wait("(() => { const c = document.querySelector('.tb-clock'); return c && c.innerText ? 1 : null; })()", 90)
    wait("document.querySelectorAll('svg.tl g.eva-station').length > 0 ? 1 : null", 30)
    time.sleep(3)  # the EVA plan (station time) arrives a few frames after the first simulation
    ev("(() => { const g = [...document.querySelectorAll('svg.tl g.eva-station')]; const v = g.find(x => x.querySelector('title').textContent.includes('Valle')) || g[3];"
       " v.dispatchEvent(new MouseEvent('click', {bubbles: true})); })(); 1")
    time.sleep(1)
    print("clock right after the click:", ev("document.querySelector('.tb-clock').innerText"))
    ev("document.getElementById('visor').focus(); 1")
    call("Input.dispatchKeyEvent", type="keyDown", key="c", code="KeyC", windowsVirtualKeyCode=67)
    call("Input.dispatchKeyEvent", type="keyUp", key="c", code="KeyC", windowsVirtualKeyCode=67)
    panel = wait("(() => { const p = document.getElementById('station-panel'); return p ? p.innerText.replace(/\\n+/g, ' / ') : null; })()", 20)
    print("station panel:", (panel or "")[:300])
    print("clock:", ev("document.querySelector('.tb-clock').innerText"), "| EVA left:", ev("document.getElementById('hud-task').innerText"),
          "| note:", ev("document.querySelector('.tb-note').innerText"))
    if not panel:
        fails.append("no station panel at a station in Walk")
    time.sleep(2)
    shot("walk")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
