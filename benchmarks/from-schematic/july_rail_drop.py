# Copyright (C) 2026 Florian Loitsch.

"""Run the pinned July author model against native KiCad board coordinates."""

import argparse
import json
import math
import shutil
import subprocess
from hashlib import sha1, sha256
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from types import ModuleType, SimpleNamespace


SIMULATOR_BLOB = "76ef67462ea28a0a908b0ba3adf9206ec84ebbf7"


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def load_simulator(path):
    # Compile the same bytes checked by the Git blob identity.
    source = path.read_bytes()
    blob = sha1(f"blob {len(source)}\0".encode() + source).hexdigest()
    require(blob == SIMULATOR_BLOB, f"July rail-drop simulator requires Git blob {SIMULATOR_BLOB}")
    provenance = {"script": {"path": str(path), "git_blob": blob, "sha256": sha256(source).hexdigest()},
                  "parser": "native pcbnew world coordinates",
                  "model_scope": "authored rail trace resistor mesh and +12V lumped transient"}
    ngspice = shutil.which("ngspice")
    if ngspice is None:
        return None, dict(provenance, status="unavailable", reason="ngspice is absent from PATH")
    try:
        tool = subprocess.run([ngspice, "--version"], capture_output=True, text=True, timeout=10, check=True)
        author = ModuleType("july_author_rail_drop")
        author.__file__ = str(path)
        exec(compile(source, str(path), "exec"), author.__dict__)
    except (ImportError, OSError, subprocess.SubprocessError) as error:
        return None, dict(provenance, status="unavailable", reason=str(error))
    try:
        kiutils_version = version("kiutils")
    except PackageNotFoundError:
        kiutils_version = None
    return author, dict(
        provenance,
        status="available",
        ngspice={"path": ngspice, "version": tool.stdout.strip()},
        kiutils_version=kiutils_version,
        sources=author.SOURCES,
        loads_a=author.LOADS,
        supply_v=author.SUPPLY_V,
        assumptions={
            "trace_copper_thickness_um": author.T_CU * 1e6,
            "copper_resistivity_ohm_m_at_20c": author.RHO_CU,
            "node_clustering_mm": author.NODE_TOL,
            "ideal_layer_bridges": ["through vias", "plated through-hole pads"],
            "ideal_same_net_links_within_each_footprint": True,
            "omitted": ["copper zones", "ground return", "connector and via resistance"],
            "solenoid_pwm": {"capacitance_uf": author.C_BULK * 1e6, "esr_ohm": author.ESR_BULK,
                             "current_a": author.SOL_PULSE_A, "frequency_hz": author.SOL_FREQ},
        },
    )


def _point(value):
    return SimpleNamespace(X=value.x / 1e6, Y=value.y / 1e6)


def _native_input(pcbnew, board, author):
    layers = tuple(board.GetEnabledLayers().CuStack())
    require(len(layers) >= 2 and layers[0] == pcbnew.F_Cu and layers[-1] == pcbnew.B_Cu,
            "expected a copper stack with a front and back")
    footprints = list(board.GetFootprints())
    refs = set()
    pads_by_net = set()
    names = {}
    adapted_footprints = []
    top_level_copper = any(item.IsOnLayer(layer) for item in board.GetDrawings() for layer in layers)
    require(not top_level_copper, "board copper graphics are outside the author model")
    zones = list(board.Zones())
    for footprint in footprints:
        ref = footprint.GetReference()
        require(ref and ref not in refs, f"missing or repeated footprint reference {ref!r}")
        refs.add(ref)
        require(not any(item.IsOnLayer(layer) for item in footprint.GraphicalItems() for layer in layers),
                f"footprint copper graphics are outside the author model: {ref}")
        zones.extend(footprint.Zones())
        face = footprint.GetLayer()
        require(face in (pcbnew.F_Cu, pcbnew.B_Cu), f"unsupported footprint face: {ref}")
        face_name = board.GetStandardLayerName(face)
        pads = []
        for pad in footprint.Pads():
            label = f"{ref}:{pad.GetNumber()}"
            code, name = pad.GetNetCode(), pad.GetNetname()
            require(code not in names or names[code] == name, f"ambiguous net identity at {label}")
            names[code] = name
            if name:
                pads_by_net.add((ref, name))
            copper = tuple(layer for layer in layers if pad.IsOnLayer(layer))
            attribute = pad.GetAttribute()
            if attribute == pcbnew.PAD_ATTRIB_PTH and copper == layers:
                span = "*"
            elif attribute == pcbnew.PAD_ATTRIB_SMD and copper == (face,):
                span = face_name
            else:
                raise ValueError(f"unsupported pad layer span or plating at {label}")
            pads.append(SimpleNamespace(position=_point(pad.GetPosition()), net=SimpleNamespace(number=code),
                                        layers=[span]))
        # The author's local-to-world transform becomes the identity.
        adapted_footprints.append(SimpleNamespace(
            position=SimpleNamespace(X=0, Y=0, angle=0), layer=face_name,
            graphicItems=[SimpleNamespace(type="reference", text=ref)], pads=pads,
        ))
    for rail, source in author.SOURCES.items():
        for ref in (source, *author.LOADS[rail]):
            require((ref, rail) in pads_by_net, f"{ref} has no native pad on {rail}")
    omitted_zones = []
    for zone in zones:
        if zone.GetIsRuleArea():
            continue
        name = zone.GetNetname()
        require(name in ("", "GND"), f"copper zone on {name} is outside the author rail model")
        omitted_zones.append({"net": name, "layers": [board.GetStandardLayerName(layer) for layer in layers
                                                     if zone.IsOnLayer(layer)]})

    traces, identities = [], []
    next_node = 0
    for item in board.GetTracks():
        identifier = item.m_Uuid.AsString()
        name = item.GetNetname()
        require(item.GetNetCode() > 0 and name, f"unassigned copper item {identifier}")
        if isinstance(item, pcbnew.PCB_VIA):
            require(item.GetViaType() == pcbnew.VIATYPE_THROUGH
                    and item.TopLayer() == layers[0] and item.BottomLayer() == layers[-1],
                    f"unsupported via layer span for {identifier}")
            trace = author.Via()
            trace.position = _point(item.GetPosition())
            points = [(trace.position.X, trace.position.Y, "*")]
        else:
            layer = item.GetLayer()
            require(item.GetClass() == "PCB_TRACK" and layer in layers
                    and item.GetWidth() > 0 and item.GetStart() != item.GetEnd(),
                    f"unsupported trace geometry for {identifier}")
            trace = author.Segment()
            trace.start, trace.end = _point(item.GetStart()), _point(item.GetEnd())
            trace.width = item.GetWidth() / 1e6
            trace.layer = board.GetStandardLayerName(layer)
            points = [(point.X, point.Y, trace.layer) for point in (trace.start, trace.end)]
        # The pinned graph builder adds each of these nodes in trace order,
        # before it adds any pads. Check this correspondence after lowering.
        traces.append(trace)
        identities.append((identifier, name, next_node, points))
        next_node += len(points)
    return SimpleNamespace(nets=[SimpleNamespace(number=code, name=name) for code, name in names.items()],
                           footprints=adapted_footprints, traceItems=traces), identities, omitted_zones


