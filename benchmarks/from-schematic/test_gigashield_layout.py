# Copyright (C) 2026 Florian Loitsch.

import copy
import importlib.util
from pathlib import Path


spec = importlib.util.spec_from_file_location("gigashield_layout", Path(__file__).with_name("gigashield-layout.py"))
replay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replay)


def test_only_zone_policy_changes_are_separated_from_pad_identity():
    pad = {"net": "GND", "function": "", "geometry": {"size": "(size 0.9 0.9)"}}
    with_override = copy.deepcopy(pad)
    with_override["geometry"]["zone_connect"] = "(zone_connect 2)"
    change = {"reference": "C28", "pad": "2", "source": [pad], "candidate": [with_override]}
    assert replay.zone_override(change)
    assert not replay.zone_override({**change, "source": None})
    assert not replay.zone_override({**change, "candidate": [{**with_override, "net": "VCC"}]})
    with_override["geometry"]["size"] = "(size 1.1 0.9)"
    assert not replay.zone_override(change)
