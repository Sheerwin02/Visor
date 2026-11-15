"""Single source of truth for the VISOR pipeline: paths (relative to the repository root) and study constants.
Measured values (gate result, rover-check bias) are NOT typed here: they are read from the result files the measuring
scripts write (data/interim/...), through gate_result() and rover_check_result()."""
import json
import os

# --- study definition (chosen before any measurement; see the doc) ---
MARS_RADIUS_M = 3396190.0          # Mars 2000 sphere, the DEM's own projection (GeoTIFF tags)
SOL_FIRST, SOL_LAST = 4534, 4865   # Curiosity's boxwork campaign sols covered by the study (PLACES)

# --- repository paths ---
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PREP = os.path.join(ROOT, "target", "release", "prep")
RAW = os.path.join(ROOT, "data", "raw")
INTERIM = os.path.join(ROOT, "data", "interim")
PROCESSED = os.path.join(ROOT, "data", "processed")
DEM = os.path.join(RAW, "MSL_Gale_DEM_Mosaic_1m_v3.tif")
PLACES = os.path.join(RAW, "localized_pos.csv")
NAVCAM_DIR = os.path.join(RAW, "navcam")
ORTHO = os.path.join(PROCESSED, "ortho", "ortho_patch.tif")
GATE_DIR = os.path.join(INTERIM, "gate")
ROVER_CHECK_DIR = os.path.join(INTERIM, "rover_check")
GATE_RESULT = os.path.join(GATE_DIR, "gate_result.json")
ROVER_CHECK_RESULT = os.path.join(ROVER_CHECK_DIR, "rover_check_result.json")
HELDOUT_RESULT = os.path.join(INTERIM, "heldout_bias", "heldout_bias_result.json")
STOPS_SOURCE = os.path.join(ROOT, "tools", "sources", "stops_source.json")
ASSETS = os.path.join(ROOT, "assets")


def _load(path, what):
    if not os.path.exists(path):
        raise FileNotFoundError(f"{what} not found at {path}: run the measuring script first (see README)")
    return json.load(open(path, encoding="utf-8"))


def gate_result():
    """Written by tools/gate_patch.py --run."""
    return _load(GATE_RESULT, "gate result")


def heldout_result():
    """Written by tools/heldout_bias.py run. Optional: None when the held-out check has not been run."""
    return _load(HELDOUT_RESULT, "held-out bias result") if os.path.exists(HELDOUT_RESULT) else None


def rover_check_result():
    """Written by tools/rover_check.py run. 'signed_median_deg' is the measured DEM slope bias used by route and navcam check."""
    return _load(ROVER_CHECK_RESULT, "rover-check result")
