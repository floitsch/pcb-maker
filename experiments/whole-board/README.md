# Whole-board reference benchmarks

Fresh unattended runs of a frozen executable and configuration are the primary
measurement. The [current evaluation](../../build/whole-board-reset-2026-09-08/evaluation.json)
compares history/boundary priority, demand feedback, and a separate placement
pair under fixed resource limits. Checkpoint recovery remains diagnostic evidence
and does not count as fresh whole-board completion.

The completed original-placement control pairs route ECC83 (9 nets), PIC (34),
and `complex_hierarchy` (50) in one pass, with identical copper within each pair.
PIC and hierarchy have zero native findings; ECC83 retains two inherited
silkscreen warnings. These are regression controls, not evidence that feedback
or recovery improves completion: neither was needed. Both Interf-U arms time out and Olimex fails source preflight. See the
[completed evaluation](../../docs/reviews/2026-09-09-fresh-whole-board-evaluation.md)
and [current status](../../docs/current-status.md) for follow-up work.

The [native-project runner](../../docs/reviews/2026-09-08-general-project-layout.md)
compiles native net classes and connects placement to adaptive routing. Earlier
[placement](../../docs/reviews/2026-09-08-unanchored-placement-and-edge-exchange.md)
and [external exchange](../../docs/reviews/2026-09-08-recovery-coverage-and-exchange.md)
investigations remain useful diagnostic history. In particular, an external
result with a rule-translation mismatch is not a matched router comparison.

The primary progress measures are complete real boards routed from zero copper,
and smaller complete boards placed without the reference positions. Local
mechanism tests diagnose these failures; passing them is not the project goal.

The starting corpus already contains five pinned upstream projects. Do not
replace a difficult source with an easier slice or remove unsupported sources
from the coverage report. Their versions, licenses and content hashes are in
[`sources.json`](../../benchmarks/real/sources.json). Upstream copper is an
evaluation reference, never routing input.

## Run the baseline

```sh
python3 experiments/whole-board/run.py build/whole-board-RUN
```

The runner verifies/fetches the pinned sources, freezes the executable, verifies
copies of all original routed projects, and runs the two existing complete-netlist
paired manifests. It checkpoints `report.json` and `index.html` after each stage.
Native reference verification automatically renders `preview.svg`; the paired
runner renders source and result, front and back, including incomplete results.
Comparison renders hide component models, fabrication graphics and silkscreen
so traces remain visible; pads, vias and board outlines remain. Native SVG
previews show both copper layers. Use a separate silkscreen view for markings;
`render-kicad-layers` can opt into component views with `show_components: true`.
Native SVG previews on supported polygonal outlines now add small component
references and stable `V-` UUID prefixes for vias. `preview-labels.json` maps
each label to the full UUID, net and native coordinates. Bodies remain hidden.
Sources with no executable paired manifest remain visible as untested.

The initial manifests have 60-second competitor process limits. Separate
capability runs can give **both** competitors an explicit larger budget:

```sh
python3 experiments/whole-board/run.py build/whole-board-ecc83-capability-RUN \
  --case kicad-ecc83-pp --router-seconds 600
```

The override writes resolved manifest copies beside the output. The original
manifests remain unchanged. Timing includes each competitor's actual execution
contract: pcb-maker performs intermediate native admission. Raw search work,
timeout status, logs and cache telemetry are in the linked comparison reports.
Do not compare an older search-only time with a newer verification-inclusive
time as though it measured the same work.

## Track A: fixed placement, complete routing

The [coupled via-refinement follow-up](../../docs/reviews/2026-09-08-coupled-via-refinement.md)
adds `refine-kicad-vias`: a diagnosed blocking net is rerouted as part of a
complete native-checked transaction. On the smaller ECC83 case, it removes
two vias at a cost of 9.654 mm more track. Whole-board cost includes the
displaced net; incomplete or more costly trials retain the original board.

Freeze footprints, pad identities, positions/orientations, stackup, outline,
keepouts, netlist and rules. Remove traces, arcs, vias and copper zones, including
cached fills. Retain rule-area keepouts. Route every required net, including
power and ground. Both competitors receive the same cold project.

The human reference is not automatically a feasibility certificate for the
router's fixed per-net widths. A read-only inventory found 15 reference segments
on seven Interf-U nets, and 11 segments on three PIC nets, narrower than those
compiled widths. For example, Interf-U uses 0.30 mm locally on nets routed at
0.40 mm by our current implementation. ECC83 and complex hierarchy have no such
segments. These are differences from routing preferences, not newly diagnosed
manufacturing violations. Together with the removed planes, they qualify the
reference comparison: neither necessity of neckdowns nor impossibility at fixed
width is established. The current frozen strategy comparisons remain unchanged.
[Inventory and combined-layer renders](../../build/whole-board-reset-2026-09-08/reference-track-widths/README.md).

