# tools/archive: scripts for features cut from the plan

Kept for history and the AI-use record; not part of the live pipeline and not maintained.
Run them from inside this folder. `make_stimuli.py` and `synth_boxwork.py` import `misleading_slope`, which stays in `tools/`:
run them with `PYTHONPATH=..` (or copy the module). All results here are SYNTHETIC, never Gale findings.

| Script | Was for | Cut because |
| --- | --- | --- |
| skyfix.py, skyfix_realdem.py (+ results) | GPS-free fix from horizon matching | Fix gate dropped from the plan |
| searchsim.py, compare.py (+ results) | search-time curve, greedy vs spiral | search estimate cut |
| headtohead_sim.py, sand_sensitivity.py, sim_lib.py, baseline_test.py | route planner comparisons, terrain factor, slope baseline | robust/effort routing not in the core |
| synth_boxwork.py (+ results) | looks-flat share on synthetic boxwork | looks-flat headline dropped |
| make_stimuli.py | human estimation test images | estimation test cut |
| l11_spike/ | challenge #11 spike | challenge switch removed from the gate |
