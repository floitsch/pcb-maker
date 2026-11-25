# Copyright (C) 2026 Florian Loitsch.
"""Tests of scripts/nightly-causes.py, the nightly's census of open causes,
on small fixture nights: the precedence, the outlines' count, the older
outputs, the boards without a result or outputs, the change against the
previous night and the error path."""

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

spec = importlib.util.spec_from_file_location("nightly_causes", HERE / "nightly-causes.py")
causes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(causes)


def pad(number, net, ref, uuid=None):
    return {"description": f"Pad {number} [{net}] of {ref} on F.Cu", "pos": {"x": 0, "y": 0},
            "uuid": uuid or f"{ref}-{number}"}


def track(net, uuid="t"):
    return {"description": f"Track [{net}] on B.Cu, length 1.0000 mm", "pos": {"x": 0, "y": 0}, "uuid": uuid}


def zone(net, uuid):
    return {"description": f"Zone [{net}] on F.Cu and B.Cu, priority 0", "pos": {"x": 0, "y": 0}, "uuid": uuid}


def item(a, b):
    return {"description": "Missing connection between items", "items": [a, b], "severity": "error",
            "type": "unconnected_items"}


def board(stage_directory, name, items, verdict=None, router=None, mode="route"):
    """A board's outputs as run.py leaves them; its row."""
    base = Path(stage_directory) / name / mode
    result = base / "result" if mode == "layout" else base
    result.mkdir(parents=True, exist_ok=True)
    (result / "drc.json").write_text(json.dumps({"coordinate_units": "mm", "unconnected_items": items}))
    (base / "verdict.json").write_text(json.dumps({"kicad_unconnected_items": len(items), "open_connections": 1,
                                                   "unconnected_terminals": 2, **(verdict or {})}))
    (result / "board-router.json").write_text(json.dumps(router or {}))
    return {"name": name, "native": {"unconnected": len(items), "errors": {}}, "exit": 1 if items else 0}


def write_night(night, stages):
    """`stages`: {stage: rows}; writes <stage>.log and nightly.json."""
    night.mkdir(parents=True, exist_ok=True)
    for stage, rows in stages.items():
        (night / f"{stage}.log").write_text("".join(json.dumps(row) + "\n" for row in rows))
    (night / "nightly.json").write_text(json.dumps({"stages": {s: {"status": "ran"} for s in stages}}))
    (night / "summary.md").write_text("# Nightly\n\n## Results\n\nrows\n")


ROUTER = {
    "nets": [{"connection": "GND", "status": "partial"}, {"connection": "MCU/SHLD", "status": "partial"},
             {"connection": "+12V", "status": "partial"}, {"connection": "RESET", "status": "partial"},
             {"connection": "VBUS", "status": "routed"}, {"connection": "SDA", "status": "partial"},
             {"connection": "SPLIT", "status": "routed"}, {"connection": "SPLIT", "status": "partial"},
             {"connection": "Net-(U1-SS{slash}TR)", "status": "partial"}],
    "diagnostics": {
        "conflicted_nets": ["SDA"],
        "dead_pads": [
            {"pad": "C19.1", "net": "GND", "access": "off_lattice"},
            {"pad": "IC2.11", "net": "+12V", "access": "no_lattice_node"},
            {"pad": "U2.AC9", "net": "RESET", "access": "no_legal_access"},
            {"pad": "LDO1.3", "net": "MCU/SHLD", "access": "start_blocked"},
            {"pad": "J1.2", "net": "OTHER", "access": "no_legal_access"},
            {"pad": "Q1.1", "net": "Net-(U1-SS{slash}TR)", "access": "unknown"}]},
}
VERDICT = {
    "closed_pads": [{"part": "U2", "pads": ["AC9 (RESET)", "AC11 (RESET)"], "open_nets": ["RESET"]}],
    "unplaced": ["R99"],
    "zone_island_recovery": {"zone_uuid": "z1", "net": "GND"},
}