Report these independently:

- Native completion, remaining open items, absolute findings, and new findings
  relative to the frozen input. A native issue in the source is documented, not
  silently repaired by weakening rules.
- Original human, pcb-maker and Freerouting track length, vias, used layers,
  copper zones and area. Track length does **not** measure paths through pours.
  Therefore human track-length ratios with a different plane strategy are
  descriptive; they do not establish better electrical routing.
- Wall time and search work, process limits, seeds, hashes and tool versions.
- Rule-translation coverage. Historical external runs with flattened PIC POWER
  classes or unrepresented Olimex local clearances are adapted experiments or
  DSN adapter failures. The native compiler now accepts all five pinned sources,
  including Olimex's local clearances; full native preflight and final admission
  are still required. Native support does not establish external translation
  coverage.

Completion precedes quality. An incomplete short route never outranks a complete
route. Among complete results retain length/via/layer tradeoffs rather than hiding
them in an arbitrary scalar. All five sources remain in coverage accounting,
with complete, incomplete, unsupported and untested distinguished.

## Track B: placement and board area

The objective is minimum **verified complete outline area** at unchanged
component inventory, netlist, layer count and electrical requirements. Use actual
outline area, not its bounding rectangle. Keep aspect-ratio and mechanical bounds
explicit. The first area targets are 100%, 95%, 90%, 80% and 70% of the source;
refine between successful targets. Search failure at one target is not proof of
infeasibility, and heuristic success need not be monotonic in size.

Run separate modes:

1. **Cold placement:** withhold human centers and rotations except explicitly
   required mechanical anchors. Use seeds 0, 1 and 2 initially.
2. **Reference-seeded compaction:** allow the human placement as a starting point;
   label that advantage. Never combine this score with cold placement.

Each source needs a versioned constraint sidecar: connector edge/spacing groups,
mounting-hole obligations, permissible overhang, antenna/other keepouts,
decoupling and power locality, component clearances and orientation/side rules.
Coordinates alone do not establish those requirements. A geometric all-free
repacking exercise can be useful, but is not a drop-in product replacement.

For every placement proposal, invoke both our router and Freerouting on that
same complete cold placement. This separates a promising placement that our
router cannot finish from one neither router can finish. Credit our end-to-end
area only when **our** router completes it; retain external completion as
diagnostic evidence. Placement-only energy, overlap-free packing, and partial
routing earn no area credit. Archive all failures and render them automatically.

### Initial area adapter diagnostic

The [complete annotation-repair pipeline](../../docs/reviews/2026-09-08-silkscreen-layout.md)
now admits cold ECC83 layouts at 80% and 70% area, with every net routed and
zero native ERC/design DRC/parity/connectivity findings. Add
`--repair-silkscreen` to the command below for that pipeline. The policy allows
all component centers to move; original product constraints are not inferred.

The [courtyard placement follow-up](../../docs/reviews/2026-09-08-courtyard-placement.md)
now accepts the human placement in the production validator and cold-routes
all nine nets at 80% area. Native checks exposed pad-edge and silkscreen
constraints. Pad-edge violations are fixed; silkscreen findings still prevent
area credit. The old
rectangle-only diagnostic below is retained as the control.

```sh
python3 experiments/whole-board/area_probe.py build/courtyard-area-RUN \
  --placement-policy benchmarks/real/ecc83-pp/placement-policy.json \
  --placement-attempts 32 --ratios 1.0 0.9 0.8 --route
```

```sh
python3 experiments/whole-board/area_probe.py build/area-RUN --seed 0 --route
```

This bounded ECC83 probe retains every physical footprint and all nine routable
nets, randomizes centers independently of human positions, and calls the
production harmonic placer at 100%, 90% and 80% area. It currently retains native
orientations and allows all centers to move, including connectors and mounting
holes. It uses conservative graphics/pad rectangles for placement and retains
exact native pads for any subsequent routing. Thus it is an adapter diagnostic,
**not the completed cold-placement benchmark**.

The human placement is rendered and checked in the same envelope model. That
control currently fails: circular-body bounding boxes overlap neighboring
parts, and a connector overhangs the outline. This prevents area admission.
Nine probes across three seeds also fail legalization. These results identify
the need for correct placement shapes and overhang constraints; they do not
prove that smaller boards are impossible. No routed smaller board is claimed.
The route branch is present but has not been exercised because no proposal has
passed placement. Do not promote it without a legal positive control and an
independent audit of native identities, rules, poses and outline.

