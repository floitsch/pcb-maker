# Dense placement probes and failure-directed recovery

The 19-connection follow-up supports keeping full-size harmonic placement as
the first candidate, while retaining alternative legal placements and using
different recovery actions for different failures. It also shows why a small
routing probe cannot be the final placement score.

[Matched-prefix playback](../../build/placement-dense-prefix19-2026-09-07/index.html)
and [separate diagnostic images](../../build/placement-dense-prefix19-2026-09-07/diagnostics.html)
retain the native boards. The [experiment scripts](../../experiments/placement-exploration/README.md#denser-paired-probes)
describe reproduction and auditing. No production defaults changed in this
follow-up.

## Controlled comparison

Three full-board placement seeds contain the same 43 components and full
connectivity. Full-size harmonic and constraint-aware junction reinsertion
were computed in the earlier placement experiment; retained poses provide
the control. Each run selects the same first 19 signal connections and starts
with zero copper. The declared policy preserves the proposed poses. The
native footprint placement is unchanged at every accepted routing step.

Each seed uses both profiles: blocked has 0.25 mm tracks and 0.20 mm clearance;
open has 0.15 mm tracks and 0.10 mm clearance. The latter fits the 0.37 mm ESP
pad gaps, but also changes available space elsewhere. These results do not
isolate pad-gap use from the general effect of narrower tracks and clearance.
Local neckdown remains unimplemented.

The executable is frozen at SHA-256
`64ad75dd548e45767a051dd37b50494042b1aa1832e690c7b7cb3681da5b424a`.
Both profiles use a 0.05 mm grid, two-million A* expansion limit per route
call, reachability preflight, shared-copper tree routing and no automatic
rip-up or fallback. Native verification accepts each new connection before
it becomes the next parent. Concurrent wall times are advisory.

| Placement | Profile | Accepted prefix | Copper mm | Vias | ESP gap crossings | A* + preflight expansions |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Harmonic | Blocked | 16/19 | 259.597 | 20 | 0 | 23,990,857 |
| Harmonic | Open | 19/19 | 322.270 | 21 | 1 | 27,244,648 |
| Junction | Blocked | 7/19 | 128.153 | 7 | 0 | 14,165,373 |
| Junction | Open | 19/19 | 362.237 | 28 | 1 | 34,617,502 |
| Retained | Blocked | 19/19 | 474.282 | 24 | 0 | 29,389,909 |
| Retained | Open | 19/19 | 477.701 | 18 | 7 | 31,417,826 |

Lengths refer to accepted copper only; work includes a failed next attempt
where present. Do not rank different completed prefixes by length or total
work. The viewer compares all seeds at the same selected prefix.

For the open profile, harmonic uses **32.5% less copper** than retained, with
three more vias; junction uses 362.237 mm and 28 vias. Harmonic also uses the
least search work of the three complete open runs. The retained seed is the
only blocked-profile seed that completes under the original policy. The
recoveries below are separate experiments, not silently substituted winners.

All 99 accepted prefix boards have zero ERC, design DRC, parity and remaining
selected-connectivity findings. Metadata warning counts are retained:
harmonic 19, junction 22, retained 21. All six source poses survive generation
unchanged; native placement hashes match throughout routing and across each
profile pair. Every accepted preview parses as SVG. JavaScript syntax checks
pass; the report does not claim a real-browser playback test.

## A search failure is not a placement failure

Harmonic/blocked accepts 16 connections, then exhausts its A* budget on
`SIG06_R_E`: 2,000,001 A* expansions plus 860,535 reachability expansions.
The parent remains intact and rollback is exact. This is not a proof that
the connection lacks space.

On that exact parent, obstacle-distance guidance finds and native-commits
the connection with 75,996 A* expansions plus **2,431,040 preprocessing
expansions**. The A* limit stays unchanged; preprocessing is additional work,
not hidden inside that limit. Keeping guidance for the next two insertions
reaches 19/19 with 326.460 mm of copper and 24 vias. The three continuation
steps use 91,542 A* plus 7,265,023 preprocessing expansions. All non-target
copper remains unchanged in every step, and components never move.

An earlier replay was interrupted with tool exit 143 before producing a
routing result. Its artifacts remain separate. The successful fresh replay
has a recorded terminal subprocess result. No routing conclusion is drawn
from the interrupted attempt.

## A genuine grid blockage can still be a route-order problem

Junction/blocked accepts seven connections. The next connection,
`SIG03_AFTER_LINK`, has three terminals and no path after exhaustive
reachability on the current grid. Raising the A* budget is inappropriate:
A* has not started.

Routing this connection first, on the same physical placement and rules,
passes native verification: 29.109 mm, two vias, 99,149 A* expansions and
1,178,992 reachability expansions. The isolation audit compares physical
pads, target-pad membership, board outlines, rule areas and native design
settings, as well as proposed poses. Earlier copper, rather than fixed
placement alone, is responsible for this particular blocked state.

The existing one-net rip-up policy tests all seven retained nets. Only
yielding `SIG03_W_R` makes the target routable. The program inserts the target,
reroutes that exact old net, and native-commits all eight connections. The
result has 158.132 mm of copper and nine vias. Component placement and the
other six retained nets are unchanged. This is a complete eight-connection
repair, not a claim of 19-connection completion for the repaired seed.

The repair spends 265,102 A* and **22,196,000 reachability expansions**,
including diagnosis and the repeated baseline route call. A* alone would
substantially understate its cost. The relevant yielding net is last in the
current lexical trial order. This motivates ranking blockers by local route
evidence and reusing the failed baseline diagnosis, rather than repeatedly
searching unrelated nets.

## Implementation direction

Use an archive of legal placements and rank actual routing outcomes. Prefer
harmonic placement initially, but preserve alternatives: the retained layout
can complete a budgeted run that the more compact harmonic layout stalls on.
The junction proposal also has a useful repaired continuation, so an early
stall is insufficient reason to discard it permanently.

Make recovery depend on evidence: budget exhaustion should trigger better
search guidance; grid disconnection should trigger isolated-route and
yielding-copper diagnosis; route-derived capacity pressure should move bodies
when copper changes are insufficient. Native admission remains the authority.
The pressure, topology and continuous-relaxation experiments supply distinct
parts of this loop; none is a universal substitute for the others.

The tests cover 19 signal connections on one real board and selected
counterfactuals. They do not establish full 52-connection completion, power
routing with mixed widths, a universal placement winner, or controlled CPU
speed superiority. Every accepted prefix and completed diagnostic retains
native verification and an automatically generated rendering.
