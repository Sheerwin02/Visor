"""Browser probe for the ops panel (redesign: Plan / Station / Risk / About): the station table, the bags, "if we
run late" (drop + Apply re-plans), the Station view (question first, four numbers) and the Risk tab (exceptions first).
Prints the visible word count per tab. Usage: serve a trunk build on 127.0.0.1:8765, then
`python tools/dev/probe_ops.py OUT.png [--gpu]`."""
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
    words = "document.querySelector('.tab-body').innerText.split(/\\s+/).filter(Boolean).length"
    fails = []
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    names = wait("(() => { const t = [...document.querySelectorAll('.tabs button')].map(b => b.textContent); return t.length ? t : null; })()", 90)
    print("tabs:", names)
    if names != ["Plan", "Station", "Log", "Risk", "Science", "About"]:
        fails.append(f"tabs {names}")
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 30)
    time.sleep(2)
    print("budget:", ev("(document.querySelector('.budget')||{}).title"))
    rows = ev("[...document.querySelectorAll('table.stations tbody tr')].map(r => r.innerText.replace(/\\s+/g, ' '))")
    for r in rows or []:
        print("  ", r)
    bags = ev("document.querySelectorAll('ol.bags li').length")
    print("bags:", bags, "| Plan words:", ev(words))
    if (bags or 0) < 5:
        fails.append("fewer than 5 bags")
    shot("plan")
    ev("[...document.querySelectorAll('.late .seg button')].find(b => b.textContent === '+30').click(); 1")
    late = wait("(() => { const l = document.querySelector('.late-line'); return l && !l.innerText.startsWith('On time') ? l.innerText : null; })()", 10)
    print("+30 min:", late)
    n0 = ev("document.querySelectorAll('table.stations tbody tr').length")
    ev("document.querySelector('.late button.primary') && document.querySelector('.late button.primary').click(); 1")
    n1 = wait(f"(() => {{ const n = document.querySelectorAll('table.stations tbody tr').length; return n < {n0} ? n : null; }})()", 40)
    print("stations before/after Apply:", n0, n1)
    if not late or not n1:
        fails.append("run-late drop or Apply did not work")
    ev("[...document.querySelectorAll('.ctl-row button')].find(b => b.classList.contains('ghost') && !b.disabled)?.click(); 1")
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 40)
    # Station: open Valle de la Luna from the table
    ev("[...document.querySelectorAll('table.stations .sn')].find(b => b.textContent.includes('Valle')).click(); 1")
    q = wait("document.querySelector('.card.station .question')?.innerText", 15)
    print("station tab active:", ev("document.querySelector('.tabs button[aria-selected=\"true\"]').textContent"), "| question:", (q or "")[:120])
    print("numbers:", ev("[...document.querySelectorAll('.nums .n')].map(n => n.innerText.replace(/\\s+/g, ' ')).join(' | ')"), "| words:", ev(words))
    if not q:
        fails.append("Station view without a question")
    shot("station")
    tab("Risk")
    time.sleep(1)
    print("risk:", (ev("document.querySelector('.risk').innerText.replace(/\\n+/g, ' | ')") or "")[:500], "| words:", ev(words))
    shot("risk")
    tab("About")
    time.sleep(1)
    print("About words:", ev(words))
    # plain-language safety line, glossary, whole-row click, Add a stop
    tab("Plan")
    time.sleep(1)
    sl = wait("(() => { const x = document.querySelector('.safety'); return x ? x.innerText.split(String.fromCharCode(10)).join(' | ') : null; })()", 10)
    print("safety line:", sl)
    chips = ev("document.querySelectorAll('.safety .watch-chip').length")
    if not sl or "watch" not in sl.lower() or (chips or 0) < 2:
        fails.append("no safety line with watch chips on Plan")
    print("glossary terms:", ev("document.querySelectorAll('details.glossary dt').length"))
    ev("document.querySelectorAll('table.stations tbody tr')[2].querySelectorAll('td')[1].click(); 1")
    t2 = wait("(() => { const h = document.getElementById('card-title'); return h ? h.innerText : null; })()", 10)
    print("row click opened:", t2)
    if not t2:
        fails.append("clicking a station row did not open it")
    tab("Plan")
    time.sleep(1)
    ev("[...document.querySelectorAll('.ctl-row.plans button')].find(b => b.textContent === 'Add a stop').click(); 1")
    note = wait("(() => { const p = [...document.querySelectorAll('#inspector p.fine')].find(x => x.innerText.startsWith('Click the map')); return p ? p.innerText : null; })()", 10)
    print("add-a-stop note:", note)
    if not note:
        fails.append("Add a stop did not start picking")
    ev("document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape'})); 1")
    shot("safety")
    # conditions for this EVA and the data-kinds key
    tab("Plan")
    time.sleep(1)
    cond = ev("(() => { const c = document.querySelector('.cond'); return c ? c.textContent.split(String.fromCharCode(10)).join(' | ') : null; })()")
    print("conditions:", cond)
    if not cond or "Radiation" not in cond:
        fails.append("no conditions with radiation on Plan")
    tab("About")
    time.sleep(1)
    print("data kinds rows:", ev("document.querySelectorAll('table.kinds tbody tr').length"))
    shot("about")
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
