# Retained copper in passage-capacity analysis

The first implementation follow-up to the algorithm experiments fixes a
specific blind spot: a passage may have room for one trace while already being
occupied by another. The existing pressure coordinator considered only the
failed trace's width, so it could identify the correct blocking component but
generate no action.

The new opt-in `passage_capacity_with_copper` policy adds electrical identity,
layer and retained-copper demand to that analysis. It identifies a 0.25 mm
shortage on the retained three-net regression and proposes a 0.3 mm wall move.
The exact-gated result advances from two routed nets to all three after one
repair. The default policy and existing single-trace control remain available.

- [Before: no repair proposed](../../build/routing-improvements/occupied-passage-2026-09-07/replay/control.html)
- [After: occupied passage opens](../../build/routing-improvements/occupied-passage-2026-09-07/replay/occupied.html)
- [Native result](../../build/routing-improvements/occupied-passage-2026-09-07/replay/native/verified/preview.png)
- [Protocol, limitations and reproduction](../../experiments/occupied-passage/README.md)

The native check also exposed a rule-parity gap. The semantic fixture declares
0.20 mm boundary clearance, while the template inherited KiCad's 0.50 mm
default. The failed native control is retained. An explicit optional template
edge-clearance setting now permits a matching-rule export without changing
the geometry or suppressing findings. The exact semantic copper passes that
native check with zero ERC, DRC, parity and connectivity findings.

This is a tested first producer of actionable bottleneck evidence, not a full
topological router. The cut estimate currently supports fixed-layer legacy
branches and the existing body/board passage geometry. Flexible layers,
arbitrary pad/keepout cuts, via occupancy, and real dense-board evaluation
remain open. Failed geometric realization must not be promoted into a proof
that a topology is impossible.

The next useful extension is to attach layer-specific failed-front evidence
to this demand model and compare moving a body with rerouting the identified
occupying net on a real stalled board. The current exact semantic and native
admission boundaries should remain in that loop.
