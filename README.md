# pcb-maker

> **Trust warning (2026-09-21):** the capability claims below were not
> confirmed by a fresh audit. Read
> [the trust audit](docs/reviews/2026-09-21-trust-audit.md) first; it supersedes this document where they disagree.

`pcb-maker` is a Rust research project for coupled PCB placement and routing.
The router may move components, placement changes may invalidate routing, and
both systems return evidence to a coordinator instead of hiding failed work.

**Automatic layout** (placement plus routing) of a KiCad board is
`cargo run --release -- layout-kicad-board ...`, built on the electrostatic
placer in `crates/pcb-placer` ([docs/placer.md](docs/placer.md)) and the router
below.

Results on the 14-board corpus are in [docs/benchmarks.md](docs/benchmarks.md).

**Routing a complete KiCad board** now goes through the in-memory
negotiated-congestion router in `crates/pcb-router`
(`cargo run --release -- route-kicad-board ...`); see
[docs/router.md](docs/router.md) for results and design. The per-net
sequential/adaptive router described further below is superseded.

The current priority is a reusable pipeline for placing and routing complete
boards across varied projects. See [current status](docs/current-status.md).
Local via/length refinements are preserved as
[deferred optimizations](docs/optimization-todos.md).

The first implementation is deliberately small. It establishes:

- the ported, strict `layout-trace` problem/geometry/topology model;
- a versioned, fail-closed pass contract for interchangeable experiments;
- the DUT bounded A* grid router as a selectable Rust strategy;
- a semantic DUT adapter that rasterizes placed components, routes legacy and
  multi-terminal nets, emits explicit route graphs/vias, and fails through
  the exact gates;
- selectable hard-reservation and bounded negotiated-congestion policies, with
  exact conflicts fed back as durable grid history rather than accepted;
- the full extracted `layout-trace-routing` corridor, visibility, route-family,
  exact assignment, fixed-context, and combinatorial-regime kernel;
- bounded native V5 route families with two certified layer runs, explicit via
  geometry, spatially diverse seed-derived sites, exact rejection evidence,
  and joint conflict-free selection across interacting nets;
- a first transactional router-to-placer repair policy that maps blocked grid
  frontiers back to movable semantic components and retains every attempt;
- five selectable `layout-trace` initial placement policies with common hard
  constraint projection and evidence;
- an independent explicit-route-graph electrical connectivity gate;
- a semantic board model separate from solver representation;
- a representation compiler with explicitly selectable endpoint-particle and
  analytic-body/body-local-attachment experiments;
- GPU-shaped structure-of-arrays state and constraint batches;
- a deterministic CPU reference backend;
- a fail-closed layout-trace/continuous-engine adapter which materializes the
  supported reduced subset into the common exact-validated candidate;
- a replaceable grid-field seam for GPU density and congestion experiments;
- inspectable frame evidence and a self-contained viewer;
- a benchmark ladder imported from the two predecessor projects.

The completed ESP32-C3 ladder remains the mechanism baseline, and the active
system milestone has restarted the harder dual-ESP32 board as a separate
52-connection native ladder. Independent cold prefixes through the first
fifteen connections are complete. Prefix 5 is the first substantial pressure point;
prefix 8 has eight non-clustered vias and demonstrates per-connection
rooted/shared topology selection. Prefixes 9 and 10 each add short, justified
two-via escapes; prefix 10's vias are 20.804 mm apart. Prefix 11 exposes and
then removes a redundant adjacent via by searching the same shared-tree
attachment coordinate on both copper layers. Prefix 12 then adds a front-only,
zero-via route. Prefix 13's clean two-via route narrowly beats a retained
zero-via control under the current scoring policy. Later prefixes are
deliberately not claimed. Prefix 14 is the first rung that needs trace-level
repair: the engine yields and reroutes `SIG01_AFTER_LINK`, then admits the full
board through native KiCad. Prefix 15 is complete under the harmonic placement;
placement feedback can improve its semantic router result but currently causes
a later native dead end, so the cold transaction rejects that pose and
independently regenerates the base placement. A bounded future-reachability
experiment is retained but not promoted: it also rejects the successful
harmonic control because it does not model future rip-up repairs. The
C3 board still has every prefix from the empty board through rung 42 complete.
Materialize and verify any prefix with:

```sh
cargo run -- materialize-kicad-rung \
  benchmarks/esp32-c3-ladder/declaration.json 0 artifacts/esp32-c3-ladder
cargo run -- verify-kicad-rung artifacts/esp32-c3-ladder/00-empty esp32-c3

cargo run -- materialize-kicad-rung \
  benchmarks/esp32-c3-ladder/declaration.json 42 artifacts/esp32-c3-ladder
cargo run -- verify-kicad-rung \
  artifacts/esp32-c3-ladder/42-gnd esp32-c3
```

The materializer uses KiCad's exported schematic netlist as connectivity
authority, then preserves the raw ERC/DRC/parity reports beside each board.
The ladder now also drives generated-parent routing experiments: every
selected child is independently native-verified before it becomes the next
parent, and failed attempts remain inspectable instead of entering the chain.

Generate and exercise the new semantic-to-native boundary with:

```sh
cargo run -- generate-semantic-kicad-ladder \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json declared \
  benchmarks/dual-esp32-ladder/template-config.json \
  benchmarks/dual-esp32-ladder
cargo run -- materialize-kicad-rung \
  benchmarks/dual-esp32-ladder/declaration.json 0 \
  artifacts/dual-esp32-ladder
cargo run -- verify-kicad-rung \
  artifacts/dual-esp32-ladder/00-empty dual-esp32
```

Growing dual-board experiments must not consume the routed result of a smaller
prefix. The cold-prefix command first removes later semantic connections,
reruns placement, creates a zero-copper board, and solves the selected prefix
inside one run. It snapshots the active problem and configs for replay:

```sh
cargo run -- solve-semantic-kicad-prefix \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json declared \
  benchmarks/dual-esp32-ladder/template-config.json 3 \
  build/dual-prefix03 \
  experiments/configs/dual-esp32-rung02-shared-tree.json
```

Placement policies can also compete through the complete cold native pipeline
instead of being selected by ratsnest distance alone:

```sh
cargo run --release -- solve-semantic-kicad-placement-portfolio \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json \
  experiments/configs/dual-esp32-placement-portfolio-declared-harmonic.json \
  benchmarks/dual-esp32-ladder/template-config.json 5 \
  build/dual-prefix05-placement-portfolio \
  experiments/configs/dual-esp32-rooted-shared-layer-portfolio-0.25mm-single-ripup-preflight-fast-native.json
```

Every entry independently regenerates placement, starts at zero copper, and
routes the entire selected prefix. Native completion outranks physical copper
plus via penalty; placement demand is only a late tie-breaker. In the first
matched prefix-5 experiment both entries native-pass, and harmonic placement
is selected at 153.261470 mm/four vias versus 192.974062 mm/four vias for the
declared-placement control. The 20.58% copper gain costs 2.68x route
expansions, so it is a placement-quality result rather than a speed result.
Harmonic configs may also select `underanchored_seed` as `declared`,
`grid_packing`, or bounded `random`. These alternatives affect only movable
Laplacian blocks without two distinct positional anchors and remain portfolio
proposals, not defaults. On prefix 5, one random seed native-completes after 11
placement attempts and uses 5.04% fewer route expansions, but adds 0.995 mm of
copper and two vias, so the historical declared fallback remains selected.
At prefix 7 the same random policy becomes 3.24% shorter and uses 8.42% fewer
route expansions, but four extra vias still lose the declared 2 mm/via score.
The retained evidence localizes one avoidable-looking pair to two branches of
`SIG02_AFTER_LINK`; this is now a shared-layer-change search target rather than
a reason to discard placement diversity.
An eight-source router-cost shared-tree entry closes that gap: it reuses an
existing B.Cu layer change, removes the close pair and one via, and adds only
0.25 mm to that connection. Promoted through both cold prefix-7 placements, it
removes one via from each board without increasing total copper. It remains
opt-in because the matched runs use about 2.1x route expansions.
A schema-v4 conditional suffix can now activate such expensive modes from
retained cheap-candidate evidence. With a four-via threshold, only
`SIG02_AFTER_LINK` activates at prefix 7; both final boards reproduce the
unconditional result byte-for-byte while route work falls 5.46% and 8.01%.
The retained source trials then show that only the nearest F.Cu source and the
first B.Cu source are needed. Bounding the conditional mode to those two
sources keeps both boards byte-identical and cuts 31–34% of the unconditional
portfolio's route expansions.

Placement entries may now opt into a bounded proposal archive before native
routing. This ports the predecessor's cheap-candidate/expensive-finalist seam
without importing its surrogate as board authority: rejected and duplicate
poses remain serialized, while every retained finalist independently starts
from zero copper and must pass the full KiCad progression. A 16-proposal
whole-board random archive still finds only one legal pose even with four times
the legalization budget. Independent 0.5 mm local mutations of the legal
harmonic parent instead produce 16/16 legal unique poses. Routing the two
cheap-ranked prefix-5 finalists selects a 153.103349 mm/four-via board, 0.158121
mm shorter than the prior harmonic control, while the second finalist also
native-passes. Both archive size and expanded native-trial count are explicit
configuration bounds.

Placement portfolio schema v3 can apply the existing routing-failure pressure
search independently to every archived finalist. The semantic routes are
diagnostic only: each unmodified parent and changed child starts a separate
zero-copper native progression, and no-op feedback is retained without
duplicating an expensive trial. On prefix 14, both parents and both children
native-pass. Feedback changes proposal 2 by one 0.5 mm resistor move and saves
0.034160 mm; it changes proposal 8 by a two-resistor push and saves 8.594701 mm
but adds two vias. Native selection still chooses proposal 2's child at
450.096586 mm/22 vias. This makes routing failures useful placement guidance
without allowing the semantic router to commit or silently discard a better
parent.

Cold capability growth now ranks completed connections before copper quality.
Harmonic prefixes 16 and 17 pass, while the original prefix-18 order stops at
14/18 across the harmonic placement, two local mutations, and a randomized
under-anchored placement that moves 38 components. All expose the same causal
pair: `SIG05_R_HEADER` becomes routable when `SIG03_R_HEADER` yields, but the
older route then cannot be restored. Swapping only those two insertion
positions completes prefix 18 with zero native findings. This makes global
order/conflict branching the next architecture target; millimetre-scale route
scores and larger A* budgets are subordinate once a prefix is already solved.

The feedback variant runs the semantic DUT tracer after placement, lets its
bounded pressure transaction move components, discards all provisional copper,
and then performs the same zero-copper native KiCad solve. This is a cold
placement/routing experiment, not a routed-board warm start:

```sh
cargo run --release -- solve-semantic-kicad-prefix-feedback \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json \
  experiments/configs/harmonic-standard.json \
  benchmarks/dual-esp32-ladder/template-config.json 14 \
  build/dual-prefix14-feedback \
  experiments/configs/dut-grid-default.json \
  experiments/configs/blocker-pressure-relational-terminal-beam-ripup.json \
  experiments/configs/dual-esp32-rooted-shared-layer-portfolio-0.25mm-single-ripup-preflight-fast-native.json
```

The retained evidence separates base placement, every semantic pressure
attempt and beam parent, optional selective-rip-up trials, selected poses,
demand diagnostics, zero-copper materialization, and every native connection
transaction. The mixed configuration is experimental; the simpler
`blocker-pressure-relational-terminal.json` remains the single-component
quality control. Feedback poses remain provisional: if their complete cold
native progression fails, manifest schema v2 retains that trial, regenerates
the base placement and all routes from zero copper inside the same command,
and transactionally selects the farther exact result. It never promotes or
continues from the feedback trial's partial rollback.

The first native one-connection transaction now restores rung 0 → 1 from the
exact empty-board artifact. It keeps the external parent immutable, retains
every failed attempt, and admits a child only after KiCad ERC, DRC, schematic
parity, and selected-net connectivity all pass:

```sh
cargo run -- insert-kicad-connection \
  benchmarks/esp32-c3-ladder/declaration.json \
  artifacts/esp32-c3-ladder/00-empty \
  run/rung-0-to-1
```

The default bounded A* route completes `T_LOCAL_COMM0` with 19.618 mm of
copper and no vias. A deliberately exhausted control returns a directory that
is byte-identical to the parent snapshot. Historical declared rungs may be
evaluated as feasibility controls, but are never selected as repairs because
they need not preserve arbitrary parent edits.

