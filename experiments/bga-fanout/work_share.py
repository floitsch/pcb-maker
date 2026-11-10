#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""The share of a run's search work charged to a grid array's nets
(docs/router.md, "Prepared fan-outs for BGAs").

    work_share.py <escapes.json entry key> <network.json>...

`network.json` is what `PCB_ROUTER_NET_WORK=<file>` wrote (search
expansions per net name over the process); the array's nets and their
rings come from census.py's `escapes.json` (the designer's side). Prints
per file the total, the array's nets' share and its inner rings' (ring 2
or more) share, and the work per net of each group against the others.
"""

import json
import sys


def main():
    escapes, key = sys.argv[1], sys.argv[2]
    balls = json.load(open(escapes))[key]["designer"]["balls"]
    strip = lambda name: name.lstrip("/")  # noqa: E731
    nets = {strip(ball["net"]) for ball in balls}
    inner = {strip(ball["net"]) for ball in balls if ball["ring"] >= 2}
    for path in sys.argv[3:]:
        work = {strip(name): value for name, value in json.load(open(path)).items()}
        total = sum(work.values())
        on = sum(value for name, value in work.items() if name in nets)
        deep = sum(value for name, value in work.items() if name in inner)
        others = len(work) - len(nets & set(work))
        print(
            f"{path}: {total:.3e} expansions over {len(work)} nets; the array's {len(nets & set(work))} nets "
            f"{on / total:.1%} ({on / total / max(1, len(nets & set(work))):.2%} a net), its inner rings' "
            f"{len(inner & set(work))} {deep / total:.1%} ({deep / total / max(1, len(inner & set(work))):.2%} a net), "
            f"the other nets {(total - on) / total / max(1, others):.2%} a net"
        )


if __name__ == "__main__":
    main()
