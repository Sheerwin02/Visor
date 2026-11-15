"""Runs relight.wgsl through wgpu (software adapter is fine) and checks it against the numpy reference.
  pip install wgpu numpy ; python test_relight.py
Checks: (1) Lambertian shade equals tools/misleading_slope.shade on a random surface (interior, no smoothing);
(2) cast shadow length behind a wall equals height / tan(sun elevation); (3) the 25-degree cross-sun plane is ~9% darker than flat.
Also exposes run_relight(dem, cell, el_deg, az_deg, ...) for use on the real DEM."""
import os, sys, struct, numpy as np, wgpu
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import misleading_slope as ms

SRC = open(os.path.join(os.path.dirname(__file__), "relight.wgsl")).read()

def run_relight(dem, cell, el_deg, az_deg, steps=256, max_dist=None):
    h, w = dem.shape
    max_dist = max_dist or float(max(h, w)) * cell
    dev = wgpu.gpu.request_adapter_sync().request_device_sync()
    mod = dev.create_shader_module(code=SRC)
    params = struct.pack("<4I4f", w, h, steps, 0, cell, np.radians(el_deg), np.radians(az_deg), max_dist)
    ub = dev.create_buffer_with_data(data=params, usage=wgpu.BufferUsage.UNIFORM)
    db = dev.create_buffer_with_data(data=dem.astype("<f4").tobytes(), usage=wgpu.BufferUsage.STORAGE)
    sb = dev.create_buffer(size=w * h * 4, usage=wgpu.BufferUsage.STORAGE | wgpu.BufferUsage.COPY_SRC)
    hb = dev.create_buffer(size=w * h * 4, usage=wgpu.BufferUsage.STORAGE | wgpu.BufferUsage.COPY_SRC)
    pipe = dev.create_compute_pipeline(layout="auto", compute={"module": mod, "entry_point": "main"})
    bg = dev.create_bind_group(layout=pipe.get_bind_group_layout(0), entries=[
        {"binding": i, "resource": {"buffer": b, "offset": 0, "size": b.size}} for i, b in enumerate((ub, db, sb, hb))])
    enc = dev.create_command_encoder()
    p = enc.begin_compute_pass(); p.set_pipeline(pipe); p.set_bind_group(0, bg)
    p.dispatch_workgroups((w + 7) // 8, (h + 7) // 8, 1); p.end()
    dev.queue.submit([enc.finish()])
    shade = np.frombuffer(dev.queue.read_buffer(sb), "<f4").reshape(h, w)
    shad = np.frombuffer(dev.queue.read_buffer(hb), "<u4").reshape(h, w)
    return shade, shad

if __name__ == "__main__":
    rng = np.random.default_rng(1)
    dem = np.cumsum(np.cumsum(rng.normal(0, .05, (96, 96)), 0), 1)
    cell = 1.0
    # (1) Lambert vs reference, shadows disabled by huge sun elevation? use 60 deg and compare unshadowed pixels only
    slope, down_az = ms.slope_aspect(dem, cell, smooth_m=0)
    # slope_aspect uses np.gradient on the raw dem when smooth_m=0 -> same central differences
    for el, az in ((15, 0), (30, 135), (60, 250)):
        sh, sd = run_relight(dem, cell, el, az, steps=96)
        ref = ms.shade(slope, down_az, el, az)
        m = np.zeros(dem.shape, bool); m[2:-2, 2:-2] = True; m &= (sd == 0)
        err = np.abs(sh[m] - ref[m]).max()
        print("lambert el %d az %3d  max |shader - numpy| = %.2e over %d px" % (el, az, err, m.sum()))
        assert err < 1e-4
    # (2) wall shadow: flat plane z=0 with a 5 m high wall (columns 40-41); sun in the east (az 90), el 20 deg => shadow to the WEST of the wall
    wall = np.zeros((40, 120)); wall[:, 80:82] = 5.0
    sh, sd = run_relight(wall, 1.0, 20.0, 90.0, steps=400, max_dist=60.0)
    row = sd[20]
    shadowed_cols = np.where(row[:80] == 1)[0]
    exp = 5.0 / np.tan(np.radians(20.0))
    print("wall shadow length %.1f m, expected %.1f m (5 / tan 20)" % (len(shadowed_cols), exp))
    assert abs(len(shadowed_cols) - exp) <= 2.0
    assert sd[20, 90] == 0 and sd[20, 100] == 0      # east of the wall (sun side): lit
    # (3) 25 degree plane across the sun
    xs = np.arange(120.0)
    plane = -np.tan(np.radians(25.0)) * np.tile(xs, (120, 1))
    sh, sd = run_relight(plane, 1.0, 30.0, 180.0, steps=60, max_dist=60.0)
    flat = np.sin(np.radians(30.0))
    v = sh[30:90, 30:90].mean()
    print("25 deg cross-sun plane brightness %.4f vs flat %.4f -> %.1f%% darker" % (v, flat, 100 * (1 - v / flat)))
    assert abs((1 - v / flat) - 0.094) < 0.01
    print("ALL SHADER TESTS PASSED (software adapter; not run in Bevy)")
