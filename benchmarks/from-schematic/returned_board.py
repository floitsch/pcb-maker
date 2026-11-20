# Copyright (C) 2026 Florian Loitsch.
"""Read-only scoring of a returned KiCad board against a standalone brief."""

import collections
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import tempfile
import time
from contextlib import contextmanager
from decimal import Decimal, ROUND_HALF_UP
from itertools import combinations, zip_longest
from pathlib import Path

import harness_common
import keepout_checks
import outline_rule_areas
from schematic import SHAPES, block_end

PROJECT_TABLES = {"sym-lib-table", "fp-lib-table"}
SEVERITIES = {"error", "warning", "exclusion"}
POINT = r"([-+\d.eE]+)\s+([-+\d.eE]+)"
QUOTED = r'"(?:[^"\\]|\\.)*"'
FORM = re.compile(rf'{QUOTED}|\(\s*([^()\s"]+)')
SEXPR_TOKEN = re.compile(rf'{QUOTED}|[()]|[^\s()"]+')
DECIMAL_ATOM = re.compile(r'[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?')
NUMBERED_DIELECTRIC = re.compile(r'dielectric [1-9][0-9]*', re.IGNORECASE)
KICAD_NANOMETRE_MM = Decimal("0.000001")
EDGE_LAYER = re.compile(rf'\(layers?\s+(?:(?:{QUOTED}|[^()\s"]+)\s+)*"?Edge\.Cuts"?(?=\s|\))')
# Same position and angle tolerances as the placer constraint report.
POSITION_EPSILON_MM = 0.001
ANGLE_EPSILON = 1e-6
# The read-only description rounds its courtyard bounds to the nearest micron.
BODY_EPSILON_MM = 0.000501


def record(path):
    return {"path": str(path), "sha256": harness_common.file_sha256(path)}


def project_files(schematic):
    """Reachable original project inputs; no search of previous run directories."""
    root = schematic.parent
    paths, unresolved = set(), set()
    pending = [schematic]
    while pending:
        sheet = pending.pop()
        if sheet in paths:
            continue
        paths.add(sheet)
        source = sheet.read_text()
        patterns = [rf'\(property\s+"Sheet\s*file"\s+({QUOTED})', rf'^F1\s+({QUOTED})']
        for pattern in patterns:
            for match in re.finditer(pattern, source, re.IGNORECASE | re.MULTILINE):
                name = json.loads(match[1])
                child = Path(os.path.normpath(sheet.parent / name))
                if not Path(name).is_absolute() and child.is_relative_to(root) and child.is_file():
                    pending.append(child)
                else:
                    unresolved.add(f"schematic sheet {name}")
    for name in PROJECT_TABLES:
        table = root / name
        if not table.is_file():
            continue
        paths.add(table)
        for match in re.finditer(rf'\(uri\s+({QUOTED}|[^\s()]+)', table.read_text()):
            uri = json.loads(match[1]) if match[1].startswith('"') else match[1]
            for prefix in ("${KIPRJMOD}/", "$(KIPRJMOD)/"):
                if not uri.startswith(prefix):
                    continue
                library = Path(os.path.normpath(root / uri[len(prefix):]))
                if not library.is_relative_to(root) or not library.exists():
                    unresolved.add(f"project library {uri}")
                elif library.is_dir():
                    paths.update(path for path in library.rglob("*.kicad_mod") if path.is_file())
                else:
                    paths.add(library)
                break
    for suffix in (".kicad_pro", ".kicad_dru"):
        companion = schematic.with_suffix(suffix)
        if companion.is_file():
            paths.add(companion)
    return {str(path.relative_to(root)): path for path in sorted(paths)}, sorted(unresolved)


def brief_evidence(arguments, schematic, layers):
    inputs = {key: record(path) for key, path in (
        ("schematic", schematic), ("constraints", arguments.constraints),
        ("outline_board", arguments.outline_board), ("binary", arguments.binary),
        ("footprints_from", getattr(arguments, "footprints_from", None)),
        ("mechanical_donor", getattr(arguments, "mechanical_donor", None)),
        ("project_settings", schematic.with_suffix(".kicad_pro")),
        ("custom_rules", schematic.with_suffix(".kicad_dru")),
    ) if path is not None and path.is_file()}
    files, unresolved = project_files(schematic)
    manifest = {name: harness_common.file_sha256(path) for name, path in files.items()}
    outline = rule_areas = None
    if arguments.outline_board is not None:
        donor = arguments.outline_board.read_text()
        outline = board_outline(children(donor))
        if getattr(arguments, "rule_areas_from_outline", False):
            rule_areas = outline_rule_areas.record_source(donor, layers)
    else:
        brief = json.loads(arguments.constraints.read_text())
        if isinstance(brief, dict) and brief.get("outline") is not None:
            try:
                outline = expected_outline(brief, None)
            except ValueError:
                # Self-run supports additional, including automatic, outlines.
                # Their intent is still captured by the authored brief bytes.
                pass
    outline_hash = outline_digest(outline) if outline is not None else None
    digest = {
        "layers": layers,
        "constraints": inputs["constraints"]["sha256"],
        "outline_geometry": outline_hash,
        "schematic": schematic.name,
        "project_files": manifest,
        "unresolved": unresolved,
    }
    if "footprints_from" in inputs:
        digest["footprints_from"] = inputs["footprints_from"]["sha256"]
    if "mechanical_donor" in inputs:
        digest["mechanical_donor"] = inputs["mechanical_donor"]["sha256"]
    if footprint_clearance_arguments(arguments):
        digest["footprint_clearance_policy"] = "preserve_supplied"
    if rule_areas is not None:
        digest["outline_rule_areas_sha256"] = rule_areas["sha256"]
    stackup = None
    if getattr(arguments, "stackup_from", None) is not None:
        inputs["stackup_from"], stackup = read_stackup_donor(arguments.stackup_from)
        require(len(stackup["copper_layers"]) == layers,
                f"stackup donor has {len(stackup['copper_layers'])} copper layers but the brief asks for {layers}")
        digest["stackup_sha256"] = stackup["sha256"]
    brief_hash = hashlib.sha256(json.dumps(digest, sort_keys=True).encode()).hexdigest()
    evidence = {"inputs": inputs, "brief_sha256": brief_hash, "project_files_sha256": manifest}
    if "footprint_clearance_policy" in digest:
        evidence["footprint_clearance_policy"] = digest["footprint_clearance_policy"]
    if outline_hash is not None:
        evidence["outline_sha256"] = outline_hash
    if rule_areas is not None:
        evidence["outline_rule_areas"] = rule_areas
    if stackup is not None:
        evidence["expected_stackup"] = stackup
    if unresolved:
        evidence["unresolved_project_inputs"] = unresolved
    return evidence, files


def footprint_clearance_arguments(arguments):
    return ["--keep-footprint-clearances"] if getattr(arguments, "keep_footprint_clearances", False) else []


def check_imported_clearance_policy(report, evidence):
    policy = evidence.get("footprint_clearance_policy")
    if policy is not None:
        require(report.get("footprint_clearance_policy") == policy and report.get("clearances") == [],
                "the import did not record preservation of the supplied footprint clearance policy")


@contextmanager
def footprint_donor_arguments(arguments, evidence, work, *, reference_only=False):
    """Give the schematic importer a private copy of the recorded geometry input."""
    donor = getattr(arguments, "footprints_from", None)
    if donor is None:
        yield []
        return
    brief = json.loads(arguments.constraints.read_text())
    additions = brief.get("add", []) if isinstance(brief, dict) else []
    if isinstance(additions, list) and not reference_only:
        require(not any(isinstance(item, dict) and item.get("from_board") is True for item in additions),
                "--footprints-from imports geometry only; constraints.add.from_board would import a board part")
    expected = evidence.get("inputs", {}).get("footprints_from", {}).get("sha256")
    require(expected is not None, "no checksum for the footprint geometry donor")
    with tempfile.TemporaryDirectory(prefix="footprint-donor-", dir=work) as temporary:
        staged = Path(temporary) / donor.name
        shutil.copyfile(donor, staged)
        require(harness_common.file_sha256(staged) == expected,
                "footprint geometry donor changed while being staged")
        yield ["--footprints-from", str(staged)]


@contextmanager
def board_donor_arguments(arguments, evidence, work, *, reference_only=False):
    """Give import private copies of the explicit board data inputs."""
    with footprint_donor_arguments(arguments, evidence, work, reference_only=reference_only) as footprint_args:
        donor = getattr(arguments, "stackup_from", None)
        if donor is None:
            yield footprint_args
            return
        expected = evidence.get("inputs", {}).get("stackup_from", {}).get("sha256")
        require(expected is not None, "no checksum for the stackup donor")
        with tempfile.TemporaryDirectory(prefix="stackup-donor-", dir=work) as temporary:
            staged = Path(temporary) / donor.name
            shutil.copyfile(donor, staged)
            require(harness_common.file_sha256(staged) == expected,
                    "stackup donor changed while being staged")
            yield [*footprint_args, "--stackup-from", str(staged)]


def stage_project(files, evidence, directory):
    require(not any(item.startswith("schematic sheet ") for item in evidence.get("unresolved_project_inputs", [])),
            "the original schematic has a child sheet that could not be staged; see unresolved_project_inputs")
    for relative, source in files.items():
        target = directory / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        require(harness_common.file_sha256(target) == evidence["project_files_sha256"][relative],
                f"project input changed while being staged: {relative}")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def numbers(value, length, label):
    require(isinstance(value, (list, tuple)) and len(value) == length
            and all(type(item) in (float, int) and math.isfinite(item) for item in value),
            f"missing or invalid {label}")
    return value


