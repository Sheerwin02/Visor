"""Browser probe for Walk mode: first-person and third-person screenshots (key C), walking forward with W.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_walk3p.py OUT.png [--gpu]`."""
import base64
import os
import sys
import time

sys.stdout.reconfigure(encoding="utf-8")
sys.path.insert(0, os.path.dirname(__file__))
import browser_smoke as b  # noqa: E402


def key(call, k, code, vk, typ):
    call("Input.dispatchKeyEvent", type=typ, key=k, code=code, windowsVirtualKeyCode=vk)


def probe(call, out):
    ev = lambda js: call("Runtime.evaluate", expression=js, returnByValue=True).get("result", {}).get("value")
    call("Emulation.setDeviceMetricsOverride", width=1440, height=900, deviceScaleFactor=1, mobile=False)
    shot = lambda n: open(out.replace(".png", f"_{n}.png"), "wb").write(base64.b64decode(call("Page.captureScreenshot", format="png")["data"]))
    wait = lambda js, t=60: next((v for _ in range(int(t / 0.4)) if (v := ev(js)) or time.sleep(0.4)), None)
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk")
    time.sleep(2)
    wait("document.body.dataset.mode === 'walk' && document.querySelector('.mb-stat')", 90)
    ev("document.getElementById('visor').focus(); 1")
    time.sleep(4)
    shot("first")
    key(call, "c", "KeyC", 67, "keyDown"); key(call, "c", "KeyC", 67, "keyUp")
    time.sleep(2)
    key(call, "w", "KeyW", 87, "keyDown"); time.sleep(4); key(call, "w", "KeyW", 87, "keyUp")
    time.sleep(2)
    shot("third")
    errs = ev("(window.__errors || []).length")
    print("screens:", out.replace(".png", "_first.png"), out.replace(".png", "_third.png"), "| errors:", errs)
    return []


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
