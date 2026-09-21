# General native-project placement and routing

The real-board area runner now accepts a native project rather than assuming
ECC83's paths, fifteen footprints, nine nets and routing dimensions. It uses
the production placer and native routers on the complete extracted netlist.
This broadening immediately exposed a pad-geometry bug that ECC83 did not.

## Pipeline changes

`experiments/whole-board/area_probe.py` accepts `--source-board`, an explicit
placement policy, and `--adaptive-config`. KiCad resolves source net classes;
the router receives their per-net widths, clearances and via dimensions.
Placement uses the maximum required global copper margin conservatively.
The board's edge and hole-spacing settings are retained. The legacy paired
ECC83/Freerouting path remains available separately.

All movable components start from randomized centers. An `explicit` movement
policy lists movable references and fixes the others at their source positions.
Unknown policy fields and references are rejected. Native orientations are
currently fixed. Multiple physical pads can share one logical pin; unnumbered
mechanical pads are retained. Physical copper centers are distinct from pin
anchors and are independently checked against native pad geometry.

An explicit `free_rectangle` outline policy permits a new rectangular board.
Its area ratio is based on the actual closed source polygon area, not its
bounding box. This matters even for the slightly sloped edge of
`complex_hierarchy`. Source outlines are never silently replaced. The original
scaled-rectangle mode remains the default. Curved outlines, multiple loops,
fixed nonrectangular placement domains and arbitrary courtyard polygons remain
adapter work; no claim of arbitrary-board coverage is made yet.

Rule extraction, board mutation and native inventory audit run in separate
Python processes. Earlier combined-process trials suffered KiCad/SWIG pointer
failures and exited with signal 11. Those trials remain failed evidence, even
where a partially written report said `finished`. The isolated complete runs
exit normally. New runs snapshot their Python driver sources as well as the
Rust executable, configuration, poses, source rules and native reports.

## Results

See [combined renders and evidence](../../build/general-layout-2026-09-08/index.html).

| Case | Components / pads / nets | Result |
|---|---|---|
| Generalized ECC83 | 15 / 33 / 9 | Complete native layout at 2413.705 mm²; zero ERC/design DRC/parity/opens; five metadata warnings |
| `complex_hierarchy` before pad fix | 68 / 165 / 50 | Legal cold placement; first route rejected by native copper clearance |
| Same frozen placement, corrected first-net routing | unchanged | The additional copper clearance violation is gone; expected connectivity reduction confirmed |
| Full `complex_hierarchy` rerun after both fixes | 68 / 165 / 50 | 13/50 nets admitted; 58 open items; seven existing annotation findings; no new native findings |

The new rectangular complex-board outline is 8057.413 mm², equal to the actual
source area. It earns no complete-layout or area credit. Placement took about
5.8 seconds; the diagnostic routing pass took 74.6 seconds. These are recorded
run times, not broad performance claims. This is all-free geometric repacking,
not an enclosure-compatible redesign; original orientations are retained.

The selected zero-overhang policy excludes a 0.035 mm courtyard overhang in
the human reference. That is reported independently from validity of the newly
generated placement. Its actual native component/pad inventory is checked after
movement and annotation repair, and the source project remains unchanged.

The larger placement initially had 77 annotation findings. Existing protected
silkscreen repair reduces these to seven. Explicit
`--route-with-annotation-findings` initially enabled a diagnostic sequential routing pass
only when no other native design findings remain. Every inserted net must add
no findings. The final report still lists the seven errors and prohibits
complete-layout credit. This exposes routing problems without confusing an
electrically useful intermediate with a finished board.

## Pad copper offset regression

KiCad reports Q301 pad 3's drill anchor at `(117.664399, 107.241522)` mm, but
its copper center at `(117.664399, 107.641522)` mm. The 0.4 mm offset is stored
inside the pad's drill form. The native router adapter had ignored it and
centered copper on the drill. The placement adapter similarly centered the
pad bounding rectangle on the pin anchor.

The Rust adapter now rotates the copper offset using the pad's absolute angle,
while retaining the drill and terminal anchor. The placement adapter uses the
actual native pad bounding-box center. On the saved first-net failure, the old
route had only 0.2685 mm clearance against a 0.3000 mm requirement. The corrected
route has no new native violation and connects all `+12V` terminals on the same
placement. This is a geometry correctness fix, not a relaxed clearance setting.

All 120 KiCad unit tests pass. The new test checks offset rotation at 0°/90°
and unchanged drill anchors. Rendered native extraction controls cover physical
pad grouping, unnumbered holes, source widths, fixed components, unknown policy
rejection and actual polygon area. Copper centers are compared with KiCad's
`ShapePos`. The final executable also reproduces all nine native-verified ECC83
candidates exactly; corrected extraction preserves its input model exactly.

## Next blocking case

`Net-(D303-A)` routes on the new placement with no prior copper, with no new
native findings. It fails after the first thirteen admitted nets. The same
geometry and routing rules are used in both probes. This identifies routing
interference for this failure; it does not prove that all fifty nets can be
routed on the placement.

The next improvement should address whole-board order/rip-up or placement
feedback using that failure. At this checkpoint the adaptive coordinator
required an annotation-clean source. The subsequent
[provisional adaptive integration](2026-09-08-provisional-adaptive-routing.md)
separates routing progress from final annotation admission and replaces the
sequential fallback with learned-order routing. Local via optimization remains
deferred.

Example command:

```sh
python3 experiments/whole-board/area_probe.py OUTPUT \
  --source-board benchmarks/real/external/kicad-complex-hierarchy/complex_hierarchy.kicad_pcb \
  --placement-policy benchmarks/real/complex-hierarchy/placement-policy.json \
  --adaptive-config benchmarks/real/complex-hierarchy/pcb-maker-layout.json \
  --ratios 1 --seed 0 --placement-attempts 8 --route --repair-silkscreen \
  --route-with-annotation-findings
```