def test_each_item_goes_to_the_first_cause_that_names_it(tmp_path):
    night = tmp_path / "2026-11-25"
    items = [
        item(pad(1, "GND", "C19"), pad(2, "GND", "R12")),          # dead pad, off_lattice
        item(pad(11, "+12V", "IC2"), track("+12V")),               # dead pad, no_lattice_node
        item(pad("AC9", "RESET", "U2"), pad(1, "RESET", "SW1")),   # closed pad beats its no_legal_access
        item(pad(3, "/MCU/SHLD", "LDO1"), track("/MCU/SHLD")),     # start_blocked; KiCad's leading slash
        item(pad(2, "GND", "J1"), track("GND")),                   # J1.2's dead pad is another net's: no match
        item(pad(1, "GND", "R99"), pad(4, "GND", "C19")),          # unplaced part beats nothing else
        item(zone("GND", "z1"), zone("GND", "z1")),                # islands of one zone
        item(pad(1, "VBUS", "J2"), track("VBUS")),                 # the router counts VBUS routed
        item(pad(1, "SDA", "U3"), track("SDA")),                   # open, in conflict
        item(pad(1, "SPLIT", "U3"), track("SPLIT")),               # two router nets of the name disagree
        item(pad(1, "NOPE", "U3"), track("NOPE")),                 # no router net of the name
        item(pad(1, "Net-(U1-SS/TR)", "Q1"), track("Net-(U1-SS/TR)")),  # access unknown
    ]
    rows = [board(night / "designer", "repo__Board", items, VERDICT, ROUTER)]
    write_night(night, {"designer": rows})
    record = causes.census({"designer": (night / "designer.log", night / "designer")}, night.name)
    stage = record["stages"]["designer"]
    assert stage["per_board"]["repo__Board"]["causes"] == {
        "dead_pad:off_lattice": 1, "dead_pad:no_lattice_node": 1, "closed_pad": 1, "dead_pad:start_blocked": 1,
        "unexplained:net_open": 1, "unplaced_part": 1, "zone_island": 1, "router_counts_routed": 1,
        "unexplained:net_open_conflicted": 1, "unexplained:net_split": 1, "unexplained:no_net_status": 1,
        "unexplained:dead_pad_access_unknown": 1}
    assert stage["items"] == 12 and stage["unexplained"] == 5
    assert list(stage["causes"]) == [c for c in causes.PRECEDENCE if c in stage["causes"]]


def test_the_outlines_count_takes_unexplained_items_only(tmp_path):
    night = tmp_path / "2026-11-25"
    items = [item(zone("GND", "a"), zone("GND", "b")), item(zone("GND", "b"), zone("GND", "c")),
             item(pad(1, "VBUS", "J2"), track("VBUS")), item(pad(1, "GND", "C19"), track("GND"))]
    verdict = {"board_outlines": {"outlines": 3, "unconnected_across": 2}}
    router = {"nets": [{"connection": "GND", "status": "partial"}, {"connection": "VBUS", "status": "routed"}],
              "diagnostics": {"dead_pads": [{"pad": "C19.1", "net": "GND", "access": "no_legal_access"}]}}
    write_night(night, {"sweep": [board(night / "sweep", "repo__Hub", items, verdict, router, mode="layout")]})
    record = causes.census({"sweep": (night / "sweep.log", night / "sweep")}, night.name)
    # The two unexplained zone items, before the routed net's; never the named dead pad.
    assert record["stages"]["sweep"]["per_board"]["repo__Hub"]["causes"] == {
        "separate_outlines": 2, "router_counts_routed": 1, "dead_pad:no_legal_access": 1}


