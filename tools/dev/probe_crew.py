"""Browser probe for the crew block (survival margin + two-person EVA): the Plan tab shows the block with the fit verdict, station time
and the rescue line; choosing 4 h and typing 60 min per stop turns it 'Does not fit'; the crew block names the roles.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_crew.py OUT.png`."""
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
    wait("document.querySelector('.tabs button')", 90)
    ev("[...document.querySelectorAll('.tabs button')].find(b => b.textContent === 'Plan').click(); 1")
    time.sleep(1)
    ev("document.querySelector('details.rules') && (document.querySelector('details.rules').open = true); 1")
    txt = wait("(() => { const c = document.querySelector('.crew'); return c && c.innerText.includes('Station time') ? c.innerText : null; })()", 60)
    print("crew block:\n", txt)
    if not txt or "Fits the EVA" not in txt or "Rescue" not in txt:
        fails.append("crew block missing the fit verdict, station time or rescue")
    ev("document.querySelector('.crew').scrollIntoView(); 1")
    time.sleep(1)
    shot("crew")
    ev("[...document.querySelectorAll('.crew .seg button')].find(b => b.textContent.startsWith('4')).click(); 1")
    ev("(() => { const i = document.querySelector('.crew input'); i.value = '60'; i.dispatchEvent(new Event('change', {bubbles: true})); })(); 1")
    nogo = wait("(() => { const v = document.querySelector('.crew .verdict'); return v && v.innerText.startsWith('Does not fit') ? v.innerText : null; })()", 30)
    print("after 4 h + 60 min/stop:", nogo, "|", ev("[...document.querySelectorAll('.crew li')].map(l => l.innerText).join(' / ')"))
    if not nogo:
        fails.append("4 h with 60 min per stop did not turn 'Does not fit'")
    roles = ev("(() => { const c = document.querySelector('.crew'); return c && c.innerText.includes('EV1') && c.innerText.includes('EV2') ? 1 : null; })()")
    print("crew roles (EV1 + EV2):", bool(roles))
    if not roles:
        fails.append("crew roles missing")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True))
