# PIC programmer real-board cold baseline

Date: 2026-09-03. Current frontier: sequence 238.

## Purpose

This is the second pinned ordinary KiCad project exercised through the generic
cold adapter and paired harness. It is intentionally a substantial step beyond
ECC83: 63 footprints, 34 routed nets, and 125 native connection items on a
160.02 x 99.06 mm two-layer board. It tests large multi-terminal power nets,
custom pads, bottom-only SMD pads, split outline edges, and route ordering.

The project defines a 0.5/0.25 mm Default net class and a 0.8/0.28 mm POWER
class for GND/VCC. The current M0 comparison uses a uniform 0.5/0.25 mm rule
with the project's 1.6/0.6 mm via. This is an explicitly reduced default-class
benchmark, not evidence that per-net-class production rules are supported.

## Adapter findings

The board encodes one rectangular side as two collinear `Edge.Cuts` segments.
The generic rectangle recognizer now accepts any gap-free set of axis-aligned
segments on the four perimeter sides, while still rejecting interior or
non-rectangular edges.

Sequence 225 exposed a more serious cold-source defect: top-level labels drawn
on F.Cu survived copper stripping but were absent from the DSN obstacle model.
All five introduced Freerouting DRC findings were track-to-copper-text
collisions. Cold statistics and the semantic copper digest now include
top-level copper graphics, and adapter schema 2 removes all 19 while retaining
silkscreen/fabrication graphics and footprint-local copper.

With that fix, sequence 226 has zero DRC findings but retains one open
connection under a conservative uniform 0.8/0.28 mm rule. Sequence 227 repeats
at the uniform Default geometry. Freerouting routes 124 of 125 items in three
passes and 7.226 seconds of router-process time, with zero introduced native
findings. The remaining item is the bottom-only pad of solder jumper JP1 on
`pic_sockets/VCC_PIC`. Freerouting uses no vias and cannot reach it. This is a
valid incomplete baseline, not a harness failure.

## pcb-maker findings

Sequence 228 was the first direct comparison. The ungated greedy runner reached
six of 34 nets before `DATA-RB7` became unreachable, but final KiCad admission
found three dangling tails. Two revealed that terminal grid states always used
F.Cu even for bottom-only SMD pads; the third was a shared-tree attachment to a
snapped terminal cell that the exact endpoint segment did not actually reach.

Sequential-router schema 2 now:

- preserves bottom-only SMD terminal layers while preferring F.Cu for
  through-hole pads;
- excludes displaced snapped endpoint cells from the attachable copper tree;
- verifies the frozen source once, accounts for the exact expected native
  connectivity reduction of every net, and transactionally commits a route
  only when KiCad finds no new ERC/DRC/parity issue;
- retains every rejected native candidate and checkpoints the last clean
  prefix.

Sequence 229 proves the first repair: VCC becomes native-clean, and the new
gate rejects the phantom-junction GND child. Sequence 230 proves the second:
VCC, GND, `Net-(D1-K)`, and `VPP{slash}MCLR` commit with exactly 125 -> 69
unconnected items and zero introduced findings. The fifth candidate connects
all ten expected items but is rejected for one real clearance violation at
JP1. Its custom pads are separated by only 0.2 mm; ending a 0.5 mm trace at the
pad anchor overlaps the neighboring pad's 0.25 mm clearance envelope. A larger
A* budget or different whole-net order does not resolve this local access
geometry.

## Consequence

Sequence 232 implements and evaluates the smaller mechanism actually required
by this failure. Grid search still targets the deterministic pad anchor, but
materialization can now clip the searched path at its first contact with real
pad copper. Filled zero-width custom-pad polygons are lowered explicitly;
unsupported custom primitives fail closed. The historical anchor policy
remains selectable as the control.

On JP1 the retained B.Cu endpoint moves from the anchor at x=148.807 mm to the
right polygon boundary at x=149.307 mm. The fifth net now exact-commits with
the expected 69 -> 59 native connectivity reduction and no introduced
finding. The route uses 728,554 expansions versus 725,075 for the rejected
anchor control (+0.48%), while physical candidate length falls from 216.150 to
204.115 mm, segments from 57 to 38, bends from 31 to 17, and the previous 1 mm
of duplicate stored copper disappears. This is worth retaining: it removes
unnecessary pad-interior copper rather than adding a speculative fanout search.

