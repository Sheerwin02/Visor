"""Browser probe for the EVA simulation (Walk): Play at 60x moves the clock, the walker and EV2; the log lists events; a click
on the timeline jumps there; W pauses (manual). Usage: serve a trunk build on 127.0.0.1:8765, then
`python tools/dev/probe_sim.py OUT.png [--gpu]`."""
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
    key = lambda k, code, vk, typ: call("Input.dispatchKeyEvent", type=typ, key=k, code=code, windowsVirtualKeyCode=vk)
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk")
    time.sleep(2)
    clock0 = wait("(() => { const c = document.querySelector('.tb-clock'); return c && c.innerText ? c.innerText : null; })()", 90)
    print("transport clock at start:", clock0, "| log rows:", ev("document.querySelectorAll('#eva-log .el-row').length"))
    if not clock0:
        return ["no transport clock"]
    ev("document.getElementById('visor').focus(); 1")
    key("c", "KeyC", 67, "keyDown"); key("c", "KeyC", 67, "keyUp")
    ev("[...document.querySelectorAll('.tb-speed button')].find(b => b.innerText.startsWith('60')).click(); 1")
    ev("document.querySelector('.tl-transport .play').click(); 1")
    time.sleep(6)
    clock1 = ev("document.querySelector('.tb-clock').innerText")
    dist1 = ev("document.querySelector('#hud-dist').innerText")
    print("after ~6 s at 60x:", clock1, "| next-stop distance", dist1, "| playhead",
          ev("(document.querySelector('.tl-playhead') || {}).style ? document.querySelector('.tl-playhead').style.left : null"))
    if clock1 == clock0:
        fails.append("clock did not advance while playing")
    shot("play")
    print("log:", ev("[...document.querySelectorAll('#eva-log .el-row')].map(r => r.innerText.replace(/\\n/g, ' ')).join(' | ')"))
    # scrub: click 70% along the timeline hit area
    ev("(() => { const h = document.querySelector('svg.tl rect.hit'); const r = h.getBoundingClientRect();"
       " h.dispatchEvent(new MouseEvent('click', {bubbles: true, clientX: r.left + r.width * 0.7, clientY: r.top + 10})); })(); 1")
    time.sleep(2)
    clock2 = ev("document.querySelector('.tb-clock').innerText")
    print("after a click at 70%:", clock2)
    if clock2 == clock1:
        fails.append("timeline click did not move the clock")
    shot("scrub")
    # manual: W pauses
    key("w", "KeyW", 87, "keyDown"); time.sleep(1.5); key("w", "KeyW", 87, "keyUp")
    time.sleep(1)
    note = ev("document.querySelector('.tb-note').innerText")
    print("after W:", note)
    if "Manual" not in (note or ""):
        fails.append("W did not switch to manual")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