def segment(a, b):
    return ("line", *sorted((a, b)))


def polygon(points):
    if len(points) > 1 and points[0] == points[-1]:
        points = points[:-1]
    require(len(set(points)) >= 3, "outline polygon has fewer than three distinct points")
    return [segment(a, b) for a, b in zip(points, points[1:] + points[:1])]


def polygon_contour(pts, number, y_sign=1):
    """Read KiCad polygon nodes, including arcs and the lines between nodes."""
    def point(key, form):
        match = re.fullmatch(rf"\({key}\s+{POINT}\s*\)", form)
        require(match is not None, f"invalid polygon {key} point")
        return number(match[1]), y_sign * number(match[2])

    spans = []
    arcs = []
    for key, form in children(pts):
        if key == "xy":
            vertex = point(key, form)
            spans.append((vertex, vertex))
        else:
            require(key == "arc", f"unsupported polygon node {key}")
            start, mid, end = (point(k, stackup_form(form, k)) for k in ("start", "mid", "end"))
            require(len({start, mid, end}) == 3, "degenerate polygon arc")
            arcs.append(("arc", min(start, end), mid, max(start, end)))
            spans.append((start, end))
    if not arcs:
        return polygon([start for start, _ in spans])
    return arcs + [segment(end, start) for (_, end), (start, _) in zip(spans, spans[1:] + spans[:1])
                   if end != start]


def rectangle(start, end):
    x0, y0 = start
    x1, y1 = end
    require(x0 != x1 and y0 != y1, "outline rectangle has no area")
    return polygon([start, (x0, y1), end, (x1, y0)])


def children(expression):
    """Yield direct S-expression forms, skipping quoted and nested content."""
    root = FORM.match(expression.lstrip())
    require(root and root[1], "invalid KiCad expression")
    expression = expression.lstrip()
    cursor = root.end()
    for form in FORM.finditer(expression, cursor):
        if form.start() >= cursor and form[1]:
            cursor = block_end(expression, form.start())
            yield form[1], expression[form.start():cursor]


def stackup_form(expression, name):
    forms = [form for key, form in children(expression) if key == name]
    require(len(forms) == 1, f"expected exactly one {name} form")
    return forms[0]


def stackup_atom(token):
    return json.loads(token) if token.startswith('"') else token


def numbered_dielectric(form):
    tokens = SEXPR_TOKEN.findall(form)
    if len(tokens) < 4 or not NUMBERED_DIELECTRIC.fullmatch(stackup_atom(tokens[2])):
        return False
    kind = SEXPR_TOKEN.findall(stackup_form(form, "type"))
    return len(kind) == 4 and stackup_atom(kind[2]) in {"core", "prepreg"}


def native_thickness_mm(token):
    return Decimal(token).quantize(KICAD_NANOMETRE_MM, rounding=ROUND_HALF_UP)


def stackup_tokens(form):
    tokens = SEXPR_TOKEN.findall(form)
    root = tokens[1]
    if root == "stackup":
        aliases = iter(numbered_dielectric(layer) for name, layer in children(form) if name == "layer")
    elif root == "layer":
        aliases = iter([numbered_dielectric(form)])
    else:
        aliases = iter(())
    parents = []
    for index, token in enumerate(tokens):
        if token == "(":
            parents.append(tokens[index + 1])
        elif token == ")":
            parents.pop()
        if (index >= 2 and tokens[index - 2:index] == ["(", "layer"]
                and parents in (["stackup", "layer"], ["layer"]) and next(aliases)):
            # Native KiCad derives these labels from the physical layer order.
            yield ["numbered dielectric"]
            continue
        if not DECIMAL_ATOM.fullmatch(token):
            yield token
            continue
        value = Decimal(token)
        if (index >= 2 and tokens[index - 2:index] == ["(", "thickness"]
                and parents in (["thickness"], ["layer", "thickness"], ["stackup", "layer", "thickness"])):
            # KiCad stores thickness in integer nanometres. Electrical
            # and other layer data keep their exact decimal precision.
            value = native_thickness_mm(token)
        sign, digits, exponent = value.as_tuple()
        digits = "".join(map(str, digits))
        significant = digits.rstrip("0")
        yield [sign, significant, exponent + len(digits) - len(significant)] if significant else [0, "0", 0]


def stackup_signature(pcb):
    """Fingerprint the complete authored layer stack and board thickness."""
    thickness = stackup_form(stackup_form(pcb, "general"), "thickness")
    thickness_tokens = SEXPR_TOKEN.findall(thickness)
    require(len(thickness_tokens) == 4 and DECIMAL_ATOM.fullmatch(thickness_tokens[2]),
            "general.thickness needs a positive thickness in millimetres")
    thickness_mm = float(native_thickness_mm(thickness_tokens[2]))
    require(math.isfinite(thickness_mm) and thickness_mm > 0, "invalid board thickness")
    active = []
    for _, form in children(stackup_form(pcb, "layers")):
        tokens = SEXPR_TOKEN.findall(form)
        require(len(tokens) > 3, "invalid layer table entry")
        name = stackup_atom(tokens[2])
        if name.endswith(".Cu"):
            active.append(name)
    count = len(active)
    copper = ["F.Cu", *(f"In{i}.Cu" for i in range(1, count - 1)), "B.Cu"]
    require(2 <= count <= 32 and count % 2 == 0 and sorted(active) == sorted(copper),
            "stackup donor needs a standard KiCad copper stack")
    stackup = stackup_form(stackup_form(pcb, "setup"), "stackup")
    authored = []
    for key, layer in children(stackup):
        if key != "layer":
            continue
        tokens = SEXPR_TOKEN.findall(layer)
        require(len(tokens) > 3, "unnamed stackup layer")
        name = stackup_atom(tokens[2])
        kind = SEXPR_TOKEN.findall(stackup_form(layer, "type"))
        require(len(kind) == 4, "invalid stackup layer type")
        if stackup_atom(kind[2]) == "copper":
            authored.append(name)
        else:
            require(not name.endswith(".Cu"), f"stackup layer {name} has no copper type")
    require(authored == copper, "stackup copper sequence differs from the active layers")
    fingerprint = json.dumps([list(stackup_tokens(thickness)), list(stackup_tokens(stackup))], separators=(",", ":"))
    return {"sha256": hashlib.sha256(fingerprint.encode()).hexdigest(),
            "thickness_mm": thickness_mm, "copper_layers": copper}


def read_stackup_donor(path):
    data = path.read_bytes()
    return {"path": str(path), "sha256": hashlib.sha256(data).hexdigest()}, stackup_signature(data.decode())


def first_stackup_difference(reference, pcb):
    def value(form):
        tokens = SEXPR_TOKEN.findall(form)
        if len(tokens) == 4:
            return stackup_atom(tokens[2])
        return " ".join(tokens[2:-1])[:160]

    def different(a, b):
        return list(stackup_tokens(a)) != list(stackup_tokens(b))

    def changed(property_name, a, b):
        return {"property": property_name, "expected": a, "actual": b}

    thickness = [stackup_form(stackup_form(text, "general"), "thickness") for text in (reference, pcb)]
    if different(*thickness):
        return changed("board thickness", *map(value, thickness))
    forms = [children(stackup_form(stackup_form(text, "setup"), "stackup")) for text in (reference, pcb)]
    for (old_name, old), (new_name, new) in zip_longest(*forms, fillvalue=(None, "")):
        if old_name != new_name:
            return changed("stackup entry", old_name, new_name)
        if not different(old, new):
            continue
        if old_name != "layer":
            return changed(old_name, value(old), value(new))
        expected_layer, actual_layer = [stackup_atom(SEXPR_TOKEN.findall(form)[2]) for form in (old, new)]
        if expected_layer != actual_layer and not (numbered_dielectric(old) and numbered_dielectric(new)):
            return changed("layer", expected_layer, actual_layer)
        for (old_key, old_field), (new_key, new_field) in zip_longest(children(old), children(new), fillvalue=(None, "")):
            if old_key != new_key:
                return {"layer": expected_layer, **changed("layer entry", old_key, new_key)}
            if different(old_field, new_field):
                return {"layer": expected_layer, **changed(old_key, value(old_field), value(new_field))}
        # KiCad also uses bare markers between dielectric sublayers.
        return {"layer": expected_layer, **changed("layer payload", value(old), value(new))}
    return None


def check_stackup(expected, pcb, reference=None):
    try:
        actual = stackup_signature(pcb)
    except (ArithmeticError, ValueError) as error:
        return {"satisfied": False, "expected": expected, "error": str(error)}
    result = {"satisfied": actual == expected, "expected": expected, "actual": actual}
    if not result["satisfied"] and reference is not None:
        before = reference.read_text()
        require(stackup_signature(before) == expected, "stackup comparison snapshot differs from the recorded donor")
        result["difference"] = first_stackup_difference(before, pcb)
    return result


def check_imported_stackup(report, evidence, board):
    expected = evidence.get("expected_stackup")
    if expected is None:
        return
    source = report.get("stackup_from")
    require(isinstance(source, dict) and source.get("source_sha256") == evidence["inputs"]["stackup_from"]["sha256"],
            "no matching stackup donor provenance in the import report")
    require(check_stackup(expected, board.read_text())["satisfied"], "imported board changed the authored stackup")


