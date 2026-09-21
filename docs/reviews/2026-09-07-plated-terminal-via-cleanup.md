# Via cleanup must use plated terminal connections

The completed open ESP32 board has 18 vias on nine nets. Ranking those nets by
route length selects `SIG03_R_HEADER`, a 5.501 mm route with two vias, for the
first local cleanup test. The existing search tries front/back layer merges
and a bounded front-layer reroute against the complete 19-connection board.

Before the fix, the front-layer merge crosses another net and is correctly
rejected. The back-layer merge is geometrically useful, but the topology code
preserves the old front-layer terminal anchor by adding a via at J4's plated
through-hole pad. Native DRC reports two `holes_co_located` records for that
same via/pad pair. The search consequently retains the original route. Its
bounded front-layer reroute also finds no path in the selected window; this
does not prove that every larger front-layer detour is impossible.

The routing model now records explicit plated-through-hole terminal identity.
Via actions remove redundant endpoint layer transitions when one target-net
plated pad, with a positive drill, spans both copper layers at that endpoint.
Coincident separate SMD pads, unplated holes and single-layer pads do not qualify.
Interior vias are preserved. Candidate segments and vias are regenerated before
geometry checks, scoring and native admission. The same handling applies to
direct actions and remove-and-reroute candidates.

Repeating the exact search on the same immutable source now selects the back
merge. That route uses one via and its plated terminal, with unchanged length.
The whole board drops from 18 to 17 vias while retaining all 19 connections,
zero native ERC/DRC-design/parity/connectivity findings and the existing 21
metadata warnings. Component placement and every other net's physical copper
match the source. The front-layer crossing is still rejected.

Validation includes 95 passing native-adapter library tests and a standalone
native regression. In that regression, the program routes from a plated pad
to a back-layer SMD pad with one via; cleanup then automatically chooses a
zero-via back-layer route. The source board remains unchanged and both native
results have automatic previews. An initial test-fixture output-path mistake
was corrected before the passing run.

[Validation](../../build/routing-improvements/esp32-via-cleanup-2026-09-07/validation.json),
[before search](../../build/routing-improvements/esp32-via-cleanup-2026-09-07/search/portfolio.json),
[after search](../../build/routing-improvements/esp32-via-cleanup-2026-09-07/search-fixed/portfolio.json),
[focused before rendering](../../build/routing-improvements/esp32-via-cleanup-2026-09-07/before-focus.png),
and [focused after rendering](../../build/routing-improvements/esp32-via-cleanup-2026-09-07/after-focus.png)
are retained. The focused views crop the native SVGs; full automatic previews
remain alongside the verification reports.

This fixes endpoint handling in the cleanup action search. Initial routing's
layer choices are now [addressed separately](2026-09-07-plated-terminal-layer-search.md);
the regression retains an explicitly seeded legacy route to protect cleanup
behavior. General interior pad-mediated graph connectivity and board-wide
cleanup by default remain open.
