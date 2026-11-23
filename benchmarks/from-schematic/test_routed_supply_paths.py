# Copyright (C) 2026 Florian Loitsch.

import pytest

from test_mechanical_runner import run
from routed_supply_paths import measure_near_paths, measure_supply_paths


VIA = '(via (at 8 4) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net "SUPPLY"))'
ZONE = """(zone (net "SUPPLY") (layer "F.Cu") (hatch edge 0.5)
    (connect_pads (clearance 0.2)) (min_thickness 0.25)
    (polygon (pts (xy 6 1) (xy 12 1) (xy 12 3) (xy 6 3))))"""
INTERMEDIATE_UUID = "f2303517-9354-45c7-ae3e-9d890c5b8121"


def fixture(tmp_path, contact, extra="", source_pads="", target_pads=""):
    pcbnew = pytest.importorskip("pcbnew")
    board_path = tmp_path / "test.kicad_pcb"
    board_path.write_text(
        f"""(kicad_pcb (version 20241229) (generator "pcbnew")
  (general (thickness 1.6))
  (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (footprint "test" (layer "F.Cu") (at 2 2)
    (property "Reference" "C1")
    (pad "1" smd rect (at 0 0) (size 2 2) (layers "F.Cu") (net "SUPPLY")) {source_pads})
  (footprint "test" (layer "B.Cu") (at 8 8)
    (property "Reference" "U1")
    (pad "1" smd rect (at 0 0) (size 2 2) (layers "B.Cu") (net "SUPPLY")) {target_pads})
  (segment (start 2.25 2) (end 12 2) (width 0.2) (layer "F.Cu") (net "SUPPLY"))
  (segment (start 8 2) (end 8 4) (width 0.2) (layer "F.Cu") (net "SUPPLY"))
  (segment (start 8 4) (end 8 7.75) (width 0.2) (layer "B.Cu") (net "SUPPLY"))
  {contact}
  {extra})"""
    )
    return pcbnew, board_path


def measure(tmp_path, contact, extra=""):
    pcbnew, board_path = fixture(tmp_path, contact, extra)
    board = pcbnew.LoadBoard(str(board_path))
    return measure_supply_paths(pcbnew, board, [("C1:1", "U1:1")])[0]


def test_supply_path_uses_pad_copper_mid_track_branch_and_via(tmp_path):
    row = measure(
        tmp_path,
        VIA,
    )
    assert row["status"] == "measured"
    assert row["trace_length_mm"] == pytest.approx(9.2, abs=0.001)
    assert row["trace_segments"][0]["from_mm"] == [3.0, 2.0]
    assert row["trace_segments"][0]["to_mm"] == [8.0, 2.0]
    assert row["trace_segments"][-1]["to_mm"] == [8.0, 7.0]
    assert row["layers"] == ["F.Cu", "B.Cu"]
    assert row["layer_changes"] == [
        {"kind": "via", "contact": row["vias"][0], "from": "F.Cu", "to": "B.Cu"}
    ]


def test_plated_through_pad_can_join_layers(tmp_path):
    row = measure(
        tmp_path,
        f"""(footprint "test" (layer "F.Cu") (at 8 4)
    (property "Reference" "J1")
    (pad "1" thru_hole rect (at 0 0) (size 1 1) (drill 0.3)
         (layers "*.Cu" "*.Mask") (net "SUPPLY") (uuid "{INTERMEDIATE_UUID}"))
    (pad "1" smd rect (at 2 2) (size 1 1) (layers "F.Cu") (net "SUPPLY")))""",
    )
    assert row["status"] == "measured"
    assert row["trace_length_mm"] == pytest.approx(9)
    assert row["intermediate_pads"] == ["J1:1"]
    assert row["intermediate_pad_contacts"] == [
        {"pad": "J1:1", "pad_uuid": INTERMEDIATE_UUID,
         "centre_mm": [8, 4], "distance_from_pad_chord_mm": pytest.approx(2**1.5)}
    ]
    assert row["vias"] == []
    assert row["layer_changes"] == [
        {"kind": "pad", "contact": "J1:1", "pad_uuid": INTERMEDIATE_UUID, "from": "F.Cu", "to": "B.Cu"}
    ]


def test_unrelated_repeated_labels_leave_unique_pad_path_measurable(tmp_path):
    pcbnew, path = fixture(tmp_path, VIA, """(footprint "test" (layer "F.Cu") (at 16 8)
        (property "Reference" "J1")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "SUPPLY"))
        (pad "1" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SUPPLY")))""")
    board = pcbnew.LoadBoard(str(path))
    usual, ambiguous = measure_supply_paths(pcbnew, board, [("C1:1", "U1:1"), ("J1:1", "U1:1")])
    assert usual["status"] == "measured"
    assert usual["trace_length_mm"] == pytest.approx(9.2)
    assert usual["intermediate_pads"] == []
    assert ambiguous["status"] == "unavailable"
    assert "requested pad is missing or ambiguous" in ambiguous["reason"]


