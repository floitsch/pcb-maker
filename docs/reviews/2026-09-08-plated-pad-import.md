# Plated pad connectivity in route analysis

The remaining-via scan could previously import only three of the nine
via-bearing nets on the ten-via PIC result. Six nets changed copper layers
through plated component pads. ECC83's remaining GND route had the same issue.
These were unsupported inputs, not evidence that their vias were necessary.

The importer now contracts pad contacts separately on each layer and connects
them with a temporary electrical graph edge for the plated barrel. It splits
output branches at that edge, retaining each layer's own contact coordinates
and the pad's terminal anchor. The barrel is never emitted as a track or a
drilled via. Existing materialization, physical quality measurement and route
action code can consume the ordinary branches without a new transition flag.

The graph must still be connected and acyclic, with uniform trace and via
dimensions. Copper cycles, dangling material, ambiguous overlapping contacts
and a physical via duplicating a pad barrel remain explicit rejections.
Coincident front/back SMD pads cannot establish a layer connection.

## Evidence

Artifacts: [combined real-board renders](../../build/remaining-vias-2026-09-08/index.html),
[per-net native results](../../build/remaining-vias-2026-09-08/roundtrips/report.json).

| Improved board | Before: importable via-bearing nets | After | Physical vias |
|---|---:|---:|---:|
| ECC83 | 1 / 2 | 2 / 2 | 3 |
| PIC | 3 / 9 | 9 / 9 | 10 |

All eleven nets were independently imported and written back into copies of
their complete source projects. Every board passes fresh KiCad ERC, design DRC,
schematic parity and connectivity checks. ECC83 retains its seven library
metadata warnings; PIC has none. Native readback confirms the original physical
via positions, sizes and drills, component poses and pad properties. Project,
schematic and rule files are preserved. Each verification automatically captures
combined copper; the report also provides translucent combined inspection views.

The full KiCad unit suite passes: 119 tests. New controls check a plated pad
whose two layer contacts have different coordinates, preservation of its one
actual via, rejection of separate coincident SMD pads, and rejection of a cycle
completed by the plated barrel. The native synthetic controls have no schematic
or ERC: KiCad confirms zero opens for the plated fixture and cycle, and one open
for the separate SMD pads. The cycle is electrically connected but outside the
importer's supported tree topology.

## What this enables next

This is an analysis coverage improvement, not a reduction in either board's
via count. Import may normalize copper inside pads, so any eventual repair must
still beat the original native board, not merely the normalized candidate.

The scanner currently searches consecutive internal via pairs on a branch,
bounded by `maximum_excursion_mm`. All ten remaining PIC vias lie outside those
patterns; on ECC83 only the two U1A-K vias form a candidate pair. Reports now
state this search scope and count vias outside the tested patterns. Zero
actions must not be interpreted as proof of optimality.

Deferred optimization: consider moving a single layer change to a
plated terminal, then test whether the affected copper can remain on one layer.
That requires respecting actual pad contact geometry and shared branch
connections when proposing a removal. The existing complete-board verification
and original-board incumbent remain the acceptance gates.

The user subsequently reprioritized general autonomous placement/routing over
this optimization. It is retained in [the TODO list](../optimization-todos.md);
no further via-refinement runs were started. The final executable reproduces
all eleven native-verified candidates byte-for-byte, and the final rendered
synthetic controls have zero design violations with the expected 0/1/0 opens.

Reproduction:

```sh
cargo test -p pcb-kicad --lib
cargo build --release
python3 experiments/whole-board/plated_import_controls.py OUTPUT/controls \
  --binary target/release/pcb-maker
# Run discover-kicad-vias for each board into OUTPUT/{ecc83,pic}-after first.
python3 experiments/whole-board/check_plated_import.py OUTPUT \
  --binary target/release/pcb-maker
```
