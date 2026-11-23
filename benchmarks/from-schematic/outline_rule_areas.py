# Copyright (C) 2026 Florian Loitsch.
"""Transfer and compare only board-level polygon keepouts from an outline donor."""

from collections import Counter
from decimal import Decimal, ROUND_HALF_UP
import hashlib
import json
import re

FLAGS = {"tracks", "vias", "pads", "copperpour", "footprints"}
ITEM_ID = re.compile(r'"(?:[^"\\]|\\.)*"|\((?:uuid|tstamp)\s+"?([0-9A-Za-z-]+)"?\s*\)')
AREA_NAME = re.compile(r'\(name\s+("(?:[^"\\]|\\.)*")\s*\)')


def _declared_layers(items):
    from returned_board import SEXPR_TOKEN, children, stackup_atom

    for key, form in items:
        if key == "layers":
            return {stackup_atom(tokens[2])
                    for _, entry in children(form)
                    if len(tokens := SEXPR_TOKEN.findall(entry)) >= 5}
    return set()


def _layers(fields, count, declared):
    active = {"F.Cu", "B.Cu", *(f"In{i}.Cu" for i in range(1, count - 1))}
    form = fields.get("layers") or fields.get("layer")
    if "layer" in fields and "layers" in fields:
        raise ValueError("board rule area declares its layers twice")
    if form is None:
        raise ValueError("board rule area lacks PCB layers")
    values = re.fullmatch(r'\(layers?\s+((?:"[^"]+"|[^\s()"]+)(?:\s+(?:"[^"]+"|[^\s()"]+))*)\s*\)', form)
    if not values:
        raise ValueError("unsupported board rule area layer expression")
    names = {part.strip('"') for part in re.findall(r'"[^"]+"|[^\s]+', values[1])}
    if "*.Cu" in names:
        names = (names - {"*.Cu"}) | active
    if "F&B.Cu" in names:
        names = (names - {"F&B.Cu"}) | {"F.Cu", "B.Cu"}
    extra_copper = {name for name in names - active if name.endswith(".Cu")}
    if extra_copper:
        raise ValueError(f"board rule area names copper layers outside the target stack: {sorted(extra_copper)}")
    if declared is not None and (undeclared := names - active - declared):
        raise ValueError(f"board rule area uses unavailable PCB layers: {sorted(undeclared)}")
    return tuple(sorted(names))


def _paths(polygons):
    from returned_board import POINT, children

    vertices = []
    for polygon in polygons:
        points = list(children(polygon))
        if len(points) != 1 or points[0][0] != "pts":
            raise ValueError("unsupported board rule area polygon")
        ring = []
        for head, form in children(points[0][1]):
            point = re.fullmatch(rf'\(xy\s+{POINT}\s*\)', form) if head == "xy" else None
            if not point:
                raise ValueError("board rule areas need explicit polygon vertices")
            nm = [(Decimal(value) * 10**6).to_integral_value(rounding=ROUND_HALF_UP)
                  for value in point.groups()]
            if not all(-(2**31) <= value < 2**31 for value in nm):
                raise ValueError("board rule area vertex exceeds KiCad's coordinate range")
            ring.append(tuple(map(int, nm)))
        if len(ring) < 3 or len(set(ring)) < 3:
            raise ValueError("empty or degenerate board rule area polygon")
        edges = tuple(sorted(tuple(sorted((a, b))) for a, b in zip(ring, ring[1:] + ring[:1]) if a != b))
        vertices.append(edges)
    if not vertices:
        raise ValueError("board rule area lacks a polygon")
    return tuple(sorted(vertices))


