# Whole-board routing and area objective

The project goal is complete autonomous placement/routing on real boards,
followed by smaller complete boards. Partial synthetic prefixes and local
improvements are diagnostic evidence, not the main success metric.

The new [benchmark contract and commands](../../experiments/whole-board/README.md)
define fixed-placement cold routing, cold placement with an area objective,
separate reference-seeded compaction, source-rule coverage and external-router
comparison. [The manifest](../../benchmarks/real/whole-board.json) keeps all five
existing pinned upstream projects visible, including unsupported and untested
cases. No additional source was needed for this first ladder; all five existing
downloads were checksum-verified again.

## Reference audit

All five original routed boards have zero native unconnected items in fresh
project copies. Only PIC passes every absolute native check as imported; the
others retain their native ERC/design/parity findings. These findings must be
resolved or documented without silently weakening the routing problem.

| Source | Footprints | Outline area mm² | Human track mm | Vias | Copper zones |
| --- | ---: | ---: | ---: | ---: | ---: |
| ECC83 | 15 | 2413.705 | 210.998 | 0 | 1 |
| PIC programmer | 63 | 15851.581 | 1745.703 | 6 | 1 |
| Complex hierarchy | 68 | 8057.413 | 1265.687 | 0 | 166 |
| Interf_u | 25 | 12191.589 | 5101.459 | 84 | 1 |
| Olimex ESP32-C3 Rev C | 61 | 1064.514 | 1068.750 | 88 | 160 |

Track length excludes connectivity through pours. A shorter human track total
with a ground plane is not directly comparable to a plane-free routing total.

## Current complete-netlist comparisons

The initial suite retained the existing 60-second manifests. ECC83's pcb-maker
process timed out after two of nine nets with 12 native opens; Freerouting
completed it. A second explicitly 600-second-per-competitor capability run
completed all nine nets with both tools:

| ECC83 capability run | Native opens | Track mm | Vias | New findings |
| --- | ---: | ---: | ---: | ---: |
| pcb-maker | 0 | 254.205886 | 0 | 0 |
| Freerouting | 0 | 254.224521 | 0 | 0 |

Placement and routing rules match. Both results retain the source's two
silkscreen findings and six footprint-link parity findings, so this is native
route admission with no regression, not an absolutely clean source project.
The second pcb-maker process actually finishes in 43.561 seconds, with 184121
A* expansions; Freerouting's process takes 7.816 seconds. pcb-maker's process
includes per-net native checks. The second run follows the first on a warmed
host/cache, so neither the larger cap nor the runtime difference establishes
an isolated performance result. The 0.019 mm route-length difference is too
small to imply meaningful superiority.

The PIC manifest reaches one of 34 nets/114 remaining native items with
pcb-maker and one remaining native item with Freerouting. pcb-maker finds a
GND candidate, but native admission rejects a dangling 0.5 mm GND track near
(113.91, 85.89) on the bottom layer. This is a geometry/materialization blocker,
not a process timeout or an exhausted search budget. The run retains the failed
candidate and native preview. The manifest still flattens the POWER class to
Default; neither result establishes source-rule completion.

## Area probes and the missing placement representation

Nine full-netlist ECC83 proposals use three independent random-center seeds and
100%, 90%, and 80% area. All fail harmonic legalization. Their inputs and
failures are rendered automatically. Original orientations remain fixed, so
these are translation probes rather than a complete cold-placement evaluation.

A required reference control exposes why these cannot yet be promoted as a
known-feasible placement benchmark: conservative graphics/pad rectangles reject
the working human placement. They report P4 outside the board and overlaps at
C2/U1, P1/U1, P2/R1 and P3/R1. The round socket's rectangular envelope occupies
usable corner space, and the connector overhang needs explicit semantics.
Consequently no area score is admitted, and no smaller routed board is claimed.
The optional legal-placement-to-native-routing branch remains unexercised.

The next placement work is to represent these bodies and legitimate overhang
correctly, preserve the human reference as a positive control, and add mechanical
and electrical constraint sidecars. Only then should legalization failures drive
placer tuning. After that, route both tools on each proposed complete placement
and lower the smallest verified complete area. This replaces another round of
surrogate-energy optimization with a measurable product objective.

## Artifacts

- [All five human references and initial paired runs](../../build/whole-board-reference-2026-09-07/index.html).
- [ECC83 complete capability comparison](../../build/whole-board-ecc83-600s-2026-09-07/index.html).
- [Area seed 0](../../build/whole-board-area-2026-09-07/index.html),
  [seed 1 with reference control](../../build/whole-board-area-seed1-2026-09-07/index.html),
  [seed 2](../../build/whole-board-area-seed2-2026-09-07/index.html).

All runner sessions reached terminal exit status. No production router or
placer code changed in this study. The new runner and area diagnostic are
experiment tools, and the area diagnostic is explicitly not a promoted
benchmark. Existing local tests remain useful only insofar as they help close
these whole-board gaps.
