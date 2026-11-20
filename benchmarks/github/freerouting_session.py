# Copyright (C) 2026 Florian Loitsch.

"""Keep benchmark PCB inputs intact when attaching native Specctra session copper."""

import re
import shutil
import tempfile
from decimal import Decimal
from pathlib import Path


COPPER = {"segment", "arc", "via"}
ATOMS = re.compile(r'"(?:[^"\\]|\\.)*"|[^\s()"]+')
FORMS = re.compile(r'"(?:[^"\\]|\\.)*"|[()]')
TOKENS = re.compile(r'"(?:[^"\\]|\\.)*"|[()]|[^\s()"]+')
IDENTITY = re.compile(r'\((?:uuid|tstamp)\s+("?[0-9a-fA-F-]{8,36}"?)\s*\)')
LEGACY_VIA_DEFAULTS = {
    ("capping", "no"), ("filling", "no"),
    ("covering", "front", "no", "back", "no"),
    ("plugging", "front", "no", "back", "no"),
}


def direct_forms(text):
    """Read child forms as original spans, skipping parentheses in strings."""
    root = re.match(r"\ufeff?\s*\(\s*([^\s()]+)", text)
    if root is None:
        raise ValueError("expected a KiCad S-expression")
    forms = []
    depth = 1
    for token in FORMS.finditer(text, root.end()):
        if token.group() == "(":
            if depth == 1:
                child = token.start()
            depth += 1
        elif token.group() == ")":
            depth -= 1
            if depth == 1:
                head = re.match(r"\(\s*([^\s()]+)", text[child:token.end()])
                if head is None:
                    raise ValueError("empty child in KiCad expression")
                forms.append((head[1], child, token.end()))
            if depth == 0:
                if text[token.end():].strip():
                    raise ValueError("unexpected content after KiCad expression")
                return root[1], forms, token.start()
    raise ValueError("unclosed KiCad expression")


def unquote(token):
    if token.startswith('"'):
        return re.sub(r"\\(.)", r"\1", token[1:-1])
    return token


def board_forms(text):
    root, children, close = direct_forms(text)
    if root != "kicad_pcb":
        raise ValueError("expected a KiCad PCB")
    return [(head, text[start:end]) for head, start, end in children], close


def single_field(block, field):
    _, children, _ = direct_forms(block)
    matches = [(start, end) for head, start, end in children if head == field]
    if len(matches) != 1:
        raise ValueError(f"native copper needs one {field} form")
    start, end = matches[0]
    tokens = ATOMS.findall(block[start:end])
    if len(tokens) != 2:
        raise ValueError(f"native copper has an unsupported {field} form")
    return start, end, tokens[1]


def native_snapshot(pcbnew, board, path):
    if not pcbnew.SaveBoard(str(path), board, True):
        raise ValueError("KiCad could not serialize a temporary session snapshot")
    forms, _ = board_forms(path.read_bytes().decode("utf-8"))
    static = []
    copper = {}
    for head, body in forms:
        if head in COPPER:
            _, fields, _ = direct_forms(body)
            identifier = "uuid" if any(kind == "uuid" for kind, _, _ in fields) else "tstamp"
            _, _, token = single_field(body, identifier)
            uid = unquote(token).lower()
            if uid in copper:
                raise ValueError(f"duplicate native copper identity {uid}")
            copper[uid] = body
        else:
            static.append((head, body))
    return static, copper


def source_dialect(forms):
    versions = [ATOMS.findall(body)[1:] for head, body in forms if head == "version"]
    if len(versions) != 1 or len(versions[0]) != 1 or not versions[0][0].isdigit():
        raise ValueError("board has no unambiguous KiCad file version")
    version = int(versions[0][0])
    if version < 20211014:
        raise ValueError(f"KiCad PCB version {version} predates the supported session attachment format")
    nets = {}
    codes = set()
    for head, body in forms:
        if head != "net":
            continue
        tokens = ATOMS.findall(body)
        if len(tokens) != 3 or not tokens[1].isdigit():
            raise ValueError("unsupported board net table")
        code, name = int(tokens[1]), unquote(tokens[2])
        if code in codes or name in nets:
            raise ValueError(f"ambiguous board net table entry {name!r}")
        nets[name] = code
        codes.add(code)
    return version, nets


def source_copper(block, version, numbered_nets, known_nets):
    if version < 20260206:
        block = omit_legacy_via_defaults(block)
    head, fields, _ = direct_forms(block)
    start, end, token = single_field(block, "net")
    name = unquote(token)
    if not token.startswith('"') or not name or name not in known_nets:
        raise ValueError(f"session {head} references an undeclared or unnamed net {name!r}")
    changes = []
    if numbered_nets:
        if not numbered_nets.get(name):
            raise ValueError(f"session {head} references {name!r}, missing from the original numbered net table")
        changes.append((start, end, f"(net {numbered_nets[name]})"))
    if version < 20260206:
        # The ordinary native session geometry works in older boards. Do
        # not downgrade custom padstacks, masks, or other new copper fields.
        standard = {"start", "mid", "end", "at", "width", "size", "drill", "layer", "layers",
                    "net", "uuid", "tstamp", "locked"}
        unknown = {kind for kind, _, _ in fields} - standard
        if unknown:
            raise ValueError(f"legacy board cannot take session {head} fields {sorted(unknown)}")
        for kind, start, end in fields:
            if kind == "uuid" and version < 20240108:
                changes.append((start + 1, start + 5, "tstamp"))
            if kind == "locked" and version < 20241229:
                values = ATOMS.findall(block[start:end])
                if values == ["locked", "yes"]:
                    changes.append((start, end, "(locked)"))
                elif values == ["locked", "no"]:
                    changes.append((start, end, ""))
    for start, end, replacement in sorted(changes, reverse=True):
        block = block[:start] + replacement + block[end:]
    return block


