# Dual-ESP32 native progressive ladder

Date: 2026-09-02

## Question

Can the harder 43-component dual-ESP32 semantic fixture enter the same
declaration-to-native-KiCad pipeline as the completed ESP32-C3 board, without
prematurely treating the whole 83-geometric-branch board as one benchmark?

## Boundary

The new bridge converts a semantic `Problem` plus any placement policy's rigid
poses into a self-contained KiCad schematic, PCB, project, and ordered ladder
declaration. It preserves 52 electrical conductor identities. In particular,
VCC has 12 terminals and GND has 16; the 83 legacy point-to-point branches are
not misrepresented as 83 independent PCB nets. VCC and GND are last in the
initial order because they are large shared-tree/plane problems.

The bridge is intentionally generic over the placement producer and the
connection order. Its generated footprints retain semantic body sizes, pad
geometry, constraints, and routing keepouts. Circular keepouts currently use
a conservative square native rule area. The board is rectangular and receives
a configured 1 mm manufacturing margin around the semantic bounds.

Growing prefixes are independent problems, not warm-start checkpoints. For
prefix N, `solve-semantic-kicad-prefix` truncates semantic connectivity to the
first N electrical identities before placement, regenerates placement,
materializes a zero-copper rung 0, and routes all N identities within that run.
It cannot accept a routed parent. The run snapshots both the original and
active-prefix problems plus every config in `cold-prefix.json`.

## Native-gate findings

The conversion work found defects before any route-quality experiment:

| Trial | Result | Interpretation |
| --- | --- | --- |
| Initial declared conversion | ERC 39, DRC 110 | Schematic pins were off-grid, rectangular pad angles were not composed with footprint rotation, and the abstract boundary put copper on `Edge.Cuts`. These were bridge defects. |
| Declared after coordinate/pad fixes | DRC 12 | `R_E_EN` overlapped four east-ESP32 pads and `C_E_EN` missed 0.2 mm clearance by 0.0252 mm. The shared placement projector checked nominal bodies but not protruding pads. |
| Existing barycentric seed | DRC 117 | Lower connectivity distance (375.417 mm) did not imply a native-legal placement. This is retained as a negative placement result, not a verdict on barycentric placement. |
| Existing random seed | DRC 55 | Native-invalid and longer at 720.375 mm connectivity distance. |
| Existing harmonic seed | No board | Its bounded legalizer stopped on an overlap. |
| Existing grid seed | No board | Its one proposal stopped on an overlap. |
| Declared plus body-and-pad placement envelope | Native complete | Six components moved; connectivity distance changed from 503.390 to 497.193 mm. Rung 0 has ERC 0, DRC 0, parity 0, and no selected-net disconnects. |

The pad-envelope correction belongs to the common placement projector, not to
the dual fixture. It conservatively bounds component bodies and every local
pad for inter-component separation while leaving the imported abstract board
constraint semantics unchanged. Tiny passive references are put on `F.Fab`
because the poor seed provides no stable collision-free silkscreen location;
larger landmarks remain visible in renders.

Rung 1 then exposed a schematic transform bug: a wire intended for ESP32 pin 8
landed on pin 13 because library-symbol Y is Cartesian while sheet Y increases
downward. The two-pin passive controls had hidden that defect. A nonzero-Y
regression test now covers it.

## Route-policy ablation

The ordered first connection is `SIG01_W_R`, from the west ESP32 to the first
freely rotatable link resistor. The default bounded DUT A* committed on its
first attempt:

| Measure | Result |
| --- | ---: |
| A* expansions | 118 |
| Physical copper | 22.080764 mm |
| Segments | 5 |
| Vias | 0 |
| Native ERC / DRC / parity / disconnects | 0 / 0 / 0 / 0 |

This proved the whole declaration → placement → schematic/PCB generation →
one-connection transaction → native KiCad verification path. It did not yet
prove the later cold-prefix experiment contract.

Rung 2 adds `SIG01_AFTER_LINK`, the first three-terminal conductor. Two
otherwise-identical 0.25 mm-grid trials started from the exact rung-1 child.
The declared-target fallback was disabled, so only a generated route could
commit:

| Policy | Complete | Copper | A* expansions | Bends | Vias |
| --- | --- | ---: | ---: | ---: | ---: |
| Rooted star | yes | 64.389766 mm | 3,445 | 10 | 0 |
| Shared copper tree | yes | 57.547883 mm | 1,372 | 9 | 0 |

Both have ERC 0, DRC 0, parity 0, and no selected-net disconnects. The shared
tree joins the south branch to the existing east-west trunk rather than
routing it independently back toward the east ESP32. It is 6.841883 mm, or
10.6%, shorter and uses 60.2% fewer expansions. This is positive evidence for
shared-copper growth on a real generated board, not grounds for making it an
unconditional policy: terminal order and later congestion can reverse the
tradeoff. These sequences remain a controlled routing-policy ablation, not a
parent that a larger-prefix experiment may consume.

## Cold-prefix correction and promoted result

The first attempted rung-3 continuation (sequence 078) reused rung-2 copper.
That answers a warm-repair question, not whether the engine can solve a larger
mess, so it is retained only as a mislabeled-method control. Sequences 079 and
080 regenerated placement and zero copper, but still exposed all 52 nets to
the placer before routing only three. That distinction is harmless to the
current declared poses, but would contaminate connectivity-sensitive placement
policies; they are retained as full-connectivity-placement controls.

Sequence 081 is the first authoritative cold prefix. Its active semantic input
contains all 43 components but only four legacy branches grouped into these
three electrical identities:

1. `SIG01_W_R`
2. `SIG01_AFTER_LINK`
3. `SIG01_R_HEADER`

Placement and all copper were regenerated from that input. The initial native
board has zero selected connections, segments, and vias. The run then reaches
prefix 3 with no fallback:

| Connection | Copper | A* expansions | Bends | Vias |
| --- | ---: | ---: | ---: | ---: |
| `SIG01_W_R` | 22.080764 mm | 118 | 4 | 0 |
| `SIG01_AFTER_LINK` | 57.547883 mm | 1,372 | 9 | 0 |
| `SIG01_R_HEADER` | 22.706517 mm | 306 | 4 | 0 |
| **Total** | **102.335164 mm** | **1,796** | **17** | **0** |

The final board stores 21 track segments and has ERC 0, DRC 0, parity 0, and
no selected-net disconnects. Its poses and routes happen to match the earlier
full-connectivity-placement control. The PCB files differ only because the
control retained unused net declarations 4–52; this is serialization evidence,
not a geometry difference. The connectivity-distance metric changes from
503.390→497.193 mm for all 52 nets to 27.387→27.269 mm for the active prefix,
which is the correct scope for later placement comparisons.

Sequence 082 independently solves prefix 4, adding `SIG02_W_R` from west
ESP32 pin 9 to `R_LINK_02.1`. Its active input has five legacy branches grouped
into four electrical identities. The new route uses 27.716344 mm, 376 A*
expansions, six segments, five bends, and no via. The complete prefix has
130.051508 mm of physical copper, 27 stored segments, 2,172 total expansions,
and again has ERC 0, DRC 0, parity 0, and no selected-net disconnects.

