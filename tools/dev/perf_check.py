"""Speed budget for the web client, on this machine's real GPU (headless Chrome, --use-angle=d3d11).

Budgets (fail the run if broken):
  - after the panel shows: no main-thread task over 50 ms (web.dev's "long task") during normal use: idle, a slope-limit
    change, the rover-correction switch, layer keys 2/4/8, Plan<->Walk twice, map/3D view (V), opening a stop card;
  - load: the panel shows within LOAD_BUDGET_MS of navigation, with at most LOAD_BLOCK_BUDGET_MS of long tasks in total.
Load numbers depend on the machine and the build (dev or release); they are printed so they can be compared over time.
Software rendering (SwiftShader) runs at about 1 frame/s and cannot meet these budgets, so this check always uses the GPU.

Usage: python tools/dev/perf_check.py http://127.0.0.1:8765/ out.png
"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import browser_smoke as b  # noqa: E402

TASK_BUDGET_MS = 50
LOAD_BUDGET_MS = 3000
LOAD_BLOCK_BUDGET_MS = 1500

EARLY = """window.__lt = []; try { new PerformanceObserver(l => l.getEntries().forEach(e => __lt.push([Math.round(e.startTime), Math.round(e.duration)])))
  .observe({type: 'longtask', buffered: true}); } catch (e) {}
window.__panelAt = null; new MutationObserver(() => { if (!window.__panelAt && document.querySelector('#mission-bar .mb-stat')) window.__panelAt = Math.round(performance.now()); })
  .observe(document, {subtree: true, childList: true});"""


def check(call, out):
    ev = lambda js: call("Runtime.evaluate", expression=js, returnByValue=True).get("result", {}).get("value")
    fails = []
    call("Page.addScriptToEvaluateOnNewDocument", source=EARLY)
    call("Network.enable")
    call("Network.setCacheDisabled", cacheDisabled=True)
    call("Page.reload", ignoreCache=True)
    t0 = time.time()
    while time.time() - t0 < 90 and not ev("window.__panelAt"):
        time.sleep(0.3)
    time.sleep(3)
    panel_at = ev("window.__panelAt")
    lt = ev("window.__lt") or []
    block = sum(d for _, d in lt)
    print(f"load: panel at {panel_at} ms; {len(lt)} long tasks, {block} ms in total, longest {max([d for _, d in lt] or [0])} ms")
    if not panel_at or panel_at > LOAD_BUDGET_MS:
        fails.append(f"panel at {panel_at} ms (budget {LOAD_BUDGET_MS})")
    if block > LOAD_BLOCK_BUDGET_MS:
        fails.append(f"load blocked {block} ms (budget {LOAD_BLOCK_BUDGET_MS})")

    def key(k, code, vk):
        call("Input.dispatchKeyEvent", type="keyDown", key=k, code=code, windowsVirtualKeyCode=vk)
        call("Input.dispatchKeyEvent", type="keyUp", key=k, code=code, windowsVirtualKeyCode=vk)

    def step(name, action, wait=2.0):
        ev("window.__lt = []; 1")
        action()
        time.sleep(wait)
        tasks = [d for _, d in (ev("window.__lt") or [])]
        worst = max(tasks or [0])
        ok = worst <= TASK_BUDGET_MS
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: longest task {worst} ms {tasks if tasks else ''}")
        if not ok:
            fails.append(f"{name}: {worst} ms task")

    focus = lambda: ev("document.activeElement && document.activeElement.blur(); document.getElementById('visor').focus(); 1")
    step("idle", lambda: None, 3)
    step("slope limit 18", lambda: ev("(() => { const i = document.querySelector('.controls input[type=range]'); i.value = '18'; i.dispatchEvent(new Event('change')); return 1; })()"), 4)
    step("rover correction off", lambda: ev("(() => { const c = document.querySelector('.controls input[type=checkbox]'); c.click(); return 1; })()"), 4)
    step("rover correction on", lambda: ev("(() => { const c = document.querySelector('.controls input[type=checkbox]'); c.click(); return 1; })()"), 4)
    for k, code, vk in (("2", "Digit2", 50), ("4", "Digit4", 52), ("8", "Digit8", 56)):
        step(f"layer key {k}", lambda k=k, code=code, vk=vk: (focus(), key(k, code, vk)))
    step("to Walk", lambda: (focus(), key("Tab", "Tab", 9)))
    step("to Plan", lambda: (focus(), key("Tab", "Tab", 9)))
    step("to Walk again", lambda: (focus(), key("Tab", "Tab", 9)))
    step("to Plan again", lambda: (focus(), key("Tab", "Tab", 9)))
    step("3D view (V)", lambda: (focus(), key("v", "KeyV", 86)))
    step("map view (V)", lambda: (focus(), key("v", "KeyV", 86)))
    step("open a stop card", lambda: ev("document.getElementById('flag-label-4').click(); 1"))
    step("close it (Esc)", lambda: key("Escape", "Escape", 27))
    step("best order", lambda: ev("[...document.querySelectorAll('button')].find(b => b.textContent === 'Best order').click(); 1"), 3)
    step("start 17:00 (shadows)", lambda: ev("(() => { const s = document.querySelector('.start-in'); s.value = '17'; s.dispatchEvent(new Event('change')); return 1; })()"), 4)
    step("another sol (4600)", lambda: ev("(() => { const i = document.querySelector('.sol-in'); i.value = '4600'; i.dispatchEvent(new Event('change')); return 1; })()"), 5)
    step("inspector tabs", lambda: ev("[...document.querySelectorAll('.tabs button')].forEach(b => b.click()); 1"), 2)
    step("layer rail switch", lambda: ev("[...document.querySelectorAll('.layer')].find(b => b.textContent.includes('Route agreement')).click(); 1"), 2)
    step("layer key 9", lambda: (focus(), key("9", "Digit9", 57)))
    return fails


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    b.key_test = check
    code = b.main(sys.argv[1], sys.argv[2], keys=True, gpu=True)
    sys.exit(code)
