# VISOR

**Measured, then used: a walking-scale Mars map that corrects itself with what the rover found.**
NASA Space Apps Challenge 2026, "Interplanetary Survival Guide: Martian Map". Gan Sheer Win, Team Wynspace.

- **Live app (no login):** https://sheerwin02.github.io/Visor/ (desktop browser with WebGL2)
- **30 s demo video:** https://www.youtube.com/watch?v=PEEnreSS2ps

VISOR plans a two-person Marswalk across the boxwork ridges of Mount Sharp, Gale crater, through the two sites Curiosity
drilled (Valle de la Luna in a hollow, Nevado Sajama on a ridge). It layers the USGS 1 m elevation model and 25 cm HiRISE
photo with what Curiosity measured on the same ground (tilt, chemistry, weather, radiation, Navcam images) and shows where
the map can be trusted. Plan the route, check each station, rehearse it in a first-person Walk, and share or print the plan.

Not endorsed by NASA, JPL, the USGS or any mission team. Sources and credits: [CREDITS.md](CREDITS.md).

## What was measured (rules written before each run)

| Test | Result |
| --- | --- |
| Does the 1 m DEM resolve ridge slopes above its noise? | **No at 1 m**: 28.5% of the patch reads 15 deg or steeper, but so does 23.4% of ground the rover found gentle. |
| Does the DEM match Curiosity's measured tilt at 85 stops? (builds on similar checks by other teams) | **Does not agree** at rover scale: median difference 4.4 deg, p90 14.4 deg; DEM about 3.4 deg too steep; still beats chance (r 0.47). |
| Does that bias correction hold on held-out sols? | **Pass**: held-out median error 4.43 to 3.86 deg; sign stable, size not. |
| Route through all six stops | All legs possible: 451 m, about 18.5 min of walking at an assumed 0.6 m/s. |

## Assumptions (labelled wherever they appear)

20 deg slope limit; 0.6 m/s flat walking speed (NASA lunar intrasite range 0.4-0.8 m/s); Tobler-shaped slope penalty
(relative only); 3 m "rover-verified" zones; 1.7 m eye height for line of sight; twilight up to 2 h (NASA). No suit,
gravity or consumables model.

## Run

```
cd app && trunk serve                          # http://127.0.0.1:8080
cd app && trunk build --cargo-profile wasm-release   # public build
```

## Test

```
cargo test                        # crates: prep, terrain-core, route-core
cargo test -p visor-app           # web client (native)
python -m pytest tools/tests -q   # pipeline self-tests + real-bundle checks
python tools/validate_bundle.py   # bundle hashes, sources, limits
python tools/dev/browser_smoke.py http://127.0.0.1:8765/ out.png --keys   # headless Chrome on a served build
```

## Repository

```
app/        Bevy web client (see app/README.md)        assets/   the generated data bundle the app loads
crates/     prep (DEM reader), terrain-core, route-core, plan-worker
tools/      Python data pipeline, sources/, tests/, dev/ (browser checks)
data/       git-ignored raw downloads and measurement outputs
```

## Rebuild the data (optional: `assets/` is already built)

Python 3.12 (`pip install -r tools/requirements.txt`) and Rust (pinned in `rust-toolchain.toml`). Download the USGS DEM
(`MSL_Gale_DEM_Mosaic_1m_v3.tif`) and PLACES `localized_pos.csv` into `data/raw/` (URLs in CREDITS.md), then run the
measurements (`tools/gate_patch.py`, `tools/rover_check.py`), `tools/build_bundle.py build` and `tools/validate_bundle.py`.
Each script's docstring gives its inputs.

## Use of Super Intelligence (SI)

VISOR is the author's idea and design; the author set the scope, approved every test rule before it ran and made each
decision. An SI (AI) assistant, Claude by Anthropic through Claude Code, was used as a tool for:

- **Research:** reading and summarising NASA, USGS, PDS, JPL and HiRISE documents and data formats. Every fact shown in
  the app links to its source, with the quote checked on the live page.
- **Code:** most of the Rust/Bevy client and Leptos interface, the route and sun crates, the Python data pipeline and the
  tests were written with the assistant.
- **Analysis runs:** the measurements were run with the assistant's help under rules the author fixed in advance, never
  changed after a result; null and failed results are reported as they came out.
- **Writing:** documentation, credits and on-screen text.

Checked by: 170 client tests, Rust crate tests, Python tests, a bundle validator and headless-browser checks; results
cross-checked against independent references (Python reference code, NASA Mars Trek sun angles, the REMS sunrise/sunset
feed, Curiosity's measured tilt). No images or video footage were SI-generated (the demo video is a screen recording of
the app); the demo video's narration is an SI-generated voice (Qwen3-TTS), labelled on screen and in the file's metadata. No NASA
data was altered.

## Licence

Copyright (C) 2026 Gan Sheer Win (Team Wynspace).

The VISOR code is licensed under the **GNU Affero General Public License v3.0 only** ([LICENSE](LICENSE)). You may use, study,
change and share it, but any copy or changed version you distribute or run as a public service must keep this notice and be
released under the same licence, with its full source. Data, images and models in `assets/` and credited in
[CREDITS.md](CREDITS.md) (NASA, USGS, University of Arizona, NASA 3D Resources) keep their own terms and are not covered by
this licence. Libraries VISOR uses keep their own licences (listed in CREDITS.md).
