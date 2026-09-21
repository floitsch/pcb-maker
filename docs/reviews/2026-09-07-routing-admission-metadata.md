# Consistent routing admission for library metadata

The open ESP32 19-connection benchmark exposed a false competition tie.
pcb-maker timed out with six committed connections. Freerouting connected
the entire target and passed base native verification, but source/result
admission reported four introduced DRC design findings. Inspection of those
four raw records shows that every one is a `lib_footprint_mismatch` warning.
The unchanged base verifier already reports that exact type/severity pair
separately as library metadata.

The comparison and sequential router had duplicated DRC filtering without this
distinction. Both now use `pcb_kicad::drc_design_issues`, backed by the same
classification as absolute verification. Raw findings remain in `drc.json`;
source and result verification summaries retain metadata-warning counts.
Changed metadata descriptions or affected items do not reject a route. Mismatch
errors, other design warnings, and unknown warning types still count as design
findings. ERC, connectivity, and schematic parity checks are unchanged.

This fixes a disagreement between existing admission policies. It does not
claim that library mismatches are irrelevant to every hardware review. They
remain available for separate inspection.

The regression tests cover changed metadata, duplicate design findings,
mismatch errors, other and unknown design warnings, and preservation of the
absolute metadata count. The existing geometry-fingerprint test still rejects
a changed design finding. All 99 library tests across pcb-kicad and
pcb-benchmark pass.

Original evidence:
[`competitive-comparison.json`](../../build/esp32-pad-gap-prefix19-open-physical-2026-09-07/competitive-comparison.json)
and [`competitive-result.json`](../../build/esp32-pad-gap-prefix19-open-physical-2026-09-07/competitive-result.json).
Historical verdicts are retained as produced; the fix does not rewrite them.

The [fresh native reference run](../../build/esp32-pad-gap-prefix19-open-admission-fixed-2026-09-07/competitive-result.json)
completes all 19 connections and passes route admission: zero ERC, DRC design,
parity or connectivity findings, with all 21 metadata warnings still counted.
Its source PCB matches the original after replacing only UUID fields. On this
new result, the old raw-finding comparison still counts four introduced issues;
all four are metadata warnings. This distinguishes the policy fix from a lucky
reroute that happened to eliminate the disputed records.

[Validation evidence](../../build/esp32-pad-gap-prefix19-open-admission-fixed-2026-09-07/validation.json)
and [automatic result rendering](../../build/esp32-pad-gap-prefix19-open-admission-fixed-2026-09-07/renders/result-front.png)
are retained. This run validates reference admission; it does not establish an
improvement in pcb-maker's 6/19 timeout result or a general performance claim.
