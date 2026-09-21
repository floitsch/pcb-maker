# Composing the two routing recoveries

The retained junction placement under blocked-gap rules now progresses from
seven to nineteen connections automatically, using the existing one-net rip-up
policy together with conditional budget recovery. All twelve committed boards
pass native verification with zero ERC, design DRC, schematic parity or selected
connection opens. All 43 component poses and unrelated copper remain unchanged.

The first insertion has a disconnected routing grid. Seven one-net trials find
one successful yielding net, `SIG03_W_R`; the target and yielded net both route.
At connection thirteen, ordinary A* exhausts its two-million expansion budget;
conditional obstacle-distance guidance succeeds. The other ten insertions use
ordinary routing alone. Final copper is 365.387 mm with 35 vias. This is a
nineteen-connection result, not a completed full board.

The continuation consumes 4,670,768 A* expansions, 35,191,946 reachability
expansions and 2,441,997 guidance-preparation expansions. These counts describe
algorithmic work, not CPU performance. Most diagnostic work occurs during the
first seven rip-up trials.

A separate posthoc diagnostic intersects an isolated target route with retained
copper, including trace widths, clearance and via diameter. Three foreign nets
conflict with that particular path. Only `SIG03_W_R` obstructs a candidate via
centered inside target pad R3.2; ranking that conflict first matches the native
oracle's sole successful trial. This is a promising ranking hint, not proof
that the via is mandatory or validation on a held-out board. It did not guide
the completed run and is not yet a production policy.

- Runner: `experiments/composed-recovery/run.py`
- Optional policy: `experiments/configs/dual-esp32-blocked-composed-prefix19.json`
- Diagnostic: `experiments/composed-recovery/analyze_conflicts.py`
- [Playback and retained evidence](../../build/routing-improvements/composed-recovery-2026-09-07/index.html)
- [Local conflict rendering](../../build/routing-improvements/composed-recovery-2026-09-07/pad-access-diagnostic/detail.svg)

Artifact manifests retain the executable SHA, inputs, child completion status
and runtime. Independent audits check every native prefix, fixed poses and
unchanged unrelated copper. The HTML has SVG/JavaScript syntax checks; no real
browser playback test is claimed.