def test_older_outputs_missing_outputs_and_no_result(tmp_path):
    night = tmp_path / "2026-11-24"
    old_router = {"nets": [{"connection": "GND", "status": "partial"}],
                  "diagnostics": {"dead_pads": [{"pad": "C19.1", "net": "GND", "wall": "..."}]}}
    rows = [board(night / "sweep", "repo__Old", [item(pad(1, "GND", "C19"), track("GND"))], {}, old_router,
                  mode="layout"),
            {"name": "repo__Bare", "native": {"unconnected": 4, "errors": {}}, "exit": 1},
            {"name": "repo__Clean", "native": {"unconnected": 0, "errors": {}}, "exit": 0},
            {"name": "repo__Error", "error": "usage or input error", "exit": 2}]
    write_night(night, {"sweep": rows})
    stage = causes.census({"sweep": (night / "sweep.log", night / "sweep")}, night.name)["stages"]["sweep"]
    assert stage["causes"] == {"unexplained:dead_pad_without_access": {"items": 1, "boards": 1},
                               "unexplained:no_outputs": {"items": 4, "boards": 1}}
    assert stage["lacking_access"] == ["repo__Old"]
    assert stage["without_outputs"] == ["repo__Bare"]
    assert stage["no_result"] == ["repo__Error"]
    assert stage["boards"] == 4 and stage["boards_with_items"] == 2 and stage["unexplained"] == 5
    text = causes.markdown({"previous": None, "stages": {"sweep": stage}})
    assert "| unexplained: a dead pad without `access` (an older binary) | 1 | 1 |" in text
    assert "- boards whose dead pads lack `access` (an older binary): 1 (Old)" in text
    assert "- boards without a result: 1 (Error)" in text


def test_a_report_with_another_count_explains_nothing(tmp_path):
    night = tmp_path / "2026-11-25"
    row = board(night / "sweep", "repo__A", [item(pad(1, "VBUS", "J2"), track("VBUS"))], {},
                {"nets": [{"connection": "VBUS", "status": "routed"}]})
    row["native"]["unconnected"] = 3
    write_night(night, {"sweep": [row]})
    stage = causes.census({"sweep": (night / "sweep.log", night / "sweep")}, night.name)["stages"]["sweep"]
    assert stage["causes"] == {"unexplained:no_outputs": {"items": 3, "boards": 1}}


def run_script(*arguments):
    return subprocess.run([sys.executable, str(HERE / "nightly-causes.py"), *map(str, arguments)],
                          capture_output=True, text=True, timeout=60)


def test_the_night_against_the_previous_one(tmp_path):
    nights = tmp_path / "nightly"
    first, second = nights / "2026-11-24", nights / "2026-11-25"
    router = {"nets": [{"connection": "GND", "status": "partial"}, {"connection": "VBUS", "status": "routed"}]}
    write_night(first, {"sweep": [board(first / "sweep", "repo__A", [item(pad(1, "GND", "C1"), track("GND"))] * 3,
                                        {}, router)]})
    write_night(second, {"sweep": [board(second / "sweep", "repo__A", [item(pad(1, "VBUS", "C1"), track("VBUS"))],
                                         {}, router)]})
    for night in (first, second):
        result = run_script(night)
        assert result.returncode == 0, result.stderr
    record = json.loads((second / "causes.json").read_text())
    assert record["previous"] == "2026-11-24"
    delta = record["stages"]["sweep"]["delta"]
    assert delta["unexplained:net_open"] == {"items": -3, "boards": -1}
    assert delta["router_counts_routed"] == {"items": 1, "boards": 1}
    summary = (second / "summary.md").read_text()
    assert summary.startswith("# Nightly\n") and "## Results" in summary
    assert "**sweep**: 1 items on 1 of 1 boards, 0 unexplained (items -2 against 2026-11-24)." in summary
    assert "| the router counts the net routed, KiCad does not | 1 | 1 | +1, +1 |" in summary
    assert "| unexplained: the router left the net open, not in conflict | 0 | 0 | -3, -1 |" in summary
    # Again: the section is replaced, not added.
    assert run_script(second).returncode == 0
    assert (second / "summary.md").read_text().count(causes.HEADING) == 1


