# Conflict diagnostic holdout and exact native net identities

Testing the conflict diagnostic on the ordinary KiCad PIC-programmer fixture
exposed two issues that the semantic ESP32 board did not exercise: custom layer
display names and exact native net spelling. The diagnostic and copper
application path now handle these cases explicitly.

## Layer identity and ranking

The diagnostic compared a retained track's board display layer name with a
candidate's canonical layer name. PIC calls its layers `top_layer` and
`bottom_layer`, so most planar conflicts were incorrectly skipped. The tool now
maps native layer IDs to `F.Cu` and `B.Cu` before comparing geometry. A controlled
copy changes only the two display names; all conflict witnesses and ranking
results remain identical after the fix. Every analysis emits SVG/PNG output.

On the PIC case, the corrected isolated-route conflict set is GND,
`VPP{slash}MCLR`, and `pic_sockets/VCC_PIC`. The resulting priority starts with
GND, which agrees with a successful recorded one-net routing counterfactual.
The exhaustive one-net oracle also finds a route after yielding VCC, even though
VCC does not conflict with this particular isolated path. Thus geometric
intersection neither proves necessary blockers nor enumerates every useful
counterfactual.

| Fixture | First route-found rank, geometric order | Lexical order |
| --- | ---: | ---: |
| ESP junction/blocked | 1 | 7 |
| PIC programmer | 1 | 1 |

The PIC case is a holdout for the previously chosen ranking rule; it does not
show a speed improvement over lexical order. The ESP result is the original
exploratory case, not a holdout. Oracle comparison checks the exact board bytes,
target identity and exhaustive singleton coverage. It uses retained routing
counterfactuals, not fresh full-board rip-up/restoration runs. The tool remains
advisory and does not alter production trial ordering.

## Native net spelling

Copper expressions were constructed by prepending `/` to a normalized connection
ID, although global nets such as GND can have native pad names without that
prefix. A fresh copy of the historical source passes native design checks, but
applying an additional candidate exposed GND versus `/GND` mismatches. The source
already contained mixed spellings; the initial attribution that application
rewrote those existing strings was too narrow. The fix avoids relying on native
alias reconciliation.

`apply_route_candidate_with_yielding_connections` now resolves exact net spelling
from pads, canonicalizes emitted and retained copper to those names, and
preserves source string escaping. Two distinct pad nets that collapse to the
same normalized connection ID are rejected before output is written. The
canonicalization covers segments, arcs, vias and zone net/name fields. This is
the candidate application path; it is not a redesign of connection IDs
throughout the importer.

The CLI now exposes the existing provisional yielding application API through
an optional JSON list argument:

```
pcb-maker apply-kicad-route-candidate BOARD CANDIDATE OUTPUT [YIELDING-CONNECTIONS.json]
```

Yielded connections remain incomplete. This command does not perform native
admission or commit a completed routing transaction.

A native regression applies the recorded **VCC-yielding** DATA-RB7 candidate,
which deliberately leaves GND copper present to exercise exact global net names.
This is a separate successful oracle candidate, not the diagnostic's first
GND-ranked choice. The negative control retains VCC; the positive control
removes it before applying the same target route.

| Case | ERC / design DRC / parity | Native open items |
| --- | --- | ---: |
| Fresh source baseline | 0 / 0 / 0 | 59 |
| Candidate with VCC retained | 0 / 2 / 0 | 53 |
| Same candidate with VCC yielded | 0 / 0 / 0 | 64 |

The two negative-control findings are DATA-RB7/VCC shorts. Neither applied case
has a target-net open. The positive case preserves all component poses and the
exact geometry/net identity of every unrelated track and via. The 64 remaining
open items include the now-unrouted VCC; this is target feasibility evidence,
not a completed-board improvement. Source files remain byte-identical.

All 104 KiCad library tests pass, including new net-spelling/escaping and
ambiguous-identity regressions. Both release builds pass. The final native
probe freezes executable SHA-256
`3ea5645f50a2d73b483f054b1274a5b4c123b6086cbfa6317ac7f516eb287d23`.
Both verifier processes complete with the expected exit one for incomplete
boards; the final runner exits zero after recording and auditing them.
The earlier `native-pic` prototype retained two failed verification results and
its Python runner ended with exit 139; it is excluded from the positive result.
Its replacement uses the Rust yielding API without Python copper mutation.

- [Study views](../../build/copper-conflict-holdout-2026-09-07/index.html)
- [PIC geometric evidence](../../build/copper-conflict-holdout-2026-09-07/pic-canonical-layers/analysis.json)
- [Native audit](../../build/copper-conflict-holdout-2026-09-07/native-exact-names/audit.json)
- Implementation: `crates/pcb-kicad/src/lib.rs`, `src/main.rs`
- Tools: `experiments/composed-recovery/{analyze_conflicts,assess_ranking,native_conflict_probe,audit_native_conflict}.py`