def test_repeated_label_on_unjoined_layers_cannot_bridge_a_path(tmp_path):
    row = measure(tmp_path, """(footprint "test" (layer "F.Cu") (at 8 4)
        (property "Reference" "J1")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "SUPPLY"))
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net "SUPPLY")))""")
    assert row["status"] == "unavailable"
    assert "no route found" in row["reason"]


def test_same_coordinate_on_two_layers_requires_copper_bridge(tmp_path):
    row = measure(tmp_path, "")
    assert row["status"] == "unavailable"
    assert "no route" in row["reason"]
    assert "trace_length_mm" not in row


def test_named_zone_withholds_trace_length(tmp_path):
    row = measure(
        tmp_path,
        VIA,
        ZONE,
    )
    assert row["status"] == "unavailable"
    assert "named copper zone" in row["reason"]


def test_near_check_separates_inferred_or_explicit_source_pad_from_geometry(tmp_path):
    near = {"part": "C1", "pin_of": "U1:1", "max_mm": 10}
    _, path = fixture(tmp_path, VIA)
    one, = run.returned_board.near_checks([near], path, measure_routed=True)
    assert one["satisfied"]
    assert one["measurement"] == "body_to_pin"
    assert one["routed_path"]["from"] == "C1:1"
    assert one["routed_path"]["to"] == "U1:1"
    assert one["routed_path"]["trace_length_mm"] == pytest.approx(9.2)
    assert one["routed_path"]["layers"] == ["F.Cu", "B.Cu"]

    _, path = fixture(tmp_path, VIA, source_pads="""
        (pad "2" smd rect (at 0 3) (size 1 1) (layers "F.Cu") (net "SUPPLY"))
        (pad "3" smd rect (at 0 5) (size 1 1) (layers "F.Cu") (net "OTHER"))""")
    ambiguous, explicit, mismatch = run.returned_board.near_checks(
        [near, dict(near, pad="1"), dict(near, pad="3")], path, measure_routed=True)
    assert ambiguous["satisfied"] and explicit["satisfied"] and mismatch["satisfied"]
    assert ambiguous["routed_path"]["status"] == "unavailable"
    assert "multiple pads on the target net" in ambiguous["routed_path"]["reason"]
    assert explicit["routed_path"]["trace_length_mm"] == pytest.approx(9.2)
    assert mismatch["routed_path"]["status"] == "unavailable"
    assert "different net" in mismatch["routed_path"]["reason"]


@pytest.mark.parametrize(("contact", "extra", "target_pads", "reason"), [
    ("", "", "", "no route found"),
    (VIA, ZONE, "", "named copper zone"),
    (VIA, "", '(pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu") (net "SUPPLY"))',
     "exactly one physical pad"),
])
def test_near_check_remains_a_geometric_gate_when_route_is_unavailable(tmp_path, contact, extra, target_pads, reason):
    _, path = fixture(tmp_path, contact, extra, target_pads=target_pads)
    near, = run.returned_board.near_checks(
        [{"part": "C1", "pin_of": "U1:1", "max_mm": 10}], path, measure_routed=True)
    assert near["satisfied"]
    assert near["routed_path"]["status"] == "unavailable"
    assert reason in near["routed_path"]["reason"]


def test_explicit_probe_can_bound_same_net_track_graph(tmp_path):
    pcbnew, path = fixture(tmp_path, VIA)
    board = pcbnew.LoadBoard(str(path))
    limited, = measure_supply_paths(pcbnew, board, [("C1:1", "U1:1")], max_tracks_per_net=3)
    assert limited["status"] == "unavailable"
    assert "3-item track/via measurement limit" in limited["reason"]


def test_near_filter_respects_native_hierarchical_and_bare_net_names(tmp_path):
    near = [{"part": "C1", "pin_of": "U1:1"}]
    pcbnew, path = fixture(tmp_path, VIA)
    text = path.read_text().replace('"SUPPLY"', '"/SUPPLY"')
    path.write_text(text)
    board = pcbnew.LoadBoard(str(path))
    selected, = measure_near_paths(pcbnew, board, near, only_net="SUPPLY")
    assert selected["routed_path"]["net"] == "/SUPPLY"
    path.write_text(text[:-1] + '''
      (footprint "test" (layer "F.Cu") (at 14 8)
        (property "Reference" "R1")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "SUPPLY"))))''')
    board = pcbnew.LoadBoard(str(path))
    assert measure_near_paths(pcbnew, board, near, only_net="SUPPLY") == []
