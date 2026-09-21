# Search the connected layers of plated terminals

The plated-terminal cleanup fix exposed an earlier routing limitation: the
native adapter chose one terminal layer before A*. A plated through-hole pad
usually defaulted to F.Cu even when starting on B.Cu would avoid a via.

The grid request now accepts electrically equivalent start and finish positions.
A* seeds every available start at zero cost, accepts any available finish and
uses the minimum existing lower bound across finishes. These choices share
one expansion budget. Reachability uses the same endpoints. A blocked primary
endpoint does not reject a request when a valid alternative remains; invalid
coordinates and entirely blocked endpoint sets still reject it.

The KiCad adapter adds another layer only when one explicitly plated target-net
pad spans both layers. It materializes the actual layer chosen by search.
Rooted branches connect to the pad anchor on their selected layer. Existing-tree
attachments can use a plated pad's other layer only when the attachment point
lies within its copper. Coincident independent SMD pads do not establish this
connection. Via-clearance and hole-spacing constraints remain enforced.

## Evidence

On the identical completed ESP32 source, routing `SIG03_R_HEADER` gives:

| Initial routing | Before | After |
| --- | ---: | ---: |
| Start layer | F.Cu | B.Cu |
| Connection vias | 2 | 1 |
| A* expansions | 75,399 | 42,646 |
| Reachability expansions | 1,255,874 | 1,048,187 |
| Whole-board vias | 18 | 17 |
| Whole-board track length | 481.290 mm | 481.290 mm |

Both outputs pass native ERC, DRC-design, parity and connectivity checks for
all 19 connections. Placement matches and the original source is unchanged.
This obtains the earlier cleanup improvement during initial route generation;
it is not an additional reduction from the previously cleaned 17-via board.
It also is not a fresh whole-board rerun or a general timing claim.

The grid tests compare combined search with the best independently searched
endpoint pair across all 4,096 small two-layer obstacle masks, with vias both
enabled and disabled, for valid endpoint sets. They check cost, reachability,
selected endpoints and shared budget behavior. All 155 library tests pass
(11 grid-router, 95 native-adapter and 49 routing tests).

Native regressions cover direct plated-to-back-layer routing with no via,
cleanup of a separately seeded legacy route, and a plated hub joining front
and back branches with zero vias under both rooted-star and shared-tree
policies. The SMD-only mandatory-via-window regression still passes. Every
native verification retains its automatic preview.

[Validation](../../build/routing-improvements/plated-terminal-search-2026-09-07/validation.json)
and [focused ESP32 result preview](../../build/routing-improvements/plated-terminal-search-2026-09-07/after-focus.png)
are retained. General interior pad-mediated graph import, local trace neck-down,
and automatic whole-board cleanup remain separate work.

## Fresh paired cold routing

The updated executable also completes both 19-connection pad-gap profiles from
empty copper. All 38 committed transactions preserve their source boards, pass
native verification and retain automatic previews. Both final boards have zero
ERC, DRC design, parity and selected-net connectivity findings, with 21 library
metadata warnings still reported. Freerouting also completes both targets.

| Profile | Track length | Vias | Pad-gap/net passages | A* expansions | Length + 2 mm/via |
| --- | ---: | ---: | ---: | ---: | ---: |
| blocked | 474.282 mm | 24 | 0 | 3,035,410 | 522.282 mm |
| open | 477.701 mm | 18 | 7 | 5,017,180 | 513.701 mm |

Both profiles reproduce the one-via improvement on `SIG03_R_HEADER`. They also
choose a different `SIG04_R_HEADER` route: adding one via shortens that route
enough to improve the configured score. The two changes leave whole-board via
counts unchanged from historical cold routing while reducing track length.
All other selected route qualities and expansion counts match their respective
historical runs. Routing configurations match for every connection.

The open profile improves the score by 8.582 mm relative to blocked, with 65.3%
more expansions. Width and clearance differ throughout each route, so this
does not isolate pad-gap availability from other clearance effects. The
fixed-placement prefix omits later signals, VCC and GND.

Observed process times are 410.8 seconds blocked and 501.7 seconds open. The
runs overlap and include native verification/rendering, so these are not
controlled timing measurements. Both runs include the symmetric-grid-query
optimization as well as terminal-layer search.

The summary script now accepts named existing comparison directories through
`--case NAME=DIRECTORY`, allowing separate runs to be paired without copying
their artifacts. It checks paired executable hashes and placement hashes.
Reprocessing the four historical smoke cases preserves every existing metric
and passes the pairing checks; Python compilation passes.

[Paired validation and per-connection changes](../../build/esp32-pad-gap-layer-search-comparison-2026-09-07/validation.json),
[full summary](../../build/esp32-pad-gap-layer-search-comparison-2026-09-07/pad-gap-summary.json),
[blocked rendering](../../build/esp32-pad-gap-prefix19-blocked-layer-search-2026-09-07/final-preview.png),
and [open rendering](../../build/esp32-pad-gap-prefix19-open-layer-search-2026-09-07/final-preview.png).