Run a bounded generated-parent sequence with:

```sh
cargo run -- progress-kicad-connections \
  benchmarks/esp32-c3-ladder/declaration.json \
  artifacts/esp32-c3-ladder/00-empty \
  run/generated-progression \
  experiments/configs/kicad-generated-progression-multiresolution-lazy-native.json
```

The coarse 0.25 mm control reaches rung 12 with local insertion, then uses a
configured single-connection rip-up at rung 13 and continues through rung 21.
A critical multi-resolution follow-up shows that this was a raster false
negative rather than a necessary topology repair: 0.125 and 0.10 mm local
routes both pass at rung 13. A three-resolution, quality-ranked progression
then commits all rungs 1–21 without rip-up, selects 477.768 mm of newly added
copper versus 553.350 mm historically, and uses 14 versus 22 vias. See
[`docs/experiments/connection-insertion.md`](docs/experiments/connection-insertion.md)
for the work/quality evaluation and retained limitations.

The recommended config generates all three route candidates, ranks them by
physical copper plus via penalty, and invokes KiCad in score order until one
passes. The exhaustive companion config native-gates all three for algorithm
comparison. Both select byte-identical boards through rung 21; the lazy policy
uses 21 rather than 63 candidate gates and labels the 42 unevaluated routes as
`candidate_generated`, never as complete.

An optional coarse-to-fine scheduling experiment is available as
`kicad-generated-progression-adaptive.json`. It always evaluates 0.25 and
0.125 mm, and evaluates 0.10 mm only when the latest live score improves the
previous best by at least 3%. Its fresh chain reaches rung 21 with 49 rather
than 63 searches and 1,738,824 rather than 2,283,335 expansions. The tradeoff
is visible: 486.083 mm/16 vias versus the exhaustive control's 477.768 mm/14
vias. A stopped prefix is only a scheduling hint; if its candidates fail
KiCad, the skipped suffix is restored before any topology fallback.

The next one-step exhaustive control starts from the better rung-21 parent and
adds only `T_PROBE_XDATA_1`. All three resolutions pass KiCad and the 0.10 mm
route commits rung 22 at 24.251 mm/four vias after 351,626 aggregate
expansions. It is 5.46% longer than the historical route, which remains a
quality control rather than an admissible child.

Rung 22 → 23 is the first generated-parent multi-terminal insertion. The
rooted-star control creates four vias and a close pair. A selectable
shared-copper tree instead lets later `XDATA` branches attach to the first
branch: the selected child is native-complete at 22.405 mm/two vias, with no
close pair, and is 6.95% shorter than the historical route. An eight-source
attachment portfolio did not help this case; nearest-only attachment produces
byte-identical boards with 44,744 rather than 65,812 expansions. A checked-in
historical-parent regression also completes `XDATA` with zero vias. The full
control and negative-ablation evidence is in
[`docs/experiments/connection-insertion.md`](docs/experiments/connection-insertion.md).

The next deliberately isolated step adds `D_BUS_DHT`. All three resolutions
pass the native gate, and the selected 0.10 mm child commits rung 24 at
40.108 mm/two vias after 133,690 aggregate expansions. It is 0.79% longer than
historical and has no close via pair.

Rung 24 → 25 adds the two-terminal `DHT` connection. Again all three
resolutions pass; the selected fine child uses 9.094 mm, zero vias, and only
388 aggregate expansions. Its historical quality gap is 1.53%.

The isolated chain then reaches rung 29. `D_BUS_ONEWIRE` is 11.44% shorter
than historical, while `ONEWIRE` and `D_BUS_US_TRIG` pass with small 1.72% and
2.65% gaps. `US_TRIG` exposes another important raster threshold: 0.125 and
0.10 mm agree on a 12.9 mm detour even though the 7.808 mm historical route
passes on the exact generated parent. A 0.05 mm search finds a native-complete
6.649 mm route in 12,604 expansions, 14.84% shorter than historical. The
better 0.05 mm child, not the detour, is the current rung-29 parent.

The chain now reaches rung 32. `D_BUS_US_ECHO` and `US_ECHO` both commit
locally; the latter again shows strong resolution sensitivity. `T_EN` is the
first hard four-terminal continuation: 0.25/0.125/0.10 mm fail against fixed
copper and a one-net rip-up fallback can repair it, while the historical tree
is genuinely invalid on the generated parent. A 0.05 mm shared-copper tree
instead commits locally at 51.009 mm/eight non-clustered vias after 334,597
expansions. It is 26.38% shorter than historical. An eight-source attachment
portfolio produces the identical board with 2,640,195 expansions, so the
nearest-only sequence-068 child is the current rung-32 parent.

Missing two-pad connections can be routed through the reviewed KiCad adapter
around the bounded DUT A* kernel. It emits evidence and declaration fragments;
it never edits the declaration implicitly:

```sh
cargo run -- route-kicad-connection \
  artifacts/esp32-c3-ladder/31-us-echo/esp32-c3.kicad_pcb \
  D_BUS_US_ECHO run/d-bus-us-echo.route.json
```

The adapter supports a deterministic rooted-star baseline for multi-terminal
nets and reserves drill spacing between independently searched branches. The
declaration can then replace a selected routed connection with native KiCad
zones and use its routed vias as a thinning seed. On rung 42 this removes all
412 GND tracks and retains 32 of 128 GND vias; the regenerated board has 403
segments, 100 vias, two zones, and still passes native ERC, refilled-zone DRC,
parity, and connectivity. This is a pipeline and topology-reduction result,
not a claim that the 32-via set or plane quality is optimal.

Existing KiCad copper can also be captured without rerouting it. The importer
canonicalizes a connected, acyclic, uniform segment/via graph and emits
deterministic root-to-terminal branches. It contracts same-layer contacts
inside terminal pads and fails closed on cycles, dangling copper, arcs, zones,
non-uniform dimensions, or cross-layer ambiguous pad contacts:

```sh
cargo run -- import-kicad-route-candidate \
  artifacts/esp32-c3-ladder/07-t-local-flex0/esp32-c3.kicad_pcb \
  T_LOCAL_FLEX0 run/t-local-flex0-imported.json
```

Measure a persisted candidate, or propose conservative same-layer shortcuts
without modifying the declaration:

```sh
cargo run -- measure-kicad-route-candidate \
  benchmarks/esp32-c3-ladder/candidates/3v3-dut-rung41.json
cargo run -- shorten-kicad-route-candidate \
  artifacts/esp32-c3-ladder/42-gnd/esp32-c3.kicad_pcb \
  benchmarks/esp32-c3-ladder/candidates/3v3-dut-rung41.json \
  run/3v3-shortened.json run/3v3-shortening-evidence.json \
  experiments/configs/kicad-retained-visibility-shortest-path.json
```

The final-board input intentionally makes this experiment future-aware. The
shortener fixes via locations and only removes same-layer branch points. A
later physical-graph audit found that the old stored-track metric was wrong:
the source contains 238.146 mm of canonical physical copper but 294.382 mm of
overlapping track objects. Greedy and retained-visibility reduce the latter to
238.414 mm, yet increase the physical union by 0.268 mm. Both processors now
roll back as `no_improvement`; their raw geometry remains sequence-023
evidence rather than a promoted result.

A continuous KiCad adapter moves selected route interiors through the
projected trace-tension engine. Via-free processing remains the default;
setting `via_mobility` to `fixed` splits a multilayer branch into uniform
same-layer particle polylines joined by shared fixed via anchors:

```sh
cargo run -- relax-kicad-route-candidate \
  artifacts/esp32-c3-ladder/42-gnd/esp32-c3.kicad_pcb \
  benchmarks/esp32-c3-ladder/candidates/3v3-dut-rung41.json \
  run/3v3-relaxed.json \
  run/3v3-relaxed-proposal.json run/3v3-relaxation-evidence.json \
  experiments/configs/kicad-projected-tension-branch-0.json
```

The raw proposal is separate from the committed candidate, so rejected motion
remains inspectable. On branch 0 the adapter reduces canonical physical
`3V3_DUT` length from 238.146 mm to 237.552 mm with 12 unchanged vias and
exact-passes rungs 41 and 42. A 0.001 mm constraint guard keeps
f32/parallel solver rounding away from the f64 acceptance boundary. Sequence
020 contains
front/back images for the source, rejected no-guard proposal, accepted guarded
proposal, and unsupported control.

On the much smaller rung-7 `T_LOCAL_FLEX0` control, the board-copper importer
round-trips four tracks and two vias through the native gate. Fixed-via
relaxation lowers the branch into three layer runs and two anchors, improves
19.243 mm to 18.964 mm, preserves both vias, and exact-passes KiCad. The
retained-visibility ablation cannot delete the bend because its direct shortcut
is blocked, so the partial particle pull is a distinct capability rather than
an expensive vertex deletion. Sequence 028 retains all front/back stages and
the default `reject` control. The layer-run-aware uniform-grid configuration in
sequence 029 is byte-identical to exhaustive while reducing projection rows
from 91,648 to 1,792 and engine particles from 179 to 7.

Switching the same processor to
`experiments/configs/kicad-projected-tension-branch-0-local.json` uses a
per-layer 2.0 mm uniform grid plus an explicit 1.0 mm motion envelope. On this
control it retains 22 of 879 possible obstacle pairs and reduces projection
work from 787,584 to 19,712 rows while producing byte-identical exhaustive
copper. Exceeding the declared envelope rejects the transaction; the 0.1 mm
negative control creates four native KiCad DRC findings and is retained only
as a raw proposal. Sequence 021 contains front/back images for the exhaustive,
equivalent local, and rejected tight-envelope states.

The first simultaneous-branch result in sequence 022 was also invalidated by
that metric correction. Exact coincident vertices did not cover collinear
shared subsegments, and its raw proposal actually grows canonical physical
copper to 274.459 mm. The selectable `shared_route_graph` policy now inserts
vertices at collinear overlap, T contacts, and same-layer crossings before
compilation. With unselected same-net context it finds nine physical contact
points, inserts seven vertices, fixes three context nodes, and deduplicates 16
tension edges. Projected tension then improves 238.146 mm to 228.465 mm. A 3.2
mm local neighborhood is byte-identical to exhaustive
while reducing projection rows from 2,812,800 to 169,728 (16.6x). Both rungs
exact-pass. Sequence 023 retains the source and every rejected/intermediate
state as front/back images; sequence 030 adds the context-aware nonzero result,
and sequence 022 remains visible as superseded provenance.

The imported real `3V3_DUT` tree also supports nonzero fixed-via motion. Jointly
selecting two branches that share two vias moves their post-via junction and
reduces canonical copper from 236.746 mm to 232.763 mm while exact-passing
rungs 41 and 42. Selecting only one branch reaches 235.943 mm because the
common junction becomes fixed context. Treating both routes as independent
particles visibly splits their shared trunks and grows copper to 292.572 mm;
the transaction rolls back even though KiCad still considers that proposal
connected and DRC-clean. A 3.2 mm uniform-grid neighborhood is byte-identical
to exhaustive with 115,200 rather than 1,298,432 projection rows. Sequence 031
retains the source, both accepted controls, the rejected raw proposal, and the
byte-identical exhaustive render.

Sequence 032 replaces that manual `[1,2]` choice with bounded graph-local
selection. Starting only from branch 1, one nearest-shared-junction hop finds
branch 2 after 830 segment-contact tests and reproduces the manual candidate,
proposal, and complete board byte-for-byte. A `maximum_selected_branches=1`
control stops before engine compilation, emits no proposal, and retains the
source. All source, accepted, manual-control, and bounded-control boards are
rendered front/back.

Sequence 033 ports `layout-trace`'s transactional via removal and relocation
semantics to immutable KiCad route candidates. A private branch-8 via gets two
remove/merge actions and twelve bounded relocation trials; all fourteen are
applied to full rung-42 projects and native-gated. Direct removal saves two
vias on the front-layer variant but creates 19 DRC findings across the dense
routing field; the bottom-layer variant creates 28. Only two relocation sites
pass. Moving the via 1 mm north improves 0.269941 mm, and applying fixed-via
tension afterward reaches 236.000102 mm versus 236.270043 mm for fixed-via
tension alone. The identical 0.475866 mm continuous gain on both seeds means
the action adds a modest independent improvement; it does not establish a new
continuous basin. Every action and both controls are rendered front/back.

