# Deferred layout optimizations

Priority reset, 2026-09-08: the user requested that we preserve these findings
and concentrate on becoming a general tool for creating complete PCB layouts.
Do not use this list as the default next-work queue. Revisit an item when it
blocks whole-board completion, or after the general placement/routing pipeline
works across varied board families.

The adaptive router now enforces this priority operationally: it stops once
native connectivity is complete. Further whole-board quality passes require
`optimize_after_routing_complete: true`. Remaining native findings still
prevent full-layout admission; routing completion does not waive them.

- **Single-via removal through plated terminals.** The improved PIC board has
  ten vias outside the scanner's pair-excursion patterns. Import now covers all
  nine via-bearing nets. Proposals need to use actual pad copper contact areas,
  preserve shared connectivity, and avoid replacing a removed via with an
  unnecessary endpoint via. Evidence:
  [plated pad import](reviews/2026-09-08-plated-pad-import.md).
- **Remaining ECC83 U1A-K pair.** The direct proposal conflicts with GND,
  Net-(P3-P1) and a target hole. Revisit multi-net yielding and terminal contact
  handling only as a bounded whole-board optimization; the complete three-via
  board remains the incumbent.
- **Prevent congestion during insertion.** Future-net demand and learned net
  order already improve complete cold routing. Later explore capacity-aware
  demand, congestion history, selective rip-up and reuse of successful route
  prefixes. Use a completion failure on a broader board family to prioritize
  this work, not millimetre-scale improvements on a solved board.
- **Refinement search efficiency.** Deduplicate direct candidates generated
  at several grid resolutions and retain independently checked blocker groups.
  Score improvements against the original native board, since imported copper
  may be normalized inside pads.

Keep the existing ECC83/PIC inputs, configuration hashes, combined renders and
native checks as regression evidence. Do not discard working mechanisms while
changing priorities.

Before reopening a TODO, identify the blocked whole-board benchmark it is
expected to help, or establish that completion is already reliable on the
current corpus. Bound the experiment and state its success criterion first.
Saving a via on an already solved board alone is not the next milestone.
The active work queue is the
[whole-board execution priority](competitive-roadmap.md#current-execution-priority-2026-09-08).