def edge_geometry(items, prefix, y_sign=1):
    """Exact Edge.Cuts primitives, independent of IDs, ordering and stroke."""
    supported = {prefix + head[3:] for head in SHAPES}
    geometry = collections.Counter()
    for head, block in items:
        if not EDGE_LAYER.search(block):
            continue
        require(head in supported, f"unsupported Edge.Cuts item {head}")
        shape = head[3:]

        def point(name):
            found = re.search(rf"\({name}\s+{POINT}\s*\)", block)
            require(found is not None, f"unsupported {head} Edge.Cuts geometry: no {name} point")
            return Decimal(found[1]), y_sign * Decimal(found[2])

        if shape == "poly":
            geometry.update(polygon_contour(stackup_form(block, "pts"), Decimal, y_sign))
            continue
        if shape == "curve":
            points = [(Decimal(x), y_sign * Decimal(y)) for x, y in re.findall(rf"\(xy\s+{POINT}\s*\)", block)]
            require(len(points) == 4, "unsupported Edge.Cuts curve: expected four control points")
            geometry[("curve", *min(points, points[::-1]))] += 1
            continue
        end = point("end")
        if shape == "circle":
            center = point("center")
            radius_squared = sum((a - b) ** 2 for a, b in zip(center, end))
            require(radius_squared > 0, "outline circle has zero radius")
            geometry[("circle", center, radius_squared)] += 1
            continue
        start = point("start")
        if shape == "rect":
            geometry.update(rectangle(start, end))
        elif shape == "arc":
            geometry[("arc", min(start, end), point("mid"), max(start, end))] += 1
        else:
            geometry[segment(start, end)] += 1
    return geometry


def board_outline(items):
    geometry = edge_geometry(((head, block) for head, block in items if head not in {"footprint", "module"}), "gr_")
    require(bool(geometry), "no board-level Edge.Cuts geometry")
    return geometry


def check_zone_ids(row, items):
    """KiCad can report clean DRC for two separate zones with one UUID."""
    counts = collections.Counter()
    for key, block in items:
        if key != "zone":
            continue
        for field, value in children(block):
            if field == "uuid":
                match = outline_rule_areas.ITEM_ID.fullmatch(value)
                if match:
                    counts[match[1].lower()] += 1
    conflicts = {uuid: count for uuid, count in sorted(counts.items()) if count > 1}
    if conflicts:
        row["duplicate_zone_uuids"] = conflicts
        row.setdefault("next_actions", []).append(
            "Give each board-level zone a unique UUID so KiCad and repair tools can identify "
            "its copper unambiguously. Refill, save, and verify the board again."
        )


def outline_digest(geometry):
    def number(value):
        return format(value.normalize(), "f") if value else "0"

    serialized = json.dumps(sorted(geometry.items()), default=number, separators=(",", ":"))
    return hashlib.sha256(serialized.encode()).hexdigest()


def footprint_cutouts(items):
    """Footprint-local cutouts at the library orientation, keyed by reference."""
    footprint = {}
    for head, block in items:
        if head not in {"footprint", "module"} or not EDGE_LAYER.search(block):
            continue
        properties = list(children(block))
        layers = [re.fullmatch(r'\(layer\s+"?([FB]\.Cu)"?\s*\)', form) for key, form in properties if key == "layer"]
        require(len(layers) == 1 and layers[0], "unknown side of footprint with Edge.Cuts")
        # KiCad stores a flipped footprint mirrored in its local Y axis.
        cutouts = edge_geometry(properties, "fp_", -1 if layers[0][1] == "B.Cu" else 1)
        reference = re.search(rf'\((?:property\s+"Reference"|fp_text\s+reference)\s+({QUOTED})', block)
        require(reference is not None, "footprint with Edge.Cuts has no reference")
        reference = json.loads(reference[1])
        require(reference not in footprint, f"duplicate reference {reference} with footprint Edge.Cuts")
        footprint[reference] = cutouts
    return footprint


def footprint_clearance_overrides(items):
    """Explicit footprint clearance fields keyed by reference."""
    overrides = {}
    for kind, block in items:
        if kind not in {"footprint", "module"}:
            continue
        parts = list(children(block))
        fields = [field for key, field in parts if key == "clearance"]
        if not fields:
            continue
        refs = [re.match(rf'\((?:property\s+"Reference"|fp_text\s+reference)\s+({QUOTED})', field)
                for key, field in parts if key in {"property", "fp_text"}]
        refs = [json.loads(match[1]) for match in refs if match is not None]
        require(len(refs) == 1 and refs[0] not in overrides,
                "an explicit footprint clearance has no unique reference")
        value = re.fullmatch(rf'\(clearance\s+({DECIMAL_ATOM.pattern})\s*\)', fields[0])
        require(len(fields) == 1 and value is not None, f"invalid footprint clearance on {refs[0]}")
        overrides[refs[0]] = Decimal(value[1])
    return overrides


def check_footprint_clearances(reference_items, actual_items):
    expected = footprint_clearance_overrides(reference_items)
    actual = footprint_clearance_overrides(actual_items)
    differences = []
    for ref in sorted(expected.keys() | actual.keys()):
        intended, found = expected.get(ref), actual.get(ref)
        if intended != found:
            differences.append({"reference": ref,
                                "expected_mm": float(intended) if intended is not None else None,
                                "actual_mm": float(found) if found is not None else None})
    return {"satisfied": not differences, "reference_overrides": len(expected),
            "candidate_overrides": len(actual), "differences": differences}


def graphic_geometry(pcbnew, graphic, fields):
    """Canonical local geometry at 10 nm, absorbing KiCad's rotation rounding."""
    shape = graphic.GetShape()

    def point(key):
        found = re.fullmatch(rf"\({key}\s+{POINT}\s*\)", fields.pop(key, ""))
        require(found is not None, f"native footprint shape has no {key} point")
        return tuple(round(float(v), 5) for v in found.groups())

    if shape == pcbnew.SHAPE_T_RECT:
        contour = rectangle(point("start"), point("end"))
        return {"type": "polygon", "contour": sorted(contour), "fields": fields}
    if shape == pcbnew.SHAPE_T_POLY:
        contour = polygon_contour(fields.pop("pts", ""), lambda v: round(float(v), 5))
        return {"type": "polygon", "contour": sorted(contour), "fields": fields}
    if shape == pcbnew.SHAPE_T_BEZIER:
        points = [tuple(round(float(v), 5) for v in xy)
                  for xy in re.findall(rf"\(xy\s+{POINT}\s*\)", fields.pop("pts", ""))]
        require(len(points) == 4, "native footprint curve has no control point evidence")
        return {"type": "curve", "points": min(points, points[::-1]), "fields": fields}
    if shape == pcbnew.SHAPE_T_CIRCLE:
        center = point("center")
        point("end")
        return {"type": "circle", "center": center, "radius": round(pcbnew.ToMM(graphic.GetRadius()), 5), "fields": fields}
    if shape in (pcbnew.SHAPE_T_SEGMENT, pcbnew.SHAPE_T_ARC):
        ends = sorted((point("start"), point("end")))
        if shape == pcbnew.SHAPE_T_ARC:
            return {"type": "arc", "ends": ends, "mid": point("mid"), "fields": fields}
        return {"type": "line", "ends": ends, "fields": fields}
    raise ValueError(f"unsupported native footprint graphic shape {shape}")


def module_layer_id(pcbnew, board, layer):
    aliases = {"Eco1.User": "User.Eco1", "Eco2.User": "User.Eco2",
               "Dwgs.User": "User.Drawings", "Cmts.User": "User.Comments"}
    layer_id = board.GetLayerID(layer)
    if layer_id == pcbnew.UNDEFINED_LAYER and layer in aliases:
        layer_id = board.GetLayerID(aliases[layer])
    require(layer_id != pcbnew.UNDEFINED_LAYER, f"unknown module contour layer {layer!r}")
    return layer_id


