"""Browser probe for execute mode and cue cards: in the Log tab, an arrival typed late at a station gives "Behind by N min"
and the run-late drop; a task check-off survives a reload (browser storage); the log CSV downloads; Print cue cards fills
#cue-print with one page per station (rendered with print media and saved as a PDF next to OUT).
Usage: serve a trunk build on 127.0.0.1:8765, then `python tools/dev/probe_exec.py OUT.png [--gpu]`."""
import base64
import glob
import os
import sys
import tempfile
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
    fails = []
    dl = tempfile.mkdtemp(prefix="visor_dl_")
    call("Browser.setDownloadBehavior", behavior="allow", downloadPath=dl)
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 90)
    time.sleep(3)
    tab("Log")
    wait("document.querySelectorAll('.log-st').length >= 5 ? 1 : null", 10)
    ev("try { localStorage.removeItem('visor.eva_log.v1') } catch (e) {} ; 1")
    ev("[...document.querySelectorAll('.ctl-row.share button')].find(b => b.textContent.includes('Clear')).click(); 1")
    time.sleep(1)
    print("head (empty):", ev("document.querySelector('.go-line').innerText"))
    # arrive at station 2 (Bococo, planned 08:30) at 08:58
    ev("(() => { const i = document.querySelectorAll('.log-st')[0].querySelector('input.t'); i.value = '08:58'; i.dispatchEvent(new Event('change', {bubbles: true})); })(); 1")
    head = wait("(() => { const g = document.querySelector('.go-line'); return g && g.innerText.startsWith('Behind') ? g.innerText : null; })()", 10)
    print("head (arrived 08:58):", head)
    if not head or "drop" not in head:
        fails.append("no lateness / drop line after a late arrival")
    ev("document.querySelectorAll('.log-st')[0].querySelector('ul.checks input').click(); 1")
    time.sleep(1)
    print("summary:", ev("[...document.querySelectorAll('.tab-body table.legs tbody tr')].slice(0, 2).map(r => r.innerText.replace(/\\s+/g, ' ')).join(' | ')"))
    shot("log")
    # CSV
    ev("[...document.querySelectorAll('.ctl-row.share button')].find(b => b.textContent.includes('CSV')).click(); 1")
    f = None
    for _ in range(20):
        f = next(iter(glob.glob(os.path.join(dl, "visor_eva_log*.csv"))), None)
        if f:
            break
        time.sleep(0.5)
    print("csv:", open(f, encoding="utf-8").read().splitlines()[:2] if f else None)
    if not f:
        fails.append("no log CSV")
    # reload: the log survives
    call("Page.navigate", url="http://127.0.0.1:8765/?v=1")
    time.sleep(2)
    wait("document.querySelectorAll('table.stations tbody tr').length >= 6 ? 1 : null", 90)
    time.sleep(2)
    tab("Log")
    kept = wait("(() => { const s = document.querySelectorAll('.log-st')[0]; return s && s.querySelector('input.t').value === '08:58' && s.querySelector('ul.checks input').checked ? 1 : null; })()", 10)
    print("kept after reload:", bool(kept))
    if not kept:
        fails.append("log not kept after reload")
    # cue cards
    tab("Plan")
    time.sleep(1)
    ev("window.print = () => { window.__printed = (window.__printed || 0) + 1; }; 1")
    ev("[...document.querySelectorAll('.ctl-row button')].find(b => b.textContent === 'Print cue cards').click(); 1")
    n = wait("(() => { const c = document.querySelectorAll('#cue-print .cc'); return c.length ? c.length : null; })()", 10)
    print("cue cards:", n, "| print called:", ev("window.__printed"), "| first:",
          (ev("document.querySelector('#cue-print .cc').innerText.replace(/\\n+/g, ' | ')") or "")[:420])
    if n != 5:
        fails.append(f"expected 5 cue cards, got {n}")
    call("Emulation.setEmulatedMedia", media="print")
    time.sleep(2)
    pdf = call("Page.printToPDF", printBackground=True, preferCSSPageSize=True)
    open(out.replace(".png", "_cards.pdf"), "wb").write(base64.b64decode(pdf["data"]))
    call("Emulation.setEmulatedMedia", media="")
    print("pdf:", out.replace(".png", "_cards.pdf"))
    return fails


b.key_test = probe
sys.exit(b.main("http://127.0.0.1:8765/", sys.argv[1], keys=True, gpu="--gpu" in sys.argv))
