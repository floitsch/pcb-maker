# ECC83 real-board cold-source adapter

Date: 2026-09-03. Promoted adapter run: sequence 221. Promoted paired run:
sequence 224.

## Question

Can a pinned, independently maintained KiCad project enter the route-only
benchmark without inheriting the human route, changing placement, or hiding
native findings?

The fixture is KiCad's GPL-licensed `ecc83-pp` demo at pinned commit
`0feeca2a807f428ad2b3fa7c1e39625cb769f02c`. The archive and required files
are hash-verified by `benchmarks/real/fetch.sh`. The competitive geometry is
copied from the project's Default net class: 0.8 mm track, 0.4 mm clearance,
and 1.2/0.6 mm via.

## Adapter and admission contract

`cold_kicad_project` copies the complete project and structurally removes
top-level track segments, arcs, vias, and copper zones. It retains footprints,
pads, net declarations, outline, graphics, project rules, libraries, and
non-copper rule-area keepouts. The strip report records before/after board
statistics and checks both exact and 100 nm placement digests.

Real upstream projects can contain native warnings unrelated to routing. The
route-only admission therefore runs KiCad on an isolated copy of the cold
source and on the result. Result connectivity must be zero. Every ERC, DRC,
and schematic-parity finding must either be absent or match a frozen-source
finding after ignoring only issue UUIDs and report order. New or moved
geometry remains a new finding. Raw absolute verification is retained next to
this differential admission and is never rewritten to look clean.

## Retained iterations

- Sequence 219 proved the adapter and routing path: 20 initial unrouted items
  became zero, but the old absolute admission rejected two upstream
  silkscreen-edge warnings and six upstream footprint-library alias warnings.
- Sequence 220 introduced baseline-aware route admission and completed, but
  review exposed that its baseline check wrote reports into `source/` after
  the recorded directory hash. The board hash was valid; directory provenance
  was not immutable.
- Sequence 221 runs baseline verification in
  `source-baseline-verification/`. The reported `source/` remains unchanged,
  so this is the promoted result.

## Sequence 221 result

The adapter removed 59 segments and one B.Cu GND zone, with no arcs or vias.
It retained 15 footprints and 13 named nets. The exact component-placement
digest before and after is
`bb1aa7947f5f5d1164d4cc2eb35fb44b1147790f71cce51ba7e06049938d7722`.

Freerouting reports 20 initial and zero final unrouted items after two passes
and 3.499 seconds in its process. The imported board has 51 segments, zero
vias, 254.225 mm physical centerline, and uses both copper layers. KiCad
reports zero result unconnected items and zero introduced ERC, DRC, or parity
findings. The two silkscreen and six alias warnings are identical in source
and result and remain visible in both raw reports.

Front/back inspection confirms a pad-only cold source and a via-free result.
Through-hole pads supply the legal layer transitions. No inherited plane
fragment or via cluster is visible.

## Direct-router experiment and formal comparison

Ordinary KiCad numeric net declarations and name-addressed routed objects now
lower through one normalized connection inventory. The direct sequential
runner discovers every named net with at least two distinct pad centers,
routes a bounded per-net portfolio against the accumulating board, checkpoints
after every committed net, and never edits its frozen source. This is a
general full-connectivity route mode; it does not translate the ECC83
schematic into a project-specific semantic fixture.

Sequence 222 retains the first one-net probe. Sequence 223 routes all nine
multi-pad nets (20 native branch items) in 0.78 seconds of router-process time
and 523,627 A* expansions. Its result has 75 segments, 10 vias, and 254.670 mm
of canonical physical centerline. KiCad reports zero unconnected items and no
introduced native finding.

Sequence 224 promotes this into the one-command paired harness. Freerouting
and pcb-maker consume the same adapted board hash and component placement at
the 0.0001 mm exchange grid, use identical 0.8/0.4/1.2/0.6 mm geometry, and
both pass differential native admission. Freerouting uses 51 segments, zero
vias, and 254.225 mm physical centerline; pcb-maker uses 75 segments, 10 vias,
and 254.670 mm. Completion is tied. The 0.445 mm length gap is negligible;
the material deficit is pcb-maker's unnecessary layer changes and excess
segmentation. Front/back inspection finds dispersed vias rather than the old
connected-via clusters, but the topology is visibly less economical.

This result closes the ordinary-KiCad route-mode gap for a small fixture; it
does not establish corpus parity. The next adapter cases must test whether the
same generic path handles larger pad counts, nontrivial outlines, zones/rule
areas, and failures that require route ordering or rip-up rather than a single
greedy pass.

Authoritative report:
`build/sequence-224-m0-ecc83-formal-comparison/competitive-comparison.json`.