def test_a_failing_census_writes_one_line_and_exits_0(tmp_path):
    night = tmp_path / "2026-11-25"
    write_night(night, {"sweep": []})
    (night / "sweep.log").unlink()
    (night / "sweep.log").mkdir()  # unreadable as a log
    result = run_script(night)
    assert result.returncode == 0
    assert result.stdout.startswith("open causes: the census failed (IsADirectoryError")
    summary = (night / "summary.md").read_text()
    assert summary.endswith("\n\n## Open causes\n\nopen causes: the census failed (IsADirectoryError: "
                            f"[Errno 21] Is a directory: '{night / 'sweep.log'}')\n")


def test_a_run_directory_and_the_output_option(tmp_path):
    run = tmp_path / "study"
    row = board(run, "repo__WLED", [item(pad(3, "+3.3V", "LDO1"), track("+3.3V"))], {},
                {"nets": [{"connection": "+3.3V", "status": "partial"}],
                 "diagnostics": {"dead_pads": [{"pad": "LDO1.3", "net": "+3.3V", "access": "start_blocked"}]}})
    (run / "results.json").write_text(json.dumps([row]))
    result = run_script("--run", f"study={run}", "--output", tmp_path / "out")
    assert result.returncode == 0 and result.stdout == "open causes: study 1 items, 0 unexplained\n"
    assert "| dead pad, start_blocked (the router's stub start) | 1 | 1 |" in (tmp_path / "out/causes.md").read_text()
    assert not (run / "summary.md").exists()


def test_end_of_reads_kicads_descriptions():
    assert causes.end_of({"description": "PTH pad a28 [/VD4] of J3", "uuid": "u"})["pad"] == "J3.a28"
    end = causes.end_of({"description": "Zone 'GND Fill' [GND] on F.Cu, B.Cu and 3 more, priority 0", "uuid": "z"})
    assert (end["kind"], end["net"], end["pad"]) == ("Zone", "GND", None)
    assert causes.net_key("/RAIL{slash}2") == causes.net_key("RAIL/2") == "RAIL/2"


def open_group(reason, terminals, *pads):
    return {"reason": reason, "terminals": terminals, "pads": list(pads)}


def test_the_routers_reasons_name_the_open_items(tmp_path):
    night = tmp_path / "2026-11-26"
    router = {"nets": [
        {"connection": "SDA", "status": "partial", "unconnected_terminals": 3,
         "open": [open_group("no_path_hard:board", 2, "U3.1", "U4.1"), open_group("lost:clean_up", 1, "R1.2")]},
        {"connection": "SCL", "status": "partial", "unconnected_terminals": 40,
         "open": [open_group("repair_skipped:budget", 40, "U3.2")]},
        {"connection": "MISO", "status": "partial", "unconnected_terminals": 2,
         "open": [open_group("no_path_static", 1, "U5.1"), open_group("unknown", 1)]},
        {"connection": "MOSI", "status": "partial", "open": [open_group("unknown", 2)]},
        {"connection": "CS", "status": "partial", "open": [open_group("some_new_reason", 1, "U6.1")]},
        {"connection": "GND", "status": "partial", "open": [open_group("no_access", 1, "C19.1")]},
    ], "diagnostics": {"conflicted_nets": ["SDA"],
                       "dead_pads": [{"pad": "C19.1", "net": "GND", "access": "off_lattice"}]}}
    items = [
        item(pad(1, "SDA", "U3"), track("SDA")),          # a listed pad: its reason
        item(track("SDA"), pad(2, "SDA", "R1")),          # the other end listed
        item(pad(9, "SDA", "U9"), track("SDA")),          # not listed, the net's reasons are mixed
        item(pad(7, "SCL", "U7"), track("SCL")),          # not listed: the net's only reason
        item(pad(1, "MISO", "U5"), track("MISO")),        # listed
        item(track("MOSI"), track("MOSI")),               # the net's only reason is unknown
        item(pad(1, "CS", "U6"), track("CS")),            # a reason the census does not know
        item(pad(1, "GND", "C19"), track("GND")),         # the dead pad's cause comes first
        item(pad(1, "SDA", "U4"), pad(2, "SDA", "R1")),   # two listed pads: the first reason in precedence
    ]
    write_night(night, {"route": [board(night / "route", "repo__Open", items, {}, router)]})
    stage = causes.census({"route": (night / "route.log", night / "route")}, night.name)["stages"]["route"]
    assert stage["per_board"]["repo__Open"]["causes"] == {
        "open:no_path_hard:board": 2, "open:lost:clean_up": 1, "unexplained:net_open_mixed": 1,
        "open:repair_skipped:budget": 1, "open:no_path_static": 1, "unexplained:open_unknown": 1,
        "open:other": 1, "dead_pad:off_lattice": 1}
    assert stage["open_by_net"] == 1
    assert stage["unexplained"] == 2
    text = causes.markdown({"previous": None, "stages": {"route": stage}})
    assert "| open: the hard repair found no path around other nets' copper on the whole board | 2 | 1 |" in text
    assert "- items under an `open:` cause by their net's only reason (their pads not listed): 1" in text


