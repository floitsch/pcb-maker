# Recovery composition and copper-conflict diagnostics

`run.py OUTPUT` reproduces the retained junction/blocked recovery using one-net
rip-up plus conditional budget guidance. Native verification creates previews;
`summarize.py OUTPUT` audits results and builds playback.

`analyze_conflicts.py OUTPUT` uses the retained ESP isolated route by default.
For another same-placement pair:

```
python3 experiments/composed-recovery/analyze_conflicts.py OUTPUT --board BOARD --candidate CANDIDATE
```

The diagnostic handles straight tracks and through vias on two copper layers.
It uses physical widths and candidate clearance and compares canonical layer
IDs, independent of display names. It ranks target-pad via conflicts first,
then other isolated-path conflicts, then remaining routed nets, with lexical
ties. It does not analyze foreign pads or filled zones, prove which blockers
are necessary, or guarantee restoration of any yielded net. SVG/PNG views are
automatic. Inputs are hashed.

`assess_ranking.py ANALYSIS ORACLE` compares this ordering with an exhaustive
recorded one-net counterfactual oracle on the exact same board bytes.
The PIC holdout currently ties lexical order; the exploratory ESP case improves
the first successful counterfactual's rank from seven to one.

For a native feasibility probe of a specific saved candidate:

```
python3 experiments/composed-recovery/native_conflict_probe.py BOARD CANDIDATE YIELDING_NET OUTPUT
python3 experiments/composed-recovery/audit_native_conflict.py OUTPUT
```

The runner copies the project, freezes its executable, and applies the same
candidate with and without yielding via the production Rust application API.
It records verifier completion and automatic previews even when the board is
incomplete. The audit expects a clean positive target case and a conflicting
negative control; it checks target opens, fixed poses, and unrelated track/via
geometry. It intentionally excludes filled zones and arcs. Yielded-net
restoration and complete-board admission remain separate work.

Retained study: `build/copper-conflict-holdout-2026-09-07`. The initial `pic`
analysis used display names incorrectly; `pic-canonical-layers` supersedes it.
The initial `native-pic` prototype is exploratory failed evidence;
`native-exact-names` is the final native regression.