All 43 component poses and the first three route-candidate files are
byte-identical between the independent prefix-3 and prefix-4 runs. Therefore
this added connection has not created placement or route interaction pressure
yet; the success is useful pipeline evidence, but not feedback about joint
repair behavior.

Sequence 083 independently solves prefix 5. It adds the three-terminal
`SIG02_AFTER_LINK` conductor from `R_LINK_02.2` to east ESP32 pin 26 and
`R_SOUTH_TAP2.1`. The complete prefix has seven legacy branches grouped into
five electrical identities:

| Measure | Prefix-5 result |
| --- | ---: |
| New-net physical copper | 64.756378 mm |
| New-net A* expansions | 25,908 |
| New-net branches / stored segments | 2 / 10 |
| New-net bends / vias | 4 / 4 |
| Complete-prefix physical copper | 194.807887 mm |
| Complete-prefix A* expansions | 28,080 |
| Complete-prefix stored segments / vias | 37 / 4 |
| Native ERC / DRC / parity / disconnects | 0 / 0 / 0 / 0 |

This is the first cold prefix with substantial interaction pressure. The new
net consumes 92.3% of the complete run's A* expansions. Its selected tree uses
two separate back-layer excursions: one toward the south tap and one toward
the east ESP32. The four vias have 1.25 mm minimum center spacing, no close
pairs, and no clusters.

Sequence 084 reruns the same independent prefix with rooted-star and
shared-copper policies in one native-gated portfolio:

| Policy | Copper | A* expansions | Vias | Minimum via spacing | Close / clustered vias |
| --- | ---: | ---: | ---: | ---: | ---: |
| Rooted star | 64.963485 mm | 28,544 | 4 | 0.901388 mm | 1 / 2 |
| Shared copper | 64.756378 mm | 25,908 | 4 | 1.25 mm | 0 / 0 |

Shared copper is 0.207107 mm (0.32%) shorter, uses 2,636 (9.23%) fewer
expansions, and avoids the rooted control's close two-via group. Both policies
are native-complete, so this is a quality/work comparison rather than a
feasibility difference.

Sequence 085 is deliberately labeled a fixed-prefix-4-parent diagnostic, not
an independent growth result and not a permissible future parent. Raising the
via transition cost from 800 to 100,000 makes the search spend 43,917
expansions, 69.5% more than the default shared-tree search, but produces the
exact same branch paths and four vias. This is strong evidence that the front
layer's raster free space is disconnected for this net under the fixed
prefix-4 copper, rather than evidence that the default simply prices vias too
cheaply. It is not a formal zero-via infeasibility proof because the current
KiCad routing configuration has no explicit maximum-via constraint.

Sequence 086 independently solves prefix 6, adding `SIG02_R_HEADER` from
`R_SOUTH_TAP2.2` to the south header. The new front-only route is
19.672741 mm, uses eight stored segments, seven bends, no vias, and 11,551 A*
expansions. The complete prefix has eight legacy branches grouped into six
electrical identities, 214.480628 mm of physical copper, 45 stored segments,
four vias, and 39,631 total expansions. Native ERC, DRC, parity, and selected
disconnect counts are all zero.

All 43 poses and the first five route files are unchanged from the independent
prefix-5 solve. The higher active-prefix connectivity distance is measured
over the added conductor, not inherited placement. Prefix 6 therefore adds
search pressure but still does not exercise cross-net rerouting or placement
feedback.

Sequence 087 independently solves prefix 7. `SIG03_W_R` adds 35.671823 mm,
10 stored segments, nine bends, no vias, and 10,457 expansions. The complete
prefix reaches 250.152451 mm, 55 stored segments, four vias, and 50,088
selected-route expansions. All poses and the first six route files are
unchanged from prefix 6.

Sequence 088 independently solves prefix 8 with the shared-tree-only config.
`SIG03_AFTER_LINK` is a three-terminal conductor and adds 48.911602 mm, 10
segments, four bends, four vias, and 4,869 expansions. The complete board has
299.064054 mm, 65 segments, eight vias, and 54,957 selected-route expansions.
The new net's closest via pair is 1.030776 mm apart. Complete-board analysis
finds the same minimum and a 3.716517 mm closest cross-net via pair, so the
eight vias form two excursions rather than a cluster. Native ERC, DRC, parity,
and selected disconnect counts remain zero.

Sequence 089 is a fixed-prefix-7-parent topology diagnostic, not a growth
result. Unlike the earlier three-terminal nets, rooted star beats shared
copper for `SIG03_AFTER_LINK`:

| Policy | Copper | Expansions | Vias | Minimum via spacing |
| --- | ---: | ---: | ---: | ---: |
| Rooted star | 48.411602 mm | 4,822 | 4 | 1.030776 mm |
| Shared copper | 48.911602 mm | 4,869 | 4 | 1.030776 mm |

Both candidates are native-complete. Rooted star is 0.5 mm shorter and uses
47 fewer expansions. This reverses the prefix-2 and prefix-5 comparisons and
is direct evidence that topology policy belongs in a per-connection portfolio.

Sequence 090 promotes that finding correctly by independently rebuilding all
eight connections from zero copper with both policies available at every
rung. Shared copper wins `SIG01_AFTER_LINK` and `SIG02_AFTER_LINK`; rooted star
wins `SIG03_AFTER_LINK`. The selected board is 298.564054 mm with eight vias
and 54,910 selected-route expansions. Evaluating the unfiltered portfolio,
however, spent 114,576 expansions and 16 native gates because five
two-terminal connections were evaluated twice even though tree policy cannot
affect them.

The resulting architecture correction canonicalizes only topology and
tree-attachment fields that are inactive for two-terminal connections before
portfolio generation. It does not merge different resolutions, costs,
clearances, or other active settings. Transaction schema v2 records terminal
count, declared entries, skipped-equivalent entries, and effective entries.
Sequence 091 proves the distinction on cold prefix 2: its two-terminal first
connection evaluates one of two declared entries, while its three-terminal
second connection retains and native-gates both; the selected route is
byte-identical to the pre-change control.

Sequence 092 repeats the complete cold prefix-8 portfolio. It skips exactly
five of 16 declared entries, evaluates and native-gates 11, and spends 91,768
portfolio expansions. That saves 22,808 expansions (19.91%) and five native
gates (31.25%). The final PCB is byte-identical to sequence 090, all poses are
identical, and the native gate remains clean.

Sequence 093 independently solves prefix 9 with that deduplicated portfolio.
`SIG03_R_HEADER` is a two-terminal connection from the south header to the
third south tap. It adds 5.908063 mm, five stored segments, two bends, two
vias, and 1,051 expansions. The complete selected board has 304.472117 mm,
70 stored segments, 10 vias, and 55,961 selected-route expansions. Portfolio
work is 92,819 expansions across 12 effective/native-gated entries; six of 18
declared entries are explicitly skipped as two-terminal topology equivalents.
All poses and the first eight selected route files match prefix 8, but were
regenerated. Native ERC, DRC, parity, and selected disconnect counts are zero.

