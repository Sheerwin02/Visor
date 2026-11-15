"""Headless browser smoke test for the VISOR web client (local only; nothing is sent anywhere).

Opens the served app in headless Chrome over the DevTools protocol, waits for real rendering (not Chrome's virtual time,
which freezes Bevy's frame loop), reports the time from navigation until the app logs "terrain ... built", the status box
state, any console errors, and saves a screenshot.

Usage (from the repository root):
  cd app && trunk build && cd dist && python -m http.server 8765 --bind 127.0.0.1     # in one terminal
  python tools/dev/browser_smoke.py http://127.0.0.1:8765/ smoke.png                   # in another
  python tools/dev/browser_smoke.py http://127.0.0.1:8765/ smoke.png --keys            # also press real keys (see key_test)
Needs: pip install websocket-client ; Google Chrome. Uses SwiftShader (software WebGL), so frame rate is not representative.
"""
import base64
import json
import os
import subprocess
import sys
sys.stdout.reconfigure(encoding="utf-8")
import shutil
import tempfile
import time
import urllib.request

import websocket

CHROME_PATHS = [r"C:\Program Files\Google\Chrome\Application\chrome.exe", "/usr/bin/google-chrome", "/usr/bin/chromium",
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
PORT = 9333


def key_test(call, out):
    """Press real keys and click through the DevTools protocol, the way a user would:
    click the canvas; Tab -> Walk (focus must stay on the canvas); M -> Plan, M -> Walk; drag the mouse to turn away;
    N -> heading within 1 degree of the bearing to the next stop; hold W -> walker moves, no faster than the flat pace;
    click the on-screen mode button -> Plan, focus back on the canvas. Returns a list of failures (empty = pass)."""
    import math
    root = os.path.join(os.path.dirname(__file__), "..", "..", "assets")
    stops = json.load(open(os.path.join(root, "stops.json"), encoding="utf-8"))["stops"]
    route = json.load(open(os.path.join(root, "route.json"), encoding="utf-8"))
    flat = route["assumptions"]["speed_flat_mps"]
    ev = lambda js: call("Runtime.evaluate", expression=js).get("result", {}).get("value")
    chip = lambda: (ev("document.getElementById('mode').textContent") or "")
    walker = lambda: ev("document.getElementById('mode').getAttribute('data-walker')")
    focus = lambda: ev("document.activeElement ? (document.activeElement.id || document.activeElement.tagName) : 'none'")
    keys = {"Tab": (9, "Tab"), "KeyM": (77, "m"), "KeyN": (78, "n"), "KeyW": (87, "w")}
    fails = []

    def key(code, kind):
        vk, k = keys[code]
        call("Input.dispatchKeyEvent", type=kind, code=code, key=k, windowsVirtualKeyCode=vk, nativeVirtualKeyCode=vk)

    # Headless Chrome renders in software (0.3 to 1 frame per second here), and Bevy caps a frame's time step at 0.25 s.
    # So every check WAITS for its expected result (up to WAIT_S) instead of sleeping a fixed time; taps can be short
    # (the app keeps a tap that starts and ends inside one frame); W's distance is only an upper-bound check.
    WAIT_S = 40.0

    def wait_until(pred, timeout=WAIT_S):
        t0 = time.time()
        while time.time() - t0 < timeout:
            try:
                if pred():
                    return True
            except Exception:
                pass
            time.sleep(0.5)
        return False

    def press(code):
        key(code, "keyDown"); time.sleep(0.2); key(code, "keyUp")

    def mouse(kind, x, y, buttons=0, button="none"):
        call("Input.dispatchMouseEvent", type=kind, x=x, y=y, button=button, buttons=buttons, clickCount=1)

    def expect_mode(want, step):
        wait_until(lambda: chip().startswith(want))
        c = chip()
        ok = c.startswith(want)
        print(f"{step}: {'ok' if ok else 'FAIL'} (chip starts {c[:12]!r}, focus {focus()})")
        if not ok:
            fails.append(f"{step}: mode is not {want}")
        if focus() != "visor":
            fails.append(f"{step}: focus left the canvas (now {focus()})")

    def heading():
        w = walker()
        return tuple(map(float, w.split(","))) if w else None

    mouse("mousePressed", 900, 400, 1, "left"); mouse("mouseReleased", 900, 400, 0, "left")
    time.sleep(1.0)
    press("Tab"); expect_mode("Walk", "Tab")
    press("KeyM"); expect_mode("Plan", "M")
    press("KeyM"); expect_mode("Walk", "M again")
    wait_until(lambda: walker() is not None)
    w0 = heading()
    if not w0:
        return fails + ["no walker position reported"]
    # turn away by dragging: many small moves with the left button held, spread over several frames
    # the pointer arrives first (a press with no pointer move before it is not something a real mouse sends)
    mouse("mouseMoved", 400, 400); time.sleep(0.8)
    mouse("mousePressed", 400, 400, 1, "left")
    for i in range(1, 31):
        mouse("mouseMoved", 400 + 10 * i, 400, 1, "left"); time.sleep(0.1)
    wait_until(lambda: abs((heading()[2] - w0[2] + 540) % 360 - 180) > 10, 20.0)
    mouse("mouseReleased", 700, 400, 0, "left"); time.sleep(1.0)
    w1 = heading()
    turned = abs((w1[2] - w0[2] + 540) % 360 - 180)
    print(f"drag: heading {w0[2]:.1f} -> {w1[2]:.1f} deg (turned {turned:.1f})")
    if turned < 10:
        fails.append(f"drag turned only {turned:.1f} deg, so the N check would prove nothing")
    press("KeyN")
    s = stops[1]  # next stop after the exit
    bearing_ok = lambda: (lambda w: abs((w[2] - math.degrees(math.atan2(s["e_m"] - w[0], s["n_m"] - w[1])) % 360 + 540) % 360 - 180) <= 1.0)(heading())
    wait_until(bearing_ok)
    e, n, hd = heading()
    want = math.degrees(math.atan2(s["e_m"] - e, s["n_m"] - n)) % 360
    diff = abs((hd - want + 540) % 360 - 180)
    print(f"N: heading {hd:.1f} deg, bearing to {s['name']} {want:.1f} deg (diff {diff:.2f})")
    if diff > 1.0:
        fails.append(f"N heading off by {diff:.2f} deg")
    t0 = time.time(); key("KeyW", "keyDown"); time.sleep(8.0); key("KeyW", "keyUp"); held = time.time() - t0
    wait_until(lambda: math.hypot(heading()[0] - e, heading()[1] - n) > 0.05, 20.0)
    time.sleep(3.0)
    e1, n1, _ = heading()
    moved = math.hypot(e1 - e, n1 - n)
    print(f"W held {held:.1f} s: moved {moved:.2f} m (flat pace would be at most {flat * held:.2f} m)")
    if not (0.05 < moved <= flat * held * 1.15):
        fails.append(f"W moved {moved:.2f} m, outside (0.05, {flat * held * 1.15:.2f}]")
    # top bar (step 3): visible in Walk mode, names the next stop, arrow points ahead after N, numbers present
    wait_until(lambda: ev("document.getElementById('hud-dist').textContent") != "")
    hud = json.loads(ev("JSON.stringify({show: getComputedStyle(document.getElementById('hud')).display, "
                        "next: document.getElementById('hud-next').textContent, dist: document.getElementById('hud-dist').textContent, "
                        "time: document.getElementById('hud-time').textContent, back: document.getElementById('hud-back').textContent, "
                        "slope: document.getElementById('hud-slope').textContent, arrow: document.getElementById('compass-target').getAttribute('data-rel')})") or "{}")
    print("top bar:", {k: v[:60] for k, v in hud.items()})
    rot = float(hud.get("arrow") or 999)  # next stop's angle from straight ahead on the compass strip
    if hud.get("show") == "none":
        fails.append("top bar hidden in Walk mode")
    if s["name"].split(":")[0] not in hud.get("next", ""):
        fails.append("top bar does not name the next stop")
    if abs(rot) > 2:
        fails.append(f"arrow at {rot} deg after N (should point ahead)")
    if not any(ch.isdigit() for ch in hud.get("dist", "") + hud.get("time", "") + hud.get("back", "")):
        fails.append("top bar numbers missing")
    labels = ev("document.querySelectorAll('.flag-label').length")
    print("flag labels:", labels)
    if labels != len(stops):
        fails.append(f"{labels} flag labels for {len(stops)} stops")
    shot = call("Page.captureScreenshot", format="png")
    walk_png = out.replace(".png", "_walk.png")
    open(walk_png, "wb").write(base64.b64decode(shot["data"]))
    print("screenshot in Walk mode:", walk_png)
    r = json.loads(ev("JSON.stringify(document.getElementById('mode-btn').getBoundingClientRect())") or "{}")
    if not r.get("width"):
        fails.append("mode button not shown")
    else:
        bx, by = r["x"] + r["width"] / 2, r["y"] + r["height"] / 2
        mouse("mousePressed", bx, by, 1, "left"); mouse("mouseReleased", bx, by, 0, "left")
        expect_mode("Plan", "mode button")
        if getattr(key_test, "plan_shot", True):
            time.sleep(2)
            shot = call("Page.captureScreenshot", format="png")
            open(out.replace(".png", "_plan.png"), "wb").write(base64.b64decode(shot["data"]))
        wait_until(lambda: ev("getComputedStyle(document.getElementById('hud')).display") == "none")
        if ev("getComputedStyle(document.getElementById('hud')).display") != "none":
            fails.append("top bar still shown in Plan mode")
    return fails


def main(url, out, timeout_s=60.0, keys=False, gpu=False):
    chrome = next((p for p in CHROME_PATHS if os.path.exists(p)), None)
    if chrome is None:
        sys.exit("Chrome not found")
    prof = tempfile.mkdtemp(prefix="visor_smoke_")
    # default: software rendering (SwiftShader), the same on every machine; --gpu: the machine's real GPU through ANGLE/D3D11
    render = ["--use-angle=d3d11", "--enable-gpu", "--ignore-gpu-blocklist", "--force_high_performance_gpu"] if gpu else ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"]
    proc = subprocess.Popen([chrome, "--headless=new", f"--remote-debugging-port={PORT}", *render,
                             "--window-size=1280,800", f"--user-data-dir={prof}", "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        tabs = None
        for _ in range(60):
            try:
                tabs = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json")); break
            except OSError:
                time.sleep(0.3)
        ws = websocket.create_connection([t for t in tabs if t["type"] == "page"][0]["webSocketDebuggerUrl"], timeout=120)
        msg_id = [0]
        events = []

        def call(method, **params):
            msg_id[0] += 1
            ws.send(json.dumps({"id": msg_id[0], "method": method, "params": params}))
            while True:
                m = json.loads(ws.recv())
                if m.get("id") == msg_id[0]:
                    return m.get("result", {})
                events.append((time.time(), m))

        call("Runtime.enable"); call("Page.enable")
        t0 = time.time()
        call("Page.navigate", url=url)
        built_at, errors = None, []
        ws.settimeout(1)  # short timeout only while polling for console events
        while time.time() - t0 < timeout_s and built_at is None:
            try:
                events.append((time.time(), json.loads(ws.recv())))
            except websocket.WebSocketTimeoutException:
                pass
            for ts, m in events:
                if m.get("method") == "Runtime.consoleAPICalled":
                    text = " ".join(str(a.get("value", "")) for a in m["params"].get("args", []))
                    if "terrain" in text and "built" in text and built_at is None:
                        built_at = ts - t0
                    if m["params"].get("type") == "error":
                        errors.append(text[:200])
                    elif "shader" in text.lower() and ("error" in text.lower() or "failed" in text.lower()):
                        # a shader that fails to build draws nothing and may log only a warning (the AR material)
                        errors.append(text[:200])
                elif m.get("method") == "Runtime.exceptionThrown":
                    errors.append(json.dumps(m["params"].get("exceptionDetails", {}))[:200])
            events.clear()
        ws.settimeout(120)  # software rendering can make captures slow
        time.sleep(3)  # let a few frames render
        st = call("Runtime.evaluate", expression="(() => { const s = document.getElementById('status'); "
                                                 "return s ? (s.style.display || 'shown') + ' | ' + s.textContent : 'no status'; })()")
        shot = call("Page.captureScreenshot", format="png")
        open(out, "wb").write(base64.b64decode(shot["data"]))
        print(f"terrain built after: {built_at:.1f} s" if built_at is not None else f"terrain NOT built within {timeout_s} s")
        print("status box:", st.get("result", {}).get("value"))
        print("console errors:", errors if errors else "none")
        print("screenshot:", out)
        fails = key_test(call, out) if keys and built_at is not None else []
        if keys:
            print("key test:", "PASS" if not fails else "FAIL: " + "; ".join(fails))
        return 0 if built_at is not None and not errors and not fails else 1
    finally:
        proc.kill()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        # the Chrome profile (30-120 MB per run) is deleted: 291 of them once filled the C: drive
        shutil.rmtree(prof, ignore_errors=True)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1], sys.argv[2], keys="--keys" in sys.argv[3:], gpu="--gpu" in sys.argv[3:]))
