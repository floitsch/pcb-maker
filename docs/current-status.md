# Current placer/router status

## Active priorities

Following the research-direction checkpoint, fresh unattended runs of a frozen
executable/configuration across the pinned boards are the primary measurement.
Resumed checkpoints remain diagnostic evidence and are reported separately.
New recovery mechanisms must earn broader completion gains before gaining depth
or board-specific tuning. Reuse the existing verification/rendering tools.

1. Complete whole boards across the pinned reference families, with every net,
   source rules and mechanical constraints retained. Report unsupported input
   features explicitly; local mechanism successes do not count as completion.
2. Make failures drive general recovery: diagnose routing conflicts, reconsider
   earlier routes, and change unconstrained placement when routing alone stalls.
   Preserve verified progress between bounded attempts.
3. Minimize board area using complete cold-placement/cold-routing runs. Compare
   against reference layouts and other routers on the same inputs and budgets.
4. Revisit via count and trace length after completion is reliable, or when a
   mechanism directly addresses a whole-board completion failure. Findings are
   retained in [deferred optimizations](optimization-todos.md).

## Latest evidence

The adaptive router now supports opt-in `initial_demand_strength`, generating
its own forecasts before the first committing pass while preserving the net
order and attachment policy. The default remains unchanged. All 192 KiCad
tests pass (one ignored); both fresh one-pass ECC83 native controls complete
all nine nets with zero opens/physical/ERC/parity findings and seven retained
library metadata warnings. Demand changes 8 vias/388.293 mm into 5 vias/343.072
mm, taking 133.88 seconds versus 122.38 including 10.92 seconds of forecasts.
These are single-sample timings and a control-board quality result, not broader
completion evidence. A frozen original-placement Interf off/on comparison is
running with the same executable, discovery order, four-pass/repair limits and
1800-second total budget, including any forecast generation.
[Native controls and combined renders](../build/whole-board-reset-2026-09-08/global-routing/initial-demand-fresh-controls/README.md).
[Frozen Interf comparison](../build/whole-board-reset-2026-09-08/global-routing/initial-demand-interf/plan.json).

The matched Interf insertion spacing test admits both placements in about
0.17 seconds of placement work. Extra spacing survives final legalization
unchanged: minimum shared-layer intercomponent pad gap is 0.525 mm without
reservation and 1.200 mm with reservation (requested 1.133 mm). Both remain
physically legal with 200 cold opens and 23/24 annotation findings. However,
both subsequent external routing arms time out at the fixed 300-second search
limit (about 330 seconds whole pipeline). Last completed external passes report
45/47 unrouted connections; these are not native-verified final states. No
completion benefit is established and no larger budget follows. The adapter
does not export partial copper on hard timeout, so only cold native renders
exist for those two routing runs. Mechanical compatibility remains outside this
equal-area replacement-rectangle experiment.
[Placement results](../build/whole-board-reset-2026-09-08/placement/interf-corridor-reservation/results.json).
[Routing results and render limitations](../build/whole-board-reset-2026-09-08/placement/interf-corridor-reservation/external-comparison/README.md).

Two held-out Interf-U checkpoint diagnostics bound the current island-bridge
result. With the same four-attempt policy, discovery-order repair reduces opens
from 88 to 84 in 73.38 seconds; count-order repair reduces 44 to 41 in 60.80
seconds. Fully connected routable nets remain 94/110 and 76/110 respectively.
Original copper and design inputs are preserved. These are partial island joins,
not additional whole-net or fresh whole-board completion. They do not justify
increasing the search bounds.
[Checkpoint results and selected combined renders](../build/whole-board-reset-2026-09-08/global-routing/island-bridge-transfer/README.md).

Automatic native island repair now completes two fresh cold boards without
agent-selected checkpoints or endpoints. One frozen executable/configuration
runs the optional Freerouting backend followed by at most four internal bridge
attempts from eight nearest existing-pad pairs. Hierarchy completes in 84.33 s
and PIC in 85.01 s, each with zero native opens/DRC/ERC/parity/metadata findings.
Both external stages independently leave one open; the first automatic bridge
completes each board. Hierarchy adds four segments and one via; PIC adds two
segments totaling 3.264 mm and no vias. All prior external copper, native pad
geometry/poses, outline and project/schematics are preserved. This establishes
fresh completion for the combined backend, not the internal router alone.

The operation is available as `bridge-kicad-open-connections` and opt-in
`island_bridges` in the external backend configuration. Both backend stages share
one pipeline deadline. Native island discovery and admission remain separate:
unsupported/padless geometry is visible, failure retains the last verified board,
and only strict native open reduction permits append-only selection. The area
driver follows the final selected project/render rather than a stale external
stage result. Validation includes 189 KiCad and 17 CLI tests, seven native island
controls, six area-result reader tests, and four native bridge controls (including
completed-board no-op, a real via between same-XY F/B pads, and bounded failure).
[Fresh plans, results and combined renders](../build/whole-board-reset-2026-09-08/global-routing/automatic-island-bridge/README.md).
[Usage and limits](native-island-bridges.md).

Bridge candidates now reuse the freshly verified baseline's ERC report through
the existing unchanged-input guard, while still running PCB DRC/parity and
automatic rendering. The 189 KiCad tests pass (one ignored). Two exact native
replays preserve final PCB bytes, selected pairs, search work and all-zero native
findings. Single-sample times decrease from 27.90 to 21.67 seconds and 26.73 to
20.26 seconds; these are diagnostic timings, not a whole-board speedup claim.
[Matched controls and renders](../build/whole-board-reset-2026-09-08/restrictions-runtime/island-bridge-erc-reuse/comparison.json).

The new diagnostic `propose-kicad-pad-pair-route` asks for a bridge between two
existing pad anchors while retaining the complete board as obstacle context.
On the retained external hierarchy result, its one frozen C303.2-to-R308.2 GND
attempt takes 0.751 seconds and appends four segments and one via. Native
verification takes 13.45 seconds and reports full completion with zero findings.
All 329 original segments, the entire non-copper board AST, project and schematics
remain exact. The combined build passes 186 KiCad and 13 CLI tests. This is a
successful checkpoint diagnosis, not a fresh unattended routing score; its manual
endpoint selection motivated the separate automatic policy described above.
[Result and preservation checks](../build/whole-board-reset-2026-09-08/global-routing/pad-pair-diagnostic/results.json).
[Combined copper rendering](../build/whole-board-reset-2026-09-08/global-routing/pad-pair-diagnostic/native/preview.svg).

Placement input handling now preserves explicitly nonphysical, silk-only
board graphics without inventing component collision bodies. Interf-U's G1
logo was the sole missing-courtyard obstruction; its complete native artwork
remains fixed and unchanged. The first fresh placement takes 2.027 seconds
for 24 physical components, with zero physical/ERC/parity findings and 45
annotation findings. It replaces the stepped outline with an equal-polygon-area
rectangle (12191.58852 mm²) and frees all physical components, including BUS1;
this does not preserve enclosure or edge-connector compatibility. The frozen
1800-second unattended internal route times out after two completed passes.
Its selected first pass retains 17/110 nets and 110 opens; pass two connects
21 nets but leaves 173 opens, and the interrupted third pass reaches seven nets
with 187 opens. Final independent verification confirms zero physical/ERC/parity
findings, 45 annotations, and exact original placed footprint/graphic/outline/
project/schematic inputs. No completion or improvement over original-placement
routing is established. Live updates incorrectly kept observing the first pass
after execution advanced; there is no evidence of a prolonged single-net stall.
[Input coverage and placement](../build/whole-board-reset-2026-09-08/placement/interf-input-coverage/results.json).
[Placement animation](../build/whole-board-reset-2026-09-08/placement/interf-input-coverage/trial/area-00/trace/index.html).
[Final routing evidence and corrected pass inventory](../build/whole-board-reset-2026-09-08/placement/interf-input-coverage/routing-results.json).

Read-only diagnosis finds that this placement reduced two facing DIP pad-row
gaps from 2.34–3.48 mm to about 0.58 mm, below a 0.908 mm signal lane including
clearance. Straight-demand crossings increased 69.4%; individual cold routes
exist, but retained routes obstruct their paths. This supports testing local
channel reservation without establishing placement as the sole cause. A proposed
matched insertion comparison uses the existing extra-pad-gap option, with native
width/clearance and grid-derived spacing. First check that spacing survives final
legalization; no new placement or routing run has been performed for this proposal.
[Bounded proposal and geometry renders](../build/whole-board-reset-2026-09-08/placement/interf-input-coverage/placement-diagnosis/proposal.md).

Frozen whole-board evaluation (2026-09-09): both priority policies finish ECC83,
PIC and complex hierarchy, with identical copper within each pair. PIC and
hierarchy have zero native findings; ECC83 retains two source silkscreen warnings.
Neither priority finishes Interf-U within 1,800 seconds: history retains 51/110
nets and 147 opens, boundary 48/110 and 150 opens. Olimex fails source preflight,
including six inherited physical clearance errors. No whole-board completion
benefit from boundary priority or deeper recovery is established.
[Completed evaluation, limits and evidence](reviews/2026-09-09-fresh-whole-board-evaluation.md).

Reference geometry qualifies these comparisons: Interf-U has 15 human trace
segments on seven nets below the compiler's per-net widths; PIC has 11 on three
nets. ECC83 and hierarchy have none. Along with removed copper planes, this means
the human references alone do not prove feasibility under our fixed-width
routing strategy. It does not establish that neckdowns are necessary or that
fixed-width routing is impossible. The archived Freerouting stricter-edge control
already leaves only five Interf-U opens and zero other native findings at exactly
the same per-net widths/clearances/via dimensions, but a 0.264 mm external edge
requirement versus native zero. It demonstrates near-completion at fixed widths;
missing neckdowns do not explain most of our remaining gap. A subsequent fresh
external run with corrected edge translation now fully solves the board, as
described below. Current internal comparison inputs remain unchanged.
[Width inventory and combined renders](../build/whole-board-reset-2026-09-08/reference-track-widths/README.md).

A fresh Freerouting run now achieves full native Interf-U completion with zero
opens, physical/ERC/parity findings or metadata warnings. It preserves the exact
cold input, placement, project, per-net dimensions and nine outline segments.
The adapter uses the pinned router's existing outside-area boundary mode to
represent native zero edge clearance without the line outline's added thickness.
Whole-pipeline time is 99.03 seconds, including verification; external routing
takes 51.76 seconds. The result has 44 vias and 5050.614 mm stored track. This is
a valid fixed-width/no-plane feasibility certificate and a substantial internal
routing gap: our matched runs still have 44/88 opens after 1,800 seconds.
Arbitrary DSN footprint-shape equivalence remains unproven; that limitation does
not negate the native-verified final board. The default external CLI path remains
unchanged, and the new mode has native positive/zero-clearance, concave-outline,
outside-pad rejection and mode-off copper-equivalence controls.
[Fresh external result and rendering](../build/whole-board-reset-2026-09-08/restrictions-runtime/interf-outside-fresh/comparison/report.json).

An optional `route-kicad-board-freerouting` command now exposes that same native
pipeline from the program, with bundled helpers, source-derived physical rules,
private completion-first settings and a whole-pipeline timeout. Thirteen binary
tests pass. Native controls run outside the repository with spaced output paths:
a completed coupon exits zero with exact prior direct-pipeline copper; a routed
coupon retaining one annotation finding exits nonzero despite subprocess success.
The existing adaptive backend remains the default. A fresh ECC83 placement and
external route through the existing area driver completes in 79.40 seconds with
full native acceptance, zero vias and 227.81 mm stored track. Five existing
library metadata warnings remain. Original input hashes and the admitted
placement's poses, project and rules are preserved. Fresh original-placement
PIC and hierarchy coverage checks use the same frozen backend and search policy.
[Usage and requirements](native-external-routing.md).
[Integration controls and combined render](../build/whole-board-reset-2026-09-08/restrictions-runtime/production-external-backend/comparison.json).