def omit_legacy_via_defaults(block):
    head, fields, _ = direct_forms(block)
    if head == "via":
        for _, start, end in reversed(fields):
            if tuple(ATOMS.findall(block[start:end])) in LEGACY_VIA_DEFAULTS:
                block = block[:start] + block[end:]
    return block


def only_export_rounding(before, after, dsn):
    if before == after:
        return True
    if dsn is None or len(before) != len(after):
        return False
    # The native DSN export gives positions on this grid. Only accept the
    # nearest on-grid coordinates, preserving exact source origins in the PCB.
    sizes = {"um": "0.001", "mm": "1", "mil": "0.0254", "inch": "25.4"}
    settings = re.findall(r"(?m)^\s*\(resolution\s+(um|mm|mil|inch)\s+(\d+)\s*\)",
                          Path(dsn).read_text(encoding="utf-8"))
    if len(settings) != 1 or int(settings[0][1]) == 0:
        return False
    unit, resolution = settings[0]
    grid = Decimal(sizes[unit]) / int(resolution)
    for (kind, left), (other_kind, right) in zip(before, after):
        if (kind, left) == (other_kind, right):
            continue
        if kind != "footprint" or other_kind != kind:
            return False
        positions = []
        without_at = []
        for body in (left, right):
            _, forms, _ = direct_forms(body)
            matches = [(start, end) for field, start, end in forms if field == "at"]
            if len(matches) != 1:
                return False
            start, end = matches[0]
            at = ATOMS.findall(body[start:end])
            positions.append((at[1:3], at[3:]))
            without_at.append(body[:start] + body[end:])
        (old, old_rotation), (new, new_rotation) = positions
        if without_at[0] != without_at[1] or old_rotation != new_rotation:
            return False
        for source, imported in zip(map(Decimal, old), map(Decimal, new)):
            if source != imported and (imported % grid != 0 or abs(imported - source) > grid / 2):
                return False
    return True


def static_identity(static, explicit_ids):
    # KiCad can create a fresh UUID when an old input pad or an implicit
    # footprint field has none. Check every explicitly authored identity
    # and compare the remaining native content without those fresh UUIDs.
    def published_id(match):
        return match[0] if unquote(match[1]).lower() in explicit_ids else "(uuid generated)"

    return [(head, IDENTITY.sub(published_id, body)) for head, body in static]


def import_session(destination, session, dsn=None):
    """Attach native imported copper atomically and return via count and track length."""
    import pcbnew

    destination = Path(destination)
    source = destination.read_bytes().decode("utf-8")
    forms, close = board_forms(source)
    version, nets = source_dialect(forms)
    explicit_ids = {unquote(token).lower() for token in IDENTITY.findall(source)}
    explicit_ids |= {"00000000-0000-0000-0000-0000" + token
                     for token in explicit_ids if len(token) == 8}
    # The benchmark directory already holds a full copy of the source
    # project. All native SaveBoard calls go to a private temporary directory.
    with tempfile.TemporaryDirectory(prefix=".session-", dir=destination.parent) as directory:
        work = Path(directory)
        board = pcbnew.LoadBoard(str(destination))
        known_nets = {net.GetNetname() for _, net in board.GetNetsByNetcode().items()}
        before_static, before_copper = native_snapshot(pcbnew, board, work / "before.kicad_pcb")
        if not pcbnew.ImportSpecctraSES(board, str(session)):
            raise ValueError("KiCad could not import the Specctra session")
        after_static, after_copper = native_snapshot(pcbnew, board, work / "imported.kicad_pcb")
        if not only_export_rounding(before_static, after_static, dsn):
            raise ValueError("session import changed the placement, nets, or other non-routing board content")
        if any(after_copper.get(uid) != block for uid, block in before_copper.items()):
            raise ValueError("session import removed or changed existing copper")
        new_ids = after_copper.keys() - before_copper.keys()
        if new_ids & explicit_ids:
            raise ValueError("session copper reuses an original board UUID")

        additions = [source_copper(block, version, nets, known_nets)
                     for uid, block in after_copper.items() if uid not in before_copper]
        candidate = work / destination.name
        for suffix in (".kicad_pro", ".kicad_dru"):
            companion = destination.with_suffix(suffix)
            if companion.is_file():
                shutil.copy2(companion, candidate.with_suffix(suffix))
        if additions:
            newline = "\r\n" if "\r\n" in source else "\n"
            insertion = "".join("  " + block.replace("\n", newline) + newline for block in additions)
            merged = source[:close] + newline + insertion + source[close:]
        else:
            merged = source
        candidate.write_bytes(merged.encode("utf-8"))
        shutil.copymode(destination, candidate)
        reopened = pcbnew.LoadBoard(str(candidate))
        reopened_static, reopened_copper = native_snapshot(pcbnew, reopened, work / "reopened.kicad_pcb")
        if static_identity(reopened_static, explicit_ids) != static_identity(before_static, explicit_ids):
            raise ValueError("reopened PCB did not reproduce the original non-routing board content")
        def comparable(copper):
            return {uid: TOKENS.findall(omit_legacy_via_defaults(block) if version < 20260206 else block)
                    for uid, block in copper.items()}
        if comparable(reopened_copper) != comparable(after_copper):
            raise ValueError("reopened PCB did not reproduce native imported copper")

        # Index access also works with SWIG builds lacking container iteration.
        tracks = reopened.Tracks()
        items = [tracks[index] for index in range(tracks.size())]
        vias = sum(item.GetClass() == "PCB_VIA" for item in items)
        length = sum(item.GetLength() for item in items if item.GetClass() != "PCB_VIA") / 1e6
        candidate.replace(destination)
        return vias, round(length, 1)
