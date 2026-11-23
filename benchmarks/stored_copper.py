# Copyright (C) 2026 Florian Loitsch.

"""Native connectivity evidence for the actual KiCad PCB on disk."""

import os
import subprocess
import tempfile
from pathlib import Path

import harness_common


def counts(drc, board, kicad_version):
    if not isinstance(drc, dict):
        return None
    source = drc.get("source")
    severities = drc.get("included_severities")
    ignored = drc.get("ignored_checks")
    essential = {"unconnected_items", "net_conflict", "missing_footprint"}
    if (not isinstance(source, str) or Path(source).name != board.name
        or not isinstance(kicad_version, str) or not kicad_version or drc.get("kicad_version") != kicad_version
        or not isinstance(severities, list)
        or not all(level in severities for level in ("error", "warning", "exclusion"))
        or not isinstance(ignored, list)
        or any(not isinstance(check, dict) or not isinstance(check.get("key"), str)
               or check["key"] in essential for check in ignored)):
        return None
    fields = [drc.get(key) for key in ("unconnected_items", "schematic_parity", "violations")]
    if not all(isinstance(items, list) and all(isinstance(item, dict) for item in items) for items in fields):
        return None
    return {"unconnected": len(fields[0]), "parity": len(fields[1]),
            "errors": sum(item.get("severity") == "error" for item in fields[2])}


def verified_fill(board, publication, kicad_version):
    """Reuse the layout's recorded check only if the saved PCB still matches it."""
    if not isinstance(publication, dict) or publication.get("status") != "saved":
        return None
    try:
        board_digest = harness_common.file_sha256(board)
    except OSError:
        return None
    if publication.get("saved_board_sha256") != board_digest:
        return None
    verified = publication.get("saved_as_stored")
    drc = harness_common.read_json(board.with_name("drc-saved-fill.json"))
    actual = counts(drc, board, kicad_version)
    fields = (("opens", "unconnected"), ("schematic_parity", "parity"), ("errors", "errors"))
    if (actual is None or not isinstance(verified, dict) or verified.get("possibly_capped_types")
        or any(type(verified.get(key)) is not int or verified[key] != actual[field] for key, field in fields)):
        return None
    return {**actual, "source": "verified_fill_publication", "board_sha256": board_digest}


def check(board, kicad_version, timeout=300):
    """Run all-track native DRC without refill and ensure it saw these PCB bytes."""
    board = board.resolve()
    try:
        board_digest = harness_common.file_sha256(board)
        with tempfile.TemporaryDirectory(prefix="pcb-maker-stored-drc-") as scratch:
            report = Path(scratch) / "drc.json"
            kicad = os.environ.get("PCB_KICAD_CLI") or "kicad-cli"
            complete = harness_common.run_group(
                [kicad, "pcb", "drc", "--format", "json", "--severity-all", "--all-track-errors",
                 "--schematic-parity", "--output", str(report), str(board)],
                timeout=timeout, capture_output=True, env=dict(os.environ, LC_ALL="C"))
            if complete.returncode != 0:
                return None, None
            drc = harness_common.read_json(report)
        actual = counts(drc, board, kicad_version)
        if actual is not None and harness_common.file_sha256(board) == board_digest:
            return {**actual, "source": "fresh_as_stored_drc", "board_sha256": board_digest}, drc
    except (OSError, ValueError, subprocess.TimeoutExpired):
        pass
    return None, None
