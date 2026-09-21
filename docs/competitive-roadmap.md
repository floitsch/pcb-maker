# Competitive survey and roadmap

> **Trust warning (2026-09-21):** the capability claims below were not
> confirmed by a fresh audit. Read
> [the trust audit](reviews/2026-09-21-trust-audit.md) first; it supersedes this document where they disagree.

Status: strategy baseline, 2026-09-03.

## Current execution priority (2026-09-08)

The strategy below is a long-term target, not an instruction to pursue every
mechanism in parallel. The user's priority is a general autonomous placer/router.
Local via and length work is retained in
[deferred optimizations](optimization-todos.md); use
[current status](current-status.md) for measured capabilities.

The next milestone is a repeatable complete-layout pipeline across the pinned
real-board families:

1. **Preserve the input contract.** Carry every component, net, source rule,
   keepout and declared mechanical constraint through import, placement,
   routing and export. Report unsupported features before expensive search.
   Keep unsupported and untested boards visible in the coverage denominator.
2. **Finish routing.** Use fixed reference placements to distinguish routing
   weaknesses from placement weaknesses. Prioritize recovery that completes
   blocked boards, with bounded work and resumable verified progress.
3. **Close the placement feedback loop.** When routing stalls, use its failure
   evidence to propose moves or rotations of unconstrained components and
   reroute affected nets. Compare against independent cold placement seeds;
   do not assume a failed routing search proves the placement infeasible.
4. **Measure complete layouts.** Run from zero copper and, for the placement
   track, regenerate movable poses. Require complete native-verified results
   before claiming smaller board area. Compare compatible external routers
   under matching inputs, rules and budgets.

A production change should address a recorded whole-board failure and retain
a regression on another board family. Focused fixtures remain useful to
explain the mechanism, but do not substitute for the whole-board result.
Archive combined front/back copper renders automatically for each experiment,
including failed candidates. Avoid expanding a local optimization campaign
unless it directly helps completion.

The latest concrete example is [net-aware cold placement](reviews/2026-09-08-placement-unlocks-routing.md):
the unchanged router connects all 50 nets on a new initial placement, while
the matched random placement's first pass leaves 30 opens. This directs the
next work toward general placement policies and whole-board coverage. Three
annotation findings still prevent complete-layout admission. The successful
seed is integrated as an explicit option; it is not yet a general solution
for anchored or disconnected designs.

This project is intended to become a better autonomous PCB placer/router, not
merely a place to demonstrate novel algorithms. Research is useful only when it
improves a board-level outcome under an exact, reproducible comparison.

The initial target is deliberately narrower than "all PCB layout":

> Generate fabrication-ready placement and routing for constrained 2--6 layer
> digital and mixed-signal boards, from a cold semantic input, with no manual
> cleanup, and beat mature routers on completion, achievable board size/layer
> count, or engineering turnaround.

RF layout, flex, HDI/package co-design, and sign-off-grade SI/PI optimization
are later targets. A tool that incompletely supports everything does not beat a
tool that reliably finishes a useful class of boards.

## What “beat” means

There is no honest single scalar score for PCB layout. The comparison is
lexicographic, with the hard results first:

1. The output imports into the target KiCad version and passes native
   connectivity and copper DRC with no waivers attributable to the engine.
2. The whole board is connected at declared trace widths, clearances, via
   geometry, layer rules, and placement constraints.
3. Fewer signal layers and smaller feasible board area win before millimetres
   of trace length.
4. Within equal completion/area/layer results, compare vias, physical copper,
   plane continuity, manufacturability, and constrained-net quality.
5. Compare reproducible work and end-to-end wall time only after the quality
   tier is equal.

“Seven millimetres less copper” is not progress when either result leaves a
connection open. A thin-route or enlarged-board intermediate is evidence, not
a successful board.

Three product claims are tracked separately:

- **Route parity:** match mature autorouters on fixed legal placements.
- **Autonomous layout:** legally place and route from constraints without a
  human repairing the result.
- **Coupling advantage:** at equal compute, bidirectional placement/routing
  beats a sequential place-then-route system. This is a hypothesis to test,
  not an architectural article of faith.

## Market and research survey

Product vendors rarely publish enough detail to reproduce their algorithms.
The table distinguishes documented mechanisms from vendor claims; marketing
claims are not treated as benchmark results.