Sequence 034 replaces blind relocation sites with a bounded analytic-feasible
radial frontier around the same branch-8 via. The first implementation exposed
an important failure: all four native slots converged to micrometre variants
of one obstacle boundary. That preliminary portfolio remains rendered. A
0.1 mm score-ordered diversity filter then retains four distinct, analytically
clear relocations; all four pass native KiCad. Including the unchanged two
removal controls, this uses six native board evaluations instead of fourteen.
The selected raw route reaches 236.426303 mm and fixed-via tension reaches
235.950438 mm. This is 0.049665 mm better than the lattice control, but the
continuous gain is again identical, so it remains an additive local result.
The 646 cheap geometry probes are bounded samples, not a complete feasible-set
or general performance claim. All preliminary, corrected, and relaxed boards
are rendered front/back.

Sequence 035 composes the `layout-trace`-style remove/merge transaction with
the bounded single-layer A* kernel ported from `testing-esp32-duts`. The long
branch-8 excursion remains necessary under the zero-new-via policy: neither a
4 mm nor 12 mm window finds a same-layer repair at 0.5, 0.25, or 0.125 mm.
That negative result is not treated as a failed mechanism. A shorter private
branch-9 excursion provides the real-board control: all three front-layer
resolutions remove two vias and native-pass, while both direct chords fail.
The selected 0.25 mm action reduces canonical tree copper from 236.745909 mm
to 232.730760 mm and the physical via count from 12 to 10. The finer grid finds
a slightly shorter local path but a worse whole-tree union, so portfolio
selection continues to use canonical physical copper rather than local A*
cost. All 15 source, materialized, and no-path states have paired front/back
images in sequence 035.

Sequence 036 makes the negative half of that experiment inspectable. The
KiCad adapter now retains `no_path` and `budget_exhausted` as structured local
search outcomes, including the exact window, raster size, obstacle inflation,
blocked occupancy, blocked frontier, and work counters. Occupancy and frontier
cells use lossless row-run encoding rather than one JSON coordinate per cell.
Each failed action writes its topology-only candidate and paired top-down
front/back SVG/PNG diagnostics; these artifacts cannot enter candidate
selection. On the real branch-8 control, the overlays show the front endpoints
separated by dense horizontal/vertical occupied bands and the back search
closing against a diagonal bundle. Sequence 036 contains the native source
and direct-removal controls plus all six diagnostic pairs.

Sequence 037 turns that spatial failure into semantic feedback. The KiCad
adapter preserves whether each raster obstacle came from a footprint pad or a
foreign routed net, carries `Router_Movable` for component pads, and attributes
each frontier cell without double-counting overlapping primitives from one
owner. All 2,596 frontier cells across the six branch-8 searches have an owner.
On `F.Cu`, fixed connector `J4` is the largest boundary followed by `D_EN` and
`D_LOCAL_COMM0`; movable `C4` and `R19` are present but are not a sufficient
explanation by themselves. On `B.Cu`, `T_EN` and `GND` lead a wholly routed-net
frontier. Dominant identities persist across resolution while low-hit tail nets
vary, so hit count is diagnostic pressure rather than a claimed minimum cut.
The sequence repeats all nine front/back stages with semantic centroid labels.

Sequence 038 tests those labels causally instead of treating hit counts as
actions. A bounded conflict-guided breadth-first search suppresses one semantic
owner, reruns the same local router, and lets each failed child expose blockers
hidden behind its parent frontier. Reduced controls prove both a sufficient
single wall and a second wall discovered only after the first is suppressed.
On the real branch, all 50 generated `F.Cu` cuts of size one or two fail; on
`B.Cu`, suppressing routed `GND` alone opens a four-point 44.152543 mm
counterfactual path. These are non-selectable ablations, not board edits: the
source remains the only selected native-complete board. Every original failure
and all 73 trials have sequential front/back images in sequence 038.

The same continuous adapter attaches selected route endpoints to real KiCad
footprint bodies. Generic oriented body/body projection now preserves the
initial signed distance to motion-reachable courtyard neighbors. On rung 1,
two retained pairs let unrestricted X/Y tension move `R1` by 0.229717 mm and
reduce a fresh A* route from 19.617588 mm to 18.173812 mm; the complete rung
native-exact-passes. Preserving all 32 footprint distances was tested and
rejected because it froze the component. Free rotation now passes too, but
only rotates 0.003058°, so useful rotation is still an open problem rather
than a claimed success. Sequences 024–025 retain every control and front/back
board state.

A source-checked port of the predecessor's reference-field relaxer provides a
less restrictive discrete repair. Moving `R2`'s visible reference sideways
lets the unrestricted `R1` proposal retain its 1.767492° rotation and pass the
complete native KiCad gate, including declaration rematerialization. The four
inherited poses reduce to three unique actions here: two pass, one transfers
the collision to `R3`, and the fourth duplicates that failure. Route quality
does not improve—the repaired free result is 18.218363 mm versus 18.174384 mm
for the courtyard-constrained run—so this is retained as capability and search
evidence, not promoted as the preferred rung-1 solution. Sequence 026 contains
front/back images of every action and the translation control.

Generate one DRC-driven action without changing the input candidate:

```sh
cargo run -- relax-kicad-reference-fields \
  path/to/board.kicad_pcb path/to/candidate.json path/to/drc.json 2 \
  run/repaired-candidate.json run/reference-relaxation-evidence.json
```

The candidate records reference UUIDs and source poses and fails closed if the
board changed. The coordinator can now run the complete portfolio from a clean
materialized rung:

```sh
cargo run -- search-kicad-reference-fields \
  path/to/source-rung esp32-c3 path/to/proposal.json \
  run/reference-action-portfolio
```

It first reproduces and native-checks the failing proposal, uses that fresh DRC
as authority, removes equivalent actions, snapshots and native-gates every
unique candidate, and writes `portfolio.json` plus `selected-candidate.json`.
On rung 1 it retains three unique actions, rejects one, accepts two, and selects
phase 2 by the documented quality/displacement/phase ordering. Sequence 027
contains the baseline, every attempted board, and an already-valid no-op
control front/back. The no-op emits zero actions and returns its source
candidate byte-identically. Text readability beyond clearance and displacement
is not yet scored.

Native KiCad experiment images can be generated without the predecessor shell
scripts:

```sh
cargo run -- render-kicad-layers \
  artifacts/esp32-c3-ladder/42-gnd/esp32-c3.kicad_pcb \
  build/progress/020-kicad-experiment \
  experiments/configs/kicad-progress-render.json
```