def _signature(zone, count, declared):
    from returned_board import children

    items = list(children(zone))
    polygons = [item for key, item in items if key == "polygon"]
    fields = {key: item for key, item in items if key != "polygon"}
    if "keepout" not in fields:
        return None
    if len(fields) + len(polygons) != len(items):
        raise ValueError("ambiguous board rule area fields")
    restrictions = []
    for key, item in children(fields["keepout"]):
        match = re.fullmatch(r'\([a-z]+\s+(allowed|not_allowed)\s*\)', item)
        if key not in FLAGS or not match:
            raise ValueError(f"unsupported board rule area flag {key}")
        restrictions.append((key, match[1]))
    if len({key for key, _ in restrictions}) != len(restrictions):
        raise ValueError("ambiguous board rule area flags")
    name = _name(fields)
    if not name and not any(value == "not_allowed" for _, value in restrictions):
        return None
    if ("filled_polygon" in fields or "fill_segments" in fields
        or not re.fullmatch(r'\(net\s+(?:0|"")\s*\)', fields.get("net", "(net 0)"))
        or not re.fullmatch(r'\(net_name\s+""\s*\)', fields.get("net_name", '(net_name "")'))
        or re.search(r'\(fill\s+yes\b', fields.get("fill", ""))
        or re.search(r'\(enabled\s+yes\b', fields.get("placement", ""))):
        raise ValueError("board rule area also contains copper or a placement assignment")
    return _layers(fields, count, declared), tuple(sorted(restrictions)), _paths(polygons), name


def _name(fields):
    match = AREA_NAME.fullmatch(fields.get("name", '(name "")'))
    if match is None:
        raise ValueError("unsupported board rule area name")
    return json.loads(match[1])


def _ids(text):
    return Counter(match[1].lower() for match in ITEM_ID.finditer(text) if match[1])


def ensure_unique_ids(staged_items, source_areas):
    source = _ids("".join(source_areas))
    extant = _ids(staged_items)
    if any(count != 1 or key in extant for key, count in source.items()):
        raise ValueError("a source outline rule area repeats an existing native item ID")


def collect(text, layers):
    from returned_board import children

    items = list(children(text))
    declared = _declared_layers(items)
    raw, signatures = [], Counter()
    for key, form in items:
        if key == "zone":
            signature = _signature(form, layers, declared)
            if signature is not None:
                raw.append(form)
                signatures[signature] += 1
    return raw, signatures


def record_source(donor, layers):
    raw, signatures = collect(donor, layers)
    if not raw:
        raise ValueError("the outline board has no board-level keepout rule areas")
    digest = hashlib.sha256(json.dumps(sorted(signatures.items())).encode()).hexdigest()
    return {"count": len(raw), "sha256": digest}


def donor_forms(arguments, evidence, layers):
    if not getattr(arguments, "rule_areas_from_outline", False):
        return []
    donor = arguments.outline_board.read_bytes()
    if hashlib.sha256(donor).hexdigest() != evidence["inputs"]["outline_board"]["sha256"]:
        raise ValueError("the outline rule area source changed while being staged")
    text = donor.decode()
    if record_source(text, layers) != evidence["outline_rule_areas"]:
        raise ValueError("the outline rule area geometry changed while being staged")
    return collect(text, layers)[0]


def check(expected, returned, layers):
    from returned_board import children

    # Donor forms were already validated against their source layer table.
    target = Counter(_signature(form, layers, None) for form in expected)
    names = {item[-1] for item in target if item[-1]}
    actual = Counter()
    collisions = unexpected_named = 0
    ids = _ids(returned)
    items = list(children(returned))
    declared = _declared_layers(items)
    for key, form in items:
        if key != "zone":
            continue
        fields = dict(children(form))
        if "keepout" not in fields:
            continue
        try:
            name = _name(fields)
        except ValueError:
            continue
        if name and name not in names:
            continue
        try:
            signature = _signature(form, layers, declared)
        except (ValueError, ArithmeticError):
            if name in names:
                unexpected_named += 1
            continue
        if signature not in target:
            if name in names:
                unexpected_named += 1
            continue
        actual[signature] += 1
        if any(ids[item_id] != 1 for item_id in _ids(form)):
            collisions += 1
    missing = (target - actual).total()
    unexpected_named += sum(amount for item, amount in (actual - target).items() if item[-1])
    return {"satisfied": not any((missing, unexpected_named, collisions)), "expected": target.total(),
            "missing_or_changed": missing, "unexpected_named": unexpected_named, "duplicate_item_ids": collisions}