The new via pair is 3.181981 mm apart and its closest foreign via is 3.0 mm
away. The complete board's minimum remains the existing 1.030776 mm pair on
`SIG03_AFTER_LINK`; prefix 9 introduces no cluster.

Sequence 094 is a fixed-prefix-8-parent high-via-cost diagnostic, not a growth
result. Unlike the prefix-5 diagnostic, it finds a different, zero-via route:

| Prefix-9 policy | Copper | Expansions | Vias | Bends |
| --- | ---: | ---: | ---: | ---: |
| Default via cost | 5.908063 mm | 1,051 | 2 | 2 |
| 125× via cost | 73.085733 mm | 79,785 | 0 | 18 |

The zero-via route is native-complete, but is 67.177670 mm or 12.37× longer
and uses 75.91× the search work. The length-only break-even is 33.588835 mm
per via, far above the current 2 mm selection penalty. These two vias are a
high-value local layer transition, not redundant clutter.

Sequence 095 independently solves prefix 10 with the deduplicated portfolio.
`SIG04_W_R` adds 25.624269 mm, six stored segments, three bends, two vias, and
4,783 expansions. The complete selected board has 330.096386 mm, 76 stored
segments, 12 vias, and 60,744 selected-route expansions. Portfolio work is
97,602 expansions across 13 effective/native-gated entries; seven of 20
declared entries are skipped as inactive two-terminal topology duplicates.
All 43 poses and the first nine selected route files match prefix 9, but were
regenerated from the pruned semantic board and zero copper. Native ERC, DRC,
parity, and selected disconnect counts are zero.

The new route uses short front-layer endpoint escapes and a long back-layer
west-east trunk. Its vias are 20.804146 mm apart. Neither new via appears in
the complete board's ten closest via pairs; the minimum remains the existing
1.030776 mm pair on `SIG03_AFTER_LINK`. Prefix 10 therefore introduces no via
cluster.

Sequence 096 is a fixed-prefix-9-parent high-via-cost diagnostic, not a growth
result and never a parent for another prefix. Raising via cost from 800 to
100,000 retains the exact same two-via path and 25.624269 mm length, while
expansions rise from 4,783 to 329,520 (68.89x). This is strong evidence that
the front-layer search space is disconnected under the fixed prefix-9 copper,
but it is not a formal zero-via impossibility proof because the router has no
hard zero-via constraint.

Sequence 097 independently solves prefix 11 with the one-source topology
portfolio. `SIG04_AFTER_LINK` is a three-terminal net. Rooted star and shared
tree produce the same 66.892553 mm geometry: 11 stored segments, three bends,
six vias, and 39,446 expansions per candidate. The board is native-complete at
396.988939 mm, 87 segments, and 18 vias, but the new net contains a connected
pair of vias exactly 1.0 mm apart. The front-layer attachment moves 1.0 mm and
immediately changes back to the layer already present at the original tree
coordinate. This is legal copper, but a locally redundant layer transition.

Sequence 098 is a fixed-prefix-10-parent attachment diagnostic, not a growth
result. Four ranked shared-tree sources include both layers at the first
attachment coordinate. Rank 1 starts directly on the existing back copper and
is selected: length stays 66.892553 mm, while stored segments fall from 11 to
10, vias from six to five, and the new net's minimum via spacing rises from
1.0 to 6.0 mm. Searching all four sources costs 153,438 expansions. Since the
winner is rank 1, sources 2 and 3 add no quality.

Sequence 099 promotes the finding through an independent cold prefix-11 run.
The shared candidate is bounded to two attachment sources and wins against
the retained six-via rooted control. Placement and the prefix-10 PCB are
byte-identical to sequence 097; the promoted prefix-11 PCB is byte-identical
to the sequence-098 diagnostic. It has 396.988939 mm, 86 stored segments, 17
vias, and 153,929 selected-route expansions. The new connection costs 76,690
expansions. Total portfolio work is 233,159 expansions across 15 effective
entries and native gates, versus 176,494 with the one-source portfolio: a
56,665-expansion or 32.11% premium for removing one via. Native ERC, DRC,
parity, and selected disconnect counts remain zero.

Sequence 100 independently solves prefix 12 with the promoted layer portfolio.
`SIG04_R_HEADER` is a two-terminal connection and is deduplicated to one
candidate. It adds 28.186238 mm, six stored segments, five bends, no vias, and
11,234 expansions, entirely on the front layer. The complete board has
425.175177 mm, 92 stored segments, 17 vias, and 165,163 selected-route
expansions. Total portfolio work is 244,393 expansions across 16 effective
entries and native gates; eight of 24 declared entries are skipped. Poses and
all prior route geometry match prefix 11. The raw intermediate PCB also names
the not-yet-hydrated twelfth net, so it is not byte-identical to the prefix-11
artifact despite identical physical geometry. Native ERC, DRC, parity, and
selected disconnect counts are zero.

Sequence 101 independently solves prefix 13 with the promoted layer portfolio.
`SIG05_W_R` is a two-terminal connection and is deduplicated to one candidate.
It adds 31.841664 mm, six stored segments, three bends, two vias, and 8,802
expansions. The vias are 18.681542 mm apart, with no close pair or cluster.
The complete board has 457.016841 mm, 98 stored segments, 19 vias, and 173,965
selected-route expansions. Total portfolio work is 253,195 expansions across
17 effective entries and native gates; nine of 26 declared entries are
skipped. Poses, prior-route quality, and prior selected work match prefix 12.
Native ERC, DRC, parity, and selected disconnect counts are zero.

Sequence 102 is a fixed-prefix-12-parent high-via-cost diagnostic, not a growth
result and never a future parent. Raising router via cost from 800 to 100,000
finds a native-complete zero-via route with 37.023645 mm, six segments, five
bends, and 9,065 expansions. The default two-via route is 5.181981 mm shorter
and takes 263 fewer expansions. With the selection objective's current 2
mm/via penalty, its score is 35.841664 mm versus 37.023645 mm, a narrow
1.181981 mm win. The length-only break-even is 2.590990 mm per via. The vias
are useful under the current policy, but unlike prefixes 5 and 10 they are not
structurally required; both outcomes remain inspectable.

Sequence 103 is the first independent cold prefix-14 failure. Both rooted and
two-source shared-tree candidates fail on branch 0 of the three-terminal
`SIG05_AFTER_LINK` net after 820,070 expansions each. The branch fails before
tree policy can diverge. No candidate reaches native gating; the exact
prefix-13 parent is selected through rollback. Total run work is 1,893,335
expansions, including the independently regenerated first 13 connections.

Sequence 104 is a fixed-prefix-13-parent 0.125 mm diagnostic, not a growth
result. It also exhaustively finds no branch-0 path, after 3,413,154
expansions, and rolls back exactly. The failure is therefore not explained by
the 0.25 mm raster alone.

