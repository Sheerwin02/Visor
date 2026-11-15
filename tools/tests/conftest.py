"""Make the pipeline modules in tools/ importable from the tests, and run from the repository root."""
import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tools"))
os.chdir(ROOT)