Explore the first discrete conflict-action frontier from two crossing declared
seed routes, then inspect all six retained alternatives in the normal route
viewer:

```sh
cargo run -- explore-conflict-actions \
  benchmarks/small/conflict-action-orthogonal-crossing.json \
  run/conflict-actions.json \
  experiments/configs/conflict-action-default.json
cargo run -- view-route \
  benchmarks/small/conflict-action-orthogonal-crossing.json \
  run/conflict-actions.json run/conflict-actions.html
```

This depth-one baseline exact-gates A-north/south, B-west/east, and via-on-A/B
states and scores them by physical trace length plus a fixed cost per via. It
is a replaceable action producer above the geometric routers.

The bounded best-first control expands either of two independent crossings,
deduplicates equivalent candidates reached in opposite action orders, and
retains all 49 unique states for playback:

```sh
cargo run -- search-conflict-actions \
  benchmarks/small/conflict-action-two-crossings.json \
  run/conflict-action-best-first.json \
  experiments/configs/conflict-action-best-first-default.json
cargo run -- view-route \
  benchmarks/small/conflict-action-two-crossings.json \
  run/conflict-action-best-first.json run/conflict-action-best-first.html
```

This is bounded state search, not a global-optimality claim. The current action
producer is still limited to straight orthogonal conflicts. A replaceable
post-action processor can exact-gate local line-of-sight or vertex-pull
shortening before states are queued. On the shared-trace control, vertex pulling
reduces the selected 20 mm/via solution from 139.533 mm to 137.534 mm while the
viewer retains unsupported action orders. This CPU reference does not yet move
components or provide the intended particle/GPU shortening pass.

Run the independent oversized-board continuation baseline. The optional final
argument writes front/back SVG and PNG images for every shrink stage:

```sh
cargo run -- continue-board \
  benchmarks/small/board-continuation-clear.json \
  run/continuation-clear.json \
  experiments/configs/board-continuation-default.json \
  build/progress/009-continuation-clear
```

Stages are exact-gated transactionally. The clear control reaches the target;
the bottleneck control retains scale 1.2 and visibly rejects scale 1.1. The
full-reroute producer remains the control. A selectable retained producer now
affinely deforms prior copper and reroutes only branches implicated by exact
geometry findings. On the two-net control it searches 7 branch-stages instead
of 12 while retaining the unaffected route; optional collinear compaction
removes the oversized grid's sampling debt. Before rerouting, an optional
replaceable continuous processor can now push an implicated rectangular body
or trace interior and commit only an exact-complete proposal. On the yielding
body control this removes both late grid reroutes (2 initial branch searches
and 194 A* expansions versus 4/418 for retained grid repair); on the fixed-wall
control it bends the trace through five shrink stages without topology search.
Fixed rectangular explicit keepouts now lower as identity-preserving virtual
obstacles: the matched control removes five reroutes and reduces A* work from
46,716 to 9,969 expansions. Clearance-only motion was initially 4.85% longer;
reusing exact vertex pull barely helped, while a new projected trace-tension
edge kernel slides bends along the active clearance boundary. Eight tension
steps per stage produce 19.406395 mm and sixteen produce 18.872391 mm, versus
19.727922 mm for grid A*. A circular keepout remains visibly unsupported and
falls back safely. Projection, tension-edge, and post-process validation work
are reported separately from A* work; this remains a CPU reference, not a GPU
performance claim. Scrollable front/back progress images for all current board
experiments live in `build/progress/`.

Run the initial particle/field diagnostic with:

```sh
cargo run -- resistor-field run/resistor-field.html
cargo run -- resistor-ablation run/resistor-ablation
cargo run -- relax-continuous \
  benchmarks/small/continuous-resistor-coupling.json \
  run/continuous-relax.json run/continuous-relax.html preserve-seed
cargo run -- relax-continuous \
  benchmarks/small/continuous-multipin-tree.json \
  run/continuous-multipin.json run/continuous-multipin.html subdivide
```

Validate an imported declarative problem with the ported schema before giving
it to any engine:

```sh
cargo run -- check benchmarks/imported/layout-trace/squeeze.json
```

Run one of the ported initial placement policies (`declared`, `grid`, `random`,
`barycentric`, or `harmonic`), or pass a serialized placement config in place
of the policy name:

```sh
cargo run -- place \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/placement.json
```

Run both independent exact gates over a persisted candidate:

```sh
cargo run -- validate \
  benchmarks/smoke/placement-only.problem.json \
  benchmarks/smoke/placement-only.candidate.json
```

Route a placed problem with the DUT A* strategy. The result envelope retains
the candidate, exact assessment, per-branch status, search count, and expansion
work even when the command exits unsuccessfully:

```sh
cargo run -- route-grid \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  run/forced-crossing.route.json \
  experiments/configs/dut-grid-default.json
```

Run the separate negotiated-congestion policy with retained pass evidence:

```sh
cargo run --release -- route-grid-negotiated \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  run/forced-crossing-negotiated.json \
  experiments/configs/dut-grid-negotiated.json
```

Negotiated routes are provisional until both congestion and the independent
exact gates are clean. Algorithm changes are evaluated on the graded small
board ladder before any larger integration case.

Build corridor epochs or run joint route-family selection as independent
layout-trace strategies:

```sh
cargo run --release -- analyze-corridors \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  declared run/forced-corridors.json \
  experiments/configs/corridor-full-width.json

cargo run --release -- analyze-route-families \
  benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json \
  declared run/power-families.json \
  experiments/configs/route-family-default.json

# Natively generate bounded two-run families around alternative via sites.
cargo run -- analyze-route-families \
  benchmarks/small/family-seeded-via-obstacle.json \
  declared run/family-seeded-via-obstacle.result.json

# Import, select, and exact-materialize an externally generated V5 portfolio.
cargo run -- materialize-route-family-portfolio \
  benchmarks/small/family-multi-run-via.json \
  benchmarks/small/family-multi-run-via.portfolio.json \
  declared run/family-multi-run-via.result.json
```

The analysis command emits a common candidate only after exact joint family
assignment. On the four-branch slice it passes both independent final gates.
Fixed-context repair is also available for a persisted candidate: it selects a
bounded connected conflict component, treats all outside copper as fixed, and
accepts a proposal only when the independent exact score improves. It should
be used on a reduced conflict fixture before a system candidate.