Sequence 105 runs all 13 bounded single-connection yielding counterfactuals.
Only removing `SIG01_AFTER_LINK` opens `SIG05_AFTER_LINK`; the target then
routes front-only as 46.784931 mm, 13 segments, 11 bends, no vias, and 6,620
expansions. The other 12 removals still exhaust branch 0. Diagnosis costs
10,746,942 expansions and is causal guidance only: the yielded net is absent,
so this is not a complete board.

Sequence 106 is a fixed-prefix-13-parent transactional repair. It removes the
identified blocking copper, routes `SIG05_AFTER_LINK`, and restores
`SIG01_AFTER_LINK` before native admission. The restored net changes from
57.016898 mm canonical physical copper with no vias to 63.268446 mm with four
vias; those vias have 10.75 mm minimum spacing and no cluster. The repair costs
10,787,680 expansions including diagnosis, and the final board has zero native
ERC, DRC, parity, or selected-connectivity findings.

Sequence 107 promotes the repair through an independent cold prefix-14 run.
Ordinary rooted and shared candidates reproduce the two 820,070-expansion
failures before bounded single-net repair is invoked. Placement and all
earlier routing are regenerated inside the run; sequence 106 is not a parent.
The final PCB is byte-identical to sequence 106 and contains 509.522335 mm of
stored copper, 112 segments, and 23 vias. Total cold-run search is 12,681,015
expansions. Most of that work is serial proof that the base route and 12 of 13
single-yield alternatives remain disconnected, making counterfactual
reachability an immediate batching/GPU target. Native ERC, DRC, parity, and
selected disconnect counts are zero.

Sequences 108–110 restart prefix 14 with zero copper and compare placement
before routing. Declared placement has 35 proper cross-net ratsnest crossings,
2.222 peak 32x32-cell demand, and 104.321 mm deposited demand. Barycentric
placement reduces these to 15, 2.842, and 54.491 mm, but visibly collapses the
active passives into one hotspot. Harmonic placement gives 17 crossings,
2.071 peak demand, and 57.197 mm deposited demand. Its lower peak and squared
demand make it the first routing candidate; barycentric remains a useful
shortest-ratsnest control rather than being discarded.

Sequence 111 adds an exact flood-fill reachability preflight to the fixed-parent
prefix-14 yielding diagnosis. It rejects the base and 12 impossible trials
before A*, while the only reachable `SIG01_AFTER_LINK` trial runs the same
6,620-expansion A* and produces byte-identical route geometry. Combined
reachability plus A* work is 1,354,817 rather than 10,746,942 expansions, an
87.4% reduction; the diagnosis takes 6.509 seconds.

Sequence 112 is a deliberately interrupted harmonic cold run. Timing through
native-complete rung 6 showed 3.68 seconds of local routing inside 146.861
seconds of progression. Repeated source, unrouted-target, and candidate KiCad
gates—not the Rust route kernel—were the dominant iteration cost.

Sequence 113 keeps native admission but reuses the immediately committed
parent report only under its exact directory hash, defers the intentionally
incomplete unrouted-target gate until fallback/rollback, and native-gates
score-ordered local candidates until the first complete child. The independent
cold harmonic run reaches prefix 14 in 135.236 seconds; the matched first six
rungs fall from 146.861 to 54.030 seconds. Rung 10 still exercises real
single-net repair: yielding and restoring `SIG02_AFTER_LINK` opens
`SIG04_W_R`, and the deferred unrouted gate runs on that failure path. The
final board has zero native ERC, DRC, parity, or selected-connectivity findings,
462.051030 mm stored copper, 129 segments, and 21 vias. Against sequence 107's
declared-placement board this is 9.3% less copper and two fewer vias, at the
cost of 17 more segments. This is the first dual-board result where regenerated
placement materially improves the independently rerouted board.

Sequence 114 checks the smaller prefix-5 semantic pressure control. The DUT
router completes all 7 branches in 6,504 expansions before any feedback, so
the pressure coordinator correctly emits no move. This is evidence that
prefix 5 is useful for topology/layer controls but not for failed-front
placement feedback under this semantic router.

Sequences 115–119 investigate pressure feedback on the harmonic prefix-14
problem before connecting it to KiCad. The initial semantic DUT route reaches
16/19 branches with five exact findings. Sequence 115 first exposes and fixes
a projector defect: a 270-degree ESP32 exactly fits its horizontal movement
region, but floating-point cosine error made the interval appear empty.
Blocker-only moves then remain at 16/19. Sequence 116 adds selectable
maximum-distance and collision-chain propagation. `ESP_E` can now move with
`C_E_B` and `C_E_EN`, and a larger move pushes `R_E_EN`; placement rejections
fall from two to zero, but routing is unchanged.

Sequence 117 changes only pressure ordering to prefer components implicated in
more distinct failed branches. A 1 mm left move of `R_LINK_05` improves the
selected state to 18/19 branches and two exact findings. Sequence 118 adds
failed-branch endpoint pulls; a 0.5 mm right move of `R_LINK_03` also reaches
18/19 with 5,686,656 total expansions versus sequence 117's 7,362,998.
Sequence 119 explores both the pressure direction and its opposite in 60
retained attempts. It still reaches at most 18/19, proving that a wider greedy
one-move neighborhood does not solve `SIG04_W_R`; neutral intermediate states
or topology actions remain hypotheses rather than silently enlarged budgets.

Sequence 120 adds the missing semantic-pressure-to-native-placement bridge.
It starts from the original JSON, prunes to 14 connections, regenerates
harmonic placement, reproduces sequence 118's selected 0.5 mm `R_LINK_03`
move, discards all 18/19 semantic copper, materializes native rung 0, and
reroutes every connection. The native run completes all 14 rungs with zero
ERC, DRC, parity, or selected-connectivity findings. Relative to the unchanged
harmonic sequence 113, stored copper falls from 462.051030 to 454.747729 mm
(1.58%), segments remain 129, and vias increase from 21 to 22. With the
current 2 mm/via penalty the score improves from 504.051030 to 498.747729 mm.
Most of the gain is a different `SIG04_R_HEADER` route (42.251923 to
34.777050 mm); the moved component's nearby nets change in both directions,
so this is evidence of coupled global rerouting, not a local-length proof.
Semantic feedback adds about 4.5 seconds and 5.69 million DUT expansions;
native progression is 135.055 seconds and 461,824 expansions. Rung 10 still
requires single-net repair, so this placement move improves quality but does
not remove the known topology conflict.

Sequences 121–129 test that remaining conflict without inheriting any routed
board. Sequence 121 replaces greedy retention with a width-four beam and
permits neutral states to remain in the frontier. It explores 141 distinct
placement attempts over three iterations, including `R_LINK_03`,
`R_SOUTH_TAP3`, `ESP_W`, and `J_CTRL_E` moves, but remains at 18/19. The
negative result says that retained placement diversity alone is insufficient
under this proposal family; it does not reject beam search.