## Implementation priorities

Adaptive routing now defaults to stopping after the first native-admissible,
fully connected result. Unchanged annotation findings remain visible and keep
full-layout admission false. Set `optimize_after_routing_complete: true` in
the adaptive configuration to spend the remaining pass budget on copper/via
quality. Archived multi-pass quality experiments require that explicit flag
when replayed with the new executable. `maximum_passes` remains an upper bound
for incomplete runs; early termination is reported as `routing_complete`.

The [net-aware placement follow-up](../../docs/reviews/2026-09-08-placement-unlocks-routing.md)
routes all 50 nets on `complex_hierarchy` in the first pass, with three
unchanged annotation findings. Use the integrated initializer with:

```sh
python3 experiments/whole-board/area_probe.py build/spectral-layout-RUN \
  --source-board benchmarks/real/external/kicad-complex-hierarchy/complex_hierarchy.kicad_pcb \
  --placement-policy benchmarks/real/complex-hierarchy/placement-policy.json \
  --placement-seed spectral_rank --placement-attempts 1 --ratios 1 \
  --adaptive-config benchmarks/real/complex-hierarchy/pcb-maker-layout.json \
  --repair-silkscreen --route-with-annotation-findings --route
```

This path invokes pcb-maker's adaptive router. The template sets its pass
budget; the matched placement experiment separately capped both runs at two
passes. Remaining annotations still prohibit complete-layout/area credit.
The seed requires all-free, fixed-orientation components and supports
disconnected graphs, including electrically unattached mounting holes. It
allocates separate seed regions by body-envelope area; these regions do not
constrain subsequent legalization. Fixed-component policies remain unsupported.
Native verification automatically renders
each routed candidate. External comparisons use `compare_native_router.py`
separately; inspect loaded layer roles, and label any explicit
`--all-copper-signal-layers` capability adaptation.

Use `--materialize-only` instead of `--route` to verify native placement and
inventory before spending a routing budget. This mode retains native renders
and findings, but never awards routing completion or board-area credit.
The [PIC placement policy](../../benchmarks/real/pic-programmer/placement-policy.json)
now imports native rectangle, circle and simple polygon courtyards on either
side. Its human geometry control passes. The default 512-sweep cold placement
fails, while an explicit larger budget reaches a native-checked placement;
see the [legalization trace study](../../docs/reviews/2026-09-08-placement-legalization-trace.md).

Add `--trace-placement` to retain a playback, final SVG and pair-contact history
under each trial's `trace/`, including rejected placements. It observes harmonic
legalization; it does not change the placement decisions. Tracing recomputes
contacts and stores every sweep, so it is opt-in. `--legalization-sweeps` and
`--placement-pair-checks` expose the two independent work limits without changing
their defaults. For example, the diagnostic PIC run uses 3072 and 6000000.

For an existing problem/config, run and render it directly:

```sh
python3 experiments/whole-board/trace_placement.py \
  problem.json placement-config.json build/placement-trace-RUN
```

The underlying `pcb-maker trace-placement` command writes diagnostic JSON even
when legalization fails and keeps the failure exit code. The wrapper retains
that exit code in `run.json`; its own successful exit means the diagnostic
artifacts were generated, not that placement succeeded.

The native-project driver now defaults to coupled recovery; use
`--pairwise-legalization` for the original comparison mode. After 128 ordinary
sweeps coupled recovery tries bounded joint translation corrections,
including movement locks and board/region/pad limits. It accumulates newly
created contacts and checks the full placement geometry before accepting.
Unsuccessful trials restore the prior poses and use the remaining ordinary
budget. `--trace-placement` shows both proposals and rollback/acceptance.
The equivalent harmonic policy JSON field is `"coupled_legalization": {}`;
omitting it in a direct library/CLI policy retains pairwise behavior. The
driver supplies it explicitly unless `--pairwise-legalization` is selected.
See the
[coupled recovery study](../../docs/reviews/2026-09-08-coupled-placement-recovery.md)
for the PIC timing comparison, scope and controls.

The [PIC source-class run](../../docs/reviews/2026-09-07-pic-source-net-classes.md)
now completes with the original POWER/Default geometry. Its native readback
audits all track/via dimensions and unchanged project/class assignments.
The uniform paired manifest below remains a separate adapted-rule comparison;
the [source-class comparison](../../docs/reviews/2026-09-08-source-class-comparison.md)
now audits matching basic dimensions and net/pin inventory. Freerouting leaves
one native open at JP1.2; pcb-maker completes. This is not a full footprint-shape
equivalence proof for the exchange format.

