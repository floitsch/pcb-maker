# Review 2026-10-21: placer side, layout flow, free dimension

Read-only review of main at 20956b3: `crates/pcb-placer` (global, anneal, legal, orient), `crates/pcb-kicad/src/board_placer/`, `placement_constraints.rs` (keepouts), `placement_room.rs`, `board_layout.rs`, `free_dimension.rs` with `src/agent_cli/free_dimension.rs`, `canonical_order.rs`, docs/placer.md and docs/constraints.md. Nothing was built or run. Each finding says whether it is **sure** (read in the code) or **suspect** (likely, not confirmed by a run).

## High

**1. The failing order test comes from the writer's source-order restoration (sure).** `canonical_order.rs:39-89, 245-255` (commit 3d28004, 2026-10-21 01:11, before ded766d) keeps a process-wide `SOURCE_ORDERS` map. It maps a digest of the reference and the canonical graphics to the graphics' order in the file. `encode` (`kicad_sexpr.rs:296-301`) uses that map to write the graphics back in the file's order. In `a_placement_does_not_depend_on_the_order_of_the_file` (`board_placer/tests.rs:1273`), the forward and backward boards have the same digests but reversed graphics, so each output is written in its own input order and the boards differ. The poses are probably equal, since the wirelength assert passes first. The failure is therefore by design, but the design contradicts docs/placer.md:549 ("the result file is byte-identical from any input order"). It also has three weaknesses:
- Last writer wins. Two boards in one process (parallel unit tests, an in-process sweep) that share a reference and identical graphics in different file orders overwrite each other's order.
- `fp_text` counts as a graphic (`graphic()`), and `write_footprint_pose` turns `fp_text` on every rotation (`bodies.rs:276`). On KiCad 7-format footprints that are rotated or relabelled, the digest therefore changes and the order is not restored. That is exactly the malformed-courtyard case the restoration exists for.
- The map is never cleared.

Fix: restore only the courtyard-layer graphics, with a digest over those graphics alone, and change the test to compare boards re-read through `parse_board`. Or drop the restoration and accept KiCad's chaining. Either way, decide which invariant holds and update placer.md.

**2. Issue 12 has a second path: the `edge_copper` level ignores the copper-to-edge rule on the other sides (sure).** In `legal.rs:265-302` `on_board`, at the relaxation levels with `edge_copper` (`lib.rs:690-695`), a part with an edge constraint is judged by its pads only:
- The constrained side keeps `copper_edge`.
- Every other side keeps `min(min_spacing, edge_margin)`, where `min_spacing` is the copper clearance (`lower.rs:1279`, 0.2 mm by default).

The overhang edge is not in `touching`. The fence's U3 (`edge: left`, `overhang: top`) can therefore get pads 0.2 mm from the top edge against the project's 0.3 mm rule. That is exactly the 0.2 mm in benchmarks/fence/README.md issue 12. The fix on the normal path (`pad_edge_deficit`, `copper_edge_offsets_signed`) does not reach this branch. The same applies to a flush part in a corner. Use `max(copper_edge, ...)` for every side that is not a constrained one. docs/constraints.md:68 says the router and DRC "enforce" the copper-to-edge clearance on a flush side. The router cannot move pads, so DRC only reports it.

## Medium

**3. A free-dimension shrink can leave or break geometry that no check covers (sure).** `free_dimension.rs:420-470` moves zones, `gr_poly`, `gr_line` and `gr_rect` point by point, and only points within `band_mm`. When the shrink is larger than the band, the moved vertices cross the vertices that stay: a pour with a notch near the edge, or a silkscreen frame, folds into a self-intersecting polygon. Texts, circles and inner cutouts just outside the band end beyond the new outline. The infeasibility checks only look at fixed footprints, keepout zones and constraint boxes, so such sizes run and fail late, or pass with a broken pour or outline. Arcs (`arc`, `gr_arc`) move rigidly by their midpoint, so an arc that crosses the band boundary leaves the segment joined to its far end behind. This is harmless in a layout, which strips tracks, but wrong for a routed input.

Two related points:
- Constraint boxes move whole by their near side (`520-548`) while zones stretch. A `region` or `keepout` spanning the free axis (a full-height strip) is infeasible at every size other than the given one when shrinking, and on growth it stops covering the far end.
- A fixed part whose origin lies outside the band but whose body sits at the edge (a centre-origin USB connector, an overhanging module) stays behind on growth without a warning. Only shrinking past it is caught.

Suggestions: stretch boxes that reach the far side; flag any non-moved item whose box ends outside the new outline; validate polygons after the move.

**4. The free-dimension search leaves much of `total_seconds` unused (sure).** In `next_step` (`free_dimension.rs:1268-1305`), the confirmation or the closest run gets `confirm_seconds.min(left)`, never `left`.
- `parallel` 1 with a clean first probe uses 1/9 + 1/3 = 44 % of the budget.
- A three-size ladder with no clean probe uses 3/9 + 1/3 = 67 %.