Sequence 122 isolates bounded selective rip-up on the unchanged harmonic
placement. It repairs two failed connections in succession but leaves
`SIG04_W_R` unrouted at 18/19. Sequence 123 composes the beam and rip-up seams
and appears to reach 19/19 semantically by moving `R_LINK_04`, then rerouting
three connections. Cold sequence 124 rejects that result at native rung 0:
the moved `R_LINK_04` pad violates clearance with `R_LINK_07`, producing two
DRC findings. This was an integration defect, not a routing success.

Sequences 125–127 progressively move the missing native rule into the common
semantic transaction boundary. Imported/pressure placements now check
pad-extended component envelopes, copper-bearing pairs receive board-rule
clearance, and collision propagation uses those same envelopes. The first
envelope version still treated an already-valid exact-clearance contact as an
old collision, so it did not propagate the new incursion. Sequence 128 adds a
1e-6 mm contact tolerance and then reaches 19/19 in one pressure iteration:
one diagonal action moves the four-component chain `R_LINK_04`, `R_LINK_02`,
`R_LINK_07`, and `R_LINK_05`; one selective rip-up completes the route. All
four proposed placement transactions pass the strengthened semantic gate.

Sequence 129 promotes that exact configuration through the cold bridge. It
again starts from the original prefix-14 semantic input, regenerates harmonic
placement, moves the four-component chain, discards the provisional 19/19
copper, creates zero-copper rung 0, and routes all 14 electrical connections.
The final native board has ERC 0, DRC 0, parity 0, and no selected-net
disconnects:

| Cold prefix-14 result | Copper | Segments | Vias | 2 mm/via score |
| --- | ---: | ---: | ---: | ---: |
| Harmonic, no feedback (113) | 462.051030 mm | 129 | 21 | 504.051030 mm |
| Single-component pressure (120) | 454.747729 mm | 129 | 22 | 498.747729 mm |
| Four-component pressure + rip-up (129) | 462.515496 mm | 131 | 20 | 502.515496 mm |

The mixed result is a capability gain, not the new quality winner: it trades
7.767767 mm and two segments for two fewer vias relative to sequence 120, and
is 3.767767 mm worse under the configured score. Semantic feedback takes
1.552 seconds; native progression takes 98.069 seconds and the whole run
100.190 seconds. This is further evidence that a GPU A* port is not the
current iteration-time bottleneck. The GPU-facing priority remains batched
placement fields and counterfactual routing work, while native KiCad process
orchestration needs caching/batching independently.

Sequences 130–140 add prefix 15, `SIG05_R_HEADER`, without using any prefix-14
poses or copper. Sequence 130 is the independent harmonic/no-feedback control.
It completes all 15 native connections at 493.939685 mm, 124 segments, and 26
vias. The new header connection itself is front-only, 21.398566 mm, eight
segments, and zero vias. Native ERC, DRC, parity, and disconnect counts are
again zero.

The feedback controls expose a new coupling boundary rather than improving
that board. Sequence 131's greedy pressure moves six components and improves
the semantic result from 16/20 to 19/20 branches, but native progression stops
at rung 11 before `SIG04_R_HEADER`. Sequence 132's mixed beam/rip-up policy
moves only `R_SOUTH_TAP4` and contacting `R_LINK_10`, then uses three semantic
selective-rip-up steps to reach 20/20. It stops at the same native rung. All 66
single- and two-old-connection counterfactuals in sequence 133 still fail
reachability, so this is not evidence for merely enlarging native rip-up from
one to two connections.

Sequence 134 aligns the semantic router with input connection order. The mixed
policy still chooses the same two-component placement. Sequence 135 removes
selective rip-up; a width-four beam reaches semantic 20/20 after moving
`R_SOUTH_TAP5`, `R_LINK_04`, and `R_SOUTH_W_C` in three iterations. Its cold
promotion in sequence 136 also stops at native rung 11. Sequence 137 limits
the beam to one iteration and isolates the first move: `R_SOUTH_TAP5` moves up
0.5 mm and semantic routing improves only to 17/20, yet native progression
again loses `SIG04_R_HEADER`.

That one-component control identifies the first divergence. Native rungs 1–8
are byte-equivalent in route quality. At rung 9 the moved, not-yet-connected
passive acts only as an obstacle and changes the transactional
`SIG03_R_HEADER` repair from 13.522976 mm/six segments to 12.522976 mm/seven
segments. The locally shorter route consumes the only later
`SIG04_R_HEADER` channel. Sequence 138 continues the other native-complete
rung-9 repair alternative from the same fixed parent; it also stops at rung
11. Sequence 139 repeats the exact failed route at 0.125 mm and still reports
no path, so this is not the known 0.25 mm conservative-raster threshold.

Sequence 140 changes the cold feedback bridge from a one-way handoff into a
native transaction. Feedback-selected placement remains provisional until the
complete zero-copper native progression reaches its target. On failure, the
same command independently regenerates the original base placement and every
route from zero copper; it never continues from the provisional rung-11
rollback. Manifest schema v2 retains both trials and names the selected
variant. In the prefix-15 control it selects `base_placement_fallback`, reaches
rung 15 with zero authoritative findings, and produces a PCB byte-identical to
sequence 130. The run takes 272.910 seconds: 110.064 seconds for the rejected
feedback progression and 160.433 seconds for fallback progression. This
restores the fully-finished-phase invariant, but it is intentionally not a
speed result or a claim that the pressure move helped.

Sequences 141–144 evaluate bounded future-connection reachability at the first
causal divergence. Transaction schema v3 can independently materialize and
route the next three still-enabled connections against every native-complete
child. The evidence is retained beside, rather than inside, candidate board
directories so accepted parents do not recursively carry diagnostic boards.
It records raster-reachability work separately from real route work and can be
used either as an advisory ranking term or as an explicitly stricter
`require_all` experiment.

The strict policy is negative on this case. For the one-move rung-9 state,
both native-complete single-rip-up repairs can route `SIG04_W_R` and
`SIG04_AFTER_LINK` but not `SIG04_R_HEADER`: two of three are reachable. The
matched harmonic state produces exactly the same classification for both of
its repairs, even though its ordinary sequential pipeline is known to reach
rung 15. The one-move transaction spends 249,734 reachability and 102,266
route expansions; the harmonic control spends 214,826 and 100,361. The
classifier therefore has no precision here and adds substantial work. Its
specific blind spot is that each future connection is checked against fixed
current copper, while the successful control can repair copper when the
future rung is actually inserted. Keep this seam optional; do not use
`require_all` for cold growth and do not infer that the placement move is
uniquely bad from this signal.

Sequence 145 makes placement itself a first-class native portfolio. The new
`solve-semantic-kicad-placement-portfolio` command accepts an explicitly
bounded list of placement configurations. It prunes connectivity once, then
each entry independently generates poses, emits a separate ladder,
materializes rung 0 with zero copper, and routes all requested connections.
No trial consumes another trial's poses or copper. Selection orders native
completion, farthest rung, total imported physical copper plus via penalty,
then placement-demand diagnostics and ordinal. Failed entries remain retained.

