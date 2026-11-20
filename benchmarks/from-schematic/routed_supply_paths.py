# Copyright (C) 2026 Florian Loitsch.
"""Reuse the production CLI's native path measurement in standalone scoring."""

import importlib.util
from pathlib import Path
import sys

source = Path(__file__).resolve().parents[2] / "crates/pcb-kicad-check/src/routed_supply_paths.py"
spec = importlib.util.spec_from_file_location("_pcb_maker_native_routed_paths", source)
native = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = native
spec.loader.exec_module(native)

UnsupportedPath = native.UnsupportedPath
make_graph = native.make_graph
measure_near_paths = native.measure_near_paths
measure_supply_paths = native.measure_supply_paths
shortest = native.shortest