The next `CLOCK-RB6` net also exact-commits. The cold frontier is therefore six
of 34 nets and 54 native unconnected items, up from four nets/69 items.
`DATA-RB7` then has no path on branch 5 after 806,802 A* expansions and a fully
exhausted reachability preflight. That is now a route-order/multi-pass/rip-up
blocker, not another terminal-contact failure. Freerouting remains at 124/125
on the identical cold source, so the conventional routing gap is still large.

Sequence 233 isolates that conclusion. From the identical zero-copper source,
`DATA-RB7` routes first with 23 stored segments, no vias, 206.608 mm, and
114,251 expansions. Native connectivity falls exactly 125 -> 119 with no new
finding. The net and fixed placement are feasible; committing the preceding
six nets removes the needed capacity. The next experiment should therefore
retain alternative net orders and, when necessary, transactionally rip up a
small implicated set. Merely increasing the failed A* budget is not indicated:
its reachability search already exhausted the physical grid.

Sequences 234--236 then distinguish the actual precedence constraint. Merely
moving `DATA-RB7` before `CLOCK-RB6` does not help. Fifteen bounded yielding
counterfactuals show that removing either GND or VCC alone makes `DATA-RB7`
routable; removing each other prior net alone does not. `VCC -> DATA-RB7 ->
GND` reverses the failure and blocks GND, while `DATA-RB7 -> GND -> VCC`
exact-commits all three (125 -> 119 -> 80 -> 69 native items). This establishes
a diagnosis-derived compatible greedy precedence without claiming that order
alone will solve the complete board.

Sequence 237 replays the full 34-net pipeline from zero copper with that order.
It reaches a 15-net, 34-item native-clean checkpoint before an external stop at
98.188 seconds. Nothing is inherited from the incremental sequence-236
control. The result has 277 segments and 51 vias: a substantial completion
advance, but visibly poor topology that is not presented as competitive.

The timing exposes a separate bottleneck. Every route-only transaction reran
unchanged schematic ERC, so even 26- and 92-expansion routes took 6.3--6.9
seconds. Sequence 238 hashes all root/subsheet/ERC-rule inputs, reuses the
frozen ERC report only when that digest is unchanged, and still runs native PCB
DRC with schematic parity and exact connectivity for every candidate. Median-
scale step time falls to roughly 3.6 seconds, about 43%. A final full ERC/DRC
run agrees exactly.

Within 60.774 seconds the same cold order now reaches 16/34 nets and 31 native
items before `PC-DATA-IN` is unreachable. Its yielding diagnosis again singles
out `DATA-RB7` and GND. This repeated large-net interaction is the concrete
input for a bounded ordinary-KiCad order coordinator; the coordinator must
regenerate every trial from zero copper and retain failed lineages, as the
existing semantic order search already does.

Sequence 239 implements that coordinator as a replaceable layer over the
sequential router. Trial 0 independently reproduces the 16-net failure and its
yielding evidence. Trial 1 applies the first causal proposal—move
`PC-DATA-IN` before `DATA-RB7`—and reroutes the entire prefix from the immutable
source. It reaches the configured 17-net bound with exactly 30 native items
remaining and no introduced finding. The child is selected despite a worse
route score (303 segments, 57 vias, 1684.510 mm), because completion reach is
ranked before copper quality. This is the intended policy at the current
frontier, not a claim that the topology is acceptable.

The two trials consume about 61.2 and 63.9 seconds of per-step work, plus 11.6
seconds for the base yielding diagnosis and one fresh native source baseline
per lineage. The coordinator works, but its next optimization should avoid
repeating invariant source verification and eventually checkpoint native DRC
in batches. Neither optimization changes the cold-replay or final native
acceptance contract.

Authoritative current report:
`build/sequence-239-m0-pic-order-coordinator/sequential-order-search.json`.