The first complete adapted-rule PIC result is now retained in
[the shared-tree junction study](../../docs/reviews/2026-09-07-pic-shared-tree-junctions.md).
Its controlled native diagnosis and generated regression led to a production
materialization fix. All 34 nets complete, with a byte-identical replay on the
final executable. The POWER-class limitation remains.

1. Preserve the complete ECC83 reference result under the recorded capability
   budget. Use the 60-second run as a separate performance target.
2. Preserve complete PIC routing with source classes and the audited Freerouting
   comparison. Restore all displaced nets before crediting a repair.
3. Preserve the human courtyard control, explicit overhang policy and now-admitted
   80%/70% ECC83 pipeline. Extend seeds and area targets and improve placement
   quality using complete routing results.
4. Drive the complete-area curve down, using router failures to guide placement
   changes. Add the Olimex rule translation and full paired manifests for the
   other two projects as capability milestones.
5. Expand with fresh independent designs after this ladder works. Existing boards
   are development cases, not unseen holdouts. Freeze a genuinely unseen set
   before tuning broad claims of superiority.

Freerouting's [CLI](https://github.com/freerouting/freerouting/blob/master/docs/command_line_arguments.md)
provides the initial external baseline. The existing harness pins its jar hash.
The Olimex source and hardware license are published in its
[upstream repository](https://github.com/OLIMEX/ESP32-C3-DevKit-Lipo).

## Proposed-route obstruction query

```sh
pcb-maker inspect-kicad-route-obstructions board.kicad_pcb candidate.json routing-config.json obstructions.json
```

This advisory query attributes actual candidate copper to intersecting retained
tracks/vias, fixed pads, keepouts and board edges without routing or invoking
KiCad. Supply explicit routing rules: imported candidates may contain inferred
defaults. Results include net identities, obstacle UUIDs and gap versus required
clearance; unsupported geometry remains explicit. Empty contacts do not prove
connectivity or native admission. See the
[native comparison controls](../../build/whole-board-reset-2026-09-08/global-routing/route-obstructions/README.md).

## Terminal access diagnosis

```sh
python3 experiments/whole-board/inspect_terminal_access.py \
  board.kicad_pcb NET routing-config.json build/terminal-access-RUN \
  --resolutions .5 .25 .125 --footprint JP1
```

The wrapper retains JSON for every terminal and local copper/grid SVGs for the
selected footprint. It distinguishes a blocked anchor sample, alternate clear
pad contacts, and straight copper continuity to the anchor. It does not claim
a routed path or native admission. `FirstPadContact` now automatically tries
clear copper contacts when ordinary terminal snapping fails; existing valid
snaps are unchanged. See the [native recovery study](../../docs/reviews/2026-09-08-terminal-copper-access.md).

Whole-board routing now retains typed search failures and can conditionally
retry an exhausted geometric search with obstacle-distance guidance. The
sequential JSON map `"obstacle_distance_fallbacks": {"2": 1}` links an explicit
guided portfolio entry to its earlier geometric entry; they must differ only
in heuristic. Grid disconnection does not activate it. The placement-routing
driver appends these retries by default; use `--no-budget-guidance` for the
supplied portfolio alone. Independent forecasts omit conditional entries.
See the [larger-board recovery and controls](../../docs/reviews/2026-09-08-whole-board-budget-guidance.md).

Yielding diagnosis also accepts `preferred_yielding_connections`, an advisory
net order that preserves all other foreign-net options. The sequential router
supplies recently changed nets first, including nets changed by earlier repairs.
This improves the useful coverage of a small diagnosis budget. A yielded route
still has to be restored and natively admitted before the transaction commits.
See the [checkpoint-based obstruction study](../../docs/reviews/2026-09-08-recent-route-obstruction.md).

The placement-routing driver supplies one nested restoration invocation, one
level and eight child diagnosis trials when rip-up is enabled and the supplied
policy omits `restoration`. Explicit `null` and custom policies are preserved;
`--no-nested-restoration` prevents supplying this default. Direct library defaults
are unchanged.

After a routing command is terminal, its sequence audit now also writes
`analysis.json` and `analysis.html`: compact evidence for the last attempted
connection, including nested repairs, protected nets, bounds and native results.
Generate it separately with:

```sh
python3 experiments/whole-board/summarize_routing_recovery.py TERMINAL_RUN_DIRECTORY
```

The report summarizes existing evidence; it does not determine process liveness
or independently admit a board. Missing combined copper views are generated in
an isolated `analysis-views/` cache, preserving retained source inputs.
See the [nested restoration study](../../docs/reviews/2026-09-08-restoration-analysis.md).
