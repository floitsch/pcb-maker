# Whole-board recovery coverage and external geometry inspection

The larger board remains incomplete at 34/50 routed nets and 24 native opens.
Broader bounded nested recovery does not improve it. The external control
reveals an exchange-rule discrepancy that must be addressed before interpreting
router comparisons as evidence about placement quality.

[Dashboard and combined views](../../build/restoration-coverage-2026-09-08/index.html)
· [Machine-readable summary](../../build/restoration-coverage-2026-09-08/summary.json).

## Recovery coverage

The same frozen executable resumes the admitted 34-net checkpoint. The new
configuration permits eight nested invocations, depth one, and up to 64 child
diagnosis trials. Six eligible outer siblings exist; all six receive nested
search. Each tests all 34 single-net yielding choices, without truncation:
204 child diagnosis trials in total. After excluding protected ancestor nets,
24 child restoration attempts run. None restores a valid complete transaction.

The unchanged result is byte-identical to the previous checkpoint. Independent
sequence and recursive repair audits reproduce native connectivity, source
rules, component poses, unrelated copper and selection. All SVGs in the report
parse successfully. This covers the previously skipped siblings, not deeper
repair, arbitrary multi-net rerouting or changed placement.

## What the external control actually establishes

The original Freerouting process finished normally in about 293 seconds, with
58 internal unrouted items and 14 internal violations. Its imported native
board has 65 opens and seven unchanged annotation findings. Internal unrouted
items and KiCad opens are different metrics.

The initial comparison failed its exact-pose assertion: all 68 footprints
were translated by at most 50 nm during SES import. The session declares a
100 nm grid. The fixed-placement importer now permits only translation within
half that grid, restores the exact original coordinates before saving, and
then checks the full native pose/pad inventory. Rotation, side, inventory and
larger translation changes remain errors. Project bytes are restored after
import, and all routed widths and via dimensions are checked independently.

The saved successful session was re-imported without rerunning Freerouting.
Replay requires matching source board/project/jar hashes, matching DSN trees
apart from the export's root pathname, and the recorded session hash. Exact
poses and dimensions now pass. The resulting native board still has 65 opens,
seven annotation findings and eight metadata warnings.

Import controls deliberately move and rotate a footprint and change session
resolution. All three are rejected, with the original board unchanged and a
combined render retained for each. A fourth control changes the archived
session without updating its hash; replay rejects it before import and retains
the rendered source. Failures in the comparison runner now
write a terminal failure status and stage instead of leaving `importing` or
`routing` in the report. Existing output directories are not overwritten.

## Automatic exchange inspection

`InspectDsn.java` loads the pinned Freerouting jar's board representation
without routing. It reports exact conflicting item identities, nets, layer,
bounding coordinates, required/actual clearance and the loaded edge-clearance
matrix. The Python wrapper deduplicates reversed item pairs and overlays
labelled hotspots on the native combined copper view. Component bodies remain
hidden. The comparison invokes this diagnostic automatically before routing.

On the larger input all 14 unique findings are seven pad/outline pairs on
both layers. The matrix requests 0.30 mm clearance, and the external outline
obstacle contributes a further 0.01 mm half-width. Its effective edge requirement
is therefore 0.31 mm; the native project requires 0.01 mm. Net-class and pin
inventory checks alone had not exposed this difference.

The ECC83 control similarly reports 12 pad/outline findings and an effective
0.41 mm edge requirement versus native 0.01 mm. Its archived external result
is nevertheless native-complete. Therefore external edge violations alone
neither prove infeasibility nor explain the larger board's routing failure.

These are observations of the external representation, not new KiCad design
violations. The runner explicitly marks the larger result as an unequal-rule
comparison. No custom-rule or arbitrary footprint-shape equivalence is claimed;
absence of reported findings does not establish either.

## Next work

Correct and test edge-rule exchange before making equal-rule router claims.
Use matched full-board runs to compare the current cold placement with
alternative placements and the fixed reference layout. Let those completion
failures prioritize routing/placement changes. Increasing nested depth or
optimizing vias on a solved board is not the default next action.

## Reproduction

```sh
python3 experiments/whole-board/compare_native_router.py \
  --sequence build/nested-recovery-2026-09-08/finish \
  --binary build/restoration-coverage-2026-09-08/pcb-maker \
  --output build/external-reimport-NEW \
  --replay-session-from build/restoration-coverage-2026-09-08/freerouting

python3 experiments/whole-board/test_native_router_import.py \
  build/external-reimport-NEW build/import-controls-NEW

python3 experiments/whole-board/inspect_dsn_geometry.py \
  build/external-reimport-NEW/input.dsn \
  build/external-reimport-NEW/source complex_hierarchy build/exchange-inspection-NEW
```

The standalone inspector expects the native directory to have current verifier
previews and labels. The pinned jar and Java defaults are recorded in the
scripts; custom paths are explicit CLI options. Rendered outputs, native
verification and hashes remain beside each experiment. Python syntax checks,
native import controls, both recovery audits and both diagnostic boards pass.
No Rust engine code changed in this follow-up.
