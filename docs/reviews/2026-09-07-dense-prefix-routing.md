# Dense ESP32 routing exposes a search-budget limit

The next benchmark extension reuses the admitted 19-connection boards with
their existing placement and copper. Both blocked/open profiles first add
connections 20–25, then continue toward 50 signal, auxiliary and control
connections. Every transaction retains native verification and an automatic
rendering. These are checkpoint continuations, not cold routing/timing trials.
Cold configurations are retained separately in the prefix-25 and prefix-50
corpora; they were not executed in this experiment.

## Completion boundary

Both profiles complete 25/25 without a rejected candidate or search-budget
failure. Placements match each other and their retained parents.

| At 25 connections | Blocked | Open |
| --- | ---: | ---: |
| Native track length | 591.947 mm | 588.927 mm |
| Vias | 40 | 28 |
| Pad-gap/net passages | 0 | 7 |
| A* expansions for connections 20–25 only | 2,903,061 | 1,460,036 |

The open profile requires less search work for these six new connections,
despite its larger expansion count on the original 19-connection prefix.
Width and clearance change throughout each profile; this does not isolate the
effect of pad-gap availability alone. No additional gap crossings appear in
these six connections.

Continuing toward 50 reaches different limits:

| Profile | Last admitted prefix | Failed next connection | A* expansions | Reachability expansions |
| --- | ---: | --- | ---: | ---: |
| blocked | 27 | `WEST_AUX_B_R` (28) | 2,000,001 | 868,791 |
| open | 31 | `WEST_AUX_D_R` (32) | 2,000,001 | 1,436,937 |

Both preserve their exact last valid boards on rollback. Their retained
prefixes pass native verification; neither completes the 50-connection target.
The kernel checks the expansion limit after incrementing its counter, hence
the reported one-extra expansion. Reachability had succeeded before A* ran.

## Diagnosis and program correction

The adapter incorrectly collapsed `BudgetExhausted` into the message “found no
path.” It now tracks budget exhaustion per branch and reports that condition
separately from finding no path on the routing grid. The existing expansion
parser still extracts work counts. The via-window unit regression checks both
conditions: a reachable route with a one-expansion budget and a fully closed
via window. All 101 adapter library tests pass. Replaying both retained failed
inputs at their original limits produces the corrected budget messages.

As a diagnostic control, only `max_expansions` was changed from two million to
six million on each exact failed input. Both found routes and passed native
verification, with zero ERC, DRC design, parity and selected-net connectivity
findings. Twenty-one library metadata warnings remain reported. Independent
native geometry comparison confirms that other-net tracks and vias are
unchanged.

| Diagnostic control | Actual A* expansions | New-route vias | Natively complete prefix |
| --- | ---: | ---: | ---: |
| blocked `WEST_AUX_B_R` | 2,664,425 | 2 | 28 |
| open `WEST_AUX_D_R` | 3,098,115 | 4 | 32 |

These controls establish legal routes on the retained layouts. They are not
committed extensions of the original runs and do not raise benchmark search
budgets. The next implementation experiment should improve guidance within
the existing limit—for example, using obstacle-aware reverse distances as a
lower bound for direction-aware A*. It must report preprocessing work alongside
A* expansions, retain the existing costs, and native-verify both resulting
routes. A larger budget alone is not the intended implementation change.

The remaining VCC and GND connections need a separate policy: the source
declares 0.35–0.60 mm branches, with 12 VCC and 16 GND terminals. The narrower
uniform signal configuration cannot silently stand in for those requirements.

[Prefix-25 validation](../../build/esp32-pad-gap-prefix25-continuation-2026-09-07/validation.json),
[prefix-50 failure evidence](../../build/esp32-pad-gap-prefix50-continuation-2026-09-07/validation.json),
and [diagnostic native validation](../../build/routing-improvements/esp32-west-aux-search-2026-09-07/validation.json)
are retained. Renderings show the [blocked diagnostic route](../../build/routing-improvements/esp32-west-aux-search-2026-09-07/larger-budget-native/target-overlay.png)
and [open diagnostic route](../../build/routing-improvements/esp32-west-aux-search-2026-09-07/open-larger-budget-native/target-overlay.png)
in orange; dashed portions are on B.Cu. Unannotated native previews remain
alongside them.