Selected same-layer copper-clearance obligations also enter a bounded
continuous segment-separation pass. The result retains the pre-correction
frame, each correction frame, the exact proposal, and transactional
acceptance/rollback evidence. Render an attempted correction with:

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-obligation.json \
  declared run/family-clearance-obligation.json
cargo run -- view-continuous-repair \
  run/family-clearance-obligation.json run/family-clearance-obligation.html
```

That natural two-route control exhausts both family domains at one certified
class, emits one clearance obligation, pushes two vertically movable endpoint
bodies by 0.050940 mm, and passes both exact gates. Exhausted short frontiers
are accepted only when every returned raw class certified; truncated or
post-certification-short frontiers still fail closed.

The next reduced rung places a rectangular body keepout exactly beside the
same repair. Separate GPU-shaped segment/body rows redirect the correction
instead of allowing a new keepout violation:

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-body-neighborhood.json \
  declared run/family-clearance-body-neighborhood.json
cargo run -- view-continuous-repair \
  run/family-clearance-body-neighborhood.json \
  run/family-clearance-body-neighborhood.html
```

Initial placement can also be fed directly into the grid-routing boundary.
That particular command is composition rather than feedback; the family
obligation path above and the coordinator repair commands are coupled loops.

```sh
cargo run -- place-route-grid \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  random run/perturbation.route.json \
  experiments/configs/dut-grid-adaptive.json
```

This case records coarse-grid `no_path` at 0.5 and 0.25 mm before succeeding at
0.1 mm; the failed attempts and blocker identities remain in the result.

Run the first actual router-to-placer feedback experiment. Starting from the
harmonic pose, the router identifies `WALL` on the failed frontier; the
coordinator samples only that component's declared vertical degree of freedom,
reroutes transactionally, and selects an exact-passing candidate:

```sh
cargo run --release -- repair-route-grid \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/perturbation-repaired.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-axis-repair.json
```

This policy is intentionally a small baseline, not the eventual push/pull
engine. It moves one blocking component per proposal and does not yet learn a
continuous correction vector.

Run the selectable pressure-directed alternative on the same reduced control:

```sh
cargo run --release -- repair-route-pressure \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/wall-pressure.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-pressure-repair.json
```

The router supplies spatial blocked-frontier centroids and the coordinator
turns them into bounded component pushes. This first controlled result passes
exactly but costs two reroutes versus one for axis sampling because the inferred
pressure is orthogonal to the wall's legal motion. The unfavorable result and
diagnosis are retained rather than hidden.

Pressure sources, component ordering, propagation, and direction exploration
are independent policies. The dual-board experiments add failed-branch
endpoint pulls, multi-branch-first blocker ordering, maximum-distance pull
chains, and a bounded bidirectional control. These extend rather than replace
the predecessor's blocker-only, hit-count, one-direction policy.

Beam width and topology repair are independent too. A width-four beam can
retain neutral, pose-distinct children and records every parent/child edge.
When `selective_ripup` is present in the pressure config, each routed placement
child may transactionally repair its already-routed baseline without repeating
the initial search. The mixed dual-prefix-14 experiment needs both a
pad-envelope four-component push and one rip-up to reach 19/19. Every imported
or proposed placement is gated first; copper-bearing component envelopes use
board-rule clearance, so a semantic success cannot silently enter KiCad with
touching pads.

The selectable passage-capacity producer projects pressure into legal motion
by measuring neighboring channel deficits and accounting for the routing grid:

```sh
cargo run --release -- repair-route-pressure \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/wall-passage.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-passage-repair.json
```

It restores the symmetric wall case to one repair attempt. On the asymmetric
one-wall control it and raw frontier pressure use one attempt and 436,739
expansions, while axis sampling uses three attempts and 868,969 expansions.

The collision-propagating variant is a separate selectable experiment. On its
two-body reduced control, `WALL` cannot make the required move without
overlapping the movable `FOLLOWER`; the bounded chain translates both by the
same vector and reroutes transactionally:

```sh
cargo run --release -- repair-route-pressure \
  benchmarks/small/passage-pressure-push-chain.json \
  declared run/wall-push-chain.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-passage-push-chain.json
```

This exact-passes in one repair. The single-body pressure control and the
eight-proposal axis control remain incomplete on the same input.

Optimize a routed multi-terminal graph with an explicit junction transaction:

```sh
cargo run --release -- optimize-route-junctions \
  benchmarks/imported/layout-trace/multi-terminal-tree.json \
  declared run/multi-terminal-junction.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/route-junction-adjacent.json
```

The reduced control splits one branch, reattaches its sibling to a stable
degree-three junction, remains exact-complete, and reduces copper length from
49.426407 mm to 49.219300 mm without another route search. This is the explicit
topology primitive now reused by search-time attachment to existing same-net
copper.

Run the imported shared-copper-tree policy directly on the same reduced
control:

```sh
cargo run --release -- route-grid \
  benchmarks/imported/layout-trace/multi-terminal-tree.json \
  run/multi-terminal-shared-tree.json \
  experiments/configs/dut-grid-shared-tree.json
```

It starts with a stable root/nearest-terminal trunk. Later growth dynamically
selects across all current tree vertices and pending terminals, then trims the
searched prefix through the last state already owned by the tree. Non-parallel
same-layer contacts inside either segment are inserted into both polylines;
every interior contact becomes an explicit split and junction. On this fixture the
result passes both exact gates, reduces copper to 46.355339 mm, and uses 144
A* expansions versus 148 for the terminal-MST control.

`shared-tree-dynamic-terminal.json` proves that current tree geometry changes
the next terminal choice. `shared-tree-prefix-trim.json` forces 15 duplicate
prefix points before an obstacle escape; trimming reduces its shared result
from a would-be 52.363961 mm to 44.863961 mm, versus 45.899495 mm for MST.
The route viewer marks committed attachments in green, searched sources moved
by trimming in amber, durable junctions with magenta diamonds, and contacts
inserted inside segments with a white X.

Evaluate the optional connected-terminal preference independently:

```sh
cargo run --release -- route-grid \
  benchmarks/small/shared-tree-terminal-junction.json \
  run/shared-terminal-junction.json \
  experiments/configs/dut-grid-shared-terminal-junction.json
```

