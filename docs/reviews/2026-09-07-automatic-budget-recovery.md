# Automatic search-budget recovery

The native router can now turn structured budget-exhaustion evidence into an
automatic conditional search. On the retained harmonic/blocked placement it
recovers from prefix 16 and reaches 19/19, without agent intervention between
connections. It selects obstacle-distance guidance only for the failed route;
ordinary routing handles the next two. Production defaults remain unchanged;
the new policy has reusable configurations for both clearance profiles.

[Native boards and machine decisions](../../build/routing-improvements/budget-trigger-2026-09-07/replay/index.html) ·
[Audit](../../build/routing-improvements/budget-trigger-2026-09-07/replay/validation.json) ·
[Reproduction](../../experiments/budget-trigger/README.md).

## Implementation

`KiCadRouteSearchFailure` records the failure kind, connection, branch, A*
expansions, reachability expansions and optional guidance-preparation work.
The evidence is constructed where the router exhausts a search or finds no
path. It is not reconstructed from a string. Other errors, including invalid
inputs and terminal-access errors, remain separate from search failures.
Grid disconnection describes the tested discrete model and selected terminal,
layer and tree-attachment choices; it is not a physical infeasibility proof.

The new detailed API returns this evidence. Existing string-returning APIs
remain compatibility wrappers with unchanged error formatting. Native local
insertion consumes the detailed API and retains an optional `route_failure`
record in its attempt evidence. Existing callers outside this insertion path
have not all migrated to the detailed API.

The conditional trigger is `search_budget_exhausted_without_candidate`.
Unlike the existing via-quality trigger, it works with two-terminal nets and
does not need a generated candidate to activate. It requires an earlier local
attempt with structured budget exhaustion and no generated local candidate.
It skips grid disconnection, unrelated errors and successful route generation.
Evidence records the observed failed attempt ordinals, activation or skip,
configuration hash and resulting attempt ordinal. Existing candidate bounds,
configuration deduplication, immutable parents and native admission apply.

## Native replay

The frozen executable is SHA-256
`5e7ee2028f12146851a36f3210918c4639eb8d6af368297d22ca0c8265d6b954`.
Both replays use retained parents from the dense placement comparison and
unchanged 0.25 mm width / 0.20 mm clearance rules. The conditional configuration
differs from ordinary routing only by selecting obstacle-distance guidance;
it keeps the same two-million A* limit and grid resolution.

| Connection / attempt | Decision | A* | Reachability | Guidance preparation |
| --- | --- | ---: | ---: | ---: |
| `SIG06_R_E`, ordinary | Budget exhausted | 2,000,001 | 860,535 | 0 |
| `SIG06_R_E`, conditional | Activate; native commit at 17 | 75,996 | 0 | 2,431,040 |
| `SIG07_W_R`, ordinary | Native commit at 18; skip guidance | 8,340 | 1,447,627 | 0 |
| `SIG07_R_E`, ordinary | Native commit at 19; skip guidance | 92,312 | 616,140 | 0 |
| Junction `SIG03_AFTER_LINK` | Grid disconnected; skip guidance; exact rollback at 7 | 0 | 2,473,171 | 0 |

Every accepted prefix and the rollback parent pass native verification, with
zero ERC, design DRC, parity or selected-connectivity findings. Metadata
warnings remain visible: 19 for harmonic and 22 for junction. Component poses
are unchanged, as is all non-target copper at each committed step.

The final harmonic board has 326.460 mm of copper and 24 vias. Independent
native copper comparison confirms it is identical to the earlier continuation
that used guidance on all three routes.

For a matched comparison starting from the same prefix-17 parent, the last
two connections use **2,164,419** combined A*, reachability and preprocessing
expansions with the conditional policy, versus **4,849,529** with always-on
guidance: **55.4% fewer**, with identical final copper. This excludes the
failed ordinary attempt needed to discover the prefix-16 budget stall and
makes no CPU timing claim. Over the complete adaptive 16→19 run, that failed
attempt is retained in the table above rather than omitted from accounting.

The library's 102 unit tests pass, including compatibility formatting,
two-terminal activation, successful-candidate suppression, disconnection
suppression, empty evidence and rejection of a budget-looking error string
without structured evidence. The release build succeeds. The replay script
checks native outcomes and renders them automatically.

## Remaining boundary

This implements one automatic response to a diagnosed stall. The separate
grid-disconnection example still needs the existing rip-up mechanism; this
test keeps it disabled to verify that budget guidance is not invoked there.
Combining these mechanisms in a broader placement/routing loop, improving
yielding-net prioritization, and testing longer prefixes remain necessary.
The open-profile configuration preserves its corresponding rules but is not
a new native open-profile result from this replay.
