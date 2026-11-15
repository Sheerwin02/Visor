"""Browser probe for day / twilight / night (plan/daylight.rs): the default 08:00 EVA has no dark stretch; a 15:00 start
runs past sunset, so the EVA lane shows twilight and night bands with a note, and seeking into them turns the Walk chip
to Twilight / Night (screenshots of each). Usage: serve a trunk build on 127.0.0.1:8765, then
`python tools/dev/probe_day.py OUT.png [--gpu]`."""
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
    chip = "(() => { const c = document.getElementById('hud-day'); return c ? c.innerText + ' | ' + c.title : null; })()"
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk")
    time.sleep(2)
    c0 = wait(chip, 90)
    print("08:00 EVA chip:", c0, "| dark bands:", ev("document.querySelectorAll('svg.tl .tl-dark').length"))
    if not c0 or not c0.startswith("Daylight"):
        fails.append("08:00 EVA does not start in daylight")
    if ev("document.querySelectorAll('svg.tl .tl-dark').length"):
        fails.append("08:00 EVA shows a dark band")
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk&hr=15")
    time.sleep(2)
    wait(chip, 90)
    n = wait("document.querySelectorAll('svg.tl .tl-dark').length || null", 30)
    note = ev("(document.querySelector('.tl-dark-note') || {}).textContent")
    print("15:00 EVA: dark bands", n, "| note:", note, "| chip:", ev(chip))
    if not n or not note:
        fails.append("15:00 EVA shows no twilight / night band or note")
    shot("day")

    def seek(cls):
        # click the middle of the first band of that kind on the timeline's hit area
        return ev("(() => { const r = document.querySelector('svg.tl .tl-dark." + cls + "'); if (!r) return null;"
                  " const b = r.getBoundingClientRect(); const h = document.querySelector('svg.tl rect.hit'); const hb = h.getBoundingClientRect();"
                  " h.dispatchEvent(new MouseEvent('click', {bubbles: true, clientX: b.left + b.width / 2, clientY: hb.top + 10})); return 1; })()")

    # Plan view of the same plan: the note sits in the timeline header
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&hr=15")
    time.sleep(2)
    pn = wait("(() => { const n = document.querySelector('.tl-dark-note'); return n && n.offsetParent ? n.textContent : null; })()", 60)
    chip_txt = ev("[...document.querySelectorAll('.watch-chip')].map(c => c.textContent).join(' | ')")
    print("Plan: timeline note", pn, "| watch chips:", chip_txt)
    if "Twilight from" not in (chip_txt or ""):
        fails.append("Plan has no twilight watch chip")
    if not pn:
        fails.append("Plan timeline shows no dark note")
    shot("plan")
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1&mode=walk&hr=15")
    time.sleep(2)
    wait("document.querySelectorAll('svg.tl .tl-dark').length || null", 90)
    for cls, word in (("twilight", "Twilight"), ("night", "Night")):
        if not seek(cls):
            print("no", cls, "band")
            fails.append(f"no {cls} band to seek to")
            continue
        got = wait(f"(() => {{ const c = document.getElementById('hud-day'); return c && c.innerText === '{word}' ? c.title : null; }})()", 20)
        print(f"seek into {cls}: clock", ev("(document.querySelector('.tb-clock') || {}).innerText"), "| chip tip:", got)
        if not got:
            fails.append(f"chip did not turn {word}")
        time.sleep(3)
        shot(cls)
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