With 1.0 mm slack it spends one extra search (42 versus 26 expansions), removes
the ordinary interior junction, and reduces copper from 12.414214 mm to
12.242641 mm. The option is disabled by default, alternative searches have an
explicit bound, and the viewer marks a selected terminal alternative with a
violet halo.

Evaluate a general bounded portfolio of geometrically distinct tree sources:

```sh
cargo run --release -- route-grid \
  benchmarks/small/shared-tree-attachment-portfolio.json \
  run/shared-tree-attachment-portfolio.json \
  experiments/configs/dut-grid-shared-portfolio.json
```

The two-source control selects an interior point which the greedy distance
estimate misses. It reduces exact-valid copper from 19.242641 mm to 17.828427
mm while increasing expansions from 368 to 500. The objective is selectable,
the candidate count and spacing are explicit bounds, and every attempted
source remains in evidence. Blue/gray viewer rings identify selected/rejected
portfolio sources. The experiment stays disabled by default because two other
reduced controls show extra work without a quality gain.

Failure-directed ordering and selective rip-up remain selectable experiments,
but their next evaluations belong on reduced cases with one known failure
mechanism. Their old dual-board measurements are retained only as historical
evidence.

Inspect successful or failed route artifacts in the self-contained viewer.
Coordinator results provide attempt playback, blocked-frontier centroids, and
proposed motion arrows. Rejected pressure attempts are retained using their
parent placement, and multi-body pushes draw one arrow per moved body:

```sh
cargo run -- view-route \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  run/forced-crossing.route.json run/forced-crossing.html
```

Open `run/resistor-field.html`, or the endpoint/rigid field-load and rotation
viewers generated from the ablation prefix. Run the full current verification
surface with:

```sh
cargo test --workspace --all-targets
```

For the fast active routing feedback loop:

```sh
cargo test -p pcb-routing small_board_ladder_is_exact_and_bounded -- --nocapture
```

The architecture, subsystem-by-subsystem migration status, conflict policy,
and protocol for investigating unfavorable experiments are documented in
[`docs/architecture.md`](docs/architecture.md) and
[`docs/migration-audit.md`](docs/migration-audit.md).

The first controlled representation ablation and its current tradeoffs are
recorded in
[`docs/experiments/two-terminal-particles.md`](docs/experiments/two-terminal-particles.md).
The first coupled continuous-to-candidate exact pass is recorded in
[`docs/experiments/continuous-candidate-bridge.md`](docs/experiments/continuous-candidate-bridge.md).
The first controlled placement ablation, including an unfavorable result that
exposed a shared-projector bug, is in
[`docs/experiments/harmonic-dual-esp32.md`](docs/experiments/harmonic-dual-esp32.md).
The first semantic DUT-router evaluation, including the current failed
placement-feedback case, is in
[`docs/experiments/dut-grid-semantic-port.md`](docs/experiments/dut-grid-semantic-port.md).
The pressure-directed feedback port and its unfavorable first comparison are
in
[`docs/experiments/pressure-directed-blocker-repair.md`](docs/experiments/pressure-directed-blocker-repair.md).
The first exact route-graph topology transaction and its rollback control are
in
[`docs/experiments/route-graph-junctions.md`](docs/experiments/route-graph-junctions.md).
The active graded feedback loop and current nearby-case measurements are in
[`docs/experiments/small-board-ladder.md`](docs/experiments/small-board-ladder.md).
The 43-component/83-branch scaling baseline and route-order comparison is
historical evidence in
[`docs/experiments/dual-esp32-grid-baseline.md`](docs/experiments/dual-esp32-grid-baseline.md).
The native 52-connection restart, including the placement-gate failures and
first complete routed rung, is in
[`docs/experiments/dual-esp32-native-ladder.md`](docs/experiments/dual-esp32-native-ladder.md).
The first thin-route/continuous-width composition, including its complete
prefix-19 topology, exact 10% stage, and diagnosed 15% rollback, is in
[`docs/experiments/dual-esp32-width-continuation.md`](docs/experiments/dual-esp32-width-continuation.md).
The corridor/route-family port, its exact-complete power-slice result, and its
bounded dual-board failure are in
[`docs/experiments/layout-trace-routing-port.md`](docs/experiments/layout-trace-routing-port.md).
The reduced native and imported multi-run/via results, blocked-site control,
fail-closed certificates, and current producer boundary are in
[`docs/experiments/multi-run-family-via.md`](docs/experiments/multi-run-family-via.md).
The concise current capability boundary and priority order are in
[`docs/current-status.md`](docs/current-status.md).
The native future-demand field, learned net ordering, ECC83/PIC improvements,
and the failed pairwise-order control are recorded in
[`docs/reviews/2026-09-08-routing-demand-and-order.md`](docs/reviews/2026-09-08-routing-demand-and-order.md).
The integrated `route-kicad-board-adaptive` command, failure recovery and
native-complete incumbent retention are documented in
[`docs/reviews/2026-09-08-adaptive-native-routing.md`](docs/reviews/2026-09-08-adaptive-native-routing.md).
Adaptive routing now stops at native connectivity completion by default;
`optimize_after_routing_complete: true` explicitly enables further quality
passes. Full-layout admission still requires the complete native gate.
Automatic via-opportunity scanning and its explicit importer coverage are in
[`docs/reviews/2026-09-08-via-discovery.md`](docs/reviews/2026-09-08-via-discovery.md).
The market survey, measurable definition of “beat,” constrained auto-placement
strategy, and gated implementation roadmap are in
[`docs/competitive-roadmap.md`](docs/competitive-roadmap.md).
The first non-convex medium-real cold route, including legacy-link
normalization, dense-pad multiresolution diagnosis, and ordered-fallback
evaluation, is in
[`docs/experiments/interf-u-cold-baseline.md`](docs/experiments/interf-u-cold-baseline.md).
The fifth pinned real-source adapter, hierarchical schematic normalization,
cold preparation, source-candidate rejections, and one-net native smoke are in
[`docs/experiments/complex-hierarchy-cold-baseline.md`](docs/experiments/complex-hierarchy-cold-baseline.md).

An optional source-preserving Freerouting backend is available through
`route-kicad-board-freerouting` and the native placement driver. Configuration,
requirements and completion semantics are in
[`docs/native-external-routing.md`](docs/native-external-routing.md).