This is the pattern fixed today for the ladder (the fence's 3000 s). Once the search knows it is in its final run, giving that run `left` seems right. Separately, docs/constraints.md (the budget paragraph) says the 6000 s example gives "probes 500 s, full 1500 s ... nine probes". The example sets neither key; the defaults give 2000 s and 667 s, so six probes.

**5. Child processes overcommit memory (suspect).** The parent caps `parallel` at one child per 3 GB of `MemAvailable` (`agent_cli/free_dimension.rs:204-211`). Each child then sizes its own routers side by side from half of `MemAvailable` at its own start: the race waves (`board_layout.rs:1202-1206`) and the ladder's `memory_limits`. All children start at the same moment, so each plans for the same free memory. On this memory-bound machine that is the risk. Pass `PCB_MAKER_MEMORY_MB` = available / parallel to each child.

`RAYON_NUM_THREADS = threads / count` does not bound the `std::thread::scope` waves. The pinned rate is shared, so the thread split changes a size's result only through the wall guard. The last, partial batch gets more threads, though, so a size's probe is not exactly "the same however many run beside it".

**6. Three different notions of a "held" part (sure in code; impact suspect).**
- `HeldParts::holds` (`placement_constraints.rs:1197-1225`, new in ded766d): `fixed`, `fixed_patterns`, locked or netless parts, minus `free`.
- The lowering (`lower.rs:1294-1316` plus `apply_constraints` at `placement_constraints.rs:2424-2522`) also fixes constraints.json's `fixed` list, `place` parts with `x`/`y`, and `on_edge` parts (body off the outline, connectors near it, parts in a designer keepout). It also frees locked or netless parts that a constraint names, or all of them under `move_all`.
- `KiCadFreeDimensionPlan::new` (`free_dimension.rs:310-333`) is a third variant that ignores `free` and `on_edge`.

Consequences:
- A keepout over a part fixed by constraints.json, or over an on-edge connector, is still written `footprints not_allowed`, and KiCad reports the part (issue 15 remains for that input).
- Parts the constraints free get "footprints allowed" and a spurious warning.

One helper should decide this from the lowered problem. Also suspect: with `copper: true` and a held part, the rule area forbids tracks and vias but allows pads, so pads inside the box may be unreachable for the router. The warning does not say so.

**7. The 5 % rule (sure; a design question).** `board_layout.rs:1356-1380`: `many_open` (more than max(10, 5 % of terminals) open after the first route) and `far_open` (narrow step, more than max(10, 2 %)) skip the moves, and also the resume (`1590`) and the polish (`1651`).
- What it protects against: moves and the resume eating the time of the ladder or the narrow step on boards where pour mode is the lever (Sisu 108 against 20; jetson's 388 s for 31 to 25).
- It only applies with `connect` and at least 4 layers (`ladder_has_rungs`). The fence's 2-layer board does get moves now, so README row B is out of date.
- The gap: on dense boards with 4 or more layers, no placement change happens after the first route at all. The only placement lever is the race before routing, which needs `seeds == 1`, no pin swaps and alternatives.
- The threshold is judged on a first route whose negotiation is capped at left/2 (`1290-1297`), so smaller budgets push more boards over it. The behaviour then depends on the budget, not only on the board.

Consider a bounded move phase after the ladder or the narrow step when they leave the board open.

## Low

**8. The narrow-step path stops well short of large budgets (sure).** `direct_narrow_budget` caps negotiation at 900 s and clean-up at 120 s (`board_layout.rs:564-566`). The continuation's `ladder_budget_seconds` is `left.min(1200)` (`1903`), unlike today's change to the ladder budget. A 3000 to 6000 s layout on that path ends near placement + first route + 2 × 1020 s. On the other side, the neck repair is gated only by the wall stop, and its checks are charged after it (`1962-1972`). Work can pass `total_seconds` by that much, and the wall guard covers the excess.

**9. The chosen run's paths are rewritten incompletely (sure).** `rewrite_paths` (`agent_cli/free_dimension.rs:73-93`) matches a bare prefix. `.../40mm-probe/layout` also matches `.../40mm-probe/layout.json` and `layout.log`, which would become `<out>.json` and `<out>.log`. Paths embedded inside strings (reasons, next_actions) are not rewritten. Neither are JSON reports beyond `verdict.json`, `board-layout.json` and `placed/board-placer.json` (for example `result/board-router.json`). Match on `from + "/"` or on equality.

**10. An explicit `--reference` is not moved with the edge (suspect).** It is passed unchanged to every child (`agent_cli/free_dimension.rs:300-302`). A reference finding between a moved and an unmoved item no longer matches "the same place" (`verdict.rs:288-330`: the distance between the items changed). Under `pass_strict`, it counts as new at every size except the given one.

**11. Ties in the canonical order can swap footprints (sure; rare).** Footprints with the same reference and the same or missing uuid are ordered by their full text, which includes `at` (`canonical_order.rs:275-292`). Their order can therefore change between the source and the placed board. The layout assigns poses to `footprint_items` by index (`board_layout.rs:1146-1149, 1180-1186`), so such footprints could receive each other's poses.

## Checked and found sound

- `pcb-placer` has no `HashMap`, no rayon and no parallel reduction. Every sort is on integers or uses `total_cmp` with an index tie-break (`legal.rs:1429` takes the smallest angle on ties). orient's canonical start makes the result independent of the input angles.
- `race_seeds` (`seed_race.rs`) decides batches and kept seeds from the seeds' work alone, with results in index order. The layout race keeps ties on the earlier placement.
- The keepout condition in `lower.rs:99-102` (a name prefix of `constraint keepout`) matches what `apply_keepouts` writes. A part inside such an area is not made default-fixed (`lower.rs:1078-1081`), and fixed-fixed pairs are skipped in `clear_of`, so a held part in its keepout stays legal.
- The free dimension re-expresses relative boxes and points correctly when the top or left edge moves (both corners round-tripped). `place` and `region` need both coordinates, so no half-specified point escapes.
- The child's wall guard stretches by pinned over measured rate, so contention between siblings does not cut them.
