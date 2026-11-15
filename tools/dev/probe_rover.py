"""Browser probe for Walk mode's crew and rover: seeks the EVA simulation to several points of the loop (third person) and
screenshots each; checks that the Curiosity tags appear and that there are no console errors.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_rover.py OUT.png [--gpu]`."""
import base64
import os
import sys
import time

sys.stdout.reconfigure(encoding="utf-8")
sys.path.insert(0, os.path.dirname(__file__))
import browser_smoke as b  # noqa: E402

AT = [0.04, 0.10, 0.50, 0.55, 0.60]


def probe(call, out):
    ev = lambda js: call("Runtime.evaluate", expression=js, returnByValue=True).get("result", {}).get("value")
    call("Emulation.setDeviceMetricsOverride", width=1440, height=900, deviceScaleFactor=1, mobile=False)
    shot = lambda n: open(out.replace(".png", f"_{n}.png"), "wb").write(base64.b64decode(call("Page.captureScreenshot", format="png")["data"]))
    wait = lambda js, t=60: next((v for _ in range(int(t / 0.4)) if (v := ev(js)) or time.sleep(0.4)), None)
    key = lambda k, code, vk, typ: call("Input.dispatchKeyEvent", type=typ, key=k, code=code, windowsVirtualKeyCode=vk)
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk")
    time.sleep(2)
    if not wait("(() => { const c = document.querySelector('.tb-clock'); return c && c.innerText ? c.innerText : null; })()", 90):
        return ["no transport clock"]
    ev("document.getElementById('visor').focus(); 1")
    key("c", "KeyC", 67, "keyDown"); key("c", "KeyC", 67, "keyUp")
    seen = set()
    for f in AT:
        ev("(() => { const h = document.querySelector('svg.tl rect.hit'); const r = h.getBoundingClientRect();"
           f" h.dispatchEvent(new MouseEvent('click', {{bubbles: true, clientX: r.left + r.width * {f}, clientY: r.top + 10}})); }})(); 1")
        time.sleep(2.5)
        tags = ev("[...document.querySelectorAll('.rover-tag')].filter(t => t.style.display !== 'none').map(t => t.innerText)") or []
        seen.update(tags)
        print(f"at {f:.2f}: clock {ev('document.querySelector(\".tb-clock\").innerText')} | rover tags {tags}")
        shot(f"{int(f * 100):02d}")
    if not seen:
        fails.append("no Curiosity tag was shown anywhere")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