| System | Documented operating model | What it establishes for us | Opportunity or limitation |
| --- | --- | --- | --- |
| [Freerouting](https://github.com/freerouting/freerouting) | Open source, rules-aware maze/expansion routing. Its documented pipeline finds incomplete connections in repeated passes, may rip up blocking copper, tightens inserted paths, and then transactionally reroutes candidates to optimize length and vias. It supports DSN/SES exchange. | This is the minimum reproducible autonomous routing baseline. Mature rip-up/retry and route ordering matter more than giving one A* search a larger budget. | It does not solve constrained component placement. Its GPL implementation cannot be copied into this MIT project, but black-box results and documented ideas can be compared. |
| [KiCad PCB Editor](https://docs.kicad.org/master/en/pcbnew/pcbnew.html#routing-tracks) | Human-directed interactive router with highlight-collision, walk-around, and shove modes; it can shove tracks/vias and observes a broad native rules system. It exports Specctra DSN rather than supplying a full-board autorouter. | Sets the integration, exact-rule, interactive-repair, and editability bar. KiCad's DRC is our independent admission authority. | The human supplies the global decisions, so it is not the direct autonomous baseline. We should complement it, not rebuild an editor first. |
| [Altium Designer](https://www.altium.com/documentation/altium-designer/pcb/routing) | Interactive walkaround/hug-and-push/push routing and glossing; guided multi-net ActiveRoute; and the [Situs topological autorouter](https://www.altium.com/documentation/altium-designer/pcb/routing/situs-topological-autorouter), which first maps topological paths and then applies routing strategies and fanout passes under design rules. | Shows the value of separating topology from geometric realization, retaining user guidance, automatic fanout, and post-route cleanup. It also illustrates the breadth of rules expected in a production tool. | Full-board autorouting remains sensitive to preparation and strategy; its own documentation recommends manual handling for problem areas and differential pairs. Placement is principally designer-led. |
| [Zuken CR-8000 / Dragon EX](https://www.zuken.com/en/product/cr-8000/multi-instance-interactive-and-automatic-routing/) | Designers define multiple strategy-driven areas; a routing consultant derives strategies from objects/rules; the tool simultaneously autoroutes and generates escape/fanout patterns. Zuken also markets [AIPR](https://www.zuken.com/en/product/cr-8000/ai-pcb-design/): routing informed by learned design examples and customer practice. | A direct production competitor for constraint-driven automation and reuse of design knowledge. Area-local strategies are a useful analogue to our overlapping tiles, provided long nets retain global context. | Public material does not expose a reproducible algorithm or benchmark. Its page describes Smart Autorouter as usable after manual placement and Smart Placement as under development, so availability and capability must be tested rather than inferred from the “AI” label. |
| [Siemens Xpedition Enterprise](https://www.siemens.com/en-us/products/pcb/xpedition/enterprise/) | Integrated, constraint- and verification-heavy flow from system definition through manufacture, with design automation and concurrent design as parts of the platform. Detailed placer/router internals are proprietary. | Establishes that rule breadth, system/MCAD context, collaboration, and verification are product requirements even if our first target is narrower. | It is not possible to infer an algorithmic advantage from public high-level material. A licensed, scripted evaluation is needed before making comparative claims. |
| [Cadence Allegro X](https://www.cadence.com/en_US/home/tools/pcb-design-and-analysis/allegro-x-design-platform.html) | Mature constraint-driven PCB implementation platform with interactive and automated layout capabilities, integrated analysis, and production data management. Algorithmic internals are proprietary. | Along with Xpedition, this is the production-rule and high-speed-flow bar, not a useful source-code template. | It can only become a quantitative baseline when a licensed evaluator can run our frozen corpus without manual board-specific repair. Until then, claims about relative quality are out of scope. |
| [Quilter](https://www.quilter.ai/product/technology) | Vendor describes a reinforcement-learning engine that explores thousands of synthetic candidate boards, balances multiple objectives, validates against physics, and automates both placement and routing. Its documented workflow returns candidates for engineer review and typically uses repeated review/resubmit cycles. | This is the closest strategic competitor. Coupling placement and routing, candidate portfolios, and physics-aware evaluation are not unique claims. | It is proprietary and its public results are not a reproducible neutral benchmark. We can differentiate through local/open execution, exact inspectable lineages, incremental repair, and stronger results—but only after measuring them. |
| [DREAMPlace](https://github.com/limbo018/DREAMPlace) and related VLSI research | Formulates global placement as differentiable wirelength plus electrostatic-style density optimization using tensor/GPU kernels, followed by legalization and detailed placement. The project reports large GPU speedups on million-cell IC benchmarks. | Strong evidence that dense demand/density fields and batched placement operators fit GPUs. It motivates our Eulerian board fields and representation compiler. | IC cells, routing grids, and benchmark objectives do not model PCB component geometry, free rotations, vias, layer changes, courtyards, or native PCB DRC. It is an inspiration and kernel reference, not a PCB baseline. |

Dedicated autorouters such as Electra and TopoR and code-first systems such as
JITX/tscircuit belong in the eventual procurement survey. They do not change
the immediate architectural conclusion: first match a transparent mature
router, then compare autonomous placement/layout against the strongest systems
we can actually run. Product breadth claims that cannot be exercised on the
same frozen inputs are recorded but not scored.

## Evidence from our current system

The project already has several valuable foundations:

- immutable semantic input and transactional candidates;
- native KiCad ERC/DRC/connectivity/parity admission;
- cold placement portfolios and several legal placement policies;
- bounded A*, shared-tree routing, selective rip-up mechanisms, explicit
  conflict actions, and typed failure/frontier evidence;
- continuous particle/analytic-body experiments, trace tension, and component
  motion behind representation compilers;
- oversized-board and thin-to-full-width continuation experiments;
- reproducible manifests, lineages, viewer evidence, and paired front/back
  renders.

It is not yet competitive:

- the dual-ESP32 prefix-19 board stalls at 23/24 after conflict-cover repair;
- continuous width growth reaches only 25% of declared trace width on that
  topology;
- routing lacks a robust whole-board multi-pass rip-up/retry loop, terminal
  escape quality, congestion history, and enough topology diversity;
- placement has useful mechanisms but no production constraint language,
  global routability model, or proven advantage from coupling;
- important production rules and geometry are missing;
- there is no GPU backend.

The decisive control is retained in
[`dual-esp32-freerouting-baseline.md`](experiments/dual-esp32-freerouting-baseline.md).
On the identical cold prefix-19 placement, at 0.25 mm width, 0.20 mm clearance,
and 0.70/0.30 mm vias, Freerouting completes 24/24 branches in seven passes and
5.90 seconds. The imported board has 161 segments, 16 vias, zero unconnected
items, and zero copper DRC findings. The placement is therefore routable. Our
failure is currently conventional router capability, not lack of compute and
not evidence that the components must move.

## Strategy

### 1. Earn table stakes before optimizing novelty

Implement, independently, the capabilities common to successful routers:

- exact obstacle geometry and a spatial index;
- terminal escape/fanout;
- multi-layer path search with configurable via/layer costs;
- congestion history and route-order diversity;
- connectivity-safe selective and regional rip-up;
- repeated whole-board passes with rollback;
- route tightening and via cleanup;
- planes/zones and a useful subset of production constraints.

This does not mean copying Freerouting. It means accepting that a novel
continuous engine cannot rescue every poor topological commitment.

### 2. Make constrained auto-placement a first-class solver

The semantic problem must express, at minimum:

- fixed, movable, and optional components;
- legal side, orientation sets or free rotation, and allowed regions;
- body, courtyard, height, edge, connector, and keepout clearances;
- relative/group constraints, alignment, ordering, and proximity limits;
- thermal/mechanical reservations and later MCAD obstacles;
- net criticality, differential-pair membership, length/skew classes, and
  power/return-path intent.

The first placer should be a portfolio, not a single fashionable algorithm:

- analytical/field placement for global movement and rotation;
- exact legalization;
- discrete local search for swaps, side changes, rotations, and group moves;
- seeded stochastic alternatives for escaping poor basins;
- quick routing probes as a routability objective.

A resistor may be an analytic rigid body, a distance-constrained particle
pair, or a shape-matching cluster inside a particular experiment. It remains a
component with exact geometry and constraints in the semantic model. No
component class is permanently encoded into a GPU kernel.

### 3. Couple through typed obligations and transactions

The router should not emit only a scalar “bad placement” score. It should emit
actionable evidence:

- terminal cannot escape in these directions;
- this cut lacks a specified amount of capacity;
- these routes/components form the blocking set;
- moving/rotating this endpoint would open a corridor;
- a differential pair needs a corridor of a particular width;
- the current topology needs a via, layer, ordering, or corridor decision.

The placer, discrete router, and continuous engine respond with candidate
transactions. A transaction states which poses, route branches, layers, vias,
or zones it changes and what it invalidates. It is committed only after exact
validation. Router-originated component motion is therefore allowed without
creating untraceable state.

Coupling is accepted only after a fixed-compute ablation against:

1. fixed/declared placement plus routing;
2. sequential constrained placement followed by routing;
3. iterative placement scored by routing but unable to change routed state;
4. fully bidirectional placement/routing transactions.

### 4. Use three representations, not one universal metaphor

- **Discrete topology:** connectivity, ordering, corridor/homotopy, layer and
  via choices, rip-up sets, and board-outline stages.
- **Continuous geometry:** exact or sampled shapes, trace vertices, body poses,
  attachment constraints, clearance contacts, and trace tension.
- **Dense fields:** per-layer occupancy, capacity, congestion, return-path and
  density pressure on overlapping multiresolution tiles.

The field is advice and a source of gradients; exact geometry remains the
authority. Global coarse fields plus overlapping tiles avoid hard boundaries
that would break long nets. A representation compiler allows particle,
analytic-body, shape-cluster, and future solvers to coexist.

### 5. Put the GPU where parallelism exists

Do not begin by porting a single serial A* search. Initial GPU candidates are:

- scatter/gather of component and copper demand;
- stencil/multigrid density and congestion fields;
- broad-phase collision/contact generation;
- batched placement candidates and continuous constraint projection;
- parallel reachability/cost fields and counterfactual blocker tests;
- route portfolios across orders, costs, and rip-up sets.

CPU code retains discrete orchestration and exact validation. A GPU feature is
promoted only when transfer-inclusive end-to-end results justify its
complexity.

### 6. Pursue two high-risk differentiators after parity

**Make-room continuation:** start with a legal enlarged board or narrow-trace
topology, then continuously approach the target outline/width. Trace tension,
component motion, via motion, and pressure fields preserve or restore space;
the discrete loop repairs topology when continuation reaches a blocker.

**Conflict-decision search:** search over meaningful repair actions rather than
only geometric grid cells: route A north/south of B, route B west/east of A,
add/move a via on either route, change a corridor/layer, move/rotate a component,
or rip up a blocking set. Fast continuous relaxation estimates and tightens
each child; exact geometry decides admission.

These are competing/composable experiments. Neither is the default until it
solves holdout boards that the conventional baseline cannot solve under the
same budget.

## Benchmark contract

Milestone 0 freezes two tracks:

- **Route-only:** preserve a supplied legal placement and start with zero
  copper.
- **Autonomous layout:** preserve only declared fixed poses and placement
  constraints; regenerate every other pose and all copper from the semantic
  input.

The corpus has four tiers:

| Tier | Contents | Initial size |
| --- | --- | ---: |
| Mechanism | Crossings, escapes, shared nets, vias, planes, rotations, local moves, diff pairs, and deliberate bottlenecks | 20 |
| Small real | Open 2-layer boards, roughly 10--75 components and 10--150 connections | 12 |
| Medium real | Open 2--6 layer boards, roughly 75--500 components and 150--2,000 connections | 8 |
| Adversarial/holdout | Dual-ESP32, squeeze/shrink cases, randomized legal placements, and unseen variants | 10 |

Every scored run records input/config/tool hashes, seed, hard work and time
budgets, machine identity, peak memory, native findings, completion, board
area, signal layers, vias, unique physical copper, rule-specific quality, and
paired front/back images. Every failure retains its last exact-valid board and
typed blocker evidence. Prefix growth is diagnostic only: every prefix is a
cold rerun and cannot inherit placement or routing decisions.

Baseline policy:

- Freerouting runs automatically on every compatible route-only board.
- KiCad is the exact validator and interactive quality reference.
- Altium, Cadence, Siemens, Zuken, and Quilter enter the quantitative table
  only when a licensed operator can run a frozen input without board-specific
  manual repair and return the final board plus settings and elapsed time.
- Public claims and hand-routed boards are labelled separately.
- Twenty percent of real boards and generated variants remain hidden until a
  milestone candidate is frozen.

## Roadmap and acceptance gates

Effort ranges are focused engineering weeks, not calendar promises. A gate is
not passed by elapsed time; an unsuccessful research result is retained and
the architecture remains usable by the next experiment.

### M0 — Competitive harness and frozen corpus (2--4 weeks)

Deliverables:

- versioned corpus manifests and licenses;
- cold route-only and autonomous-layout runners;
- DSN/SES Freerouting adapter and native KiCad import/validation;
- canonical physical-copper, via, area, layer, runtime, and completion metrics;
- paired renders, machine-readable lineage, failure classification, and result
  comparison report;
- native-process result cache keyed by complete input/tool/config hashes.

Acceptance:

- at least 20 mechanism, 12 small-real, 8 medium-real, and 10 holdout cases are
  declared; at least half of the real corpus is executable immediately;
- one command reruns ours and Freerouting from zero copper and produces a
  self-contained report;
- three same-seed runs have identical semantic result hashes, or explicitly
  declared nondeterministic distributions;
- no result is marked complete unless native KiCad reports zero connectivity
  and relevant DRC findings;
- prefix-19 sequence 193 is reproduced from the manifest, including its 24/24
  Freerouting result.

Implementation status on 2026-09-03: the versioned cold route-only schema,
semantic source regeneration, exact-rule DSN/SES adapter, bounded external
process, native KiCad admission, failure checkpoints, raw provenance hashes,
order-independent copper digest, and mandatory paired renders are implemented.
Sequence 198 is the current one-command prefix-19 Freerouting reproduction.
Sequences 193, 196, 197, and 198 agree on the exact copper digest and native
result; volatile KiCad UUIDs and serialization order are explicitly separated
from semantic decisions. Sequence 201 adds the first combined report: both
pcb-maker and Freerouting solve cold dual-ESP32 prefix 3 under matching rules
and placement, with separate exact-gated boards, timings, logs, hashes, and
renders. Sequences 204/205 add a fail-closed, content-addressed native KiCad
verification cache: the warm run restores seven distinct exact results and
reduces pcb-maker process time from 38.824 to 3.965 seconds without changing
the final board bytes. Sequence 210 adds the checkpointed corpus runner and
proves the independent cold-growth pipeline at prefixes 0--3: all four cases
finish and both routers exact-complete every case. Sequence 211 adds common
physical centerline, overlap, width-weighted track area, via/drill/annulus,
refilled-zone, rectangular-board-area, and per-layer metrics. The current
straight-track corpus has exact centerline union; primitive area limitations
remain explicit and do not affect completion ranking. Sequence 215 adds six
independent mechanism inputs after retained template/legacy-pad failures in
sequences 212 and 214. Both routers exact-complete all six; the paired runner
also no longer censors pcb-maker when Freerouting is incomplete. Sequence 218
expands this to twelve both-solved mechanisms with timeout-aware partial-result
classification. Sequence 221 adds the first executable pinned real-board
source. Its cold adapter removes inherited tracks/vias/arcs/copper zones while
retaining placement, project semantics, and rule-area keepouts; baseline-aware
native admission permits only findings already present identically in the
frozen source and never permits remaining connectivity. Freerouting completes
the ECC83 fixture's 20 initial unrouted items in two passes with zero
introduced native findings. M0 remains open: twelve mechanisms, four pinned
real-source declarations, and three executable real sources are still short of
the declared 20/12/8/10 corpus. Sequence 224 adds a generic direct sequential
route mode and the first formal real-board paired result: both routers connect
all 20 native ECC83 items from the same frozen project with no introduced
native findings. Freerouting uses 51 segments and no vias; pcb-maker uses 75
segments and 10 vias. Ordinary-KiCad ingestion is therefore operational on one
small fixture, while real-corpus breadth and topology quality remain open.
Sequences 225--230 add the 63-footprint PIC programmer as an executable but
unsolved real case. Freerouting reaches 124/125. pcb-maker's new per-net native
transaction gate retains four exact-clean nets, then identifies a custom-pad
terminal-escape blocker; invalid raster-valid children no longer contaminate
the durable prefix.
Sequence 231 structurally adapts the 61-footprint Olimex ESP32-C3 source, then
stops it at rule preflight: the uniform DSN contract loses the board's 1.85 mm
local mounting-hole pad clearances, and the apparent Freerouting completion has
17 introduced native DRC findings plus two opens. It is not counted as a
scoreable third real source. Equivalent lowering or an explicit unsupported-rule
rejection is required before either router may be compared on it.
Sequence 232 replaces the custom-pad anchor tail with selectable first-contact
clipping against explicitly lowered filled polygon copper. It exact-commits
JP1 and the following net, advancing the clean PIC frontier from four to six
nets (69 to 54 remaining native items) while shortening the affected route by
12.035 mm. The next failure is an exhaustively unreachable `DATA-RB7` branch,
which moves this case from terminal escape to M1 multi-pass/order repair.
Sequences 233--238 prove that failure-directed order is material on the same
fixed placement. Bounded yielding counterfactuals derive `DATA-RB7 -> GND ->
VCC`; an independent zero-copper replay reaches 16/34 clean nets and 31
remaining native items, versus six nets/54 items for discovery order. The next
failure, `PC-DATA-IN`, is causally unblocked by yielding `DATA-RB7` or GND, so
the next deliverable is a bounded ordinary-KiCad order coordinator that
regenerates each lineage from zero copper. Route-only native admission also
cuts typical step time about 43% by hashing/reusing immutable ERC inputs while
retaining per-step PCB DRC, parity, and connectivity authority.
Sequence 239 implements that coordinator. Its base trial reproduces the
16-net failure; a causally generated child moves `PC-DATA-IN` before
`DATA-RB7`, reroutes from the immutable cold source, and reaches the configured
17-net/30-item bound with zero native findings. Lineage, proposals, diagnoses,
full trials, and completion-first selection are serialized. Wider bounded
search, regional rip-up, and the still-poor 57-via topology remain M1 work.

Sequences 240--244 admit the fourth pinned declaration, KiCad `interf_u`, as
the third executable real source and first executable medium-real source. The
adapter now reconstructs closed non-convex line-loop outlines and blocks exact
polygon boundary transitions; a copied-source normalization repairs obsolete
footprint-library links without changing the PCB. Its cold frontier reaches a
natively admitted 16/110 nets. Dense PGA escape requires a 0.125 mm fallback;
0.5 and 0.25 mm have disconnected graphs regardless of A* budget. An explicit
ordered-first-admitted portfolio cuts expansions 678,377 -> 84,511 and routed
step time 235.664 -> 137.534 seconds versus exhaustive resolution scoring.
Sequences 245--246 add layer-aware obstacle broad-phase rasterization and
reproduce the final board byte-for-byte in 94.104 routed-step seconds, a 60.1%
reduction from the exhaustive baseline.
Sequences 247--250 add KiCad's 68-footprint hierarchical analog demo as the
fifth pinned declaration and fourth executable real source. The first
root-only repair leaves 58 parity mismatches; a general hierarchy traversal
then normalizes the referenced child sheet, preserves the PCB byte-for-byte,
and reaches zero source ERC/DRC/parity findings. Cold preparation removes 364
tracks, 166 copper zones, and two copper graphics while preserving placement
and area. A one-net `-VAA` smoke reduces 112 -> 109 native opens with 14
segments/no vias. It also fixes stale result verification artifacts by
committing the admitted board and native reports together; the replay is
board-byte-identical. Sequences 253--257 add the first admitted
autonomous-placement mechanism. Negative controls expose unattached movable
bodies and under-anchored connected blocks without counting them as placement
successes. The accepted case proves translation, free rotation,
connectivity-distance improvement, identical cold input to both routers, and
two native-complete outputs. These properties are now executable per-manifest
obligations rather than interpretation of an image. Sequence 259 adds the
fourteenth mechanism: F.Cu-only to B.Cu-only SMD connectivity. Both routers
native-complete with one via/two used layers, and the manifest enforces those
properties independently. Sequences 260--270 add a fixed-rule-area bottleneck
and expose/fix silent keepout loss in progressive materialization and grid
lowering. The 3 mm both-layer passage fails exhaustive 0.25 mm reachability but
both-solves at 0.125 mm, identifying resolution fallback rather than more A*
budget as the required mechanism. At sequence 270, M0 remained open at fifteen mechanisms, five
pinned real declarations, and four executable real sources; the required
20/12/8/10 corpus breadth is unchanged. The historical twelve-case aggregate
must be replayed under corrected keepout semantics. Sequence 271 adds a
render-only rule-area overlay, so subsequent front/back journal images expose
these constraints without altering any board accepted by native verification.
Sequences 272--275 add the sixteenth mechanism and a generic post-route
processor seam. Identical cold results can replace a named routed connection
with front/back zones, retain stitch vias, refill through KiCad, and face the
same native admission. The first GND control both-solves with two zones and at
least one via. Sequences 276--277 persist pcbnew's filled polygons, measure
2,049.837046 mm2 on both outputs, and enforce a 1,800 mm2 minimum so empty zone
declarations cannot satisfy the mechanism.
Sequences 279--282 add a final-board differential-pair acceptance seam. The
symmetric control passes with two 26.000000 mm routes, zero skew, and equal
zero-via counts. On the asymmetric obstacle, both tools are electrically
complete and native-clean but fail the same 0.500000 mm skew limit:
Freerouting leaves 1.547585 mm and pcb-maker 1.839150 mm. The comparison now
includes result acceptance in both top-level solved flags. Only the positive
control is admitted; the negative proves that pair-aware routing or length
tuning is still absent, and spacing/uncoupled-length rules remain unmodeled.
M0 is now open at eighteen mechanisms, five pinned real declarations, and four
executable real sources.

Sequences 283--292 retract the historical all-solved mechanism aggregate. Once
rule areas are genuinely preserved, monolithic legacy component bodies trap
their own pads; decomposed access ports restore the intended body/trace model.
The user's ESP render review then exposes an independent angle-sign error:
semantic 270-degree pads were edge-facing instead of landing on their declared
interior seed endpoints. KiCad footprint angles are now negated, direct
rule-area geometry retains semantic rotation, and the real ESP coordinates are
unit-tested. Ports are per-net and per-layer rather than global holes through
both copper layers. The corrected sequence-291 aggregate finishes all 17
reports with 13 both-solved, one pcb-maker-only, two Freerouting-only
60-second timeouts, and one neither-solved fixed-wall placement baseline.
Sequence 292 preserves the pcb-maker-only decoupling result under the stricter
layer-specific representation. One final full replay remains before this can
be the canonical M0 aggregate; the fixed-wall case is now the next concrete
auto-placement/coupling milestone.

Sequences 293--298 add that milestone as a separate positive rather than
weakening the fixed-placement negative. Routing failure moves one vertically
constrained wall, then discards its semantic route and freezes the resulting
zero-copper KiCad placement for both competitors. Two rejected paired runs
first exposed comparison fallback and then a route-adapter false negative. The
latter came from half-cell-diagonal obstacle inflation, not A* budget. Exact
continuous collision checks on every grid edge restore the physical 1.8 mm
channel. Sequence 298 both-solves with matching placement and zero native
findings. This admits one router-originated translation mechanism; the roadmap
still requires rotation, multi-component movement, equal-budget ablation, and
hidden-board gains before claiming general coupling advantage. The expanded
18-case aggregate remains pending.

### M1 — Conventional route-only parity (6--10 weeks)

Deliverables:

- robust multi-pass provisional board;
- terminal escape/fanout and multi-layer search;
- connectivity-safe regional rip-up, alternative route orders, congestion
  history, and rollback;
- post-route tightening and redundant-via removal;
- typed pass-level evidence and strict per-pass budgets.

Acceptance:

- dual-ESP32 prefix 19 completes 24/24 at declared rules from zero copper on
  its existing placement, with zero native findings, in no more than 2x the
  Freerouting wall time on the same warmed host;
- ours solves no fewer route-only small-real boards than Freerouting under the
  same 60-second budget; a one-sided paired 95% interval must exclude a deficit
  larger than one board;
- every input terminates at its declared budget without corrupting
  connectivity; unsuccessful runs retain a valid parent and blocker evidence;
- increasing A* budget alone is shown separately from multi-pass rip-up/retry.

This is the first parity gate. Continuous squeezing and auto-placement do not
excuse failing it.

### M2 — Constrained auto-placement baseline (5--8 weeks)

Deliverables:

- versioned placement-constraint model covering the minimum set above;
- global analytical/field placer, exact legalizer, and bounded discrete/stochastic
  portfolio;
- free rotation where permitted, not only 90-degree orientation choices;
- fast routability probes, while final scoring still uses a cold M1 router;
- viewer frames for forces, density, legalizer actions, rotations, and rejected
  poses.

Acceptance:

- all mechanism and small-real inputs either produce an exactly legal placement
  or a typed, independently checked infeasibility/budget result;
- on at least 20 randomized starts per selected board, the placer removes
  dependence on a lucky initial arrangement: the 10th-percentile routed
  completion is at least as good as the declared/random baseline median;
- on the autonomous-layout small-real corpus, sequential placement plus M1
  routing solves at least 80% with no hand edits;
- every constraint type has a positive, negative, rotation, and interaction
  fixture and is checked independently of the placer.

### M3 — Prove or reject placement↔routing coupling (6--10 weeks)

Deliverables:

- typed routing obligations and capacity/blocker fields;
- transactional component move/rotate/swap/group actions initiated by routing;
- selective invalidation and rerouting of affected copper;
- equal-budget four-way ablation defined in the coupling strategy;
- at least two independent coupling policies so the interface is not an
  implementation disguise.

Acceptance:

- all four ablations use identical semantic inputs, seed sets, exact gates, and
  total deterministic-work or wall-time budgets;
- on hidden autonomous-layout boards, full coupling either (a) solves at least
  three boards unsolved by every sequential baseline, or (b) improves exact
  whole-board solve rate by at least 20% relative with a paired 95% bootstrap
  interval above zero;
- it regresses solved-board count by no more than one and reports any quality
  tradeoff on boards all methods solve;
- at least one accepted result requires a router-originated component rotation
  and one requires a multi-component move;
- if these criteria fail after two materially different policies, the result is
  documented and coupling remains experimental rather than a product claim.

### M4 — Production-rule core (8--12 weeks)

Deliverables:

- arbitrary pad/keepout/outline geometry and reliable spatial queries;
- net classes, per-layer rules, via classes and blind/buried vias;
- differential-pair routing, length/skew constraints, and matched groups;
- plane/zone creation, connectivity, thermal rules, island checks, and
  purposeful stitching rather than routed-ground via clusters;
- placement/routing keepouts and basic return-path/critical-net intent.

Acceptance:

- each supported rule has exact positive/negative fixtures and survives
  import/export round trips;
- five representative 4--6 layer real boards complete with zero applicable
  native findings and no manual cleanup;
- GND connectivity is proven after zone refill, with no redundant connected-via
  clusters and no isolated plane islands above the configured threshold;
- unsupported rules fail before optimization with a precise diagnostic; they
  are never silently ignored.

### M5 — Continuous make-room engine (6--10 weeks)

Deliverables:

- movable traces, vias, and exact compound components with body-local pads;
- multiresolution density/capacity field plus exact contact projection;
- narrow-to-declared-width continuation and enlarged-to-target-board
  continuation;
- trace tension, lane spacing, remeshing, and local discrete repair on a stalled
  continuation frontier;
- representation ablations for analytic bodies, particle pairs/clusters, and
  dense fields.

Acceptance:

- every committed continuation stage is native-exact; provisional invalid
  states cannot replace the last valid frontier;
- the full-width suite reaches 100% declared width, and the shrink suite reaches
  its target outline, on at least 90% of mechanism cases;
- against M3 without continuous motion, M5 either solves two additional hidden
  boards or reduces discrete rip-up/reroute work by at least 30% at equal solved
  count; the paired interval must exclude zero;
- representation choice is based on completion, work, memory/traffic, and
  stability—not an isolated attractive animation.

### M6 — GPU portfolio and field backend (5--9 weeks)

Deliverables:

- `wgpu` implementations for field, broad-phase/contact, and batched-candidate
  kernels; optional reachability portfolio after profiling;
- deterministic CPU semantic reference and CPU/GPU parity tests;
- batched board/candidate scheduling sized for small boards;
- transfer, dispatch, readback, native-validation, and end-to-end timings.

Acceptance:

- exact final solve counts match CPU for the frozen parity set; differences in
  floating-point trajectories are retained but cannot weaken validation;
- on workloads large enough to occupy the GTX 1650, promoted kernels achieve at
  least 10x throughput and the optimizer phase at least 3x speedup including
  transfer/readback;
- after native-validation caching, representative end-to-end experiments are at
  least 1.5x faster; otherwise the backend remains an optional research result
  and no further GPU port is prioritized;
- memory fits within 4 GiB with a declared bound and no data-dependent device
  allocation failure.

### M7 — Topology-action search and shrink-to-fit (8--14 weeks)

Deliverables:

- conflict graph and bounded best-first/beam/MCTS-compatible coordinator;
- actions for either route going around the other, layer/via changes, selective
  rip-up, component moves/rotations, and corridor reassignment;
- continuous fast repair/estimate followed by exact admission;
- large-board construction, staged shrink schedule, pressure localization, and
  repair at newly active bottlenecks;
- learned policies may rank actions but never replace exact gates or the
  non-learned baseline.

Acceptance:

- at least three adversarial/holdout boards unsolved by the M4 conventional
  router complete exactly under the same maximum budget;
- separate ablations attribute the win to topology-action search,
  continuation, component motion, or their combination;
- the selected lineage remains connected and exact-valid at every committed
  shrink stage and reaches the declared target outline/width;
- results reproduce across at least 20 seeds or disclose the empirical solve
  probability and budget-to-success distribution.

### M8 — Competitive preview (6--10 weeks)

Deliverables:

- stable local CLI/API, experiment plug-in interfaces, and KiCad round trip;
- interactive viewer with synchronized placement, routing, field, action, and
  validation lineage;
- pause/resume, crash-safe checkpoints, incremental engineering changes, and a
  high-level Toit board-description API after the engine boundary stabilizes;
- public corpus report with failures, ablations, and reproducible commands.

Acceptance:

- an unattended 100-board campaign terminates without a crash, silent rule
  omission, or unrecoverable candidate;
- on the declared target class, ours has a statistically supported Pareto win
  over Freerouting in autonomous layout and no route-only completion deficit;
- at least one runnable licensed/commercial comparison is included, or the
  report explicitly limits the claim to reproducible open baselines;
- five external users regenerate fabrication-ready KiCad boards and can locate
  the cause of every failure from retained evidence without modifying engine
  code.

## Promotion and stop rules

An experiment is promoted only if it crosses a milestone acceptance metric or
removes a demonstrated architectural blocker. A first negative result does not
kill an approach: inspect its failure evidence, run a matched control, and ask
whether the implementation, representation, budget, or hypothesis failed.

Conversely, research does not receive unlimited exemption from results:

- no tuning against one difficult board while the small corpus regresses;
- no inherited prefix placement or copper in cold-growth claims;
- no invalid thin/enlarged intermediate counted as a solved board;
- no quality optimization before completion when the completion set differs;
- no GPU rewrite without a profile and an end-to-end speed gate;
- no “AI” or “coupled” advantage without a same-input, same-budget ablation;
- no commercial superiority claim from vendor marketing or screenshots.

## Immediate priority order

1. Finish M0 and make the Freerouting comparison automatic across a useful
   small-board corpus.
2. Implement M1's multi-pass rip-up/retry router until fixed-placement parity is
   real; do not spend the next cycle squeezing a 23/24 topology.
3. In parallel at the design level, freeze the M2 placement constraint schema
   and benchmark definitions, but avoid polishing libraries or UI.
4. Build the sequential constrained placer, then run M3's coupling ablation.
5. Add rule breadth in response to corpus exclusions and only then promote the
   make-room and GPU research paths.

This order preserves the project's distinctive ideas while preventing them
from masking missing fundamentals. It also gives each new idea a credible
baseline to beat.