The next frozen external corpus checks expose adapter limits. PIC stops before
routing because adding the edge class weakens six mixed POWER clearances from
0.280 to 0.250 mm. Early structure-scope references create those classes before
the external loader initializes their mixed clearances and default item roles.
A scoped post-load edge-row correction now preserves those settings, passes
positive/zero native controls with unchanged copper, and lets fresh PIC routing
proceed. That run ends with one open and zero other native findings in 54.65
seconds; the external router itself reports the same remaining connection.
No retry or search-policy tuning follows it. Hierarchy finishes with
15 native opens and no other findings while only its back layer is active.
Its front "power" designation is a KiCad user guide, not a track prohibition,
so that result does not measure equivalent two-layer routing capability.
The native backend now maps both cold copper layers to externally routable DSN
layers while retaining original native names/roles and other DSN content;
both signal and power controls now pass full native verification, preserving
source metadata and identical signal-control copper. A fresh hierarchy run uses
both layers and reduces 15 opens to one (58.52 seconds whole pipeline, 8.54 seconds
external process, zero vias, 1377.14 mm). External and native checks agree that
one GND connection remains open; there are zero other native findings. No routing
completion credit or extra order/budget trial follows that partial result.
[KiCad layer semantics](https://docs.kicad.org/10.0/id/pcbnew/pcbnew.html).
[Layer controls and combined renders](../build/whole-board-reset-2026-09-08/restrictions-runtime/native-layer-semantics/results.json).

Read-only via inspection finds enabled, native-matching full-span vias for PIC's
remaining VCC_PIC connection, hierarchy's remaining GND connection and successful
Interf GND. The current external backend also completes an unchanged opposite-layer
SMD coupon with one newly created via and zero native findings. This rules out a
general missing-via configuration defect; it does not resolve the boards' remaining
access/search failures. No last-net routing retry or policy tuning was performed.
[Via availability controls and render](../build/whole-board-reset-2026-09-08/restrictions-runtime/via-availability/comparison.json).

A subsequent read-only diagnosis locates a concrete PIC exchange mismatch:
the DSN/loaded pad for the remaining isolated JP1-2 fills its native concave
notch, overlapping JP1-1 where native clearance is 0.28 mm. The native source
is physically clean and has accessible pad contacts. This is strong evidence
of an artificial endpoint obstruction, though no corrected routing replay has
yet established causality. The pinned external loader explicitly convexifies
polygon pad shapes and stores one convex shape per layer, so replacing only
the DSN polygon text cannot preserve this pad. Exact support needs a declared
compound-pad representation and import/identity controls. Hierarchy's remaining
GND islands instead contain 21 and five plated pads, with accessible contacts on
both layers and no loaded-source clearance findings; its score-stagnation stop
remains unexplained. Neither diagnosis changes the previous routing scores.
[Endpoint diagnosis and labelled combined overlays](../build/whole-board-reset-2026-09-08/restrictions-runtime/remaining-open-access/comparison.json).

Rounded-rectangle pad copper now lowers through the shared native routing model
as its inset polygon and rounded edges, instead of a bounding rectangle. The advisory
obstruction query accepts this supported geometry, while malformed, chamfered and
per-layer variants are refused explicitly. Native KiCad 10.0.6 collision samples
and odd-nanometre rotated-pad readback agree with the new geometry; 180 KiCad tests
pass. The same four retained hierarchy chord questions now identify fixed pads
and obstructing routed nets where every query previously returned unsupported.
All four direct chords are obstructed; this does not prove that detours are
unavailable. No source copper changes or new routing run accompany the diagnosis.
Nearby rounded-pad support in the joint movement engine remains separate.
[Geometry controls and real-board query evidence](../build/whole-board-reset-2026-09-08/restrictions-runtime/roundrect-geometry/hierarchy-queries/comparison.json).

Deferred forecasting passes 143 KiCad and eight benchmark tests, byte-identical
ECC83/PIC native replays, a four-pass changed-order continuation and explicit
optimization equivalence. Its fresh Interf-U run retains 55 nets/143 opens and
one admitted pass before timing out during forecasts. This is progress within
the same budget, not completion. Opt-in first-admitted repair retains 57 nets
and 140 opens under the same cap, also timing out during forecasts. Later repair
work can become more expensive after the changed early choice, so this policy
remains experimental. A separate matched comparison tests one cold retry
ordered by observed failures before paying for independent forecasts.
That opt-in retry passes a native order-dependent control: horizontal-first
blocks the second net; the program automatically retries vertical-first and
completes both with zero findings. The off control also completes after
forecasting, with identical copper and search work. Bounded continuation,
incumbent/pass-limit and complete-on-first-pass controls pass; fresh matched
Interf-U retry-on now times out at 1,800 seconds after two returned passes,
retaining pass zero at 55 nets/143 opens with zero physical/ERC/parity findings.
The observed retry routes seven nets before failing at PC-A4; it does not
improve the incumbent. The matched off repeat also times out at 55 nets/143
opens, after one returned pass. First-pass copper, selections, search work and
native findings match exactly; no incumbent gain is established. The original
off process's unexplained early interruption remains separate from solver
failure.
[Permanent order-dependent control](../benchmarks/competitive/mechanisms/observed-order.md).

A separate whole-sweep prototype addresses an earlier global decision: the
native sequential pass stops at its first failed net, leaving later nets
unattempted. The prototype automatically retains the admitted prefix and tries
the remaining identities once, sharing one repair allowance across internal
resumes. It accepts only fresh source input and stops on unclassified errors.
A three-net native control advances from one to two connected nets with zero
physical/ERC/parity findings; the blocked net remains explicitly failed. This
establishes broader coverage, not completion. The frozen fresh Interf-U pair
now retains 55/110 nets and 143 opens when stopping, versus 85/110 and 101 opens
when continuing under the same four-repair allowance and 1,800-second cap.
The continuation times out with 15 failed and ten still-unattempted identities.
Both retained boards have zero physical/ERC/parity findings; exact first-pass,
rotation-inventory and final-prefix audits pass. Fourteen returned internal
resumes spend 380 seconds outside new route-step timers. An opt-in in-process
sweep now passes 152 KiCad tests, native H/V/L traversal and adaptive-retry
controls, unchanged completed ECC83 copper/work, and malformed-inventory
rejections. A fresh four-pass adaptive Interf-U off/on pair changes only the
continuation flag. Its on arm has returned a full first sweep at 94/110 connected,
16 failed and zero unattempted, with 88 opens and zero physical/ERC/parity
findings; GND and VCC account for 63 of those opens. Off retains 55/110 and
143 opens. Both now time out at the original 1,800-second cap. Off has an
interrupted four-net second pass; on spends its remaining time forecasting.
The retained first passes remain best. Final independent audits of both
incumbents and off's interrupted second pass pass outside the cap. Relative to
200 source opens, off resolves 57 and on resolves 112; net count alone obscures
the remaining large-net obligations. Neither arm establishes whole-board
completion.
[Protocol and renders](../build/whole-board-reset-2026-09-08/global-routing/whole-sweep/README.md).

A retained-layout corridor diagnostic identifies an earlier source of congestion:
the accepted MD2 route uses seven vias and 101.01 mm, versus zero vias and
39.48 mm in the external result. Adding the reference path to its earlier
internal prefix, on a disposable copy, gives exactly three native conflicts with
MD0, CS1- and MA0. Exact earlier copper, component poses and original project
rules are preserved. Those nets and MD1/MD2 all use F.Cu externally:
longer earlier paths coexist with much shorter later paths. This supports testing
coordinated geometry before failure triggers repair; it proves neither a minimum
blocking set nor a whole-board improvement. No reference copper is promoted.
[Native diagnosis and combined close-up](../build/whole-board-reset-2026-09-08/global-routing/reference-corridors/README.md).

The program's own stored cold MD2 forecasts supply short zero-via alternatives
without external geometry: 35.31 mm at coarse resolution and 34.14 mm at fine.
Applying them separately to the same earlier prefix gives native conflicts with
MD0/MA0 for coarse and only MD0 for fine. All earlier copper, poses, project bytes
and source hashes remain exact. The advisory geometry query now attributes the
exact native foreign-item UUIDs in 8–10 ms, with 161 KiCad tests passing. It is
explicitly advisory and reports unsupported geometry. One bounded existing
transaction yields MD0, inserts short MD2 and restores MD0: native connectivity
matches the original MD2 insertion, but combined MD0/MD2 cost worsens from
140.25 mm/seven vias to 145.54 mm/eight vias. It transfers the detour rather than
improving the pair. Proactive use is not promoted; no whole-board gain is claimed.
[Program-only forecast controls](../build/whole-board-reset-2026-09-08/global-routing/reference-corridors/forecast-probe.json).

An opt-in `relax-kicad-joint-routes` adapter now moves a proposed target and
explicitly selected neighboring trace interiors together through the existing
constraint/tension engine. Three frozen first-attempt controls are admitted:
one movable blocker, a two-neighbor chain, and a fixed-pad case that requires a
detour. All finish with zero native findings, exact original electrical endpoints,
preserved neighboring connections and unchanged source poses/project/rules.
Each uses the same 64-step preset, with five saved progress frames; end-to-end
times including baseline/final native verification are 29–30 seconds. Witness
solutions were not supplied to the solver. This establishes bounded joint
movement on small controls, not whole-board completion or automatic obstruction
selection. Initial support was simple uniform-width, via-free front-layer chains
between two front-only pads on rectangular boards; unsupported inputs are refused.
Review added early preparation limits and rejection of adjacent backtracking.
All 169 KiCad tests pass; a separate retained-artifact control applies the tightened
guards to all three admitted proposals without changing their PCB bytes or
rerunning the solver. The measured solver binary and guarded source are archived
separately.
[Adapter contract and evidence](../build/whole-board-reset-2026-09-08/global-routing/joint-relaxation-adapter/README.md).
[Saved-frame playback](../build/whole-board-reset-2026-09-08/global-routing/joint-relaxation-adapter/playback.html).

The joint request now excludes fixed obstacles beyond the declared movement
envelope while retaining actual widths, local clearances, touching envelopes
and the full-board final checks. A frozen derivative adds 32 distant connected
nets: the old adapter exhausts its 256-point allowance before solving, whereas
the new adapter admits the repair at unchanged limits with exact original local
copper at every saved frame. All three previous controls likewise remain native
clean and frame-for-frame identical; 172 KiCad tests pass. This removes dependence
on unrelated remote context, not the remaining representation limits. Read-only
real-board analysis confirms Interf's implicated traces are already simple
front-layer chains. The subsequent adapter accepts fixed plated endpoints and
nonrectangular outlines only when the complete motion envelopes provably remain
inside the exact outline. SMD and plated-pad native controls pass. The first
frozen real MD2/MD0 attempt still rejects preparation at the 256-point allowance
after 13.56 seconds, before producing any solver frames; the verified original
remains selected. No larger-budget retry follows. This exposes a remaining
representation limit, not evidence of solver convergence or whole-board gain.
PIC additionally has branches, vias, rounded pads and inherited annotations.
[Context experiment and playback](../build/whole-board-reset-2026-09-08/global-routing/joint-context-envelope/README.md).
[Real-board transfer analysis and combined overlays](../build/whole-board-reset-2026-09-08/global-routing/joint-transfer/README.md).
[First real transfer result](../build/whole-board-reset-2026-09-08/global-routing/joint-real-transfer/README.md).

An opt-in adaptive initial order now stably prioritizes electrical terminal
count, using existing discovery data without forecasts or named-net exceptions.
The default preserves discovery order; an explicit caller order conflicts with
the automatic option and is rejected before routing. All 156 KiCad tests pass,
including coincident opposite-layer contacts, stable ties and default behavior.
A frozen discovery/count comparison keeps sweep enabled in both arms and all
other routing policies unchanged. Both ECC83 native controls complete all nine
nets with zero opens/physical/ERC/parity and the same two source annotations;
discovery preserves all prior committed boards exactly, and the changed count
order is verified. Both fresh Interf-U runs now time out at 1,800 seconds:
discovery retains 94/110 nets with 88 opens; count retains 76/110 with 44 opens.
Count completes GND/VCC but loses other smaller nets. This is a coverage tradeoff,
not completion. Both selected and interrupted second-pass checkpoints pass
independent native audits outside the cap. The discovery first sweep exactly
reproduces its predecessor, and all 110 independent forecasts agree by identity.
Both PIC controls complete 34/34 with zero native findings. Discovery preserves
prior copper and uses 10 vias; count uses one repair and 12 vias, with 0.703 mm
less track. Final native/adaptive audits pass. No broader completion gain is
established, and the option is not promoted. Runs use verbatim frozen Python
helpers as well as executables/configurations.

An isolated timing replay of Interf-U MA16 reproduces the archived fine-grid
candidate and board exactly (314,045 expansions). Routing takes 3.757 seconds:
planar raster construction accounts for 3.172 seconds, while A* takes 0.094.
Full native verification separately spends 6.463 seconds in ERC, 6.503 in PCB
DRC and 0.425 rendering. This single-attempt profile identifies two distinct
costs; it does not establish the cacheable fraction of raster work. Ordinary
routing already reuses frozen ERC inputs. That hash-guarded reuse now extends
inside repair, with 149 KiCad tests passing. A matched MA4 repair retains exact
candidate copper, search work and native findings while reducing full ERC calls
from nine to one; all nine PCB checks and renders remain. Observed runtime is
219.3 versus 157.1 seconds. A second generated SMD repair likewise retains exact
copper, work and native findings, reducing ERC calls from three to one; its
observed wall time is 104.6 versus 106.7 seconds, so no speedup is claimed there.
The corrected compound-path replay also retains every candidate, all 139,578
expansions, intermediate/final PCB bytes and native findings exactly, with one
fresh ERC and five unchanged PCB checks/renders. An initial isolated harness
had omitted the CLI's floating-point JSON parsing feature and is not counted
as an equivalence pass. The KiCad library now declares that feature explicitly
for standalone consumers too. These are local repair controls, not a
whole-board timing claim.
[Measured phases](../build/whole-board-reset-2026-09-08/restrictions-runtime/route-phase-diagnostic/fine.log).

A second isolated MA16 replay separates outline from obstacle work while
requiring all 12,730,976 planar-mask entries, candidate/work, PCB bytes and
native findings to match. The original mask takes 3.129 seconds; an outline-only
pass takes 2.608 and obstacle checks conditioned on those exact flags take
0.657. Their sum is 4.35% above the original, so these are not exact additive
phase shares. Extra traversal/allocation alone takes 0.014 seconds. This
supports exact outline reuse within a request before a persistent geometry
cache. The resulting production change caches endpoint containment and shares
outline edge results across layers, retaining every boundary-segment and
per-layer obstacle check. All 154 KiCad tests, broad geometry-mask controls,
exact MA16 candidate/work/PCB/native findings and a byte-identical nine-net
ECC83 regression pass. MA16 route CLI time is 3.925 versus 2.041 seconds in
the matched observation; no whole-board speedup is claimed. Temporary caches
are dropped before search.

The separate anchored ECC83 placement fails. A contact-direction fallback
passes 48 placement tests and preserves prior PIC/complex results, but fails
the real case at its frozen 16-trial budget; it remains experimental. The
harmonic seed puts all six passives inside fixed occupied space. Obstacle-aware
seeding passes 52 placement tests, but the frozen off/on evaluation rejects
both anchored ECC83 and anchored PIC placements. ECC83's seed initially clears
all fixed-space contacts; subsequent legalization reintroduces them and stalls.
Both PIC arms exhaust the same shared pair-check budget. No arm earns a cold
route. The projection and branch fallback remain experimental; increasing
iteration or branch limits is not justified by these results.

A subsequent single-order insertion policy places the largest free footprints
first against anchors and already inserted footprints. It passes 55 placement
tests and produces a legal anchored ECC83 placement in 0.446 seconds. A fresh
cold route then connects all nine nets in pass zero (121.8 seconds), preserving
all nine anchors and the original outline. There are zero physical/ERC/parity
findings; the routed output retains 43 annotation findings. One separate use of
the existing silkscreen repair clears all 43 in 17.6 seconds and earns full
native admission, with three library metadata warnings. Exact copper, poses
and outline are preserved. Anchored PIC still exhausts the unchanged 1M
pair-check budget during insertion at R9;
no extra order or larger budget has been tried.

Certified geometric rejection now avoids exact checks for provably separated
footprints, with 59 placement tests passing. ECC83 retains exact poses, final
copper and routing work. Under the same 1M exact-check cap, anchored PIC inserts
39 of 52 free components before failing at C7, compared with 19 before the
change. An isolated diagnostic confirms the first 19 poses are identical and
production rolls back atomically. It visits 946,851 candidates, almost all
colliding: the remaining limitation is enumerating occupied space near the
seed, not evidence that the board cannot fit. The partial placement is not
admitted or routed.
[Placement comparison and renders](../build/whole-board-reset-2026-09-08/placement/seed-projection-broadphase/index.html).

Opt-in collision-interval skipping now completes anchored PIC placement at the
same 1M work cap: all 52 movable components are inserted in 0.192 seconds of
search, spending 362,259 charged operations including interval preparation and
row certificates. The final geometric sweep brings placement work to 364,212
operations. Native
geometry/ERC/parity checks pass; all 11 anchors, five original outline edges
and the previously accepted 39 poses remain exact. There are 125 cold opens
and 52 annotation findings, so this is not yet a completed layout. ECC83's
entire placement remains identical. All 65 placement tests and four previous
result controls pass. Cold routing is evaluated separately; the search time
excludes materialization and native verification.
[PIC placement rendering](../build/whole-board-reset-2026-09-08/placement/collision-intervals/pic-insert/area-00/trace/final.svg),
[paired results](../build/whole-board-reset-2026-09-08/placement/collision-intervals/results.json).

The paired PIC cold routes expose a downstream cost of first-admitted repair.
Its retained PC-DATA-IN path blocks VPP_ON later, requiring another repair where
best-score routes directly. In the saved counterfactuals, removing PC-DATA-IN
restores a path while removing GND alone does not. Several earlier routes differ,
so this does not isolate every causal change. The result motivates considering
pending connectivity when choosing repairs, rather than assuming that cheaper
local search produces a faster complete board. The best-score fresh run now
times out at 1,800 seconds, retaining pass zero at 25/34 nets and 13 opens.
Its 52 design findings are exclusively silkscreen annotations; physical,
ERC and parity findings remain zero. First-admitted also times out at 1,800
seconds, retaining 24/34 nets and 16 opens. Its latest interrupted pass has
24 nets and 17 opens, so inspecting that journal does not reverse the comparison.
It explores more passes but does not improve the best-score incumbent.
The original placement previously routed all 34 nets, so fast legal insertion
has not yet established a routability improvement. No policy is promoted from
this local observation.
[Combined comparison with the obstructing trace highlighted](../build/whole-board-reset-2026-09-08/placement/collision-intervals/repair-downstream-diagnosis/index.html).

The native placement adapter now accepts explicit `net_tension_weights`, making
the engine's existing attraction control available without changing physical
net widths or clearances. Omitted policies reproduce both extracted native
problems exactly; three focused controls cover exact net identities, valid
weights and atomic rejection of invalid maps. A frozen GND weight of 0.05
produces legal anchored PIC and ECC83 placements with unchanged physical rules,
anchors and outlines, and zero physical/ERC/parity findings. PIC retains 125
cold opens and 69 annotations; ECC83 has 20 opens and 41 annotations. Surrogate
crossing/demand measures are mixed, so no congestion improvement is claimed.
The weighted ECC83 cold route completes all nine nets in pass zero, with zero
opens/physical/ERC/parity findings and 41 annotations. It uses zero vias and
191.732119 mm of copper versus 198.411540 mm before weighting. One separately
timed invocation of the existing silkscreen cleanup clears all 41 in 18.375
seconds and earns full native admission, with five library metadata warnings;
exact copper, poses, outline and source/project preservation checks pass.
The weighted PIC fresh repeat now connects all 34 nets in pass zero in 1,271.9
seconds, versus the unweighted placement's 25/34 at the 1,800-second cap. It uses
53 vias and 2,693.804626 mm of physical copper, with zero opens/physical/ERC/parity
findings. All 69 remaining findings are silkscreen annotations. One separately
timed use of the existing protected cleanup reduces them to 19 in 45.989 seconds,
with exact copper, poses, outline, source/project and protected non-silk data.
Full native admission remains false; 17 library metadata warnings are reported.
The original interrupted weighted run is excluded. Both routing arms use the same
frozen executable and budgets. This is explicit placement intent, not automatic
ground detection, and does not establish improvement over the original human
placement, which also routed completely.
[Weighted placement comparison and renders](../build/whole-board-reset-2026-09-08/placement/explicit-net-weights/index.html).

An opt-in seed-insertion policy now reserves additional copper-pair spacing
between different footprints, using the same expanded primitives for exact,
broad-phase and collision-interval checks. Body geometry, layer interactions,
fixed anchors and actual board rules remain unchanged; zero preserves the old
behavior. Seventy placement tests pass. Frozen off/on native placements on
ECC83 and PIC all pass physical/ERC/parity checks, with exact anchors/outlines
and unchanged true problem geometry. Off poses reproduce the archived results;
every admitted seed pose survives the final true-geometry sweep unchanged.
The predeclared extra gap is largest routed width plus placement clearance
(total pad gap 1.60 mm on ECC83 and 1.36 mm on PIC), at original attraction
weights, order and work budgets. Both ECC83 cold routes now complete all nine
nets in pass zero, with zero vias/opens/physical/ERC/parity findings. Physical
length changes from 198.411540 to 198.376720 mm and expansions from 1,985 to
1,896; this is essentially unchanged routing outcome. Annotation counts are
43/44. Both matched PIC cold routes now time out at 1,800 seconds. Off retains
25/34 nets and 13 opens; on retains 28/34 and six opens. Both have zero
physical/ERC/parity findings, exact component poses and outlines, and unchanged
sources; annotation counts are 52/49. Selected pass zero is better than each
interrupted second pass (21/26 connected nets). This improves partial coverage
at the frozen budget, but establishes neither completion nor global escape
capacity. A separate frozen hierarchy comparison completes fresh placement and
routing at original versus 90% board area under its existing all-free policy.
Both route 50/50 in pass zero with zero opens/physical/ERC/parity findings.
Actual area drops from 8057.413 to 7251.671 mm²; vias increase from 10 to 13 and
track length from 1326.369 to 1398.228 mm. Routing takes 487.55/504.86 seconds
in these observations. Exact admitted poses, outlines and original project bytes
are retained; 74/89 annotation findings prevent full native admission. This is
electrically complete geometric repacking, with no enclosure-compatibility claim.
[Area results and combined renders](../build/whole-board-reset-2026-09-08/placement/complex-area-90/results.json).
[Spacing controls and renders](../build/whole-board-reset-2026-09-08/placement/routing-space-reservation/results.json).

The second frozen area comparison uses PIC's existing all-free placement policy.
Original-size placement succeeds and cold routing connects 34/34 in pass zero
(308.01 seconds, 39 vias, 2798.90 mm), with zero opens/physical/ERC/parity findings
and 36 annotation findings. At 90% area, placement exhausts the same one-million
pair-check budget at sweep 508 with 21 contacts; it never reaches native
materialization or routing. Residual penetration is still decreasing: summed
residual falls from 4.538 to 1.796 mm between sweeps 384 and 508. The coupled
solver was invoked only at sweep 128 and failed on inconsistent/singular rows;
it never retries the later changed contact system. A proposed bounded retry
schedule is deferred after a more fundamental numerical counterexample: the
production projector rejects x >= 2, y >= 2, x+y >= 3 as singular although (2,2)
is the minimum-norm feasible solution. It succeeds when the redundant diagonal
is removed. The corrected active-set step verifies normal dependency and uses a
bounded dual pivot to release an old constraint. All 75 placement tests pass,
including contact chains, incompatible inequalities, uncertain numerical rank
and exact work limits. Native comparisons preserve PIC original, hierarchy90 and
anchored ECC poses/work exactly. PIC90 still fails at the same budget and rolls
back to exactly the same trajectory. No retry policy is added. Neither the
counterexample nor the slow residual decay proves
that the reduced PIC board is infeasible.
[PIC convergence rendering](../build/whole-board-reset-2026-09-08/placement/pic-area-90/reduced/area-00/trace/convergence.svg).
[Numerical counterexample and rendering](../build/whole-board-reset-2026-09-08/placement/dependent-row-control/result.json).

An exact replay then identifies the incompatible branch: a 17-component
horizontal chain needs 147.630032 mm between bounded centers, with only
146.708301354 mm available. Existing direction branching already targets its
constraint support. One frozen eight-trial experiment within unchanged total
budgets admits PIC90 on branch two (14 rounds, 619 solves, 283251 pair checks).
A separate anchored ECC failure remains rejected and hierarchy90 remains exactly
unchanged. Known-answer controls distinguish vertical freedom from an otherwise
identical horizontal-only impossibility. No new branching or retry mechanism was
added. The earned fresh internal PIC90 route completes 34/34 in pass zero in
607.23 seconds, with zero opens/physical/ERC/parity findings, 34 vias and
2874.415 mm stored copper. Exact admitted poses, outline, project, schematics
and frozen source hashes are preserved. Its 14266.423122 mm² area is 90% of
the original 15851.5812 mm². Thirty silkscreen findings still prevent full native
admission. The original all-free control had 39 vias, 2798.900 mm and 308.01
seconds on the older 1f3a binary; generated routing configuration/order match
exactly, but this is not an isolated timing comparison.
[Placement animation](../build/whole-board-reset-2026-09-08/placement/branch-eight/pic-reduced-8/area-00/trace/index.html).
[Smaller-board routing result and combined render](../build/whole-board-reset-2026-09-08/placement/branch-eight/results.json).

One frozen held-out ECC83 all-free placement/routing case also succeeds at 90%
area (2172.334337 mm²). Placement uses 23 solves and 13755 checks; branch recovery
is unused, so this checks preset generalization rather than proving a second
branch rescue. Fresh internal routing completes 9/9 in pass zero in 112.96 seconds,
with one via, 240.039 mm copper and zero opens/physical/ERC/parity findings.
Seven silkscreen findings remain. Source hashes, admitted poses, outline,
project and schematics are exact. The frozen routing policy is the same
completion-first adaptive policy used for PIC90, not a timing-matched replay of
the older explicitly ordered ECC comparison. Interf was considered first but
excluded from this unchanged placement adapter because of missing courtyard
geometry; the successful fallback does not erase that input-coverage limit.
[Held-out result and combined render](../build/whole-board-reset-2026-09-08/placement/heldout-ecc83-area90/routing-results.json).
[Placement animation](../build/whole-board-reset-2026-09-08/placement/heldout-ecc83-area90/trial/area-00/trace/index.html).

Supplemental generated SMD cases preserve the same placement under blocked/open
pad-gap rules. They already exposed a trimming bug: an in-pad via is deleted,
leaving a back-layer track disconnected from a front-only pad. An exact-parent
control retains the via and passes native verification with unchanged search
work. The general endpoint-trimming fix passes 145 KiCad tests and the native
control. A fresh ECC83 regression completes all nine nets with unchanged copper
and findings. On the fresh blocked-pad-gap SMD case, the corrected executable
completes all 19 selected connections in pass zero (329.5 seconds), with zero
native opens/design/ERC/parity findings and 21 unchanged metadata warnings.
The original exhausts four passes at 11/19 and 12 opens (1,551.7 seconds).
The corrected open-pad-gap case also completes all 19 connections in pass zero
(220.8 seconds), with the same clean native categories and unchanged metadata
warnings. Its original control exhausts four passes at 9/19 and 14 opens
(977.3 seconds). Both corrected results retain
all eight rule areas and the exact common placement. These are supplemental
generated-case results, not complete ESP32 product results.

Coincident front/back SMD pads now remain separate electrical terminals through
routing, saved-candidate round trips, discovery and expected-open accounting.
Via-only copper is accepted by the affected materialization/import boundaries.
All 148 KiCad tests pass. Matching two-/three-contact native projects now route
their complete net with one required via and zero native findings; the fresh
ECC83 regression preserves original copper and its two annotation findings.
[Electrical-terminal results](../build/whole-board-reset-2026-09-08/global-routing/electrical-terminals/results.json).

The cold-routing adapter now reads effective copper-edge and hole-to-hole
clearances from KiCad, including omitted project fields. Explicit malformed
values and unsupported custom rules still fail before native loading. Four
focused tests, zero/nondefault readbacks and an unchanged ECC83 compiled
configuration pass. Matched omitted/default-explicit projects produce identical
one-net candidates, search work and PCB bytes, with zero native findings and
automatic renders. Source project bytes remain unchanged; no manufacturing
defaults are guessed or written back.
[Native rule controls](../build/whole-board-reset-2026-09-08/restrictions-runtime/effective-native-rules/native-comparison.json).

A native control exposes another lowering gap: a clean cold source with a
0.50 mm board minimum and a 0.40 mm class preference produces a connected
0.40 mm candidate that KiCad rejects for track width. The compiler now takes
the maximum of the native preference and global minimum. The same source
routes cleanly at 0.50 mm with 21 expansions; an ordinary 0.40 mm control retains
exact candidate/work/PCB/native findings. Explicit zero, lower and omitted
minimum readbacks, malformed-value guards, unchanged ECC83/PIC configurations
and source/render checks pass. This corrects track-width lowering; other global
minima are not established by this control.
[Width control and correction](../build/whole-board-reset-2026-09-08/restrictions-runtime/minimum-track-width/after/comparison.json).

A two-net native coupon proves the corresponding global-clearance gap: class
clearances of 0.25/0.28 mm permit an internally accepted 0.30 mm gap, which
KiCad rejects against the board minimum of 0.35 mm. The compiler now floors
each class clearance at KiCad's effective global minimum. On the unchanged
source, legal pad access closes the target with zero native findings and the
same 61 expansions/cost 6,000. A lower-minimum control preserves exact archived
candidate/PCB geometry; ECC83, PIC and blocked/open SMD compiled configurations
remain unchanged. Native zero/omitted readbacks, validation guards and automatic
renders pass. Existing foreign-net/pad clearance maxima were already correct
and remain unchanged.
[Clearance control and correction](../build/whole-board-reset-2026-09-08/restrictions-runtime/minimum-clearance/after/comparison.json).

Native class extraction now accepts KiCad-resolved composite classes directly,
including inherited fields from Default. A partial Foreign class resolves the
same via/drill geometry as its complete equivalent; candidate, PCB and native
findings match exactly with zero violations. Four guard tests pass, and all five
existing benchmark configuration controls remain unchanged. Global width and
clearance floors still apply.
[Composite-class control and render](../build/whole-board-reset-2026-09-08/restrictions-runtime/composite-native-classes/comparison.json).

Matched diagnosis benchmark (2026-09-08): on the same frozen 79-net `interf_u`
board, eight history-priority removals find no route for PC-RD; eight
boundary-priority removals find seven target routes and four fully restored
candidates. The selected ENBBUF/PC-RD repair has 105 opens and no native
design/ERC/parity findings. Native audits and exact copper receipt checks pass.
This is an isolated 80-net alternative. The history/compound sequential run is
now independently audited at 83/110 nets and 100 opens, preserving all 67
inherited commits. It stops at PROG- after unsuccessful single and pair
diagnoses. Boundary priority is being tested there without new target hints.
[Comparison, provenance and combined views](reviews/2026-09-08-history-versus-boundary-diagnosis.md).

Compound routing recovery (2026-09-08): the sequential router now automatically
reproduces the 66/110-net `interf_u` result from its committed 65-net checkpoint,
reducing native opens from 125 to 123 with no design/ERC/parity findings. It
tries bounded pair removal and both restoration orders, then uses fresh boundary
evidence during nested restoration. Independent compound and sequential audits
pass, including all 65 inherited boards and the actual coordinates/layers of
every final changed-route receipt. Four planted incorrect receipts are rejected.
The Rust policy remains opt-in; 143 KiCad and eight benchmark tests pass.
The disabled control retains 65 nets unchanged. Audited schema-8 continuation
reaches 67 nets and 121 opens by repairing PC-A8 through PC-A4; PC-A9 is next,
with repair skipped only because that run consumed its one-invocation allowance.
[Implementation and evidence](reviews/2026-09-08-compound-routing-recovery.md).

Reachable-boundary recovery (2026-09-08): a verified experimental `interf_u`
result reaches 66/110 routed nets and 123 opens, with no native design/ERC/parity
findings. Soft forecasts alone failed; exact prepared-grid maps showed that
BUS1 gained broad access while U2 remained isolated. Boundary-guided repair
then restores PC-A7 and PC-A2. The new optional automatic priority reproduces
the final repair with no named hints and byte-identical selected copper.
Original-parent and repair audits preserve all other connections, poses, copper
and rules. All 142 KiCad, 15 grid-router and eight benchmark tests pass; disabled
priority replays the prior candidate/errors/work exactly. The complete compound
recovery sequence still needs coordinator integration; the retained sequential
journal remains at 65 nets. [Implementation, region maps and native results](reviews/2026-09-08-reachable-boundary-recovery.md).

Terminal obstruction coverage (2026-09-08): all 65 single-net removals fail
on PC-A7. Five of the 64 tested pairs containing PC-A6 permit a counterfactual
route, but all ten direct restoration orders fail. Typed endpoint context
separates the blocked U2–BUS1 branch from the later branch toward U9. A reciprocal
repair confirms that PC-A6 and PC-A7 repeatedly block each other's connector
escape. No new routing progress is admitted: 65/110 nets and 125 opens remain.
All 19 materialized pair stages preserve unrelated copper, poses and rules;
the full-single and reciprocal native audits pass. The program now retains
terminal-specific failures, supports pair-only diagnosis and renders compact
coverage reports. All 141 KiCad and eight benchmark tests pass. Next, test
alternative connector escapes that account for displaced-net restoration.
[Coverage, implementation and combined renders](reviews/2026-09-08-terminal-obstruction-coverage.md).

Nested restoration and compact analysis (2026-09-08): enabling one bounded
nested repair level resolves the PC-A4 → PC-A3 → PC-A2 chain. Continued routing
reaches 65 of 110 nets / 125 opens with no native design findings. Both sequence
audits and all four repair trees pass independent checks. PC-A7 remains
unresolved after eight sampled blockers; the diagnosis is truncated. A new
automatic audit report exposes search failures, protected nets, restoration
chains, bounds and native results with linked combined copper views. Missing
views are cached separately from immutable inputs; summary, cache and browser
controls pass. The placement-routing driver supplies conservative nested bounds
when rip-up is enabled and no restoration policy is declared. [Evidence and
compact reports](reviews/2026-09-08-restoration-analysis.md).

Recent-change obstruction diagnosis (2026-09-08): frozen checkpoints show
PC-A0 alone closes PC-A1's passage. Routing PC-A1 first and restoring PC-A0
passes native checks. Yielding diagnosis now accepts ordering hints, and the
sequential coordinator prioritizes recently changed nets, including restored
nets. Under a matched one-trial budget, recent order repairs PC-A0 (57th in
lexical order); lexical order fails. A broader continuation reaches 62 of 110
nets / 131 opens with no native design findings, compared with 57 / 140 before
this change. All three sequence audits, 139 KiCad adapter tests and eight
benchmark tests pass. PC-A4 now fails because its yielded PC-A3 cannot be
restored; the provisional result is rejected. [Evidence and combined renders](reviews/2026-09-08-recent-route-obstruction.md).

Whole-board budget guidance (2026-09-08): `interf_u`'s OE- failure was a
reachable fine-grid search that exhausted four million A* expansions. Existing
obstacle-distance guidance finds a native-admissible path. The whole-board
coordinator now retains typed failures and failed-search work, and conditionally
retries only exhausted searches with otherwise identical settings. Resumption
advances the retained 55-net board to 57 nets / 140 opens, with no native design
findings. PC-A1 and the separate MD5 control are grid-disconnected and skip the
retry. All three native sequence audits, 137 KiCad adapter tests and eight
benchmark tests pass. ECC83 completes all nine nets with every candidate and
final copper hash identical to the archived control; its guided entries are
skipped and forecasts retain their ordinary attempt count. The placement-routing driver adds these conditional entries
by default; direct JSON remains opt-in. [Implementation and combined renders](reviews/2026-09-08-whole-board-budget-guidance.md).

Terminal copper access recovery (2026-09-08): a new local diagnostic identifies
JP1's blocked nearest grid sample and clear contacts elsewhere on its existing
pad copper. `FirstPadContact` now uses such contacts when ordinary snapping
fails, with pad-internal continuity and permitted-layer checks. The frozen PIC
four-net prefix's failed VCC_PIC insertion passes native gates at both 0.5 and
0.125 mm, reducing opens from 69 to 59 with no new findings. The formerly
working 0.25 mm candidate is exactly unchanged. All 134 KiCad adapter and eight
benchmark tests pass. The faster coupled placement now also cold-routes all 34
PIC nets on the 0.5 mm grid alone, in one pass, with zero opens and six unchanged
silkscreen findings. Sequence and adaptive-selection audits pass; full-layout
admission remains false. [Evidence and combined renders](reviews/2026-09-08-terminal-copper-access.md).

Coupled placement recovery (2026-09-08, following the user's convergence
feedback): weights were constant; sequential pair corrections propagated
movement slowly through crowded groups. Bounded joint translation recovery
reduces PIC placement from 2,039 ordinary sweeps to 128 sweeps plus three joint
rounds. Same-input/executable tracing runs measure 42.975 versus 2.876 seconds.
Native geometry/inventory pass, and the new placement's first routing pass
connects all 34 nets with zero opens and six unchanged annotation findings.
Independent sequence and adaptive-selection audits pass.
The 68-component geometry control needs two joint rounds after sweep 128;
disabling recovery replays the earlier trace/result exactly. All 47 placement
tests pass, including fixed/axis/bounds controls and transactional rollback.
The native-project driver enables this recovery by default, with an explicit
pairwise comparison option; direct policy JSON remains opt-in.
[Implementation, animations and measured limits](reviews/2026-09-08-coupled-placement-recovery.md).

Placement failure diagnosis (2026-09-08): the new optional legalization tracer
shows that the PIC run is converging slowly rather than stuck. An explicit
larger budget reaches a legal placement at sweep 2,039; native geometry and
inventory audits pass for all 63 footprints. Six annotation findings and 125
cold opens remain before routing. Two routing passes then stop after four nets
at 69 opens: a back-side terminal fails grid access even on the empty board.
A 0.25 mm probe connects that net with no new findings (59 opens), while
0.125 mm fails access again. Routing has resumed from the verified four-net
prefix with a coarse/fine ordered portfolio. That resumed run has now finished
all 34 nets with zero opens; its independent sequence audit passes. Tracing
reproduces the prior 68-component result exactly; all 42 placement tests pass,
and browser seeking/playback checks pass. Motion extrapolation reduces residuals
but remains rejected and experimental. The 110-net run's first pass is audited
at 55 nets / 143 opens with four committed repairs and no design findings;
its second pass is now terminal at 52 nets. The first pass remains selected;
both sequence audits and the adaptive selection audit pass.
[Evidence and animations](reviews/2026-09-08-placement-legalization-trace.md).

Native placement coverage (2026-09-08): the PIC adapter now imports all 63
footprints, including rectangle primitives, three non-rectangular courtyards
and the back-side solder jumper. Convex-part unions preserve usable notches;
body/pad exclusions honor component sides. Disconnected spectral seeding also
positions all six mounting holes without reading their original centers.
The human geometry control passes under explicit reference overhang allowances,
and the successful larger-board seed/poses replay exactly. Custom-pad shape
translation and mutation controls pass. All 18 model, 41 placement, 130 KiCad
and eight benchmark tests pass. Cold PIC legalization still fails after 512
sweeps at C2/U5; no new completed placement or area result is claimed. Capturing
the rejected poses/contact history is the next diagnostic.
[Implementation, controls and renderings](reviews/2026-09-08-native-placement-geometry.md).

Completion-first routing (2026-09-08): the native adaptive coordinator now
stops after its first accepted zero-open result. Further quality passes require
`optimize_after_routing_complete: true`. Native ECC83 controls pass for default
stopping, explicit optimization, retained annotation failures and continued
incomplete search. First-pass copper matches the archived reference exactly;
all 130 KiCad and eight benchmark tests pass. This avoids spending the remaining
budget on via/length work once routing is complete. Full-layout admission still
requires every native gate.
[Implementation, controls and combined views](reviews/2026-09-08-completion-first-routing.md).
The next family-level experiment routes all 110 `interf_u` nets from its fixed
reference placement with native classes and the unchanged non-convex outline.
It is still running. The external zero-edge-clearance translation is unsupported;
a separately labelled stricter-edge control leaves five native opens and no
design findings, and is not an equal-rule comparison.

Net-aware cold placement (2026-09-08): the larger board's spectral rank seed
now routes **all 50 nets with zero native opens in its first pass**. Three
unchanged silkscreen findings remain, so full-layout admission and area credit
are still withheld. The matched random placement's finished first pass routes
30 nets and leaves 30 opens. Both preserve the same components, rules and area;
both two-pass jobs are terminal. Both spectral passes connect every net;
the random job's retained result leaves 30 opens. Both retain pass 0. Independent sequence,
selection and spectral repair-tree audits pass. The general native-project runner now exposes `--placement-seed
spectral_rank` and reproduces the proven seed and accepted poses exactly.
External controls expose another exchange issue: the source front layer is
declared `power`, leaving only B.Cu active in earlier Freerouting runs. With an
explicit two-signal-layer adaptation, Freerouting leaves zero opens on the
spectral placement and two on the random placement. This adaptation and the
remaining geometry-equivalence limits are recorded explicitly.
[Evidence, integration and combined copper views](reviews/2026-09-08-placement-unlocks-routing.md).

Unanchored placement and edge exchange (2026-09-08): the larger all-free
benchmark does not receive harmonic positional attraction. All 68 components
use a random fallback; setting every net's attraction weight to zero reproduces
every pose exactly. A first net-aware spectral rank seed reduces the
connectivity-distance surrogate from 12,321.361 to 6,535.585 mm and estimated
cross-net crossings from 1,597 to 107. Production legalization and native
inventory/geometry audits pass at unchanged area, rules, 68 components,
165 pads and 50 nets. Three annotation findings remain after repair. This seed
was unrouted at that stage; the follow-up above establishes electrical routing
completion while retaining the outstanding annotation findings.
The external edge adapter now matches the nominal native global edge rule
while preserving all non-edge clearance matrix entries. The larger input's
14 external edge findings disappear, but its rerun still ends at 66 native
opens with seven annotation findings. Strict-rule and unrepresentable-rule
controls pass. ECC83 exposes a separate submicrometre boundary-export rounding
issue; full exchange equivalence remains unproven.
[Findings, controls and native views](reviews/2026-09-08-unanchored-placement-and-edge-exchange.md).

Recovery coverage and exchange inspection (2026-09-08): expanding nested
recovery from one to eight permitted invocations exercises all six available
siblings, 204 untruncated single-net child diagnoses and 24 child restoration
attempts. None commits; the **34/50-net, 24-open** board is retained byte for
byte and both independent audits pass. This closes the earlier coverage gap,
not the whole-board routing problem.
The external comparison exposes an input-contract issue: Freerouting's loaded
edge requirement is effectively 0.31 mm versus the native project's 0.01 mm.
A new automatic diagnostic reads the external rule matrix and renders its
14 layer-specific pad/edge findings. Exact-pose session import now restores
only bounded coordinate rounding and rejects real moves, rotations and
unsupported resolution. Re-importing the existing session leaves 65 native
opens and seven unchanged annotation findings; it is explicitly not an
equal-rule comparison. ECC83 also exposes the edge discrepancy despite its
historical external result being complete, so these findings do not explain
the larger failure by themselves. No new router execution or placement
infeasibility is claimed.
[Evidence, controls and combined views](reviews/2026-09-08-recovery-coverage-and-exchange.md).

Automatic nested recovery (2026-09-08): the coordinator reproduces the two-net
repair and continues to **34/50 routed nets / 24 native opens**, advancing from
27 nets / 36 opens across three resumed invocations. Four new committed repairs
restore their displaced nets; other new nets route normally. All committed
prefixes and all repair alternatives pass independent audits. Existing seven
annotation findings remain, with unchanged placement, rules and unrelated
copper. Rollback preserves the original board, altered candidate receipts are
rejected on resume, and ECC83 remains byte-identical. All 129 KiCad and eight
benchmark tests pass. The repair auditor now uses hashed immutable input
snapshots rather than a caller result that can change after a repair commits.
At that stage, `Net-(R212-Pad2)` exhausts its one nested invocation on a
truncated child diagnosis; five sibling repairs receive no nested search. Broader
child/sibling coverage is now tested above, without additional progress. Neither
run establishes a placement failure.
[Integration, limits and combined views](reviews/2026-09-08-nested-routing-recovery.md).

Whole-board restoration (2026-09-08): an audited two-net repair advances the
larger board from 27 nets / 36 opens to **28 nets / 34 opens**. Inserting
`Net-(C201-Pad2)` requires rerouting `+12V`, whose restoration also requires
yielding and restoring `Net-(U102-CAP+)`. Seven existing annotation findings
remain, with no new findings or disconnections and unchanged unrelated copper,
placement and rules. Automatic integration of this nested recovery remains the
next step; the current coordinator still stops at 27 nets.
The new opt-in multi-source tree attachment mode removes sampled-source limits.
It does not solve this conflict by itself. On cold ECC83 it reproduces the
complete board byte for byte with 91.1% fewer A* expansions. All 127 KiCad tests
pass. [Evidence and combined views](reviews/2026-09-08-tree-attachment-and-restoration.md).

Active priority (2026-09-08, user-directed): broaden autonomous whole-board
placement and routing. Local via/length optimizations are
[deferred TODOs](optimization-todos.md). The real-board runner now accepts
native projects, source net classes and explicit movement/outline policies.
ECC83 completes through the general path. A cold 68-component/50-net
`complex_hierarchy` run exposed and fixed ignored pad copper offsets.
Explicit provisional annotation handling now lets the adaptive coordinator run
on this generated placement: future-net demand reduces open items from 58 to
42 (13 to 23 routed nets). A learned order routes 30 nets but leaves 65 open
items, so it does not displace the better result. Seven source annotation
findings remain; no completed larger board or area improvement is claimed.
The blocked target now has a native-checked repair: four single-net yielding
choices can be restored after target insertion, reducing opens from 42 to 41.
The coordinator now invokes this repair automatically and continues routing.
The completed demand-guided run advances from 23 nets / 42 opens to 27 nets /
36 opens, committing two repairs. Its first 23 boards match the preceding run
byte for byte. A third repair cannot restore `+12V` after inserting
`Net-(C201-Pad2)`; 0.25 mm grid probes also fail in these frozen contexts.
All 124 KiCad tests passed at that stage, and enabled-but-unused recovery reproduces the clean
ECC83 board exactly. The separate no-demand comparison was interrupted at a
25-net checkpoint while evaluating another repair; it is not a completed run.
Checkpoint resumption now retains those 25 verified commits and advances one
bounded step to 26 nets / 37 opens. Its independent audit reproduces the entire
inherited prefix. Corrupt-commit rejection, torn-output recovery and clean ECC83
controls pass; all 126 KiCad tests pass. The demand-guided 27-net result remains
the better partial board. See [checkpoint resumption](reviews/2026-09-08-sequential-checkpoint-resume.md).
[General pipeline](reviews/2026-09-08-general-project-layout.md) ·
[Adaptive integration](reviews/2026-09-08-provisional-adaptive-routing.md) ·
[Partial native rip-up](reviews/2026-09-08-partial-native-ripup.md) ·
[Automatic recovery](reviews/2026-09-08-sequential-ripup-recovery.md).

Plated pad analysis (2026-09-08): imported copper can now connect branches
through plated pads without inventing drilled vias. Remaining-via coverage on
the improved PIC board rises from 3/9 to 9/9 via-bearing nets; ECC83 rises from
1/2 to 2/2. All eleven independent real-net round trips pass native verification,
and all 119 KiCad tests pass. Via counts remain three and ten. The scanner now
states that isolated vias lie outside its current pair-excursion search.
[Evidence and next limitation](reviews/2026-09-08-plated-pad-import.md).

Adaptive native routing (2026-09-08): `route-kicad-board-adaptive` now generates
future-copper forecasts, learns net priorities from failures and cost inflation,
and retains its best native-verified board across bounded distinct passes.
It reproduces the three-via / 345.004 mm ECC83 result automatically, recovers
from a known bad order, and rejects a later cheaper but incomplete board.
Budget exhaustion retains the cold source with an explicit incomplete result.
The integrated command also reproduces PIC's reduction from 18 to 10 vias with
source classes intact. Combined front/back copper is now the primary comparison
view. Placement remains fixed in these routing experiments.
[Implementation and evidence](reviews/2026-09-08-adaptive-native-routing.md) ·
[Demand/order controls and PIC holdout](reviews/2026-09-08-routing-demand-and-order.md).

Original PIC net classes (2026-09-07): the program now completes all 34 PIC nets
with unchanged source project settings and native class assignments. GND/VCC
use 0.8 mm tracks and 0.28 mm clearance; Default uses 0.5/0.25 mm. Per-net
geometry also supplies foreign-net clearance floors to routing and repair.
The result has zero native findings/opens, 1826.911 mm and 18 vias. Native
readback audits every copper width/via dimension and every component pose.
All 108 KiCad and seven benchmark tests pass; disabled behavior reproduces
the earlier complete route byte-for-byte. A matching source-class Freerouting
export remains outstanding.
[Evidence and playback](reviews/2026-09-07-pic-source-net-classes.md).

Complete PIC cold routing (2026-09-07): explicit shared-tree junction vertices
resolve the dangling GND rejection. The adapted-rule benchmark advances from
one to all 34 nets: zero native findings/opens, 1867.509 mm, 20 vias, unchanged
placement. Freerouting leaves one open on the same input. A controlled split
preserves exact native copper support; a generated regression fails before the
fix and passes afterward. All 106 KiCad tests pass. The final executable replays
all 34 candidates and boards identically and receives fresh native verification.
Original POWER-class rules and the placement-area objective remain outstanding.
[Evidence and playback](reviews/2026-09-07-pic-shared-tree-junctions.md).

Whole-board objective reset (2026-09-07): complete real-board cold routing and
minimum verified complete board area now define the main progress measures.
A new reference suite audits all five pinned upstream boards (all have zero
opens) and replays the two executable paired whole-netlist comparisons. ECC83
completes with both tools under an explicit capability budget: pcb-maker
254.206 mm / 0 vias; Freerouting 254.225 mm / 0 vias, with no new findings.
PIC exposes a dangling GND candidate at the second insertion. Nine area probes
remain unadmitted: conservative placement rectangles reject the human reference
itself, demonstrating the need for correct bodies and overhang constraints.
[Benchmark contract](../experiments/whole-board/README.md) ·
[Results and renderings](reviews/2026-09-07-whole-board-objective.md).

Conflict holdout and native net identity (2026-09-07): the PIC holdout exposed
a display-layer-name bug in the geometric diagnostic and fragile slash-prefixed
net spelling in copper application. The diagnostic now uses native layer IDs;
application resolves exact pad net names and rejects ambiguous aliases. A
matched native VCC-yielding probe preserves GND and all other retained copper,
connects DATA-RB7 without design findings, and correctly remains incomplete
until VCC is restored. All 104 KiCad tests pass.
[Evidence, limitations and views](reviews/2026-09-07-conflict-holdout-and-net-identity.md).

Orientation dense validation (2026-09-07): refined placements complete nineteen
connections under both profiles with all 38 prefixes native-clean. Copper
geometry and A* counts match the unrefined reference at every prefix. Preflight
work is about 9.6–9.7% higher, so this supports local quality and preserved
completion, not a global speed claim. Refinement remains optional.
[Evidence and comparison](reviews/2026-09-07-orientation-dense-validation.md).

Production orientation refinement (2026-09-07): initial placement and archives
can now opt into bounded, legality-checked quarter-turn refinement after
legalization. The full board accepts nine rotations across eight components
without moving centers; a matched native LED probe shrinks from 5.441 to
2.027 mm, with zero vias and clean design checks. All 33 placement tests pass,
and an independent audit checks all 297 trial scores and exact replay. This
remains opt-in; the dense validation above preserves copper but shows higher preflight work.
[Implementation, evidence and animation](reviews/2026-09-07-production-orientation-refinement.md).

Corrected placement dense follow-up (2026-09-07): both clearance profiles now
complete nineteen connections with ordinary routing, no recovery, and all 38
accepted prefixes native-clean. Blocked: 314.514 mm / 25 vias; open: 316.530 mm /
23 vias. This preserves all corrected harmonic poses and retains automatic
previews. Historical comparisons show a mixed length/via tradeoff.
[Evidence and playback](reviews/2026-09-07-corrected-placement-dense.md).

Orientation analysis (2026-09-07): a fixed-center counterfactual tool ranks
quarter-turns, exposes per-branch tradeoffs and validates them against production
placement constraints. Eight of its twelve leading suggestions are legal; four
are rejected for geometric conflicts. Its top suggestion, flipping R_LED,
reduces the focused native LED route from 5.441 to 3.441 mm with clean checks.
Planted controls recover a known solution and reject invalid rotations. This is
a diagnostic; production refinement is now available as described above.
[Local before/after evidence](reviews/2026-09-07-orientation-counterfactuals.md).

Harmonic locality fix (2026-09-07): region bounds now participate in harmonic
relaxation, and harmonic/port attraction honors declared `tension_weight`. The
benchmark already marks GND with a weak 0.05 weight. LED/resistor separation
falls from 19.374 to 3.317 mm; the focused native LED route falls from 20.240 to
5.441 mm with zero vias and clean design checks. All 26 placement tests pass.
The dense follow-up above now covers this changed placement.
[Evidence and before/after](reviews/2026-09-07-harmonic-locality.md).

Composed recovery (2026-09-07): the earlier junction placement under blocked-gap
rules now advances automatically from seven to nineteen connections by combining
one-net rip-up with conditional budget guidance. All twelve commits retain
native-clean previews, unchanged poses and unchanged unrelated copper.
[Evidence and diagnostic ranking hint](reviews/2026-09-07-composed-recovery.md).

Automatic budget recovery (2026-09-07): native insertion now consumes
structured search failures and can conditionally invoke obstacle-distance
guidance when ordinary routing exhausts its budget without a candidate.
The retained harmonic/blocked board progresses from 16 to 19 automatically;
guidance is skipped for its next two ordinary successes and for the separate
grid-disconnected control. The last two routes use 55.4% fewer search-state
expansions than always-on guidance, with identical copper. The policy is
opt-in, native-verified and rendered; 102 KiCad library tests pass.
[Implementation and evidence](reviews/2026-09-07-automatic-budget-recovery.md).

Dense placement follow-up (2026-09-07): six frozen 19-connection probes now
compare retained, harmonic and junction seeds under both ESP pad-gap profiles.
All three finish with open-gap rules; harmonic uses 322.270 mm versus
retained's 477.701 mm, with 21 versus 18 vias. Blocked harmonic stalls at 16
on search budget, then reaches 19 with obstacle-distance guidance. Blocked
junction stalls at seven on grid disconnection; isolation identifies retained
copper as the cause, and existing one-net rip-up restores a native-complete
eight-connection board. These are separate diagnostic recoveries. All 99
accepted baseline prefixes retain checked native previews.
[Evidence](reviews/2026-09-07-dense-placement.md) ·
[Matched-prefix playback](../build/placement-dense-prefix19-2026-09-07/index.html).

Occupied-passage follow-up (2026-09-07): an opt-in pressure policy now counts
retained foreign copper when sizing a passage-opening move. A three-net
regression advances from 2/3 with no proposed repair to 3/3 after one 0.3 mm
wall move; the exact semantic copper passes native KiCad under explicitly
matching edge-clearance rules. [Evidence and scope](reviews/2026-09-07-occupied-passage-repair.md).

Algorithm exploration (2026-09-07): pressure-field routing, capacity-aware
topology, actual continuous-engine playback and broader placement now have
reproducible experiments and retained animations. Five selected placements
native-complete the same three-connection probe; the existing full-size
harmonic placer is the strongest of those partial probes. The new evidence
favors congestion-directed topology and placement feedback. Production defaults
are unchanged. [Assessment](reviews/2026-09-07-algorithm-exploration.md) ·
[Visual dashboard](../build/algorithm-exploration-2026-09-07/index.html).

Latest native routing checkpoint (2026-09-07): the paired ESP32 clearance
profiles both complete 25 signal connections from retained verified boards.
With the original two-million A* budget, blocked routing stops at 27 and open
at 31. Exact-input diagnostic replays prove the next routes are natively valid
with more search work. The adapter now distinguishes budget exhaustion from
grid disconnection. [Evidence and the next search-guidance experiment](reviews/2026-09-07-dense-prefix-routing.md)
include automatic renderings. VCC/GND still require their declared wider
branches; the signal benchmarks do not establish complete-board routing.

The sections below preserve the earlier capability history.

This is the evidence-backed capability boundary as of 2026-09-04. “Solved”
always means the durable candidate passes the independent exhaustive geometry
and electrical gates; provisional router output is reported separately.

The first M0 competitive runner is now operational. A versioned manifest can
regenerate the dual-ESP32 prefix-19 semantic input before placement, enforce
zero inherited copper, run a hash-pinned Freerouting build under explicit
budgets and rules, import its session, exact-gate it in KiCad, and retain a
self-contained report plus source/result front/back images. Sequence 198
reproduces the 24/24, 161-segment, 16-via baseline. Sequence 201 runs both tools
from one manifest on prefix 3 and proves equal cold placement/rules plus two
native-complete results. Freerouting uses 14 segments/no vias in 3.50 router
seconds; pcb-maker uses 22 segments/two vias in 40.86 process seconds, with
per-connection native admission included. Sequences 204/205 add a fail-closed
content-addressed cache for those native checks. Seven warm hits reduce the
pcb-maker subprocess from 38.824 to 3.965 seconds and the full paired run from
54.034 to 19.170 seconds while preserving the final board byte-for-byte.
Sequence 210 adds a checkpointed corpus aggregate and independently reruns
dual-ESP32 prefixes 0--3. Both routers exact-complete all four cases under
matching placement/rules; the run records 16 cache hits and 32 uniquely named
front/back progress images. Sequence 211 adds shared physical centerline,
overlap, track/via/zone/board-area, and per-layer metrics; it exposes that
Freerouting keeps prefix 3 on one layer while pcb-maker uses two vias and both
layers. Sequence 218 expands to twelve independent route-only mechanisms. Both
routers native-complete all twelve within the 60-second per-competitor cap on
the warmed host; the run retains 96 unique images and exposes distinct
one-layer-detour versus two-via-direct crossing strategies. Sequences 212 and
214 remain as diagnosed template/legacy-adapter failures, and sequence 217
proves timeout-aware selection of an exact partial checkpoint. The larger
ten-net case finishes at 59.949 seconds only after 11 native-cache hits, so this
is not a cold-runtime parity claim. The broader frozen real/holdout corpus is
still missing, so M0 is not complete. Sequence 221 adds the first executable
pinned real-board source: the cold adapter removes 59 inherited ECC83 tracks
and one copper zone without changing placement. Sequence 224 is the first
formal ordinary-KiCad paired result. Both routers consume the exact same
adapted board and placement, connect all 20 native items, and introduce no ERC,
DRC, or parity finding. Freerouting uses 51 segments/no vias/254.225 mm;
pcb-maker's generic sequential route mode takes 523,627 expansions and 0.803
process seconds but uses 75 segments/10 vias/254.670 mm. Completion parity on
one small fixture is real; route topology quality and corpus breadth are not
yet competitive. Sequences 225--230 make the second pinned source executable:
the 63-footprint PIC programmer has 125 native items. Freerouting reaches
124/125 with no DRC regression. pcb-maker now transactionally native-gates
each direct route, honors bottom-only pads, and excludes phantom snapped tree
contacts; it retains a clean four-net frontier before a custom solder jumper
requires a terminal access point other than the pad anchor. This is a typed
terminal-escape blocker, not an A* budget conclusion.

Sequence 232 resolves that typed blocker without a speculative fanout search:
an opt-in materializer clips anchor-directed paths at first contact with exact
filled custom-pad polygon copper. JP1 and `CLOCK-RB6` native-commit, advancing
the clean PIC frontier from four nets/69 opens to six nets/54 opens. The JP1
route is 12.035 mm shorter with 19 fewer segments and 14 fewer bends for 0.48%
more A* expansions than the rejected anchor control. `DATA-RB7` then has no
path after exhaustive reachability, so the active blocker has moved to
route-order/multi-pass repair.

Sequence 233 confirms the diagnosis: `DATA-RB7` exact-routes first on the same
zero-copper placement (125 -> 119 native items, zero new findings) with 23
stored segments, no vias, and 114,251 expansions. It is feasible in isolation;
the preceding six committed nets consume the needed capacity. More A* budget
is not the next lever because the failed reachability search was exhaustive.

Sequences 234--238 turn that observation into a causal order result. Yielding
either GND or VCC makes `DATA-RB7` reachable; the compatible three-net
precedence is `DATA-RB7 -> GND -> VCC`. A full zero-copper replay reaches 16/34
nets and 31 remaining native items before `PC-DATA-IN` becomes unreachable,
versus six nets/54 items for discovery order. The checkpoint has 51 vias, so
this is a completion advance rather than a topology-quality claim.

Route-only native admission now hashes the complete frozen ERC input tree and
reuses ERC only while that digest is unchanged. KiCad PCB DRC still runs with
schematic parity and exact connectivity on every transaction, and a final full
verification agrees. This cuts typical per-step time from about 6.5 to 3.6
seconds (roughly 43%); KiCad DRC, not A*, remains the dominant iteration cost.
At the new frontier, bounded counterfactuals identify `DATA-RB7` and GND as the
causal blockers for `PC-DATA-IN`, giving the next ordinary-KiCad order-search
trial concrete evidence rather than a blind permutation.

Sequence 239 closes that architecture gap. A bounded ordinary-KiCad order
coordinator retains parent/proposal/diagnosis lineages and reroutes every child
from the immutable zero-copper source. Its first child moves `PC-DATA-IN`
before diagnosed blocker `DATA-RB7` and reaches the configured 17-net bound
with 30 remaining items and zero native findings. Selection correctly favors
that farther clean frontier over its worse 303-segment/57-via topology. The
mechanism works; broader search and topology repair remain open.

Sequences 240--244 add the third executable pinned source and first
medium-real case: KiCad's 25-footprint, 110-routable-net `interf_u` board.
Legacy footprint links are normalized reproducibly in a copied project, all
inherited copper is removed without changing placement, and the adapter now
routes against its exact non-convex T outline. A cold one-net smoke exact-passes.
The 0.5 and 0.25 mm graphs then prove topologically unable to escape dense PGA
pads; 0.125 mm succeeds, so this is not an A* budget problem. A 16-net cold run
native-admits every requested net. Making fine resolution an ordered fallback
instead of an always-run score competitor reduces expansions from 678,377 to
84,511 and routed-step time from 235.664 to 137.534 seconds, while the valid
lineage also uses 8 rather than 10 vias. Whole-board raster rebuilding and
per-rung KiCad DRC then dominate. Sequences 245--246 apply the existing
layer-aware obstacle broad phase to grid rasterization. The isolated fine route
is byte-identical and about 49% faster; the full 16-net board is byte-identical
while routed-step time falls again to 94.104 seconds, 60.1% below the original
exhaustive run. The full 110-net board remains unsolved.

Sequences 247--250 add the fifth pinned declaration and fourth executable real
source, KiCad's 68-footprint `complex_hierarchy` analog demo. General
hierarchy-aware footprint-link normalization repairs the root plus reused child
sheet while preserving the PCB bytes. Cold preparation removes 364 tracks,
166 copper zones, and two copper graphics and leaves exactly 112 native opens
with zero ERC/DRC/parity findings. A bounded four-terminal `-VAA` smoke routes
14 segments/87.769 mm/no vias and reduces the count to 109. The first smoke
also exposed stale copied result reports; candidate board and native reports
now commit together, and sequence 250 reproduces the earlier PCB byte-for-byte
while its result artifact correctly records 109 opens. The full 50-net board is
not yet attempted.

Sequence 231 exercises a third pinned source, the Olimex ESP32-C3 board, as a
rule-capability preflight. Cold adaptation removes 768 tracks, 88 vias, and 160
copper/teardrop zones while retaining placement and the rule-area keepout, but
the current uniform DSN contract cannot express the source's 1.85 mm local
mounting-hole pad clearances. Freerouting's visually complete result therefore
introduces 17 native DRC findings and still has two opens. The source is not a
router score: pcb-maker is deliberately not run until equivalent rule lowering
can be proved for both competitors.

Sequences 253--257 add the first honestly counted autonomous-placement
mechanism. Three tempting imported controls retain their declared poses because
their movable body is electrically unattached or their connected block lacks
two distinct positional anchors; they remain negative controls. The valid
fixture places a free two-pad resistor between fixed west/east terminals,
translates it 5 mm, rotates it 90 degrees, and reduces straight connectivity
distance by 2.790966 mm. Both routers solve the identical regenerated cold
placement with zero native findings. The benchmark now enforces minimum moved,
translated, and rotated component counts and minimum connectivity-distance
reduction before routing, so an unchanged placement cannot pass by agreement.
Sequence 259 adds a mandatory layer-transition mechanism: an F.Cu-only SMD
terminal must connect to a B.Cu-only terminal. Both routers use exactly one via
and two copper layers with zero native findings. Minimum via/layer counts are
now executable obligations on both results.

Sequences 260--270 add the fifteenth mechanism and correct a material adapter
gap discovered by its images. Semantic rule-area keepouts were generated in the
full template but silently removed during progressive materialization. They are
now preserved, component-body areas allow their own pads, explicit routing
keepouts still forbid pads, and the grid adapter rasterizes track-blocking
rule-area polygons per layer. `movable-blocker` now genuinely detours around
one retained body. A new both-layer 3 mm passage is disconnected on our 0.25 mm
grid after 5,362 reachability cells but both-solves at 0.125 mm; Freerouting
solves both. This is a typed raster-resolution failure, not insufficient A*
budget. The audit also proves that abstract per-net allowed-layer restrictions
are not yet lowered into KiCad net rules. M0 now has fifteen admitted mechanisms,
five pinned real declarations, and four executable real sources; the old
sequence-218 aggregate must be replayed because affected semantic source hashes
correctly changed.

Sequence 271 makes those non-physical constraints visible in every subsequent
native board render. A disposable render input overlays each applicable rule
area as an outline-only front/back silkscreen polygon and is deleted after
rendering; native verification and the durable board continue to use the
unmodified PCB. The accepted bottleneck images now expose both wall halves and
the trace through their 3 mm opening.

Sequences 272--275 add the sixteenth mechanism: cold routed copper can be
transactionally replaced by refilled GND planes before native admission. A
four-terminal net spans front-only and back-only pads. Freerouting finishes as
two zones/one stitch via; pcb-maker finishes as two zones/two stitch vias. Both
have zero remaining tracks, opens, or native findings. A shared cold-baseline
report is reused only after its board hash matches the current source. Plane
acceptance now requires zone, layer, and via counts independently from both
tools. Sequence 276 adds a pcbnew fill-and-save step before independent native
refill; both final PCBs now contain two serialized filled polygons and measure
2,049.837046 mm2 of layer-weighted filled-zone area. Sequence 277 requires at
least 1,800 mm2 in addition to the zone/layer/via counts, so empty declarations
cannot pass. An opt-in render-only outline and readable net label make each
plane visible on front/back journal images.

Sequences 279--282 add exact differential-pair result acceptance. The straight
positive control both-solves with two 26.000000 mm, zero-via routes and zero
skew. The asymmetric obstacle control is more informative: both tools make
electrically complete, native-clean boards, but Freerouting leaves 1.547585 mm
skew and pcb-maker leaves 1.839150 mm against a 0.500000 mm limit. Both are now
reported unsolved under the same top-level acceptance semantics. This proves
the measurement/admission seam and exposes the absence of coupled pair routing
or post-route length tuning. Pair spacing and uncoupled-length constraints are
still unsupported. Only the straight positive counts.
M0 now has eighteen admitted mechanisms, five pinned real declarations, and four
executable real sources.

Sequences 283--292 correct two adapter assumptions that invalidated the old
mechanism aggregate. Preserved component-body keepouts need explicit ports for
their own pads, and semantic angles must be negated when serialized as KiCad
footprint angles. The user's ESP render observation caught the latter: pads
that should face the interior were mirrored toward the board edge. The actual
ESP fixture now has a coordinate regression against its semantic seed endpoint.
Ports retain the body, use the connected net's width/clearance envelope, and
open only the pad's copper layer. The corrected sequence-291 aggregate finishes
all 17 reports: 13 both-solved, one pcb-maker-only decoupling case, two
Freerouting-only pcb-maker timeouts at exact rung 3, and one neither-solved
fixed-placement wall case. Sequence 292 preserves the decoupling win after
layer-specific tightening. A final full replay of that tightened representation
is still outstanding, so the historical all-solved claim remains retracted.

Sequences 293--298 convert that fixed-wall negative into a separate positive
router-to-placer case. The initial route fails; passage-capacity feedback moves
`WALL` 0.8 mm within its vertical region and the semantic route completes in
one repair. That route is discarded before a frozen zero-copper KiCad source
is given to both competitors. Placement acceptance observes one translation
and the 0.0001 mm digest gate proves equal poses. The first fair comparison
exposed that pcb-maker's half-cell-diagonal raster inflation erased the legal
1.8 mm passage before A*. Grid nodes now use physical clearance and every
planar edge receives an exact continuous obstacle check. Sequence 298 then
both-solves with zero findings: pcb-maker uses 37.431241 mm/no vias and
Freerouting 38.566705 mm/no vias. This proves one router-originated constrained
translation, not general placement/routing coupling; the 18-case aggregate is
pending.

## What works

| Capability | Current evidence |
| --- | --- |
| Semantic input and candidate boundary | Layers, pads, keepouts, constrained/free component poses, vias, two-terminal branches, and explicit multi-terminal route graphs round-trip through Rust. |
| Initial placement | Declared, grid, deterministic random, barycentric, and harmonic policies share exact projection and free rotation. Inter-component exclusion now uses a conservative body-plus-pad envelope, after the dual board proved that nominal-body legality could still create native pad shorts. A bounded cold portfolio routes every placement independently from zero copper and ranks native completion before board copper quality. Its first dual prefix-5 comparison selects harmonic at 153.261470 mm/four vias over declared at 192.974062 mm/four vias—a 20.58% copper improvement, despite 2.68x routing expansions. Under-anchored harmonic blocks can now retain declared poses or receive grid/bounded-random seeds. One corrected random control native-completes the same rung on attempt 11 with 5.04% fewer route expansions, but 0.995 mm more copper and two extra vias. At prefix 7 that same random policy is 3.24% shorter, but four extra vias keep its configured whole-board score behind the historical fallback. A diverse router-cost shared-tree entry subsequently removes one via from both cold boards without adding copper; the placement winner remains historical by 1.372 mm. Schema-v4 conditional activation reproduces both promoted PCBs byte-for-byte while saving 5.46%/8.01% route expansions. |
| DUT raster routing | Hard reservation, negotiated congestion, failure-directed order, and selective rip-up are selectable and retain bounded-search evidence. |
| Corridor/family routing | Exact corridors, cut bases, visibility repair, alternative family generation, copper-pair classification, joint assignment, fixed context, and context-correction diagnosis are ported. A fully exhausted frontier with fewer than K families is now admitted only when every returned raw class certified. V5 supports ordered runs and explicit via geometry. The native bounded producer exact-passes clear, blocked-site, and interacting two-net controls. The two-net assignment selects distinct via sites from 40 analyzed family pairs; its one-site control proves bounded infeasibility. The independent file importer exercises the same materialization boundary. |
| Small exact boards | The active grid ladder has nine exact-gated rungs: 1, 2, 2, 2, 2, 4, 6, 8, and 10 branches. It progresses from an ESP32 pad escape through rotatable-resistor cases, an explicit tree, a forced two-layer crossing, power/geometry slices, a header breakout, and a resistor-link bundle. Every rung has a deterministic work ceiling; dual-ESP32 is excluded. |
| Dual-ESP32 restart | The 43-component semantic fixture generates a self-contained 52-electrical-connection KiCad ladder without splitting shared VCC/GND identity. Every cold prefix prunes later connectivity before regenerating placement and starting from zero copper. Harmonic prefixes 16 and 17 are native-complete. The original prefix-18 order reproducibly stops at 14/18: `SIG05_R_HEADER` is unreachable after only 11–19 reachability cells, yielding `SIG03_R_HEADER` opens it, and then the old connection cannot be restored. Two small local mutations and a visibly different randomized-under-anchored placement moving 38 components all hit that same pair. Swapping only the order of `SIG03_R_HEADER` and `SIG05_R_HEADER` makes the identical prefix-18 connectivity and harmonic placement complete 18/18 with zero ERC, DRC, parity, or connectivity findings. The generic cold order coordinator now rediscovers that swap. Prefix 18 remains the full-width native capability boundary; prefixes 19–52 and segment simplification remain open. |
| Thin-route width continuation | A generic cold order coordinator automatically rediscovers the prefix-18 order swap. Prefix 19 still stops at rung 14 with full-width native insertion, but a priority-order 5%-width zero-copper run finds a complete 24/24 semantic topology. Opt-in per-point/body trust regions, exact rectangular/circular fixed-pad lowering, and a trace-first/component-fallback processor exact-grow that fixed topology through 25% declared width. At 27.5%, trace-only motion improves 99 segment findings to 90 across four route pairs; component fallback rolls back. A bounded conflict-cover outer loop reaches 23/24 and proves its leading failure is `no_path` on both grids. A cold exact-rule Freerouting reference completes the identical fixed placement at 24/24 in seven passes/5.90 seconds, proving this is primarily a multi-pass routing capability gap. Thin/intermediate stages are never promoted; our forced full-width validation still has 502 findings and therefore no native KiCad admission is claimed. |
| Route-graph topology | A bounded adjacent-junction transaction reduces the three-terminal control from 49.426407 mm to 49.219300 mm. The selectable shared-copper policy dynamically selects among all pending terminals/current tree vertices, trims already-owned prefixes, recognizes non-parallel same-layer contacts inside either segment, and emits explicit split/junction graphs. Optional bounded terminal preference removes an interior junction and improves 12.414214 mm to 12.242641 mm for 16 extra expansions. A general two-source portfolio selects a shorter interior attachment (19.242641 to 17.828427 mm) for 132 extra expansions; negative controls keep it opt-in. All controls pass the exact gates. |
| Differential-pair acceptance | Final named routes are imported and measured independently for physical length and via count. The symmetric control passes at 0.000000 mm skew/zero via difference. The asymmetric obstacle leaves 1.547585 mm Freerouting skew and 1.839150 mm pcb-maker skew; both native-clean boards correctly fail the 0.500000 mm result contract. Coupled spacing, maximum uncoupled length, pair-aware routing, and length tuning remain missing. |
| Component-body pad ports | Native semantic conversion retains component-body rule areas but decomposes them around per-net, per-layer access ports. Pinless bodies stay closed. Semantic rotations are converted to KiCad's opposite footprint-angle sign and are checked against real ESP seed endpoints. Corrected decoupling is pcb-maker-only; the fixed-wall placement case remains intentionally unreachable without moving a component. |
| Frozen router-to-placer passage | A failed semantic route identifies a movable wall, passage-capacity pressure translates it within its declared region, and all semantic copper is discarded. Both routers then consume the identical zero-copper placement. Exact continuous grid-edge collision checks preserve the resulting narrow legal channel; both outputs native-complete with zero findings. |
| Discrete conflict actions | A coordinator-level, replaceable action producer compiles declared ratsnest seeds and retains A-north/south, B-west/east, and via-on-A/B states. All six one-conflict states exact-pass; the via penalty changes the selected topology. A bounded best-first layer resolves two independent conflicts with 13 expanded states, 84 transitions, 49 unique states, 36 order duplicates, and 36 exact solutions. A shared-trace control retains unsupported action orders and still finds 12 exact states. A replaceable post-action processor exact-gates every local optimization before fingerprinting: vertex pulling reduces the independent selected route from 81.518 to 79.921 mm and the high-via shared-trace result from 139.533 to 137.534 mm. Candidate fingerprints, parents, histories, optimization evidence, errors, scores, transitions, full candidates, and exact assessments are serialized and playable. The heuristic is not proven admissible, so this is not a global-optimality claim. |
| Progressive connection insertion | The native KiCad transaction snapshots and verifies an immutable parent, hydrates exactly the next connection, and admits only complete ERC/DRC/parity/connectivity children. A fresh three-resolution chain commits rungs 1–21 locally at 477.767901 mm/14 vias versus 553.350268 mm/22 vias historically; score-ordered and fail-closed adaptive modes retain their measured work/quality tradeoffs. `T_PROBE_XDATA_1` commits rung 22. Shared-tree `XDATA` commits the first four-terminal child at rung 23 with 22.404598 mm/two well-separated vias versus the rooted star's 25.487881 mm/four vias/one close pair. Nearest-only attachment reproduces the wider portfolio boards with 44,744 rather than 65,812 expansions; the 1 mm diversity ablation is negative at 192,451. Isolated native-complete steps then reach rung 29. `D_BUS_ONEWIRE` is 11.44% shorter than historical; `DHT`, `ONEWIRE`, and `D_BUS_US_TRIG` have small gaps. Ordinary `US_TRIG` refinement falsely converges on a 65% detour, while the historical route passes on the exact generated parent and 0.05 mm A* finds a 6.648961 mm route, 14.84% shorter than historical. Every control and child has paired front/back evidence. Historical targets remain controls only. |
| Oversized-board continuation | Replaceable producers scale a board about its center, exact-gate each transaction, and retain the last valid candidate. The clear control passes scales 1.5 through 1.0; the bottleneck passes through 1.2 and visibly rejects 1.1. A retained-copper producer affinely maps poses, junctions, traces, and vias, snaps terminals to body-local pins, compacts optional sampling debt, and selectively reroutes only invalid branches. A replaceable motion seam first compiles exact trace/trace and trace/rectangular-body findings into bounded continuous corrections. On the yielding-body control, scales 1.5 through 1.2 need no correction; scales 1.1 and 1.0 move the implicated body and exact-pass with only the 2 initial branch searches/194 A* expansions, versus 4/418 for retained grid repair and 12/1,174 for full reroute. A fixed-wall control bends only the implicated trace through five stages (2 searches/10,066 expansions versus 7/46,679 for grid repair). Fixed rectangular explicit keepouts lower as virtual obstacles without becoming component bodies: the matched control needs 1 initial search/9,969 expansions and no reroutes versus 6/46,716 for discrete fallback. Clearance-only motion was 20.684615 mm, 4.85% longer than grid A*. Reusing exact vertex pull spent 485 validation calls for only 0.011052 mm final improvement. A new flat trace-tension edge objective followed by hard projection slides bends along the active boundary: 8 steps/stage produce 19.406395 mm with 6,400 projections; 16 produce 18.872391 mm with 12,800, versus the 19.727922 mm grid route. Circular shapes remain visibly unsupported and use exact selective fallback. Force, tension-edge, projection, and exact-postprocess work are retained separately, so this is not a runtime claim. |
| Router-to-placer feedback | Reduced symmetric and asymmetric movable-blocker fixtures are solved transactionally by axis sampling, blocked-frontier pressure, and passage-capacity pressure. Pressure source, ordering, direction, relational/pad-aware collision propagation, beam width, neutral retention, and selective rip-up are independent. A four-component mixed action survives cold prefix 14. Prefix 15 supplies the first negative downstream-coupling case: semantic completeness selects legal component moves whose independently rerouted native boards stop four rungs early. The bridge now retains the failed native trial and selects an independently regenerated base placement only after full native progression; it never continues from partial feedback copper. This preserves completion but costs 272.910 seconds in the measured fallback run. On real KiCad rung 1, projected tension transfers through body-local attachments; generic courtyard projection moves `R1`, shortens its route, and native-passes. A reference-field action recovers more rotation but is longer, so its native portfolio selects the better sideways action. |
| Continuous representation controls | The CPU push/pull reference supports endpoint-distance and analytic-body/body-local-attachment representations for two-terminal parts. Both pass a free-rotation control; matched ablation evidence records pose, residuals, record/scalar-row work, and movement without selecting a premature default. |
| Continuous candidate bridge | A native two-branch recovery control lowers layout-trace input into either passive representation, moves the resistor by 0.999955 mm, materializes component poses/traces/route graphs, and exact-passes. A second control uses an analytic rectangular body with three arbitrary local pins and lowers a five-terminal net into four branches/one exact-valid graph. Normal lowering subdivides long trace segments; preserving an under-sampled seed is explicit. Invalid engine poses remain invalid after materialization. |
| Family/continuous handoff | Selected family-pair and selected-family/fixed-context copper obligations resolve to semantic polylines, component bodies, and body-local endpoint attachments, then compile to flat segment/segment and oriented segment/body constraint families under one 100,000-row ceiling. The first natural control moves two endpoint bodies by 0.050940 mm and exact-passes; its all-fixed control rolls back. A second natural control uses two trace/body rows to prevent the repair from substituting a keepout violation; disabling those rows exact-rejects. |
| Inspection | Continuous HTML shows rotated analytic bodies, attachment residuals, trace/body and trace/trace constraints, fields, particles, corrections, and working playback. Route HTML separately shows bodies, pads, layer-colored traces, vias, failed intents, exact violations, blocked-frontier centroids, proposed motion, rejected projected poses, multi-body push arrows, attempt playback, and per-stage continuation bounds. Every current board experiment also gets deterministic front/back SVG and PNG snapshots in sequential `build/progress/` entries; missing routes appear as red dashed ratsnest lines. Native KiCad journal renders use a disposable outline-only silkscreen overlay to expose rule-area constraints without changing the verified board. |
| KiCad progressive board pipeline | The Rust declaration covers all 42 multi-pad nets of one ESP32-C3 board, and every prefix from rung 0 through rung 42 passes the full gate. It preserves all 36 footprints, the outline, rules, and graphics; asks KiCad to export the schematic netlist as PCB connectivity authority; and emits self-contained schematic, PCB, project, libraries, raw ERC/DRC reports, and a verification report. A KiCad-to-grid adapter rasterizes rotated pads, tracks, vias, exact rectangular or closed-line polygon outlines, and target holes. Rooted-star routing remains the deterministic multi-terminal control; selectable shared-tree growth attaches later branches to owned copper, retains a bounded attachment portfolio, and records searched versus actual trimmed join points. Both policies reserve drill spacing across branches. A fail-closed board-copper importer canonicalizes connected, acyclic, uniform segment/via graphs into deterministic root-to-terminal branches. Rung 7 round-trips four tracks/two vias, then fixed-via continuous lowering uses three same-layer polylines and two shared anchors to improve 19.243 mm to 18.964 mm; the result retains both vias and native-exact-passes. Per-run uniform-grid selection is byte-identical to exhaustive with 1,792 rather than 91,648 projection rows and 7 rather than 179 engine particles. On rung 41 the importer converts the real 13-terminal `3V3_DUT` tree to 12 branches, 44 canonical segments, and 12 vias, then native-exact-passes. A zero-force selected-subtree control found that f32 rewriting could detach shared prefixes from ten unselected branches and add 1.380 mm of duplicate copper even while KiCad passed. Eleven exact context anchors plus exact no-op writeback now make the same two-shared-via lowering byte-identical after 1,120 projections. Quality also separates canonical physical copper from stored overlapping track objects: the original `3V3_DUT` is 238.146 mm physical versus 294.382 mm stored. This correction rejects the previously promoted retained-vertex and exact-shared-point proposals. Projected tension reaches 237.552 mm on branch 0. A route-graph policy inserts seven vertices at nine selected/context contacts, fixes three context nodes, deduplicates 16 physical tension edges, and reaches 228.465 mm across six branches. Its 3.2 mm local neighborhood is byte-identical to exhaustive with 169,728 rather than 2,812,800 projection rows. Rungs 7/41/42 exact-pass their respective experiments, rejected proposals remain inspectable, and native front/back rendering is available through the Rust CLI. |

Placement portfolios have a schema-v3 cheap archive / feedback evolution /
expensive-finalist boundary. Rejected and duplicate proposals remain visible,
archive ranking is explicitly advisory, and every retained finalist starts from zero
copper before native routing. Whole-board random reseeding retains only one of
16 legal proposals even after legalization grows from 64 to 256 sweeps, so
that negative result is not treated as an A* or placement-budget shortage.
Independent local mutations of a legal harmonic base instead produce 16/16
legal unique proposals. The first two-finalist prefix-5 run native-passes both
boards and selects 153.103349 mm/four vias, 0.158121 mm shorter than the prior
harmonic control.

Schema-v3 placement portfolios now compose those archived parents with
bounded routing-failure feedback. Each pressure child and its unchanged parent
is independently rebuilt from zero copper; a no-op feedback result is retained
without duplicating the native trial. In the first prefix-14 run all four
boards exact-pass. The two feedback lineages improve native copper by 0.034160
mm and 8.594701 mm respectively, but the larger change adds two vias. The
selected board is proposal 2's child at 450.096586 mm/89 segments/22 vias.
This ports archived failure-directed parent composition, not a claim that
semantic routing rank predicts the native winner.

The declaration now also owns the native GND-zone lifecycle and verification
automatically refills zones. Its selected tested point removes all 412 GND
tracks and 96 of 128 GND vias; the whole rung falls from 815 segments/196 vias
to 403 segments/100 vias plus two zones and remains native-complete.

The first nonzero shared-via-tree experiment jointly moves two branches beyond
two fixed shared vias and improves canonical `3V3_DUT` copper from 236.746 mm
to 232.763 mm. One-branch motion reaches only 235.943 mm because the common
junction becomes fixed context. Independent particles split shared trunks and
grow copper to 292.572 mm, so the physical-union transaction rolls back even
though KiCad accepts the raw proposal. Local/exhaustive graph-aware candidates
are byte-identical with 115,200 versus 1,298,432 projection rows.
One bounded nearest-shared-junction hop now grows seed branch 1 to `[1, 2]`
after 830 contact tests and reproduces the manual candidate, proposal, and
complete board byte-for-byte. A one-branch ceiling stops before engine work.
A native-gated port of `layout-trace`'s via lifecycle now atomically removes
or relocates one physical via across its repeated branch representation. The
first 14-action branch-8 portfolio finds two legal relocation sites and
selects a 1 mm move worth 0.269941 mm. Both direct removal directions fail
with 19/28 DRC findings. Relocation plus fixed-via tension reaches 236.000102
mm versus 236.270043 mm for tension alone; the continuous gain itself is
identical, so this is an additive topology improvement rather than evidence
of a different relaxation basin.
A bounded analytic-feasible radial frontier now replaces blind lattice sites
for this action family. On the same via it uses 646 cheap analytic probes to
retain four spatially distinct relocations; all four pass native KiCad, while
the full portfolio needs six rather than fourteen native board evaluations.
Its selected raw/relaxed lengths are 236.426303/235.950438 mm. An initial run
collapsed all four slots onto one numerical boundary and is retained as the
reason for score-ordered 0.1 mm non-maximum suppression. This frontier is
sampled and scale-parameterized, not a proof of continuous feasibility or a
general speed result.

Remove-then-repair now composes that topology boundary with the bounded DUT
single-layer A* kernel. The branch-8 long excursion has no zero-new-via path
under 4 mm or 12 mm windows at three resolutions, while a shorter private
branch-9 excursion removes two vias and native-passes at all three front-layer
resolutions. The selected 0.25 mm result reaches 232.730760 mm and 10 vias from
236.745909 mm and 12 vias. The 0.125 mm action finds the shortest local path
but a worse physical tree, confirming that whole-tree copper—not local path
cost—must select topology actions. Failed searches now retain typed no-path or
budget outcomes, exact losslessly row-run-encoded occupancy/frontier grids,
work counters, a topology-only candidate, and paired front/back overlays. The
real branch-8 render shows where both layer choices close. Portfolio schema v5
now preserves obstacle ownership and attributes every frontier cell to grouped
footprint pads or foreign routed nets, including declared component mobility
and physical centroids. Across all three resolutions, the front is led by
fixed `J4`, `D_EN`, and `D_LOCAL_COMM0`, with movable `C4`/`R19` also present;
the back is led by `T_EN` and `GND`. Hit counts are pressure evidence rather
than a causal minimum cut, and no mixed action is generated yet.

## What does not work yet

- Native generated-parent progression is proven through rung 32 with no
  required rip-up. One live-prefix adaptive schedule reduces routing work by
  23.85%, but costs 1.74% copper and two vias; it is experimental, not a
  promoted default or an optimum. It does not yet use corridor/board-state
  evidence to schedule resolution, search both routing orders, yield two old
  connections, move components, or compose continuous shortening into
  insertion. Score-ordered admission reduces candidate gates from 63 to 21,
  but total wall time also includes repeated source/target gates, KiCad
  startup, and materialization; no proportional runtime claim is made. The
  one-old-net fallback is verified on the coarse-grid control, but no genuine
  fine-grid topology failure has exercised it. Counterfactual failure
  expansions are aggregated, while failure kind remains encoded as text. The
  first shared-tree case favors one nearest attachment search, but terminal
  order, alternative first trunks, and when to expand that portfolio remain
  unresolved; the generated parent uses two vias where the checked-in parent
  routes the same net without one.

- The completed KiCad board is still not a quality-routing result. Native GND
  zones eliminate the 412-track GND tree and reduce its vias from 128 to 32,
  but those vias are an order-sensitive thinning of the old router's seed set,
  not a purpose-built stitching solution or a minimum. Plane impedance,
  island area, thermal behavior, and yielding-zone interaction with later
  local repairs are not optimized. The reconstructed footprint library still
  causes 11 metadata warnings, retained separately from design findings.

- The KiCad raster adapter currently requires a rectangular `Edge.Cuts`
  outline and rejects non-target arcs. Foreign zones are deliberately yielding:
  their polygons are omitted from the hard-obstacle raster while their pads
  and retained stitching vias remain obstacles, and KiCad refill gates every
  committed board. It does not score plane-area or island damage. It is
  CPU-only. Its conservative cell
  inflation found valid routes for missing bus legs, multi-terminal power
  nets, and repairs against later copper. Both retained-vertex shorteners keep
  every via fixed. The continuous KiCad handoff now optionally splits a
  via-bearing branch into same-layer runs joined by fixed shared anchors. The
  continuous engine still does not move, insert, remove, or optimize vias. A
  separate discrete portfolio can now remove/merge or relocate an explicitly
  identified physical via and native-gate every candidate. Removal can invoke
  bounded single-layer local A* at multiple resolutions, but it has no
  insertion action, cannot use a compensating layer change, and does not yet
  retain a failed-frontier overlay. The board-copper importer
  accepts connected uniform trees, but rejects cycles, dangling material,
  arcs/zones, and cross-layer pad-mediated contacts. The first real
  terminal-component control can move one explicitly
  marked footprint and persist its pose. Its static uniform-grid neighborhood is
  exact-equivalent on one branch and six simultaneously moving branches.
  Collinear overlap, T contacts, crossings, mixed fixed/interior junctions,
  fixed shared vias, and contacts with unselected same-net copper are
  normalized for the selected graph. Two-branch nonzero shared-via motion now
  works. Automatic nearest-shared-junction selection now finds the same local
  two-branch bundle from one seed; choosing seeds, simultaneous motion across
  the wider tree, repeated neighborhood rebuilds,
  and general multi-footprint motion remain. Nearby courtyard preservation now
  supplies generic body/body rows, but it is conservative, does not model
  silkscreen/reference envelopes, and has only been exercised with one movable
  footprint. The retained-vertex shorteners optimize branch
  paths rather than physical graph length and now roll back on this board. The
  rooted star remains a baseline, not a quality shared-tree claim.

- The active small-board ladder now reaches a realistic eight-branch header
  breakout and ten-branch resistor bundle, but it does not yet cover a
  realistic multi-terminal shared bus or repeated placement/routing feedback.
  The discovered via-bearing controls are deliberately single-branch mechanism
  tests, not footprint-scale density. Current four-terminal and obstructed shared-tree
  cases still isolate algorithms rather than footprint-scale density.
- The in-process family producer can generate one seed-derived via transition
  and two independently certified runs when endpoint pad layers differ. It
  samples a configured number of sites along the seed, searches a configured
  number of per-run classes, and admits families in spatial round-robin order.
  It does not yet search a general two-dimensional via lattice, multiple vias,
  more than two layer runs, or layer changes chosen independently of endpoint
  requirements. The first interacting control is deliberately parallel and
  obstacle-free; it is not evidence for dense multilayer negotiation.
- Continuous materialization supports legacy and terminal-MST multi-terminal
  graphs plus arbitrary analytic rectangular body-local pins. It still lacks
  vias, multi-layer runs, and relational placement constraints. Family repair
  now couples same-layer via-free endpoints to component bodies and generates
  oriented contacts against semantic rectangular body keepouts. The flat
  engine now also has generic oriented body/body clearance rows, including
  explicit clearance and preserve-initial modes; family repair does not yet
  lower those rows or contacts against explicit pad/keepout shapes around the
  local correction neighborhood.
- Large-board visibility work is expensive. The default 25M geometry-unit
  ceiling stops three tested two-branch repairs before assignment. A 100M-unit
  ablation improves one component, proves another bounded infeasible, and still
  exhausts on the third. More search helps, but is not a general fix.
- The route viewer does not yet show corridor cells/gates, alternative cut
  words, assignment conflicts, or synchronized continuous relaxation frames;
  continuous frames currently use a separate viewer.
- There is no GPU compute backend yet. The hot-state architecture anticipates
  one, but current routing and placement are CPU reference implementations.
  On the current host both NVIDIA and Vulkan compute are unavailable because
  the driver stack is not usable. Prefix-14 timing also shows that a GPU port
  of A* would not currently dominate iteration speed: the first mixed
  placement/topology promotion spends 1.552 seconds in semantic feedback
  versus 98.069 seconds in native progression and 100.190 seconds total.
  Earlier phase timing likewise identifies repeated KiCad process admission as
  dominant. GPU work should target batched placement demand/force fields and
  counterfactual reachability once a driver is available, while native
  orchestration remains hash-cached and fail-closed.

- Recursive conflict-action search currently inherits the narrow producer:
  only straight orthogonal same-layer conflicts can be expanded. Exact local
  vertex pulling now shortens action states, but it is CPU-only, does not move
  components or repair topology, and is not yet the intended particle/GPU
  shortening pass. The configurable remaining-violation heuristic has no
  admissibility proof.

- Retained continuation now composes a thin zero-copper grid topology with
  staged declared-width growth. Mixed-layer traces lower as same-layer runs;
  existing vias remain fixed shared anchors; fixed rectangular and circular
  pads lower exactly; and selected motion closes over all incident branches.
  An opt-in semantic trust region caps every trace point/body independently,
  exact-gates bounded backtracking trials, and preserves the unbounded control.
  A two-phase processor tries trace motion before connected component motion.
  Prefix 19 exact-passes through 25% and rolls back both processors at 27.5%.
  Movable compound pad/keepout children, relational placement constraints,
  movable vias, local contact neighborhoods, and outer discrete repair remain.
  Every stage and rejected proposal has paired front/back route images.

## Where the engines get stuck

The failure modes are now distinguishable rather than collapsed into “no
route”:

1. The real KiCad ladder exact-passes all 43 phases. Earlier prefix-only routes
   conflicted with copper activated in later rungs; rerouting against the next
   prefix demonstrated why future-aware repair and retained alternatives
   matter. Historical 3V3 and GND copper produced 26 and 277 DRC findings;
   persisted rooted-star replacements reduce both to zero, at the cost of very
   poor segment/via counts. The declared GND-zone pass now removes 412 of those
   segments and 96 vias while retaining native completeness. The immediate
   problem is now quality and zone-aware repair, not pipeline completion. A
   prefix-only `3V3_DUT` shortening passed rung 41 but produced
   two GND shorts at rung 42; treating rung 42 copper as obstacles retained
   most of the shortening and passed both rungs. This is direct evidence for
   future-aware alternatives or transactional later-net repair.
2. A one-branch wall fixture deliberately defeats routing alone. The current
   transactional coordinators solve it by moving the implicated component.
   The pressure policy needs two attempts and 868,461 total expansions versus
   one attempt and 436,241 for axis sampling: its frontier pressure is
   horizontal while the wall may only move vertically. Passage-capacity
   pressure fixes that case in one attempt, and both pressure producers beat
   axis sampling on the asymmetric control.
3. A second reduced wall control requires a physical neighbor to yield. Axis
   sampling exhausts eight moves and single-body passage pressure is rejected;
   a bounded two-body collision chain exact-passes in one reroute. This only
   establishes same-vector propagation for one contact, not general packing.
4. Search-time shared-tree growth dynamically chooses pending terminals and
   legal existing vertices, trims through the last same-layer tree contact,
   and emits explicit split branches/junction incidence. Non-parallel contacts
   inside the new and existing segments are materialized exactly. Bounded
   terminal alternatives are selectable and retain their work/selection
   evidence. A general bounded, spaced source portfolio is also selectable and
   retains every trial. Collinear interior contacts, contacts at vias,
   whole-tree lookahead, and selective-ripup ownership of synthetic branches
   remain.
5. Some routed candidates retain trace/trace or via/trace conflicts which
   negotiated congestion alone cannot remove.
6. The corridor family generator can spend its visibility geometry budget
   before producing a certifiable frontier on the full obstacle field.
   The native via producer obeys the same work-budget contract, but its current
   seed-site frontier can miss a legal via elsewhere on the board.
7. A successfully assigned family pair can still have an explicit
   copper-clearance repair obligation. The bounded continuous pass now handles
   the supported same-layer case, can push endpoint bodies, and prevents one
   reduced repair from substituting a rectangular-body violation. If the
   required bodies and points are fixed, the matched control correctly rolls
   back rather than silently detaching endpoints.
8. Exact context correction shows that at least one tested bounded-infeasible
   pair cannot be fixed by promoting up to three outside routes. That failure
   needs a deeper/different family or continuous correction, not guessed
   rip-up.
9. Dual prefix-14 failed-front pressure improves the semantic route from 16/19
   to 18/19 branches, but placement-only greedy and beam searches cannot route
   `SIG04_W_R`. A mixed pressure/rip-up action reaches 19/19 only after moving
   a four-resistor pad-envelope contact chain. Its first apparent success was
   a semantic/native mismatch; the corrected transaction survives cold native
   promotion. It uses two fewer vias but is 3.767767 mm worse than the
   single-move board under the configured objective, separating capability
   from quality.
10. Dual prefix 15 is complete without feedback, but every tested
    semantic-preferred placement stops native progression at rung 11. The
    smallest control moves only inactive `R_SOUTH_TAP5` by 0.5 mm; rungs 1–8
    remain unchanged, while rung 9 takes a 1 mm shorter repair that blocks rung
    12. Neither the alternate generated rung-9 topology, any single/two-net
    yield, nor 0.125 mm routing opens it. The transactional base fallback
    restores a finished board but deliberately does not claim placement gain.
11. In the original prefix-18 order, three placement basins all stop at rung
    14 on the same `SIG03_R_HEADER`/`SIG05_R_HEADER` conflict. A direct route
    has no reachable raster path, not an exhausted A* budget. Swapping only
    those two insertion positions completes all 18 connections. The engine
    can therefore solve this board slice, but the fixed greedy global order
    cannot discover the needed decision itself.
12. The generic order coordinator rediscovers that prefix-18 swap, but
    full-width prefix 19 again stops at rung 14 across three order/placement
    trials. Thin-first priority routing instead finds all 24 branches. Width
    continuation now exact-passes through 25% after adding per-point trust
    regions and exact fixed-pad geometry. At 27.5%, trace-only motion improves
    the state but stalls on four route pairs; connected-component fallback is
    worse and rolls back. This is now residual pressure for a discrete outer
    action, not a topology impossibility or A* budget ceiling. Sequences
    189–192 add conflict-cover and blocker-frontier rerouting: the best cover
    reaches 23/24, and detailed evidence proves `SIG02_W_R` is `no_path` at
    both 0.25 and 0.125 mm. Sequence 193 then gives the decisive control:
    Freerouting routes the identical fixed-placement board at 0.25 mm trace
    width and 0.70/0.30 mm vias, completing 24/24 in seven passes and 5.90
    seconds with zero KiCad copper/connectivity findings.

## Historical priorities (superseded by the active priorities above)

1. Close the conventional routing gap demonstrated by sequence 193. Make
   terminal and internal vias safe across selective edits, then implement
   repeated whole-board rip-up/retry passes driven by failed-route blocker
   evidence. Preserve each pass and compare completion before copper/vias;
   the immediate target is the external reference's exact 24/24 prefix-19
   board, not another small score change.
2. Feed the resulting multi-pass router back into width continuation and
   placement. Only invoke topology/rip-up actions after bounded trace and
   component squeezing fails, and reroute every cold benchmark from zero
   copper rather than retaining Freerouting's decisions.
3. Attach exact compound pad/keepout geometry rigidly to movable bodies and
   lower relational placement constraints. Existing vias remain fixed anchors;
   keep insertion/removal/relocation as typed discrete actions until a genuine
   movable-via representation is tested. Re-run both trace-only and coupled
   width continuation after these constraints are exact.
4. Localize continuous constraint compilation around exact closest-point
   witnesses. Sequence 188's 90 segment findings collapse to four route pairs;
   avoid moving and comparing every point on their full-board polylines while
   preserving fixed context anchors and exact topology.
5. Turn the now-causal blocker evidence into a replaceable mixed-action
   producer. `B.Cu` has a size-one GND cut, so first test a small
   zone/plane-aware or rip-up-and-restore action rather than treating GND tree
   copper as permanent obstacles. `F.Cu` exhausts all 50 generated size-one/two
   cuts, so combine bounded foreign-net rerouting with declared component
   motion rather than increasing cut size blindly. Retry the target repair
   transactionally and keep the source as fallback. Exercise both it and the
   analytic-feasible relocation frontier on
   additional vias before tuning sampling, resolution, or diversity scale.
   Keep via insertion as a separate action family and preserve the
   lattice/frontier and branch-8/9 controls as regressions. Add transactional local-neighborhood
   rebuild/retry when the declared motion envelope is exceeded, then
   characterize convergence and a two-trace contact control. The GND rooted
   star remains a distant stress regression, not the tuning target.
5. Generalize the working reference-action portfolio into a typed mixed-action
   coordinator that can compare label moves, component moves, copper detours,
   and vias while retaining their different costs. Add text-readability
   scoring, then compare discrete documentation repair with richer silkscreen
   envelopes on useful free rotation and more than one movable footprint.
   Reuse the continuous processor after discrete actions while keeping
   topology search separate from geometric relaxation.
6. Extend fixed rectangles to circles, movable compound keepouts, explicit
   pads, less restrictive rotation-aware placement clearance, and junction endpoints. Keep the
   unsupported cases visible and preserve selective fallback before trying an
   ESP32 prefix.
7. Extend passage pressure beyond axis-aligned component-body gaps to rotated
   keepouts/pads; extend collision propagation beyond its two-body same-vector
   control with fixed-contact, budget, longer-chain, rotation, and group tests.
8. Generalize the now-working bounded single-via producer: derive adaptive
   two-dimensional sites from corridor geometry, add obstacle-determined
   interacting-net choices, and support multiple transitions before promoting
   it beyond reduced cases.
9. Extend the family/continuation adapters to lower the new body/body rows and
   explicit pad/keepout shapes, then test rotation-driven exact correction and
   adaptive trace sampling before adding layer/via and relational-constraint
   bindings.
10. Batch the measured placement-demand, failed-front, and counterfactual
   reachability kernels behind the same CPU/GPU work contract once a usable GPU
   driver is present. The first coupled cold run spends about 4.5 seconds in
   semantic feedback and 135.055 seconds in native progression, so a device
   router is not currently the shortest path to faster end-to-end iteration.
   Keep native KiCad admission outside the device loop and reduce/batch those
   process launches independently.
11. Finish certified context promotion for cases where the exact correction
   solver actually selects outside branches.
12. Add corridor/family/pressure overlays and iteration playback so failed
   generation and correction can be inspected rather than inferred from final
   counts.

The dual-ESP32 case is now an active cold-prefix native ladder, with prefix 18
native-complete under both the diagnosed order and the generic order-search
coordinator. Prefix 19 has a complete 24-branch thin semantic topology and an
exact 25%-width continuation stage, but our engine has no full-width or
native-complete board. The sequence-193 external control proves the identical
fixed placement is routable at full declared geometry: Freerouting reaches
24/24 in 5.90 seconds and the imported KiCad board has no copper or
connectivity findings.
Later semantic connections are pruned before placement and every cold run
starts routing from zero copper; smaller solved boards are never parents. The
first three-terminal
net is 10.6% shorter with shared-copper growth than with rooted-star growth,
and both alternatives native-pass. Prefix 5 is the first significant pressure
point, with four well-spaced vias and targeted rooted/shared and high-via-cost
controls. The third three-terminal net reverses the topology result and selects
rooted star, while equivalent two-terminal portfolio work is now removed with
byte-identical output. Prefix 9's two-via escape is 67.177670 mm shorter than
its native-complete zero-via control. Prefix 10 adds a clean back-layer trunk;
its fixed-parent high-via-cost control retains the exact same two-via geometry
at 68.89× the search work. Prefix 11 reveals a one-source attachment weakness:
a two-source layer portfolio removes an adjacent via without changing copper
length, and the independent cold rerun promotes that result. Prefix 12 then
adds a clean front-only route. Prefix 13's widely spaced two-via route narrowly
beats a native-complete zero-via control under the current scoring policy.
Prefix 14 is the first trace-level repair: yielding and rerouting
`SIG01_AFTER_LINK` makes room for the new net and native-passes the full board.
The historical 71/83
negotiated result remains provenance rather than a target: the next feedback
comes from the first independently regenerated prefix that fails, not from
rerunning all 83 geometric branches at once.

The active ladder measurements are in
[`experiments/small-board-ladder.md`](experiments/small-board-ladder.md). The
real-board supplemental route history is in
[`../benchmarks/esp32-c3-ladder/route-evidence.md`](../benchmarks/esp32-c3-ladder/route-evidence.md).
The
historical larger-board measurements remain in
[`experiments/dual-esp32-grid-baseline.md`](experiments/dual-esp32-grid-baseline.md)
and
[`experiments/layout-trace-routing-port.md`](experiments/layout-trace-routing-port.md).
The active restart evidence is in
[`experiments/dual-esp32-native-ladder.md`](experiments/dual-esp32-native-ladder.md).
The prefix-19 thin-first/continuous experiment and its retained failed proposal
are in
[`experiments/dual-esp32-width-continuation.md`](experiments/dual-esp32-width-continuation.md).
The exact external capability reference is in
[`experiments/dual-esp32-freerouting-baseline.md`](experiments/dual-esp32-freerouting-baseline.md).
