# Copyright (C) 2026 Florian Loitsch.
"""Entry point appended to the shared routed supply measurement."""

if __name__ == "__main__":
    import json
    from pathlib import Path
    import sys

    import pcbnew

    board_path, request_path, output, version = sys.argv[1:]
    if pcbnew.GetBuildVersion().strip() != version:
        raise ValueError("KiCad CLI and PCB bindings have different versions")
    board = pcbnew.LoadBoard(board_path)
    if board is None:
        raise ValueError("cannot load the saved PCB")
    request = json.loads(Path(request_path).read_text())
    limit = request.get("max_tracks_per_net", 512)
    if type(limit) is not int or limit < 1:
        raise ValueError("max_tracks_per_net must be a positive integer")
    if "pairs" in request:
        paths = measure_supply_paths(pcbnew, board, request["pairs"], max_tracks_per_net=limit)
    else:
        paths = measure_near_paths(pcbnew, board, request["near"], only_net=request.get("net"),
                                   max_tracks_per_net=limit)
    Path(output).write_text(json.dumps({"paths": paths}), encoding="utf-8")