def test_a_split_nets_items_go_to_the_copy_holding_their_pads(tmp_path):
    night = tmp_path / "2026-11-26"
    router = {"nets": [
        {"connection": "GND", "status": "routed", "copy": 0, "pads": ["C1.2", "U1.4"]},
        {"connection": "GND", "status": "partial", "copy": 1, "pads": ["C101.2", "U101.4", "R101.1"],
         "open": [open_group("no_path_hard:window", 1, "R101.1")]},
        {"connection": "GND", "status": "partial", "copy": 2, "pads": ["C201.2", "U201.4"],
         "open": [open_group("repair_skipped:hopeless", 1)]},
        {"connection": "VCC", "status": "routed", "copy": 0, "pads": ["U1.1"]},
        {"connection": "VCC", "status": "partial", "copy": 1, "pads": ["U101.1"]},
    ]}
    items = [
        item(pad(2, "GND", "C1"), pad(2, "GND", "C101")),   # two copies: the router split them by design
        item(pad(4, "GND", "U1"), track("GND")),            # copy 0, routed
        item(pad(1, "GND", "R101"), track("GND")),          # listed open in copy 1
        item(pad(4, "GND", "U201"), track("GND")),          # copy 2, its only reason
        item(track("GND"), track("GND")),                   # no pad: the copies disagree
        item(pad(1, "VCC", "U101"), track("VCC")),          # copy 1 open, no reasons (an older binary)
    ]
    write_night(night, {"sweep": [board(night / "sweep", "repo__Panel", items, {}, router, mode="layout")]})
    stage = causes.census({"sweep": (night / "sweep.log", night / "sweep")}, night.name)["stages"]["sweep"]
    assert stage["per_board"]["repo__Panel"]["causes"] == {
        "split_across_copies": 1, "router_counts_routed": 1, "open:no_path_hard:window": 1,
        "open:repair_skipped:hopeless": 1, "unexplained:net_split": 1, "unexplained:net_open": 1}


def test_items_across_outlines_are_named_by_their_uuids(tmp_path):
    night = tmp_path / "2026-11-26"
    items = [item(zone("GND", "a"), zone("GND", "b")), item(zone("GND", "c"), zone("GND", "d")),
             item(pad(1, "VBUS", "J2"), track("VBUS"))]
    verdict = {"board_outlines": {"outlines": 2, "unconnected_across": 1, "unconnected_across_items": [["b", "a"]]}}
    router = {"nets": [{"connection": "GND", "status": "partial"}, {"connection": "VBUS", "status": "routed"}]}
    write_night(night, {"sweep": [board(night / "sweep", "repo__Hub", items, verdict, router, mode="layout")]})
    stage = causes.census({"sweep": (night / "sweep.log", night / "sweep")}, night.name)["stages"]["sweep"]
    # The item named by its uuids, not the first unexplained one.
    assert stage["per_board"]["repo__Hub"]["causes"] == {
        "separate_outlines": 1, "unexplained:net_open": 1, "router_counts_routed": 1}
