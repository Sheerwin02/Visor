"""Pipeline tests: each module's synthetic self-test, plus validation of the real assets/ bundle.
Run from the repository root:  python -m pytest tools/tests -q
Self-tests use synthetic data only (method checks, never Gale results)."""
import os

import pytest


def test_rover_check_method():
    import rover_check
    rover_check.selftest()


def test_mars24_sun_position():
    import mars_sun
    mars_sun.selftest()


def test_rad_method():
    import rad
    rad.selftest()


def test_conditions_method():
    import conditions
    conditions.selftest()


def test_heldout_bias_method():
    import heldout_bias
    heldout_bias.selftest()


def test_provenance_method():
    import provenance
    provenance.selftest()


def test_ridge_tracing_method():
    import ridges
    ridges.selftest()


def test_astronaut_parts():
    import astronaut
    astronaut.selftest()


def test_trek_tile_math():
    import trek
    trek.selftest()


def test_chemistry_parsing():
    import chem
    chem.selftest()


def test_navcam_panorama_geometry():
    import navcam_pano
    navcam_pano.selftest()


def test_photo_view_direction():
    import photo_view
    photo_view.selftest()


def test_trek_sun_parsing():
    import trek_sun
    trek_sun.selftest()


def test_curiosity_model():
    import curiosity
    curiosity.selftest()


def test_overview_tiles_and_traverse():
    import overview
    overview.selftest()


def test_route_planner():
    import route
    route.selftest()


def test_bundle_builder():
    import build_bundle
    build_bundle.selftest()


def test_shading_reference():
    import misleading_slope
    misleading_slope.selftest()


@pytest.mark.skipif(not os.path.exists("assets/manifest.json"), reason="no built bundle")
def test_real_bundle_is_valid():
    import validate_bundle
    assert validate_bundle.main("assets") == 0


@pytest.mark.skipif(not os.path.exists("data/interim/rover_check/rover_check_result.json"), reason="no measurement results")
def test_bundle_uses_measured_values_not_typed_ones():
    """The route's bias and the manifest's measured limits must come from the result files (single source of truth)."""
    import json
    import config
    rc, gate = config.rover_check_result(), config.gate_result()
    route = json.load(open("assets/route.json"))
    assert route["assumptions"]["bias_deg"] == rc["signed_median_deg"]
    limits = json.load(open("assets/manifest.json"))["measured_limits"]
    assert str(rc["median_abs_deg"]) in limits["rover_check"] and str(gate["patch_share_pct"]) in limits["gate_1m"]


def test_no_stale_paths_in_live_code():
    """Live pipeline code must not use the pre-restructure layout."""
    import glob
    import re
    bad = re.compile(r"visor/target|visor/terrain-core|data/(MSL_|localized_pos|navcam|ortho|rover_check|gate)\b|\.\./\.\./assets")
    hits = [f for f in glob.glob("tools/*.py") + glob.glob("app/src/**/*.rs", recursive=True) + ["app/index.html"] if bad.search(open(f, encoding="utf-8").read())]
    assert not hits, hits
