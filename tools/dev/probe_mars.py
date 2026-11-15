"""Browser probe for the Mars view: ?view=mars opens it; the zoom ladder (Mars, Gale crater, Curiosity's route, This walk)
loads basemap tiles at each step; a landing-site card opens; 'Open the EVA planner' returns to the map; About lists the
missions. Needs the network (tiles stream from NASA Mars Trek and Esri).
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_mars.py OUT.png`."""
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
    loaded = ("(() => { const im = [...document.querySelectorAll('.mars-tiles img')]; const ok = im.filter(i => i.complete && i.naturalWidth > 0).length;"
              " return im.length && ok >= im.length * 0.8 ? ok + '/' + im.length + ' ' + (im[0].src.match(/(tiles\\/Mars\\/EQ\\/[^/]+|OnMars\\/CTX)/) || [''])[0] + ' z' + im[0].src.split('/').slice(-3)[0] : null; })()")
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&view=mars")
    time.sleep(2)
    if not wait("!!document.querySelector('.mars-ladder')", 90):
        return ["Mars view did not open"]
    for step in ["Mars", "Gale crater", "Curiosity's route", "This walk"]:
        ev(f"[...document.querySelectorAll('.mars-ladder button')].find(b => b.textContent === {step!r}).click(); 1")
        time.sleep(1)
        got = wait(loaded, 40)
        scale = ev("document.querySelector('.mars-scale').innerText")
        print(f"{step}: tiles {got} | scale {scale}")
        if not got:
            fails.append(f"{step}: tiles did not load")
        shot(step.split()[0].lower().replace("'s", ""))
    print("sites:", ev("[...document.querySelectorAll('.mars-site title')].map(t => t.textContent)"))
    ev("[...document.querySelectorAll('.mars-ladder button')].find(b => b.textContent === 'Mars').click(); 1")
    time.sleep(1)
    wait(loaded, 30)
    ev("document.querySelector('.mars-site').dispatchEvent(new MouseEvent('click', {bubbles: true})); 1")
    card = wait("document.querySelector('.mars-card h3')?.innerText", 10)
    print("card:", card, "|", (ev("document.querySelector('.mars-card .desc')?.innerText") or "")[:80])
    if not card:
        fails.append("site card did not open")
    time.sleep(2)
    shot("card")
    ev("[...document.querySelectorAll('.mars-ladder button')].find(b => b.textContent.startsWith('Open the EVA')).click(); 1")
    closed = wait("document.querySelector('.mars') ? null : 'closed'", 10)
    print("planner:", closed)
    if not closed:
        fails.append("Open the EVA planner did not close the Mars view")
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'About').click(); 1")
    ms = wait("(() => { const h = [...document.querySelectorAll('#inspector h4')].find(x => x.innerText.toLowerCase().includes('nasa missions')); return h ? h.innerText + ' | ' + [...document.querySelectorAll('ul.missions > li > b')].map(x => x.innerText).join(', ') : null; })()", 20)
    print("about:", ms)
    if not ms:
        fails.append("missions list missing in About")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu=True))
