# Occupied passage repair

Run after building the release executable, using a fresh directory:

```sh
cargo build --release
python3 experiments/occupied-passage/run.py build/occupied-passage-new-run
cargo test --test occupied_passage -- --nocapture
```

The runner always captures semantic HTML playback and front/back SVG/PNG
images, including the incomplete control. It exports the selected semantic
poses and exact copper into KiCad, then invokes native verification, which
automatically captures another SVG. The regression test also saves candidate
JSON and SVG for its positive and negative controls in the printed temporary
directory.

## Failure and resulting behavior

`benchmarks/small/passage-pressure-occupied.json` has three 0.4 mm nets,
0.2 mm clearance, and a vertically movable wall. Initially each passage is
1.15 mm wide. One trace requires 0.8 mm; two require 1.4 mm.

The old `passage_capacity` policy routes two nets, finds the wall blocking the
third, but proposes no move because 1.15 exceeds its single-trace requirement.
The new opt-in `passage_capacity_with_copper` identifies the foreign net
already occupying each passage. Its 0.25 mm deficit rounds outward to a
0.3 mm movement on the 0.1 mm grid. Moving the wall opens one 1.45 mm passage
and leaves the other at 0.85 mm. All three nets then route and pass independent
semantic validation. A fixed-wall negative control correctly remains partial.

| Policy | Completed | Repair attempts | Total A* expansions |
| --- | ---: | ---: | ---: |
| Existing single-trace capacity | 2/3 | 0 | 121,152 |
| Including retained copper | 3/3 | 1 | 192,350 |

The second row includes the same initial failed routing plus 71,198 expansions
for the successful reroute. This is a capability improvement, not a speedup
claim against an incomplete run.

## Implementation boundary

The coordinator's existing passage policy still provides body/board geometry,
legal movement projection, collision-chain options, rerouting and exact
admission. The added producer samples a cut at the moving body's midpoint and
collects retained trace centerline crossings on the target layer. It merges
nominal width bands by electrical identity, so tessellation vertices and
overlapping aliases do not multiply occupancy. Same-net copper and copper on
other layers are excluded. The evidence includes the target, layer, cut,
occupants, width bands, required gap, deficit and proposed displacement.

This is an advisory packing estimate. It is not a proof of topology
infeasibility or the exact capsule intersection at oblique crossings. It does
not model via diameters in the cut, all possible cross-sections, pin escape,
or arbitrary curved/rotated passage boundaries. Explicit legacy branches with
one allowed layer are supported. Flexible layer assignments and unsupported
multi-terminal branch identities retain the old single-trace calculation;
layer-specific failure evidence is needed before extending them. Exact
rerouting and validation remain the authority.

The default remains the existing frontier policy. The new configuration is
also retained in `experiments/configs/blocker-occupied-passage-repair.json`.

## Native verification and rule parity

The initial native export inherited KiCad's 0.50 mm copper-to-edge clearance
and rejected 199 segments, while reporting no connectivity problem. The
semantic router and validator use the fixture's 0.20 mm clearance at the
boundary. That failed export and its rendering remain in the artifact tree.

`SemanticKiCadTemplateConfig` now accepts optional
`copper_edge_clearance_mm`, validates it, and exports it to the native project.
None preserves prior behavior. This experiment explicitly selects 0.20 mm to
match its declared semantic boundary rule; no violations are suppressed and
the PCB outline is not enlarged.

The matching-rule replay preserves the exact semantic candidate, inserts its
segments without native rerouting, and passes KiCad with ERC=0, DRC=0,
parity=0, opens=0 and metadata warnings=0. Semantic and native physical copper
length agree at 54.793102423 mm. This does not establish validity under the
retained stricter 0.50 mm rule or on a full ESP32 board.

Validation: 44 coordinator unit tests, 101 KiCad adapter unit tests, and the
new rendered integration regression pass. The regression covers the old
stall, successful occupied-passage move and fixed-wall failure. Kernel tests
cover electrical aliases, retessellation, actual segment layers and unresolved
layer choices. The replay reproduces the earlier control and repaired
candidate geometry exactly.

Retained evidence:
`build/routing-improvements/occupied-passage-2026-09-07/validation.json`.
The current native result is under `replay/native/verified/`; interactive
before/after views are `replay/control.html` and `replay/occupied.html`.