The first prefix-5 comparison is clean: both declared and harmonic trials
reach rung 5 and independently pass native ERC/DRC/parity/connectivity with
zero findings. Declared placement produces 192.974062 mm, 23 segments, and
four vias. Harmonic placement produces 153.261470 mm, 24 segments, and four
vias, a 39.712592 mm/20.579% copper reduction. It also reduces the straight
placement surrogate from three crossings/28.342730 squared demand to two/
13.389797. The cost does not move in the same direction: harmonic routing
uses 205,568 expansions versus 76,564 and takes 41.445 seconds versus 38.591.
This is evidence that placement materially improves board geometry, not that
the current A* workload becomes cheaper. Harmonic still reports 32
under-anchored blocks at this short prefix, so unused components largely
retain their input seeds until connectivity gives the placer information.

Sequences 146–147 turn that limitation into an explicit seed-policy
experiment. `HarmonicPorts` now keeps its historical declared fallback by
default, but can instead apply deterministic grid or bounded random poses only
to movable components in under-anchored Laplacian blocks. Harmonic-active
blocks are unchanged before the shared legalizer. The complete config remains
part of each cold portfolio entry, and all outcomes pass through the same
placement projection and native routing pipeline.

Sequence 146 exposed an attempt-accounting defect rather than a negative
random result: a proposal error returned before consuming attempts 2–16. The
generic generator loop now retains such an error and continues its declared
budget; a focused two-attempt regression covers the boundary. Sequence 147 is
the corrected experiment. Grid still fails its one placement attempt, and
random seed 0 exhausts all 16 attempts without a legal pose. Random seed 1
succeeds on attempt 11, reseeds all 32 under-anchored blocks, and reaches a
fresh native-complete rung 5 with zero ERC/DRC/parity/connectivity findings.

The random candidate is useful but does not beat the historical fallback. It
uses 154.256284 mm/23 segments/six vias versus 153.261470 mm/24 segments/four
vias. Its ratsnest distance and squared demand are slightly lower
(19.915691/12.990090 versus 20.390155/13.389797), and it uses 195,199 rather
than 205,568 route expansions, but final physical copper is 0.994814 mm longer
and the two extra vias raise the configured score by 4.994814 mm. This is a
small quality regression, not grounds to remove the seed policy: one of two
seeds produced a valid whole board, placement changed 38 components, and the
result demonstrates diversity that the retained fallback cannot provide.

Sequence 148 advances only to prefix 7, where `SIG03_W_R` first connects the
formerly isolated `R_LINK_03` block to one fixed ESP32. Both historical and
random-seed-1 placements independently reach a native-complete rung 7. The
random candidate improves physical copper from 204.632007 to 198.004254 mm
(6.627754 mm/3.24%) and route expansions from 251,335 to 230,185 (8.42%), but
uses ten vias rather than six. With the declared 2 mm/via portfolio objective,
its score is 218.004254 versus 216.632007, so the historical fallback remains
selected.

The random candidate's only cluster diagnostic is specific rather than a
general via pile: the two `SIG02_AFTER_LINK` branches enter B.Cu at
`[53.375, 52.125]` and `[54.375, 52.125]`, exactly 1 mm apart, then diverge.
They are connected by a short F.Cu fork, contain no overlapping track length,
and the whole board exact-passes. A single shared layer change followed by a
B.Cu branch is nevertheless a plausible improvement. The current rooted-star
and two-source shared-tree portfolio entries both produce the same four-via
connection, exposing a concrete attachment/topology search gap rather than a
native-validity problem.

Sequences 149–150 isolate that topology choice on the immutable random
placement rung-4 parent. An eight-source, 2 mm-spaced, via-first shared-tree
control reduces `SIG02_AFTER_LINK` from four vias to two and removes the close
pair, but grows copper from 47.595358 to 57.505260 mm; its 61.505260 score loses
the 55.595358 baseline. The same diverse sources under the existing
router-cost objective find the useful middle point: the second branch reuses
the first branch's B.Cu entry, the connection has 47.845358 mm/three vias/no
close pair, and its score improves to 53.845358. Both diagnostics pass the
native gate. The balanced search costs 207,367 expansions versus 49,818 for
the selected baseline route.

Sequence 151 promotes the balanced mode as a third portfolio entry and reruns
both placements cold through rung 7. The result survives later insertion:
each complete board loses one via with unchanged physical copper and no via
clusters. Historical harmonic becomes 204.632007 mm/35 segments/five vias
(score 214.632007); random seed 1 becomes 198.004254 mm/33 segments/nine vias
(score 216.004254). The placement winner therefore remains unchanged by
1.372246 mm. This quality gain is not free: total route expansions rise from
251,335 to 539,198 for historical placement and from 230,185 to 475,674 for
random placement, while both runs still invoke seven native gates. The next
architectural step is evidence-directed activation of the diverse source
search after a cheaper multi-terminal candidate exposes enough vias, rather
than making eight-source search unconditional.

Sequence 152 adds that pass boundary as transaction schema v4. A bounded
conditional routing suffix observes the cheapest already-generated candidate
and, in this experiment, activates the diverse router-cost entry only when it
has at least four vias. Every decision stores the trigger, observed score and
via count, activation result, config hash, and optional attempt ordinal.
Two-terminal connections are explicitly inapplicable; the condition neither
skips native admission nor makes a routability claim.

Both cold prefix-7 trials declare seven decisions, activate exactly once on
`SIG02_AFTER_LINK`, and skip the other six. Their final PCBs are byte-identical
to sequence 151 and independently exact-pass. Historical route work falls
from 539,198 to 509,752 expansions (5.46%); random falls from 475,674 to
437,552 (8.01%). Each retains ten generated local candidates and seven native
gates. This is a modest but real work reduction, so conditional activation is
worth retaining. It does not restore the much cheaper sequence-148 work:
finding the useful diverse candidate itself remains expensive.

Sequence 153 inspects the retained attachment trials rather than merely
shrinking their bound. Both placements selected rank 1: the layer-diverse B.Cu
source immediately following the nearest F.Cu source. Ranks 2–7 never win;
some route into the tree and trim back to the exact rank-1 contact after doing
a complete search. A two-source, 2 mm-spaced/router-cost fixed-parent control
therefore reproduces sequence 150's PCB byte-for-byte while reducing route
expansions from 207,367 to 96,747 (53.3%). Its candidate JSON differs because
it correctly retains only the two searches actually performed.

Sequence 154 promotes that bound through the schema-v4 conditional cold
portfolio. Both final PCBs remain byte-identical to sequences 151–152 and
independently exact-pass. Historical work falls from the unconditional
539,198 to 357,636 expansions (33.7%); random falls from 475,674 to 326,932
(31.3%). Compared with sequence 148 before the one-via improvement, the cost
is now about 42% additional route work rather than 107–115%. Each trial still
records seven trigger decisions, one activation, six skips, ten generated
candidates, and seven native gates.