def _require_complete(rail, results, expected_loads):
    require(not results.get("error"), results.get("error"))
    require(results.get("unreached_loads") == [] and results.get("loads_a") == expected_loads,
            f"{rail} did not reach all configured loads: {results.get('unreached_loads')}")
    require(results.get("foreign_nets_on_copper") == [],
            f"{rail} includes other named pad nets: {results.get('foreign_nets_on_copper')}")
    drops = results.get("ir_drop_mv", {})
    require(drops.keys() == expected_loads.keys() and all(math.isfinite(drop) for drop in drops.values()),
            f"{rail} did not yield finite operating-point voltages for every load")
    require(math.isfinite(results.get("v_supply_pad", math.nan))
            and results.get("worst_drop_mv") == max(drops.values())
            and drops.get(results.get("worst_load")) == results["worst_drop_mv"],
            f"{rail} returned inconsistent operating-point data")
    if rail == "+12V":
        transient = results.get("transient", {})
        require(all(math.isfinite(transient.get(key, math.nan)) for key in ("rail_min_v", "rail_max_v", "droop_mv")),
                "+12V did not yield a finite transient droop")


def simulate(pcbnew, board, author):
    native, identities, omitted_zones = _native_input(pcbnew, board, author)
    graph = author._build_graph(native)
    separate_pads = author._build_graph(native, False)
    nodes, resistors, pads, _ = graph
    for _, _, start, points in identities:
        require(nodes.pts[start:start + len(points)] == points, "native copper does not match the author graph order")
    rails = {}
    for rail, source in author.SOURCES.items():
        src = next((node for ref, name, node in pads if ref == source and name == rail), None)
        require(src is not None, f"the author graph has no {rail} source at {source}")
        component = author._component(resistors, src)
        unexpected = [{"uuid": uuid, "net": name} for uuid, name, start, points in identities
                      if name != rail and any(nodes.find(index) in component for index in range(start, start + len(points)))]
        require(not unexpected, f"{rail} modeled copper component includes unexpected track or via nets: {unexpected}")
        results = author._simulate_net(rail, graph, separate_pads)
        _require_complete(rail, results, author.LOADS[rail])
        rails[rail] = {key: value for key, value in results.items() if key != "deck"}
    return {"status": "measured", "rails": rails, "ignored_zones": omitted_zones}


def score_board(pcbnew, path, expected, simulator, unavailable_reason):
    if simulator is None:
        return {"status": "unavailable", "reason": unavailable_reason}
    if expected is None:
        return {"status": "unavailable", "reason": "layout produced no PCB"}
    try:
        require(path.read_bytes() == expected, "rail-drop PCB does not match the scored bytes")
        board = pcbnew.LoadBoard(str(path))
        require(board is not None and path.read_bytes() == expected, "KiCad could not load the exact scored PCB")
        return simulate(pcbnew, board, simulator)
    except (ValueError, OSError, TypeError, RuntimeError, KeyError, subprocess.SubprocessError) as error:
        return {"status": "unavailable", "reason": str(error)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", type=Path, help="routed July powder-doser PCB")
    parser.add_argument("--rail-drop-simulator", required=True, type=Path, metavar="PINNED_LOCAL_PY")
    args = parser.parse_args()
    try:
        board = args.board.resolve()
        expected = board.read_bytes()
        simulator, provenance = load_simulator(args.rail_drop_simulator.resolve())
    except (ValueError, OSError) as error:
        parser.exit(2, f"{error}\n")
    try:
        import pcbnew
    except ImportError as error:
        score = {"status": "unavailable", "reason": str(error)}
    else:
        score = score_board(pcbnew, board, expected, simulator, provenance.get("reason"))
    print(json.dumps({"board": {"path": str(board), "sha256": sha256(expected).hexdigest()},
                      "simulator": provenance, "rail_drop": score}, indent=2))
    return 0 if score["status"] == "measured" else 1


if __name__ == "__main__":
    raise SystemExit(main())
