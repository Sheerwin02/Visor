"""Browser probe for placed relay / working spots (app/src/scene/place.rs): the Plan chip places the suggested relay (R pin,
card with Move / Remove, chip turns to 'Relay placed', rl= in the link); Move + a map click re-places it; Remove drops it;
the working-spot chip places W pins (ws= in the link); a link with rl= / ws= reopens them, in Plan and in Walk.
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_place.py OUT.png [--gpu]`."""
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
    click_text = lambda sel, txt: ev("(() => { const b = [...document.querySelectorAll(" + repr(sel) + ")].find(x => x.textContent.includes("
                                     + repr(txt) + ")); if (!b) return null; b.click(); return b.textContent; })()")
    pin = lambda pid: ev("(() => { const p = document.getElementById(" + repr(pid) + "); return p && p.style.display !== 'none' ? p.style.left + ',' + p.style.top : null; })()")
    q = lambda: ev("location.search")
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    if not wait("[...document.querySelectorAll('.watch-chip')].some(c => c.textContent.includes('Place a relay'))", 120):
        return ["no 'Place a relay point' chip"]
    click_text(".watch-chip", "Place a relay")
    p0 = wait("(() => { const p = document.getElementById('place-relay'); return p && p.style.display !== 'none' ? p.style.left + ',' + p.style.top : null; })()", 40)
    card = wait("(() => { const d = document.getElementById('drill'); return d && d.textContent.includes('Relay stands here') ? d.innerText : null; })()", 20)
    chip = wait("[...document.querySelectorAll('.watch-chip')].map(c => c.textContent).find(t => t.includes('Relay placed')) || null", 20)
    print("relay pin:", p0, "| chip:", chip, "| link:", q())
    print("card:", (card or "").replace("\n", " | "))
    if not p0 or not card or not chip or "rl=" not in (q() or ""):
        fails.append("suggested relay not placed (pin, card, chip or link missing)")
    time.sleep(2)
    shot("relay")
    rl0 = q()
    # Move: arm, then click the map (real mouse events on the canvas, centre-left of the map area)
    click_text("#drill button", "Move")
    armed = wait("document.body.getAttribute('data-placing') === '1' || null", 10)
    print("armed:", armed, "| note:", ev("(document.querySelector('#drill .dr-note') || {}).textContent"))
    x, y = 640, 380
    rl_of = "(new URLSearchParams(location.search).get('rl'))"
    r0 = ev(rl_of)
    call("Input.dispatchMouseEvent", type="mouseMoved", x=x, y=y, button="none", buttons=0)
    time.sleep(1.5)
    call("Input.dispatchMouseEvent", type="mousePressed", x=x, y=y, button="left", buttons=1, clickCount=1)
    time.sleep(0.6)
    call("Input.dispatchMouseEvent", type="mouseReleased", x=x, y=y, button="left", buttons=0, clickCount=1)
    moved = wait("(() => { const r = " + rl_of + "; return r && r !== " + repr(r0) + " ? r : null; })()", 20)
    print("after move: rl", r0, "->", moved, "| armed now", ev("document.body.getAttribute('data-placing')"))
    if not moved:
        fails.append("Move + map click did not re-place the relay")
    time.sleep(1.5)
    shot("moved")
    click_text("#drill button", "Remove")
    gone = wait("(!document.getElementById('place-relay') && !location.search.includes('rl=')) || null", 20)
    print("removed:", gone, "| chip back:", ev("[...document.querySelectorAll('.watch-chip')].some(c => c.textContent.includes('Place a relay'))"))
    if not gone:
        fails.append("Remove did not drop the relay")
    # working spots
    w = click_text(".watch-chip", "Place safer working spots")
    print("work chip:", w)
    if w:
        wp = wait("document.querySelectorAll('.place-pin[id^=\"place-work-\"]').length || null", 20)
        wc = wait("(() => { const d = document.getElementById('drill'); return d && d.textContent.includes('Work from here') ? d.innerText : null; })()", 20)
        print("W pins:", wp, "| link has ws:", "ws=" in (q() or ""), "| card:", (wc or "").replace("\n", " | "))
        if not wp or not wc or "ws=" not in (q() or ""):
            fails.append("working spots not placed")
        time.sleep(2)
        shot("work")
    else:
        print("no working-spot chip on this plan (no steep target with a gentler spot)")
    # reopen from the link, Plan then Walk
    link = "http://127.0.0.1:8765/" + (q() or "")
    link = link + ("&" if "?" in link else "?") + "rl=150.0_400.0"
    call("Page.navigate", url=link)
    time.sleep(2)
    r = wait("(() => document.getElementById('place-relay') ? 1 : null)()", 120)
    print("reopened link: relay pin", r, "| W pins", ev("document.querySelectorAll('.place-pin[id^=\"place-work-\"]').length"))
    if not r:
        fails.append("link with rl= did not reopen the relay")
    call("Page.navigate", url=link + "&mode=walk")
    time.sleep(2)
    wr = wait("(() => document.getElementById('place-relay') ? 1 : null)()", 120)
    time.sleep(3)
    print("walk: relay pin element", wr, "| shown at", pin("place-relay"))
    shot("walk")
    if not wr:
        fails.append("relay pin missing in Walk")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