Sequences 155–158 port the useful boundary from the predecessor's placement
population search: generate a cheap bounded archive, retain rejected and
duplicate proposals, and route only an explicit finalist count. The Rust
archive ranks crossings, squared/peak demand, and ratsnest length only as a
prefilter. Every retained finalist still regenerates a zero-copper KiCad board,
routes the complete requested prefix, and competes on native completion and
physical copper/via score. Existing configs retain their first-feasible
behavior; archive expansion is opt-in and has separate entry and native-trial
bounds.

The first archived policy is deliberately negative. Sequence 155 evaluates
all 16 random under-anchored harmonic proposals: 15 fail exact placement
exclusion and only the previously known proposal 10 survives. It reaches
native-clean prefix 5 at 154.256284 mm/six vias. Sequence 156 raises harmonic
legalization from 64 to 256 sweeps and the pair-check ceiling from one to four
million, but still retains exactly the same one proposal. The final blockers
change, showing cycling among overlapping pairs rather than a simple exhausted
budget. No second native routing trial is fabricated from this result.

Sequence 157 replaces whole-board random reseeding with independent local
mutations of the legal harmonic parent, following the predecessor's baseline
plus population pattern without copying its routing score. All 16 proposals
are feasible and unique. Sequence 158 routes the two best cheap finalists from
zero copper. Both exact-pass prefix 5 with four vias. Proposal 2 moves
`R_LINK_01` and the constraint-projected `R_LINK_08` upward by 0.25 mm and is
selected at 153.103349 mm/24 segments/four vias after 202,630 expansions.
Proposal 8 shifts `C_W_B` and `ESP_W` and reaches 153.469038 mm/24 segments/four
vias after 205,493 expansions. The prior harmonic control was 153.261470 mm
with four vias and 205,568 expansions, so the selected mutation is 0.158121 mm
shorter and uses 1.43% less route work. This is a small positive result, not a
claim that the cheap archive rank predicts routing quality or that local
mutation should replace harmonic placement.

Sequence 159 adds the missing composition between archived initial parents and
routing-failure-directed placement. The placement archive again evaluates 16
independent local mutations and retains proposals 2 and 8. For each parent the
semantic DUT router runs the bounded failed-terminal pressure policy, but its
copper is discarded. The unchanged parent and selected pressure child each
start a separate zero-copper native progression. The declared four-trial bound
therefore covers two lineages without making either feedback child a warm
continuation or a mandatory commit.

All four trials reach native-clean prefix 14. Proposal 2's semantic state
improves from 16/19 to 18/19 branches after moving `R_LINK_03` right by 0.5 mm;
its native board changes from 450.130747 mm/22 vias to 450.096586 mm/22 vias.
Proposal 8 begins at 15/19 and reaches 17/19 after a diagonal 0.5 mm push of
`R_LINK_03` propagates into `R_LINK_08`; its native board improves from
457.717367 mm/21 vias to 449.122666 mm/23 vias. Under the two-millimetre via
penalty that second child improves its own parent by 4.594701 mm, but still
loses to proposal 2's 494.096586 mm score. This is useful negative evidence
against selecting a pressure child solely by semantic completion and positive
evidence for retaining multiple placement parents through feedback.

Sequences 160–165 deliberately switch the primary metric from copper score to
the furthest fully native-verified prefix. Independent cold harmonic runs pass
prefixes 16 and 17. The original prefix-18 order stops at rung 14 before
`SIG05_R_HEADER`: reachability rejects the direct route after only 11 cells,
so additional A* expansions cannot change that state. Yielding
`SIG03_R_HEADER` permits the target route, but restoring the yielded connection
is itself unreachable.

Sequence 163 reroutes two local-mutation placements and both stop on the same
pair at 14/18. Their sub-millimetre pose differences demonstrate that the
cheap placement archive is not providing meaningful exploration here.
Sequence 164 therefore uses randomized under-anchored harmonic placement. Its
first finalist visibly moves 38 components yet still stops on the same pair;
its second finalist is rejected at the zero-copper native gate with one DRC
finding. This separates the recurring routing-order defect from a particular
harmonic packing basin.

Sequence 165 changes no connectivity and uses the original harmonic placement,
but swaps the insertion positions of `SIG03_R_HEADER` and
`SIG05_R_HEADER`. The cold run completes 18/18 after 943,660 aggregate route
expansions. The final KiCad gate reports zero ERC, DRC, schematic-parity, and
selected-net connectivity findings. Rung 15 still uses a transactional
single-net repair, so this is evidence for global order/conflict branching,
not evidence that one fixed replacement order is generally correct. Copper
length and via count are intentionally not used to describe this as progress.

Reproduce an independent prefix without providing any previous board:

```sh
cargo run -- solve-semantic-kicad-prefix \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json declared \
  benchmarks/dual-esp32-ladder/template-config.json 14 \
  build/dual-prefix14 \
  experiments/configs/dual-esp32-rooted-shared-layer-portfolio-0.25mm-single-ripup.json
```

## Inspection journal

Paired native front/back images are sequential under `build/progress/`:

- 070: initial declared conversion failure;
- 071: barycentric native failure;
- 072: random native failure;
- 073: native-complete empty board;
- 074: first-rung schematic-parity failure;
- 075: native-complete first routed connection;
- 076: rooted-star unrouted/control and native-complete rung 2;
- 077: shared-tree unrouted/control and native-complete rung-2 ablation;
- 078: native-complete but non-authoritative warm-start rung-3 control;
- 079: manually composed full-connectivity-placement cold routing run, all stages;
- 080: enforced command control before semantic prefix pruning;
- 081: authoritative prefix-3 cold solve after pre-placement connectivity pruning;
- 082: authoritative prefix-4 cold solve; prior poses/routes remain unchanged;
- 083: authoritative prefix-5 cold solve and the first four-via pressure point;
- 084: independent prefix-5 rooted/shared portfolio, with both complete boards;
- 085: fixed-parent high-via-cost diagnostic; exact same four-via geometry;
- 086: authoritative prefix-6 cold solve; front-only addition, prior work unchanged;
- 087: authoritative prefix-7 cold solve; front-only addition, prior work unchanged;
- 088: independent prefix-8 shared-only control;
- 089: fixed-parent prefix-8 rooted/shared topology diagnostic;
- 090: independent prefix-8 topology portfolio before equivalent-work removal;
- 091: focused cold prefix-2 topology-equivalence regression;
- 092: promoted prefix-8 topology portfolio with equivalent work removed;
- 093: authoritative prefix-9 cold solve with a short, clean two-via escape;
- 094: fixed-parent prefix-9 high-via-cost zero-via detour diagnostic;
- 095: authoritative prefix-10 cold solve with a clean two-via back-layer trunk;
- 096: fixed-parent prefix-10 high-via-cost diagnostic; exact same two-via path;
- 097: first cold prefix-11 solve; complete, but with an adjacent via pair;
- 098: fixed-parent four-source attachment diagnostic; one via removed;
- 099: promoted cold prefix-11 two-source layer portfolio;
- 100: authoritative prefix-12 cold solve; front-only and zero-via;
- 101: authoritative prefix-13 cold solve; clean, widely spaced two-via route;
- 102: fixed-parent prefix-13 high-via-cost zero-via diagnostic;
- 103: first cold prefix-14 failure and exact rollback;
- 104: fixed-parent 0.125 mm failure and exact rollback;
- 105: exhaustive single-yield diagnosis; one causal blocker;
- 106: fixed-parent transactional single-net repair; native-complete;
- 107: promoted cold prefix-14 single-net repair.
- 108: declared zero-copper placement control;
- 109: barycentric zero-copper placement control;
- 110: harmonic zero-copper placement control;
- 111: exact reachability-preflight yielding diagnosis;
- 112: interrupted harmonic cold timing diagnosis through rung 6;
- 113: promoted harmonic cold prefix 14 with fast native orchestration.
- 114: prefix-5 semantic no-pressure control;
- 115: prefix-14 blocker-only pressure control;
- 116: relational/collision propagation control;
- 117: multi-branch-first pressure improvement to 18/19;
- 118: failed-terminal pressure improvement to 18/19;
- 119: bounded bidirectional pressure control, still 18/19;
- 120: pressure-selected pose rerouted cold through native prefix 14.
- 121: width-four placement beam; diverse but still 18/19;
- 122: selective-rip-up-only control; still 18/19;
- 123: provisional mixed semantic success, later invalidated by native DRC;
- 124: cold native placement rejection exposing the pad-envelope gap;
- 125: zero-clearance semantic import-gate control;
- 126: copper-pair rule-clearance gate rejecting the false success;
- 127: pad-envelope push-chain control before boundary tolerance;
- 128: legal four-component push plus one selective rip-up, semantic 19/19;
- 129: cold native promotion, complete with zero authoritative findings.
- 130: independent cold prefix-15 harmonic control, native-complete;
- 131: greedy semantic improvement, native rollback at rung 11;
- 132: semantic-complete mixed placement/rip-up, same native rollback;
- 133: 66 single/two-yield counterfactuals, all unreachable;
- 134: input-order mixed semantic control, complete but same placement;
- 135: input-order placement-only beam, semantic 20/20 in three moves;
- 136: cold promotion of sequence 135's policy, native rollback at rung 11;
- 137: one-move causal control, isolating `R_SOUTH_TAP5` as the perturbation;
- 138: alternate native rung-9 repair continuation, also rolling back at 11;
- 139: 0.125 mm failed-route control, still no path;
- 140: transactional feedback rejection and independent native-complete base
  fallback selected in one self-contained cold run.
- 141: strict three-rung lookahead rejects the one-move rung-9 repair;
- 142: the same strict lookahead falsely rejects the successful harmonic control;
- 143: all native-complete one-move rip-up alternatives have the same 2/3 result;
- 144: all matched harmonic alternatives also have the same 2/3 result.
- 145: declared/harmonic cold placement portfolio through prefix 5; both
  complete, harmonic selected with 20.58% less copper.
- 146: preliminary under-anchored seed portfolio, invalidated by a discovered
  proposal-retry accounting defect.
- 147: corrected grid/random under-anchored seed portfolio; random seed 1
  native-completes rung 5, but the declared fallback remains the quality winner.
- 148: matched prefix-7 declared/random-under-anchored control; random is 3.24%
  shorter but has four more vias, so the declared fallback remains selected.
- 149: fixed-parent via-first shared-layer-change control; two vias but an
  overlong detour.
- 150: fixed-parent router-cost shared-layer-change control; three vias, no
  close pair, and a 1.75 mm score improvement.
- 151: cold prefix-7 promotion; one via removed from each placement without
  added copper, with the historical placement still selected.
- 152: conditional diverse-routing promotion; sequence-151 boards reproduced
  byte-for-byte with 5.46%/8.01% fewer route expansions.
- 153: fixed-parent two-source layer-diversity control; selected board
  byte-identical with 53.3% fewer expansions than the eight-source route.
- 154: cold two-source conditional promotion; boards remain byte-identical
  with 31–34% less route work than the unconditional portfolio.
- 155: random-under-anchored proposal archive; 15/16 placement failures and
  the sole known survivor routed through native prefix 5.
- 156: longer-legalization zero-copper control; still only one survivor.
- 157: local-mutation zero-copper archive; 16/16 unique legal proposals and
  two retained finalists.
- 158: cold prefix-5 routing of both local-mutation finalists; both complete,
  with proposal 2 narrowly improving the historical harmonic board.
- 159: two archived prefix-14 parents plus their failure-directed children;
  all four native-pass, with proposal 2's child selected at 450.096586 mm and
  proposal 8's child showing the larger within-lineage copper improvement.
- 160: independent cold harmonic prefix 16, native-complete.
- 161: independent cold harmonic prefix 17, native-complete.
- 162: original-order prefix-18 rollback at 14/18, failed direct route,
  provisional `SIG03_R_HEADER` yield, and matched successful prefix-17 controls.
- 163: two near-identical local-mutation prefix-18 lineages, both stopping at
  14/18 on the same blocking pair.
- 164: broad randomized-under-anchored placement controls; one moves 38
  components and still stops at 14/18, while the other fails the empty-board
  native DRC gate.
- 165: targeted header-order swap, with routed rungs 9 and 15 and the final
  native-complete 18/18 front/back board.
- 166: generic cold order search rediscovers the same swap and completes 18/18.
- 167: first prefix-19 order-search attempt fails the placement gate.
- 168: longer legalization evaluates three independent orders; all stop at
  rung 14.
- 169: targeted placement/order variants stop at rungs 14, 14, and 11.
- 170–192: thin-route width-continuation experiments. A priority 5%-width seed
  routes all 24 branches; trust-region trace/component repair with exact fixed
  pad geometry grows it through 25%, conflict-cover repair reaches 23/24 at
  27.5%, and blocker expansion exposes terminal-via lowering.
- 193: cold exact-rule Freerouting reference routes the identical fixed
  placement at full width, 24/24, in seven passes and 5.90 seconds.

## Next decision

Keep sequence 165 as the hand-authored prefix-18 regression, sequence 166 as
proof that the generic order coordinator can rediscover it, and sequences
162–164 as causal controls. Full-width prefix 19 still plateaus at rung 14, but
thin-first routing now finds all 24 branches and the continuous engine grows
that fixed topology to an exact 25%-width state. At 27.5%, trace-only repair
reduces 99 segment findings to 90 across four route pairs; connected-component
fallback is worse and exact rollback preserves the 25% parent.

The next experiment is therefore a conventional multi-pass rip-up/retry
baseline: fix terminal/internal via handling, grow failed conflict covers with
their strongest blockers, and reroute each pass transactionally from an
auditable parent. Then resume continuation and placement feedback from the
best exact result. Exact compound shapes on movable bodies remain a parallel
engine requirement. Do not spend the next budget on copper/via score, wider
placement seeds, larger A* ceilings, or more unconstrained projection frames.
The detailed evidence is in
[`dual-esp32-width-continuation.md`](dual-esp32-width-continuation.md) and
[`dual-esp32-freerouting-baseline.md`](dual-esp32-freerouting-baseline.md).