def normalized_footprints(pcbnew, board, *, electrical, module_groups=(), modules_only=False,
                          selected_parts=None, all_graphics=False):
    """Read native padstacks and local physical graphics in the front-side frame."""
    by_reference = collections.defaultdict(list)
    copper_layers = set(board.GetEnabledLayers().CuStack())
    copper = {board.GetStandardLayerName(layer) for layer in copper_layers}
    graphics_layers = copper_layers | {pcbnew.F_Fab, pcbnew.B_Fab, pcbnew.F_CrtYd, pcbnew.B_CrtYd,
                                       pcbnew.F_Mask, pcbnew.B_Mask, pcbnew.F_Paste, pcbnew.B_Paste}
    formatter = pcbnew.PCB_IO_KICAD_SEXPR()
    module_layers = collections.defaultdict(set)
    for group in module_groups:
        for ref in group["parts"]:
            module_layers[ref].add(module_layer_id(pcbnew, board, group["layer"]))

    def forms(item):
        formatter.Format(item)
        return [(key, value.strip()) for key, value in children(formatter.GetStringOutput(True))]

    for part in board.GetFootprints():
        if selected_parts is not None and part.GetReference() not in selected_parts:
            continue
        if modules_only and part.GetReference() not in module_layers:
            continue
        part_graphics = graphics_layers | module_layers.get(part.GetReference(), set())
        local = pcbnew.FOOTPRINT(part)
        if local.IsFlipped():
            local.Flip(local.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        local.SetOrientationDegrees(0)
        local.SetPosition(pcbnew.VECTOR2I(0, 0))
        pads = {}
        for pad in local.Pads():
            geometry = {"attribute": pad.GetAttribute(), "shape": pad.GetShape()}
            for key, form in forms(pad):
                if key in {"uuid", "net", "pinfunction", "pintype", "locked", "teardrops"}:
                    continue
                # On these pads KiCad records the copper layers connected by
                # the last zone fill. The pad's authored layer policy is separate.
                if (key == "zone_layer_connections" and pad.GetAttribute() == pcbnew.PAD_ATTRIB_PTH
                        and pad.GetRemoveUnconnected()):
                    continue
                if key == "layers":
                    layers = {json.loads(token) for token in re.findall(QUOTED, form)}
                    # KiCad drops disabled copper layers on save. A through
                    # pad can also spell out every active layer or use *.Cu.
                    active = {board.GetStandardLayerName(layer) for layer in copper_layers if pad.IsOnLayer(layer)}
                    if active and active == copper:
                        active = {"*.Cu"}
                    geometry[key] = sorted(active | {layer for layer in layers if not layer.endswith(".Cu")})
                elif key == "at":
                    _, x, y, *rotation = form[1:-1].split()
                    geometry[key] = [round(float(x), 5), round(float(y), 5), *rotation]
                else:
                    geometry[key] = form
            item = {"geometry": geometry}
            if all_graphics:
                # Whole-instance copies must keep each native pad's
                # geometry, even if two pads share a number.
                item["uuid"] = pad.m_Uuid.AsString()
            if electrical:
                item.update(net=pad.GetNetname(), function=pad.GetPinFunction())
            pads.setdefault(pad.GetNumber(), []).append(item)
        body = []
        ignored = {"uuid", "locked"} if all_graphics else {"uuid", "net", "locked"}
        for graphic in local.GraphicalItems():
            if not isinstance(graphic, pcbnew.PCB_SHAPE):
                if all_graphics and graphic.GetLayer() in copper_layers | {pcbnew.Edge_Cuts}:
                    raise ValueError(f"add {part.GetReference()}: unsupported native graphic on a copper or edge layer")
                continue
            if not all_graphics and graphic.GetLayer() not in part_graphics:
                continue
            fields = {key: value for key, value in forms(graphic) if key not in ignored}
            body.append(graphic_geometry(pcbnew, graphic, fields))
        reference = part.GetReference()
        order = lambda item: json.dumps(item, sort_keys=True)
        signature = {"body": sorted(body, key=order), "pads": {pin: sorted(items, key=order) for pin, items in pads.items()}}
        if electrical:
            signature["library"] = local.GetFPIDAsString()
        position = part.GetPosition()
        by_reference[reference].append({
            "signature": signature,
            "uuid": part.m_Uuid.AsString(),
            "position_mm": [pcbnew.ToMM(position.x), pcbnew.ToMM(position.y)],
            "angle_degrees": part.GetOrientationDegrees(),
            "side": "back" if part.IsFlipped() else "front",
        })
    require(bool(by_reference) or selected_parts is not None, "no footprints to check")
    return {
        ref: {
            "occurrences": sorted(occurrences, key=lambda item: (
                item["position_mm"], item["side"], item["angle_degrees"], item["uuid"],
                footprint_signature_key(item["signature"]),
            )),
        }
        for ref, occurrences in by_reference.items()
    }


def footprint_signature_key(signature):
    return json.dumps(signature, sort_keys=True, separators=(",", ":"))


def footprint_occurrences(signature):
    if signature is None:
        return []
    return signature.get("occurrences", [{"signature": signature}])


def occurrence_evidence(occurrence):
    signature = occurrence["signature"]
    return {**{key: value for key, value in occurrence.items() if key != "signature"},
            "signature_sha256": hashlib.sha256(footprint_signature_key(signature).encode()).hexdigest(),
            "pads": sorted(signature["pads"]), "body_shapes": len(signature["body"])}


def footprint_differences(expected, actual):
    differences = []

    def compare(ref, source, candidate, **evidence):
        for field in ("library", "body"):
            if source.get(field) != candidate.get(field):
                differences.append({"reference": ref, "difference": field, "source": source.get(field),
                                    "candidate": candidate.get(field), **evidence})
        for pad in sorted(source["pads"].keys() | candidate["pads"].keys()):
            a, b = source["pads"].get(pad), candidate["pads"].get(pad)
            if a != b:
                differences.append({"reference": ref, "pad": pad, "source": a, "candidate": b, **evidence})

    for ref in sorted(expected.keys() | actual.keys()):
        sources = footprint_occurrences(expected.get(ref))
        candidates = footprint_occurrences(actual.get(ref))
        if len(sources) <= 1 and len(candidates) <= 1:
            if not sources or not candidates:
                differences.append({"reference": ref, "difference": "missing" if not candidates else "extra"})
            else:
                compare(ref, sources[0]["signature"], candidates[0]["signature"])
            continue
        if len(sources) != len(candidates):
            differences.append({"reference": ref, "difference": "count", "source_count": len(sources),
                                "candidate_count": len(candidates)})
        source_uuids = collections.Counter(source.get("uuid") for source in sources)
        candidate_uuids = collections.Counter(candidate.get("uuid") for candidate in candidates)
        candidates_by_uuid = {candidate["uuid"]: index for index, candidate in enumerate(candidates)
                              if candidate.get("uuid") and candidate_uuids[candidate["uuid"]] == 1}
        matched_sources, matched = set(), set()
        # Where stable IDs survived, associate identical models by identity
        # before matching interchangeable models with newly allocated IDs.
        for source_index, source in enumerate(sources):
            uuid = source.get("uuid")
            index = candidates_by_uuid.get(uuid)
            if (index is not None and source_uuids[uuid] == 1
                    and source["signature"] == candidates[index]["signature"]):
                matched_sources.add(source_index)
                matched.add(index)
        by_signature = collections.defaultdict(collections.deque)
        for index, candidate in enumerate(candidates):
            if index not in matched:
                by_signature[footprint_signature_key(candidate["signature"])].append(index)
        missing = []
        for source_index, source in enumerate(sources):
            if source_index in matched_sources:
                continue
            matches = by_signature.get(footprint_signature_key(source["signature"]))
            if matches:
                matched.add(matches.popleft())
            else:
                missing.append(source)
        for source in missing:
            uuid = source.get("uuid")
            index = candidates_by_uuid.get(uuid)
            source_occurrence = occurrence_evidence(source)
            if index is not None and index not in matched and source_uuids[uuid] == 1:
                candidate = candidates[index]
                compare(ref, source["signature"], candidate["signature"], source_occurrence=source_occurrence,
                        candidate_occurrence=occurrence_evidence(candidate))
                matched.add(index)
            else:
                differences.append({"reference": ref, "difference": "missing", "source_occurrence": source_occurrence})
        for index, candidate in enumerate(candidates):
            if index not in matched:
                differences.append({"reference": ref, "difference": "extra", "candidate_occurrence": occurrence_evidence(candidate)})
    return differences


def expected_outline(brief, donor):
    donor_outline = board_outline(children(donor.read_text())) if donor is not None else None
    outline = brief.get("outline")
    if outline is None:
        require(donor is not None, "an exact outline requires --outline-board or an explicit outline in the brief")
        return donor_outline
    require(isinstance(outline, dict) and set(outline) <= {"x", "y", "width", "height", "polygon"},
            "unsupported brief outline; use a fixed rectangle or polygon, or supply --outline-board")
    x, y = map(lambda value: Decimal(str(value)), numbers([outline.get("x"), outline.get("y")], 2, "outline x/y"))
    if "polygon" in outline:
        require(not {"width", "height"} & outline.keys() and isinstance(outline["polygon"], list)
                and len(outline["polygon"]) >= 3, "invalid outline polygon")
        points = [tuple(Decimal(str(value)) for value in numbers(pair, 2, "outline point"))
                  for pair in outline["polygon"]]
        x0, y0 = min(p[0] for p in points), min(p[1] for p in points)
        geometry = collections.Counter(polygon([(x + a - x0, y + b - y0) for a, b in points]))
    else:
        width, height = numbers([outline.get("width"), outline.get("height")], 2, "outline width/height")
        require(min(width, height) > 0, "outline dimensions must be positive")
        geometry = collections.Counter(rectangle((x, y), (x + Decimal(str(width)), y + Decimal(str(height)))))
    require(donor_outline is None or donor_outline == geometry,
            "the explicit brief outline disagrees with --outline-board; supply one intended outline")
    return geometry


def glob_pattern(pattern):
    """KiCad placement reference syntax: *, ?, and backslash escapes."""
    require(isinstance(pattern, str) and pattern, "empty or invalid part selector")
    tokens = re.findall(r"\\.|.", pattern)
    return re.compile("".join(".*" if token == "*" else "." if token == "?" else re.escape(token[-1])
                              for token in tokens))


def native_body_box(pcbnew, footprint):
    """Project the nominal local courtyard and pad extent to the board axes."""
    angle = math.radians(-footprint.GetOrientationDegrees())
    position = tuple(footprint.GetPosition())
    local = pcbnew.FOOTPRINT(footprint)
    local.SetOrientationDegrees(0)
    local.SetPosition(pcbnew.VECTOR2I(0, 0))
    front, back = pcbnew.F_Cu, pcbnew.B_Cu
    courts = {pcbnew.F_CrtYd, pcbnew.B_CrtYd}
    supported = {pcbnew.SHAPE_T_SEGMENT, pcbnew.SHAPE_T_RECT, pcbnew.SHAPE_T_CIRCLE,
                 pcbnew.SHAPE_T_ARC, pcbnew.SHAPE_T_POLY}
    graphics = []
    for graphic in local.GraphicalItems():
        layer = graphic.GetLayer()
        if (not isinstance(graphic, pcbnew.PCB_SHAPE) or graphic.GetShape() not in supported
                or not (layer in courts | {pcbnew.F_Mask, pcbnew.B_Mask} or pcbnew.IsCopperLayer(layer))):
            continue
        # The placer uses the graphic centre lines without stroke width.
        graphic.SetWidth(0)
        box = graphic.GetBoundingBox()
        graphics.append((layer, graphic.GetShape(),
                         (box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom())))

    def projected(centre, half, theta, origin=(0.0, 0.0)):
        sine, cosine = math.sin(theta), math.cos(theta)
        x = origin[0] + cosine * centre[0] - sine * centre[1]
        y = origin[1] + sine * centre[0] + cosine * centre[1]
        dx = abs(cosine) * half[0] + abs(sine) * half[1]
        dy = abs(sine) * half[0] + abs(cosine) * half[1]
        return x - dx, y - dy, x + dx, y + dy

    pads = []
    pad_layers = set()
    for pad in local.Pads():
        shapes = {pad.GetShape(layer) for layer in pad.GetLayerSet().CuStack()}
        require(pcbnew.PAD_SHAPE_CUSTOM not in shapes,
                f"body-to-pin near needs standard pad geometry for {footprint.GetReference()}")
        if pad.GetAttribute() == pcbnew.PAD_ATTRIB_SMD:
            pad_layers.update(layer for layer in (front, back) if pad.IsOnLayer(layer))
        size = tuple(max(a, b) for a, b in zip(pad.GetSize(), pad.GetDrillSize()))
        clearance = 0
        if pad.GetAttribute() == pcbnew.PAD_ATTRIB_NPTH:
            clearance = pad.GetLocalClearance()
            if clearance is None:
                clearance = local.GetLocalClearance() or 0
            clearance = max(0, clearance)
        theta = math.radians(-pad.GetOrientationDegrees())
        extent = projected(tuple(pad.GetOffset()), [width / 2 for width in size], theta, tuple(pad.GetPosition()))
        pads.append(tuple(value + (clearance if index >= 2 else -clearance)
                          for index, value in enumerate(extent)))

    def local_box(items):
        boxes = [box for _, _, box in items] + pads
        require(bool(boxes), f"near has no physical body geometry for {footprint.GetReference()}")
        lower = [min(box[i] for box in boxes) for i in range(2)]
        upper = [max(box[i] for box in boxes) for i in range(2, 4)]
        centre = [(a + b) / 2 for a, b in zip(lower, upper)]
        half = [max((b - a) / 2, pcbnew.FromMM(0.1)) for a, b in zip(lower, upper)]
        return centre, half

    if courts <= {layer for layer, _, _ in graphics}:
        mounted = next(iter(pad_layers)) if len(pad_layers) == 1 else footprint.GetLayer()
        copper, mask, court = ((front, pcbnew.F_Mask, pcbnew.F_CrtYd) if mounted == back
                               else (back, pcbnew.B_Mask, pcbnew.B_CrtYd))
        own = [(layer, shape, box) for layer, shape, box in graphics if layer not in {copper, mask, court}]
        _, half = local_box(own)
        round_body = (len(own) == 1 and own[0][1] == pcbnew.SHAPE_T_CIRCLE
                      and abs(half[0] - half[1]) < 0.5)
        if not round_body:
            graphics = own
    local_centre, local_half = local_box(graphics)
    return projected(local_centre, local_half, angle, position)


def near_checks(entries, board_path, *, measure_routed=False):
    """Measure literal pad-to-pad and body-to-pin constraints on the returned PCB."""
    require(isinstance(entries, list), "near must be a list")
    if not entries:
        return []
    parsed = []
    for entry in entries:
        selectors = {"part", "pin_of"}
        require(isinstance(entry, dict) and selectors <= set(entry)
                and set(entry) <= selectors | {"pad", "max_mm", "minimize"},
                "independent near scoring requires part and pin_of, with optional pad")
        part, pad, target = entry["part"], entry.get("pad"), entry["pin_of"]
        require(all(isinstance(s, str) and s for s in (part, target))
                and ("pad" not in entry or isinstance(pad, str) and pad), "invalid near pad selector")
        ref, separator, target_pad = target.partition(":")
        require(separator and ref and target_pad, f"pin_of {target!r} must name REF:PAD")
        minimize = entry.get("minimize", False)
        require(type(minimize) is bool, "near minimize must be a Boolean")
        limit = entry.get("max_mm")
        if minimize:
            require(pad is not None, "near minimize: true needs part, pad, pin_of")
            require(limit is None, "near minimize: true cannot be combined with a numeric max_mm")
        else:
            limit, = numbers([limit], 1, "near max_mm")
            limit = max(0, limit)
        parsed.append((part, pad, ref, target_pad, limit))
    require(board_path is not None, "missing candidate board for native near measurement")
    try:
        import pcbnew
    except ImportError as error:
        raise ValueError("near scoring needs KiCad's pcbnew Python bindings") from error
    board = pcbnew.LoadBoard(str(board_path))
    require(board is not None, "cannot read candidate pad centres")
    footprints = collections.defaultdict(list)
    for part in board.GetFootprints():
        footprints[part.GetReference()].append(part)

    def exact_part(ref):
        parts = footprints[ref]
        require(len(parts) == 1, f"near needs one footprint for {ref!r}")
        return parts[0]

    def centre(ref, number):
        points = {tuple(p.GetPosition()) for p in exact_part(ref).Pads() if p.GetNumber() == number}
        require(len(points) == 1, f"near needs one pad centre for {ref}:{number}")
        return points.pop()

    checks = []
    for part, pad, ref, target_pad, limit in parsed:
        target = centre(ref, target_pad)
        check = {"part": part, "pin_of": f"{ref}:{target_pad}"}
        if pad is None:
            box = native_body_box(pcbnew, exact_part(part))
            distance_nm = math.hypot(*(max(box[i] - value, 0, value - box[i + 2])
                                       for i, value in enumerate(target)))
            check.update(measurement="body_to_pin", body_bounds_mm=[round(v / 1e6, 6) for v in box],
                         target_centre_mm=[pcbnew.ToMM(v) for v in target])
        else:
            distance_nm = math.dist(centre(part, pad), target)
            check["pad"] = pad
        distance = distance_nm / pcbnew.FromMM(1)
        check["distance_mm"] = round(distance, 6)
        if limit is None:
            check.update(kind="pad_near_objective", measured=True)
        else:
            check.update(kind="near", max_mm=limit, excess_mm=round(max(0.0, distance - limit), 6),
                         # The placer allows 0.05 mm of relation slack when snapping to its grid.
                         within_placer_tolerance=distance <= limit + 0.05, satisfied=distance <= limit + 1e-6)
        checks.append(check)
    if measure_routed:
        from routed_supply_paths import measure_near_paths

        for check, result in zip(checks, measure_near_paths(pcbnew, board, checks), strict=True):
            check["routed_path"] = {key: value for key, value in result["routed_path"].items()
                                    if key != "trace_segments"}
    return checks


def pad_distance_checks(entries, board_path):
    """Measure the exact-pad form required by pad-pair objectives."""
    require(isinstance(entries, list), "near must be a list")
    require(all(isinstance(entry, dict) and {"part", "pad", "pin_of"} <= set(entry)
                for entry in entries), "pad-to-pad scoring requires part, pad, pin_of")
    return near_checks(entries, board_path)


def module_contour_gap_mm(first, second):
    """Shortest native segment distance for disjoint polygons, including corners."""
    def edges(polygon):
        for index in range(polygon.OutlineCount()):
            chains = [polygon.COutline(index)]
            chains.extend(polygon.CHole(index, hole) for hole in range(polygon.HoleCount(index)))
            for chain in chains:
                yield from (chain.CSegment(i) for i in range(chain.SegmentCount()))

    other_edges = list(edges(second))
    squared = min(a.SquaredDistance(b) for a in edges(first) for b in other_edges)
    return math.sqrt(squared) / 1e6


def module_nonoverlap_checks(entries, board_path):
    """Measure mounted contours using KiCad's placed graphics."""
    require(isinstance(entries, list), "module_nonoverlap must be a list")
    if not entries:
        return []
    require(board_path is not None, "missing candidate board for module geometry")
    try:
        import pcbnew
    except ImportError as error:
        raise ValueError("module contour checks need KiCad's pcbnew Python bindings") from error
    board = pcbnew.LoadBoard(str(board_path))
    by_ref = collections.defaultdict(list)
    for footprint in board.GetFootprints():
        by_ref[footprint.GetReference()].append(footprint)
    checks = []
    for entry in entries:
        require(isinstance(entry, dict) and {"parts", "layer"} <= set(entry) <=
                {"parts", "layer", "min_gap_mm"},
                "module_nonoverlap needs parts and a graphic layer with optional min_gap_mm")
        refs, layer = entry["parts"], entry["layer"]
        minimum = entry.get("min_gap_mm", 0.0)
        require(type(minimum) in (int, float) and math.isfinite(minimum) and minimum >= 0,
                "module_nonoverlap min_gap_mm must be a finite nonnegative length")
        require(isinstance(refs, list) and len(refs) >= 2 and
                all(isinstance(ref, str) and ref.strip() for ref in refs) and len(set(refs)) == len(refs),
                "module_nonoverlap needs at least two distinct exact references")
        require(isinstance(layer, str) and layer.strip(), "module_nonoverlap needs a named layer")
        layer_id = module_layer_id(pcbnew, board, layer)
        contours = {}
        for ref in refs:
            require(len(by_ref[ref]) == 1, f"module_nonoverlap needs one exact footprint for {ref!r}")
            footprint = by_ref[ref][0]
            # The native shapes already include footprint translation, rotation and flip.
            outline_board = pcbnew.BOARD()
            for graphic in footprint.GraphicalItems():
                if graphic.GetLayer() == layer_id and isinstance(graphic, pcbnew.PCB_SHAPE):
                    edge = graphic.Duplicate()
                    edge.SetLayer(pcbnew.Edge_Cuts)
                    outline_board.Add(edge)
            polygon = pcbnew.SHAPE_POLY_SET()
            require(outline_board.GetBoardPolygonOutlines(polygon, False) and polygon.Area() > 0,
                    f"{ref} has no usable closed module contour on {layer}")
            contours[ref] = (footprint.IsFlipped(), polygon)
        for first, second in combinations(refs, 2):
            same_side = contours[first][0] == contours[second][0]
            area = 0
            gap = None
            if same_side:
                first_poly, second_poly = contours[first][1], contours[second][1]
                intersection = pcbnew.SHAPE_POLY_SET()
                intersection.BooleanIntersection(first_poly, second_poly)
                area = intersection.Area()
                gap = 0.0 if area else module_contour_gap_mm(first_poly, second_poly)
            checks.append({"kind": "module_nonoverlap", "parts": [first, second], "layer": layer,
                           "same_side": same_side, "intersection_mm2": area / 1e12,
                           "distance_mm": gap, "min_gap_mm": minimum,
                           "satisfied": area == 0 and (gap is None or gap + 1e-6 >= minimum)})
    return checks


def checks_for(brief, description, board_path=None):
    """Measure the supported mechanical constraints at the supplied poses."""
    require(isinstance(brief, dict) and type(brief.get("version")) is int and brief["version"] == 1,
            "expected version 1 placement constraints")
    kinds = {"place", "rotation", "region", "front", "back"}
    unknown = set(brief) - kinds - {"version", "comment", "move_all", "outline", "keepout", "near", "module_nonoverlap"}
    require(not unknown, f"unsupported placement constraint kinds: {', '.join(sorted(unknown))}")
    require(type(brief.get("move_all", False)) is bool, "invalid move_all")
    footprints = description.get("footprints")
    require(isinstance(footprints, list) and footprints and all(isinstance(part, dict)
            and isinstance(part.get("reference"), str) and part["reference"] for part in footprints),
            "missing footprint reference evidence")
    outline = description.get("outline")
    require(isinstance(outline, dict) and outline.get("from_footprints") is False,
            "no usable board-level outline in the candidate description")
    board_min = numbers(outline.get("minimum"), 2, "board minimum")
    board_max = numbers(outline.get("maximum"), 2, "board maximum")
    require(all(low < high for low, high in zip(board_min, board_max)), "invalid board outline bounds")

    def offset(entry):
        origin = entry.get("origin")
        require(origin in (None, "board", "absolute"), f"unsupported origin {origin!r}")
        return (0, 0) if origin == "absolute" else board_min

    def matching(pattern):
        compiled = glob_pattern(pattern)
        result = [part for part in footprints if compiled.fullmatch(part["reference"])]
        require(bool(result), f"part selector {pattern!r} matches no candidate footprint")
        return result

    def measured(kind, part, expected, actual, satisfied):
        return {"kind": kind, "part": part["reference"], "satisfied": satisfied,
                "expected": expected, "actual": actual}

    def angle_delta(a, b):
        return abs((a - b + 180) % 360 - 180)

    checks = near_checks(brief.get("near", []), board_path, measure_routed=True)
    checks.extend(module_nonoverlap_checks(brief.get("module_nonoverlap", []), board_path))
    for kind in sorted(kinds):
        entries = brief.get(kind, [])
        require(isinstance(entries, list), f"{kind} must be a list")
        for entry in entries:
            if kind in {"front", "back"}:
                expected = "F.Cu" if kind == "front" else "B.Cu"
                for part in matching(entry):
                    layer = part.get("layer")
                    require(layer in {"F.Cu", "B.Cu"}, f"missing layer evidence for {part['reference']}")
                    checks.append(measured(kind, part, expected, layer, layer == expected))
                continue
            require(isinstance(entry, dict), f"invalid {kind} entry")
            if kind == "region":
                require(set(entry) <= {"parts", "x", "y", "origin"}, "unsupported region; give x and y ranges")
                selectors = entry.get("parts")
                require(isinstance(selectors, list) and selectors, "region needs part selectors")
                x, y = offset(entry)
                xx = sorted(numbers(entry.get("x"), 2, "region x"))
                yy = sorted(numbers(entry.get("y"), 2, "region y"))
                region = [x + xx[0], y + yy[0], x + xx[1], y + yy[1]]
                for selector in selectors:
                    for part in matching(selector):
                        body = numbers(part.get("body"), 4, f"body bounds for {part['reference']}")
                        require(body[0] < body[2] and body[1] < body[3],
                                f"no usable body bounds for {part['reference']}")
                        fits = (all(body[i] >= region[i] - BODY_EPSILON_MM for i in (0, 1)) and
                                all(body[i] <= region[i] + BODY_EPSILON_MM for i in (2, 3)))
                        checks.append(measured(kind, part, region, body, fits))
                continue
            reference, uuid = entry.get("part"), entry.get("uuid")
            require(isinstance(reference, str) and reference and (uuid is None or isinstance(uuid, str)),
                    f"invalid {kind} footprint reference or UUID")
            parts = [part for part in footprints if part["reference"] == reference
                     and (uuid is None or part.get("uuid") == uuid)]
            require(len(parts) == 1, f"{kind} needs one exact candidate footprint for {reference!r}; found {len(parts)}")
            part = parts[0]
            at = numbers(part.get("at"), 3, f"pose for {reference}")
            if kind == "place":
                require(set(entry) <= {"part", "uuid", "origin", "x", "y", "angle"},
                        "unsupported place; give an exact x, y and angle")
                x, y = offset(entry)
                expected = numbers([entry.get("x"), entry.get("y"), entry.get("angle")], 3, "place x/y/angle")
                expected = [x + expected[0], y + expected[1], expected[2]]
                fits = (math.dist(at[:2], expected[:2]) <= POSITION_EPSILON_MM and
                        angle_delta(at[2], expected[2]) <= ANGLE_EPSILON)
                checks.append(measured(kind, part, expected, at, fits))
            else:
                require(set(entry) <= {"part", "uuid", "angle", "angles"}, "unsupported rotation entry")
                angles = entry.get("angles", [])
                require(isinstance(angles, list), "rotation angles must be a list")
                if entry.get("angle") is not None:
                    angles = angles + [entry["angle"]]
                require(bool(angles), "rotation needs an allowed angle")
                numbers(angles, len(angles), "rotation angles")
                fits = any(angle_delta(at[2], angle) <= ANGLE_EPSILON for angle in angles)
                checks.append(measured(kind, part, angles, at[2], fits))
    return checks


def missed_constraints(checks):
    return [check for check in checks if check["kind"] != "pad_near_objective" and not check["satisfied"]]


def pad_near_retry(checks, missed, footprints):
    """Give a placement retry for missed authored pad limits with a movable source."""
    pinned = {check["part"] for check in checks if check["kind"] == "place"}
    movable = {part["reference"] for part in footprints if part.get("locked") is False} - pinned
    indices = []
    pairs = []
    for index, check in enumerate(missed):
        if (check["kind"] != "near" or "pad" not in check or "pin_of" not in check
                or check["part"] not in movable or check["part"] == check["pin_of"].partition(":")[0]):
            continue
        indices.append(index)
        pairs.append(
            f'{check["part"]}:{check["pad"]} to {check["pin_of"]} measured {check["distance_mm"]} mm '
            f'({check["excess_mm"]} mm over the {check["max_mm"]} mm limit)'
        )
    if not indices:
        return None
    return {
        "kind": "refine_pad_near",
        "constraints_missed_indices": indices,
        "placer_config": {"tuning": {"refine_pad_near": True}},
    }, (
        f'{". ".join(pairs)}. Try a new layout from the copper-free starter in a fresh directory. '
        'For standalone layout, save placement_retry.placer_config as a JSON file and pass it to '
        '--placer-config. For layout-kicad-board, merge the same settings into placer in layout.json. '
        'Verify the result against the same brief.'
    )


def native_is_excluded(finding):
    return finding.get("excluded") is True or finding.get("severity") == "exclusion"


def native_counts(report, board_name, transcript, *, schematic_parity=True):
    require(isinstance(report, dict) and isinstance(report.get("source"), str)
            and Path(report["source"]).name == board_name and report.get("coordinate_units") == "mm"
            and isinstance(report.get("kicad_version"), str) and report["kicad_version"],
            "no fresh KiCad report for the staged board")
    included = report.get("included_severities")
    require(isinstance(included, list) and all(severity in included for severity in SEVERITIES),
            "KiCad report lacks all-severity evidence")
    ignored = report.get("ignored_checks", [])
    require(isinstance(ignored, list) and all(isinstance(check, dict) for check in ignored),
            "malformed ignored_checks in KiCad report")
    require(not any(check.get("key") == "unconnected_items" for check in ignored),
            "KiCad skipped the unconnected-items check. Enable it in the project rules to verify connectivity.")
    active, excluded = {}, {}
    for field in ("violations", "unconnected_items", "schematic_parity"):
        findings = report.get(field)
        require(isinstance(findings, list) and all(isinstance(item, dict)
                and isinstance(item.get("type"), str) and item["type"]
                and item.get("severity") in ("error", "warning", "exclusion")
                and type(item.get("excluded", False)) is bool for item in findings),
                f"missing or malformed KiCad {field} evidence")
        active[field] = [item for item in findings if not native_is_excluded(item)]
        excluded[field] = len(findings) - len(active[field])
    parity = None
    if schematic_parity:
        total = len(report["schematic_parity"])
        require(f"Found {total} schematic parity issues" in transcript.splitlines(),
                "KiCad did not confirm it completed schematic parity")
        parity = len(active["schematic_parity"])
    else:
        require(not report["schematic_parity"], "unexpected schematic parity findings for a board-only check")
    counts = collections.Counter(finding["severity"] for finding in active["violations"])
    return {"drc_errors": counts["error"], "drc_warnings": counts["warning"],
            "drc_exclusions": excluded["violations"], "unconnected": len(active["unconnected_items"]),
            "unconnected_exclusions": excluded["unconnected_items"], "parity": parity,
            "parity_exclusions": excluded["schematic_parity"], "kicad_version": report["kicad_version"],
            "kicad_checked": True}


def explain_parity(row, report, imported, report_path):
    findings = [item for item in report.get("schematic_parity", []) if not native_is_excluded(item)]
    if not findings:
        return
    row["parity_report"] = report_path
    row["schematic_parity_by_type"] = dict(collections.Counter(item["type"] for item in findings))
    missing = set()
    for finding in findings:
        if finding["type"] != "net_conflict":
            continue
        pin = re.fullmatch(r"No pad found for pin (\S+)(?: \(.*\))? in schematic\.?", finding.get("description", ""))
        items = finding.get("items", [])
        if (pin and isinstance(items, list) and len(items) == 1 and isinstance(items[0], dict)
                and items[0].get("description", "").startswith("Footprint ")):
            missing.add((items[0]["description"].removeprefix("Footprint "), pin[1]))
    matched = [pin for pin in imported.get("unmatched_pin_details", [])
               if (pin.get("reference"), pin.get("pin")) in missing]
    actions = row.setdefault("next_actions", [])
    if matched:
        row["unmatched_pin_details"] = matched
        groups = collections.defaultdict(list)
        for pin in matched:
            groups[pin["reference"], pin["connection"], pin["next_action"]].append(pin["pin"])
        for (reference, connection, advice), pins in groups.items():
            label = f"pin {pins[0]} has" if len(pins) == 1 else f"pins {', '.join(pins)} have"
            actions.append(f"{reference} {label} no matching footprint pad. The source import classifies "
                           f"the connection as {connection.replace('_', '-')}. {advice[0].upper() + advice[1:]}.")
    if len(matched) < len(findings):
        actions.append(f"Review the schematic_parity entries in {report_path} for the affected footprints and nets.")


def explain_footprint_edges(row, findings, returned):
    """Identify pad-to-cutout errors within verified donor footprints by native UUID."""
    edges = [item for item in findings if item["type"] == "copper_edge_clearance"
             and item["severity"] == "error" and not native_is_excluded(item)]
    if not edges:
        return
    owners = collections.defaultdict(list)
    for footprint in returned.GetFootprints():
        for graphic in footprint.GraphicalItems():
            if graphic.GetLayerName() == "Edge.Cuts" and graphic.GetClass() == "PCB_SHAPE":
                owners[graphic.m_Uuid.AsString()].append((footprint.GetReference(), "edge"))
        for pad in footprint.Pads():
            owners[pad.m_Uuid.AsString()].append((footprint.GetReference(), "pad"))
    grouped = collections.Counter()
    for finding in edges:
        items = finding.get("items")
        if not isinstance(items, list) or len(items) != 2:
            continue
        matches = [owners.get(item["uuid"], []) for item in items
                   if isinstance(item, dict) and isinstance(item.get("uuid"), str)]
        if len(matches) != 2 or any(len(match) != 1 for match in matches):
            continue
        (ref_a, kind_a), (ref_b, kind_b) = (match[0] for match in matches)
        if ref_a == ref_b and {kind_a, kind_b} == {"edge", "pad"}:
            grouped[ref_a] += 1
    if grouped:
        row["pad_to_footprint_cutout_errors"] = dict(sorted(grouped.items()))
        affected = ", ".join(f"{ref} ({count})" for ref, count in sorted(grouped.items()))
        row.setdefault("next_actions", []).append(
            f"Review the footprint pads and their own Edge.Cuts shapes at {affected}. "
            "Their geometry matches the supplied donor. Compare KiCad's measured clearances in drc.json "
            "with the original project rules before revising the footprint or mechanical brief."
        )


def check_rule_input(text):
    """Reject visibly broken form structure before deferring to KiCad."""
    source = "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))
    while source.strip():
        source = source.lstrip()
        form = FORM.match(source)
        require(form and form[1] in {"version", "rule"}, "invalid original .kicad_dru form")
        source = source[block_end(source, 0):]


def check_rule_transcript(transcript, log="drc.log"):
    for line in transcript.splitlines():
        if (re.search(r"custom rules?|rule file|\.kicad_dru", line, re.IGNORECASE)
                and re.search(r"\b(?:error|failed|invalid|warning)\b", line, re.IGNORECASE)):
            raise ValueError(f"KiCad reported a custom rule input problem; see {log}")


def verify(arguments, schematic, work, layers):
    import mechanical_checks

    """Stage the original project and check the candidate without changing it."""
    started = time.monotonic()
    row = {"name": schematic.stem, "mode": "returned-board", "pass": False,
           "status": "unverified", "stage": "inputs", "layers": layers, "kicad_checked": False}
    project = work / "project"
    board = project / schematic.with_suffix(".kicad_pcb").name
    env = os.environ.copy()
    env.update({"LC_ALL": "C", "XDG_CONFIG_HOME": str(work / "kicad-config")})
    kicad = env.get("PCB_KICAD_CLI") or "kicad-cli"

    def command(label, args):
        process = harness_common.run_group(args, capture_output=True, text=True, timeout=arguments.timeout, env=env)
        transcript = process.stdout + process.stderr
        (work / f"{label}.log").write_text(transcript)
        require(process.returncode == 0, f"{label} exited {process.returncode}; see {label}.log")
        if label in ("drc", "drc-as-saved"):
            check_rule_transcript(transcript, f"{label}.log")
        return process.stdout

    try:
        evidence, files = brief_evidence(arguments, schematic, layers)
        row.update(evidence)
        row["candidate_board"] = record(arguments.candidate_board)
        row["rules"] = {"project_settings": "original" if "project_settings" in row["inputs"] else "KiCad defaults",
                        "custom_rules": "original" if "custom_rules" in row["inputs"] else "absent"}
        stage_project(files, row, project)
        shutil.copyfile(arguments.candidate_board, board)
        require(harness_common.file_sha256(board) == row["candidate_board"]["sha256"],
                "candidate changed while being staged")
        settings = board.with_suffix(".kicad_pro")
        if settings.exists():
            require(isinstance(json.loads(settings.read_text()), dict), "invalid original project settings")
        custom_rules = board.with_suffix(".kicad_dru")
        if custom_rules.exists():
            check_rule_input(custom_rules.read_text())
        brief_source = arguments.constraints.read_bytes()
        require(hashlib.sha256(brief_source).hexdigest() == row["inputs"]["constraints"]["sha256"],
                "placement constraints changed while being staged")
        brief = json.loads(brief_source)
        require(isinstance(brief, dict), "expected placement constraints object")

        row["stage"] = "mechanical"
        items = list(children(board.read_text()))
        check_zone_ids(row, items)
        outline = expected_outline(brief, arguments.outline_board)
        actual = board_outline(items)
        donor_rules = outline_rule_areas.donor_forms(arguments, row, layers)
        if donor_rules:
            row["rule_area_check"] = outline_rule_areas.check(donor_rules, board.read_text(), layers)
        row["outline"] = {"satisfied": actual == outline, "expected_primitives": sum(outline.values()),
                          "actual_primitives": sum(actual.values()), "missing_primitives": sum((outline - actual).values()),
                          "additional_primitives": sum((actual - outline).values())}
        output = command("describe", [str(arguments.binary), "describe-kicad-board", str(project), schematic.stem])
        (work / "describe.json").write_text(output)
        description = json.loads(output)
        require(isinstance(description, dict) and description.get("board_id") == schematic.stem,
                "no description for the staged board")
        copper = description.get("copper_layers")
        require(isinstance(copper, list) and copper and all(isinstance(layer, str) for layer in copper),
                "no copper-layer evidence")
        expected_layers = ["F.Cu", *(f"In{i}.Cu" for i in range(1, layers - 1)), "B.Cu"]
        row["copper_layers"] = {"expected": expected_layers, "actual": copper, "satisfied": copper == expected_layers}
        mechanical_donor = getattr(arguments, "mechanical_donor", None)
        addition_refs = mechanical_checks.additions_for(brief, mechanical_donor)
        edge_refs = mechanical_checks.edges_for(brief)
        row["constraints"] = checks_for({key: value for key, value in brief.items() if key not in {"add", "edge"}},
                                        description, board)
        row["constraints_missed"] = missed_constraints(row["constraints"])
        areas = keepout_checks.areas_for(brief.get("keepout", []), description["outline"]["minimum"])
        if areas:
            try:
                import pcbnew
            except ImportError as error:
                raise ValueError("authored keepouts need KiCad's pcbnew Python bindings for verification") from error

        row["stage"] = "reference_import"
        # Resolve footprints from the original schematic and its explicit
        # geometry input. The returned board supplies no implicit fallback.
        reference = work / "reference"
        with board_donor_arguments(arguments, evidence, work, reference_only=bool(addition_refs)) as donor_args:
            output = command("reference-import", [str(arguments.binary), "import-kicad-netlist",
                             str(project / schematic.name), str(reference), schematic.stem,
                             "--layers", str(layers), *donor_args, *footprint_clearance_arguments(arguments)])
        (work / "reference-import.json").write_text(output)
        imported = json.loads(output)
        require(isinstance(imported, dict) and imported.get("missing") == []
                and type(imported.get("footprints")) is int and imported["footprints"] > 0,
                "the original schematic import has incomplete footprint evidence")
        reference_board = reference / board.name
        check_imported_clearance_policy(imported, evidence)
        if imported.get("footprint_clearance_policy"):
            row["footprint_clearance_policy"] = imported["footprint_clearance_policy"]
        check_imported_stackup(imported, evidence, reference_board)
        if "expected_stackup" in evidence:
            row["stackup_check"] = check_stackup(evidence["expected_stackup"], board.read_text(), reference_board)
        netlist = reference_board.with_suffix(".net")
        require(netlist.is_file() and netlist.stat().st_size > 0, "no netlist from the original schematic")
        reference_items = list(children(reference_board.read_text()))
        if addition_refs:
            # Only the mechanical donor supplies the added references.
            # The reference import never sees the brief's add entries.
            row["stage"] = "mechanical_geometry"
            staged_donor = work / "mechanical-donor.kicad_pcb"
            require(mechanical_donor is not None, "no explicit mechanical donor")
            shutil.copyfile(mechanical_donor, staged_donor)
            require(harness_common.file_sha256(staged_donor) == row["inputs"]["mechanical_donor"]["sha256"],
                    "mechanical donor changed while being staged")
            donor_items = list(children(staged_donor.read_text()))
            for kind, form in donor_items:
                if kind not in {"footprint", "module"}:
                    continue
                reference_property = [re.match(rf'\((?:property\s+"Reference"|fp_text\s+reference)\s+({QUOTED})', field)
                                      for name, field in children(form) if name in {"property", "fp_text"}]
                if any(match and json.loads(match[1]) in addition_refs for match in reference_property):
                    reference_items.append((kind, form))
        expected_cutouts = footprint_cutouts(reference_items)
        if "footprint_clearance_policy" in evidence:
            row["footprint_clearance_check"] = check_footprint_clearances(reference_items, items)
        actual_cutouts = footprint_cutouts(items)
        different_cutouts = [part for part in sorted(expected_cutouts.keys() | actual_cutouts.keys())
                             if expected_cutouts.get(part) != actual_cutouts.get(part)]
        row["footprint_cutouts"] = {"satisfied": not different_cutouts,
                                    "expected_parts": sorted(expected_cutouts), "actual_parts": sorted(actual_cutouts),
                                    "different_parts": different_cutouts}
        module_groups = brief.get("module_nonoverlap", [])
        has_donor = getattr(arguments, "footprints_from", None) is not None
        if has_donor or module_groups or addition_refs or edge_refs:
            row["stage"] = "footprint_geometry"
            try:
                import pcbnew
            except ImportError as error:
                raise ValueError("footprint donor verification needs KiCad's pcbnew Python bindings") from error
            expected_board = pcbnew.LoadBoard(str(reference_board))
            returned = pcbnew.LoadBoard(str(board))
            require(expected_board is not None and returned is not None, "cannot read the imported and returned footprint geometry")
            if addition_refs:
                donor = pcbnew.LoadBoard(str(staged_donor))
                require(donor is not None, "cannot read the staged mechanical donor")
                require(addition_refs.isdisjoint(mechanical_checks.by_reference(expected_board)),
                        "add references conflict with the imported schematic")
                row["constraints"].extend(mechanical_checks.addition_checks(pcbnew, donor, returned, addition_refs))
            if edge_refs:
                row["constraints"].extend(mechanical_checks.edge_checks(pcbnew, expected_board, returned, edge_refs))
            options = {"electrical": False, "module_groups": module_groups,
                       "modules_only": not (has_donor or addition_refs or edge_refs)}
            expected = normalized_footprints(pcbnew, expected_board, **options)
            actual = normalized_footprints(pcbnew, returned, **options)
            # Explicit additions are measured against their separate donor
            # above. All other footprints still go through the schematic
            # model comparison.
            for ref in addition_refs:
                actual.pop(ref, None)
            differences = footprint_differences(expected, actual)
            row["footprint_geometry"] = {
                "satisfied": not differences,
                "reference_parts": sum(len(footprint_occurrences(signature)) for signature in expected.values()),
                "candidate_parts": sum(len(footprint_occurrences(signature)) for signature in actual.values()),
                "reference_board": record(reference_board),
                "model": "Padstacks and geometric shapes on Fab, CrtYd, copper, mask, paste and declared module contour layers in the local footprint frame. "
                         "Pad centres and graphic coordinates are rounded to 10 nm. Pad copper layers follow the active stack. "
                         "Native schematic parity checks net assignments separately.",
                "differences": differences,
            }

        row["stage"] = "verification"
        drc_path = work / "drc.json"
        drc_args = [kicad, "pcb", "drc", "--format", "json", "--severity-all", "--schematic-parity", "--all-track-errors"]
        has_zones = any(head == "zone" for head, _ in items) or any(
            form[1] == "zone" for head, block in items if head in ("footprint", "module")
            for form in FORM.finditer(block))
        if has_zones:
            stored_path = work / "drc-as-saved.json"
            transcript = command("drc-as-saved", [*drc_args, "--output", str(stored_path), str(board)])
            row["as_saved"] = native_counts(harness_common.read_json(stored_path), board.name, transcript)
            row["as_saved"]["drc_report"] = stored_path.name
        drc_board = board
        save_refill = any(area["copper"] for area in areas)
        if has_zones or save_refill:
            drc_args.append("--refill-zones")
        if save_refill:
            # Measure both submitted copper and what the current project would
            # pour, keeping the submitted project and board byte-identical.
            filled = work / "filled"
            stage_project(files, row, filled)
            drc_board = filled / board.name
            shutil.copyfile(board, drc_board)
            drc_args.append("--save-board")
        transcript = command("drc", [*drc_args, "--output", str(drc_path), str(drc_board)])
        if drc_board != board:
            require("Saved board" in transcript.splitlines(), "KiCad did not confirm saving the refilled copper")
        require(harness_common.file_sha256(board) == row["candidate_board"]["sha256"],
                "verification changed the staged candidate board")
        drc = harness_common.read_json(drc_path)
        row.update(native_counts(drc, board.name, transcript))
        explain_parity(row, drc, imported, drc_path.name)
        if has_donor and row.get("footprint_geometry", {}).get("satisfied") and row["footprint_cutouts"]["satisfied"]:
            explain_footprint_edges(row, drc["violations"], returned)
        row["drc_report"] = "drc.json"
        stored = row.get("as_saved", row)
        stored_clean = all(stored[field] == 0 for field in ("drc_errors", "unconnected", "parity"))
        if areas:
            counts = collections.Counter(part["reference"] for part in description["footprints"])
            held = {check["part"] for check in row["constraints"] if check["kind"] == "place"
                    and check["satisfied"] and counts[check["part"]] == 1}
            row["constraints"].extend(keepout_checks.measure(
                pcbnew, board, areas, description, held, drc_board if drc_board != board else None))
            row["keepout_model"] = ("Full modeled footprint bodies without placer spacing margins. "
                                    "Only authored exact poses can permit a held courtyard overlap. "
                                    "Copper checks cover tracks, vias and both stored and refilled pours.")
        row["constraints_missed"] = missed_constraints(row["constraints"])
        row["pass"] = (not row.get("duplicate_zone_uuids") and row["outline"]["satisfied"] and row["footprint_cutouts"]["satisfied"]
                       and row.get("rule_area_check", {}).get("satisfied", True)
                       and row.get("stackup_check", {}).get("satisfied", True)
                       and row.get("footprint_geometry", {}).get("satisfied", True)
                       and row.get("footprint_clearance_check", {}).get("satisfied", True)
                       and row["copper_layers"]["satisfied"]
                       and not row["constraints_missed"] and stored_clean and row["drc_errors"] == 0
                       and row["unconnected"] == 0 and row["parity"] == 0)
        indeterminate = any(check.get("indeterminate_parts") and not check["encroaching_parts"]
                            and not check["copper_intrusions"] for check in row["constraints_missed"])
        row["status"] = "accepted" if row["pass"] else "unverified" if indeterminate else "rejected"
        if indeterminate:
            row["error"] = "keepout geometry needs review; see constraints_missed"
        if row["pass"]:
            row["stage"] = "complete"
        elif row["status"] == "rejected":
            if not row.get("footprint_clearance_check", {}).get("satisfied", True):
                row.setdefault("next_actions", []).append(
                    "Restore the supplied footprint clearance settings listed in footprint_clearance_check and rerun KiCad DRC.")
            if not row.get("stackup_check", {}).get("satisfied", True):
                row.setdefault("next_actions", []).append(
                    "Restore the authored board thickness and layer stack from --stackup-from, save the PCB, and resubmit it.")
            if not stored_clean and row["unconnected"] == row["drc_errors"] == row["parity"] == 0:
                row.setdefault("next_actions", []).append(
                    "Refill the zones using the original project settings, save the PCB, and resubmit it. "
                    "KiCad found issues in the submitted copper that cleared after refill."
                )
            retry = pad_near_retry(row["constraints"], row["constraints_missed"], description["footprints"])
            if retry is not None:
                row["placement_retry"], action = retry
                row.setdefault("next_actions", []).append(action)
    except subprocess.TimeoutExpired:
        row["error"] = f"verification step timed out after {arguments.timeout} s"
    except (ValueError, ArithmeticError, TypeError, OSError, RuntimeError) as error:
        row["error"] = str(error)
    row["seconds"] = round(time.monotonic() - started)
    return row
