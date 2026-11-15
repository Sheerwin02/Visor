"""Browser probe for planning a new walk and comparing plans: Save plan; New plan + three map clicks + Done; a new station's
plan started from the nearest Curiosity stop, edited and saved ("your plan"); Save plan again; the comparison table
(Now + saved plans, best marked); Open the first plan (stations and plans restored); saved plans survive a reload.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_plans.py OUT.png [--gpu]`."""
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
    tab = lambda name: ev(f"[...document.querySelectorAll('.tabs button')].find(b => b.textContent === {name!r}).click(); 1")
    btn = lambda text: ev(f"(() => {{ const b = [...document.querySelectorAll('#inspector button')].find(b => b.textContent.trim() === {text!r}); if (b) b.click(); return !!b; }})()")
    rows = "document.querySelectorAll('table.stations tbody tr').length"
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait(f"{rows} >= 6 ? 1 : null", 90)
    time.sleep(3)
    ev("try { localStorage.removeItem('visor.plans.v1') } catch (e) {} ; 1")
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait(f"{rows} >= 6 ? 1 : null", 90)
    time.sleep(3)
    btn("Save plan")
    time.sleep(0.5)
    # new plan: three clicks on the map, then Done
    btn("New plan")
    n1 = wait(f"{rows} === 1 ? 1 : null", 10)
    print("new plan rows:", ev(rows), "| note:", ev("[...document.querySelectorAll('#inspector p.fine')].map(p => p.innerText).find(t => t.startsWith('New plan'))"))
    for (x, y) in [(560, 300), (640, 470), (720, 600)]:
        # a real click: the pointer arrives first, the press carries the button state
        call("Input.dispatchMouseEvent", type="mouseMoved", x=x, y=y, button="none", buttons=0); time.sleep(0.8)
        call("Input.dispatchMouseEvent", type="mousePressed", x=x, y=y, button="left", buttons=1, clickCount=1); time.sleep(0.4)
        call("Input.dispatchMouseEvent", type="mouseReleased", x=x, y=y, button="left", buttons=0, clickCount=1)
        time.sleep(1.5)
    btn("Done adding")
    n = wait(f"(() => {{ const n = {rows}; const g = document.querySelector('.budget'); return n === 4 && g ? n : null; }})()", 40)
    print("after 3 clicks:", n, "| go:", ev("document.querySelector('.budget') && document.querySelector('.budget').title"))
    if not n1 or n != 4:
        fails.append("new plan with three stations did not plan")
    time.sleep(2)
    shot("new")
    # station plan for the second station, from the nearest Curiosity stop
    ev("document.querySelectorAll('table.stations .sn')[1].click(); 1")
    time.sleep(1)
    start = wait("(() => { const b = [...document.querySelectorAll('#inspector button')].find(b => b.textContent.startsWith('Start from')); return b ? b.textContent : null; })()", 10)
    print("start button:", start)
    ev("[...document.querySelectorAll('#inspector button')].find(b => b.textContent.startsWith('Start from')).click(); 1")
    time.sleep(0.5)
    ev("(() => { const t = document.querySelector('.editor textarea'); t.value = 'Are the nodules here like those at the drill pair?'; t.dispatchEvent(new Event('change', {bubbles: true})); })(); 1")
    btn("Save")
    q = wait("(() => { const q = document.querySelector('.card.station .question'); return q ? q.innerText : null; })()", 15)
    print("station question:", q, "| tag:", ev("document.querySelector('.card.station .tag-line') && document.querySelector('.card.station .tag-line').innerText"))
    if not q or "nodules here" not in q:
        fails.append("edited station plan not shown")
    shot("station")
    # save and compare
    tab("Plan")
    time.sleep(1)
    btn("Save plan")
    t = wait("(() => { const t = document.querySelector('table.cmp'); return t ? t.innerText.replace(/\\t/g, ' | ').replace(/\\n+/g, ' // ') : null; })()", 10)
    print("compare:", (t or "")[:700])
    if not t:
        fails.append("no comparison table")
    shot("compare")
    # open Plan 1 (the published plan)
    ev("(() => { const th = [...document.querySelectorAll('table.cmp thead th')].find(h => h.innerText.startsWith('Plan 1')); th.querySelector('button').click(); })(); 1")
    back = wait(f"{rows} === 6 ? 1 : null", 40)
    print("Plan 1 reopened:", bool(back), "| rows:", ev(rows), "| address:", (ev("location.search") or "")[:90],
          "| saved:", [x.get("query", "")[:60] for x in __import__("json").loads(ev("localStorage.getItem('visor.plans.v1')") or "{}").get("plans", [])])
    if not back:
        fails.append("Open did not restore the saved plan")
    # reload: saved plans kept
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait(f"{rows} >= 6 ? 1 : null", 90)
    kept = wait("(() => { const s = [...document.querySelectorAll('#inspector h4')].find(x => x.textContent.startsWith('Saved plans')); return s ? s.textContent : null; })()", 15)
    print("after reload:", kept)
    if not kept or "(2)" not in kept:
        fails.append("saved plans not kept")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
