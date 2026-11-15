"""Frame rate (requestAnimationFrame over 5 s) in Plan map, Walk first person and Walk third person (key C).
Usage: python tools/dev/probe_fps.py URL OUT.png [--gpu]"""
import os, sys, time, json
sys.stdout.reconfigure(encoding="utf-8")
sys.path.insert(0, os.path.dirname(__file__))
import browser_smoke as b

def probe(call, out):
    ev = lambda js: call("Runtime.evaluate", expression=js, returnByValue=True, awaitPromise=True).get("result", {}).get("value")
    print("renderer:", ev("""(() => { const c = document.createElement('canvas').getContext('webgl2');
        const x = c && c.getExtension('WEBGL_debug_renderer_info'); return x ? c.getParameter(x.UNMASKED_RENDERER_WEBGL) : 'n/a'; })()"""))
    time.sleep(3)
    ev("document.getElementById('visor').focus(); 1")
    for label in ("plan map", "walk", "walk third person"):
        if label == "walk":
            call("Input.dispatchKeyEvent", type="keyDown", key="Tab", code="Tab", windowsVirtualKeyCode=9)
            call("Input.dispatchKeyEvent", type="keyUp", key="Tab", code="Tab", windowsVirtualKeyCode=9)
            time.sleep(3)
        if label == "walk third person":
            call("Input.dispatchKeyEvent", type="keyDown", key="c", code="KeyC", windowsVirtualKeyCode=67)
            call("Input.dispatchKeyEvent", type="keyUp", key="c", code="KeyC", windowsVirtualKeyCode=67)
            time.sleep(2)
        r = ev("""new Promise(r => { let n = 0, mx = 0, last = performance.now(); const t0 = last;
            const f = () => { const now = performance.now(); mx = Math.max(mx, now - last); last = now; n++;
                              if (now - t0 < 5000) requestAnimationFrame(f); else r([n * 1000 / (now - t0), mx, document.body.dataset.mode]); };
            requestAnimationFrame(f); })""")
        print(f"{label}: {r[0]:.1f} frames/s, longest frame {r[1]:.0f} ms, mode {r[2]} (requestAnimationFrame over 5 s, headless)")
    return []

b.key_test = probe
sys.exit(b.main(sys.argv[1], sys.argv[2], keys=True, gpu="--gpu" in sys.argv[3:]))
