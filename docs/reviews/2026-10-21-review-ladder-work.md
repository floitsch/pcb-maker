# Review 2026-10-21: the ladder and its work accounting

Read-only review of 35456a1 (ladder fault isolation), 6825ce3 (wall guard
cuts), 8608a51 (all pour work charged) and bbdb205 (probe overhead) as they
stand together on main at 20956b3. Files read: board_router/ladder.rs,
route.rs, budget.rs, attempt.rs, lower.rs, result.rs, work_rate.rs;
pcb-router's router.rs, router/build.rs, prepare.rs, driver.rs,
negotiate.rs, vias.rs, fill.rs, profile.rs, wall.rs, grid.rs (static maps),
geometry.rs (PolygonIndex); src/main.rs; the four changelog sections. No
build, no run. "Sure" means the code says so; "suspect" means I reason
about the effect without having run it.

## 1. The static-map cache makes the charged work depend on how many rungs ran side by side (medium-high; mechanism sure, effect suspect)

`STATICS_CACHE` (crates/pcb-router/src/router/build.rs:687) keeps weak
entries plus one strong entry, `cache.1`, the maps built last
(build.rs:777). Who finds maps and who rebuilds them therefore depends on
build order. In `rung_wave` (crates/pcb-kicad/src/board_router/ladder.rs:1921-1937)
a wave of two or more rungs holds every rung's maps in rung order, so
`cache.1` ends as the wave's last rung (tracks); one rung alone holds
nothing and builds its own, so `cache.1` ends as the rung that ran last.
The wave size comes from `rungs_side_by_side` (ladder.rs:1876), which
follows MemAvailable. A later router of the same key (the seed retry
ladder.rs:2038, the seed rounds ladder.rs:2224, the steps ladder.rs:2452,
and the connect rung after a probed tracks leader) then either finds its
maps (12 ns a node, `BUILD_SHARED_MAP_NANOS`) or rebuilds them (22 ns a
node plus every test), and `charge_result` (ladder.rs:689) puts the
difference into `charged.lattice_seconds`, which feeds `search_share`
(ladder.rs:707), `bound_left`, `rung_left` and every later share. The
changelog's promise that the side-by-side count "by design changes no
result" held before this package because a rung the order never reached
counted for nothing; the cache breaks it by one lattice build per reuse
(0.3 to 1.5 s on the expansion tier, 6 s on ChengminLi's 16 classes,
against a 30 s budget). The same holds for `POUR_MASKS`
(prepare.rs:325, four keys FIFO, filled only by `hold_static_maps`): a
router finds the masks for free or counts them in `prepare_counts`.
A fix that keeps determinism: charge the *canonical* price (a found map
costs what a build would have, or the hold always charges the full build
and routers always count the shared rate), or key the charge on the rung,
not on the cache state.

## 2. The probes' hold is charged before the memory gate decides whether probes run, and its memory is counted nowhere (medium; sure)

`probes` builds and charges `held` for every rung (ladder.rs:1108-1118)
before the first router is built; the `too_small_for(megabytes, 2)` check
(ladder.rs:1163) then clears the rungs and breaks. On a machine where two
routers do not fit, up to three static-map builds are charged to the ladder
and serve nobody (only `cache.1` survives; the regular rungs rebuild and
charge again). The held maps are also resident at once however many probes
fit: before this package the peak was N probes of full maps; now it is K
held statics (connect, connect+stubs or exclusive, tracks: three keys for
four rungs) plus the running probes' own maps, and with one probe at a
time that is about two routers' worth where the gate assumed one.
`memory_report` (build.rs:416) counts `statics` in full for every router,
shared or not, so `routers_that_fit` is conservative per router but blind
to the held sets. Suggested order: build the first router, pass the memory
gate, then hold and charge.

## 3. Process-wide retention of maps (medium-low; sure)

`cache.1` keeps the most recently built static maps alive for the rest of
the process (build.rs:777); after a 0.05 mm rung that is roughly half of
that router (jiran-ble-lite: 7 GB a router) held through the seed rounds
and steps at the regular pitch. `POUR_MASKS` keeps four keys of
full-board `Vec<bool>` masks per connected net, never released. A layout's
hundreds of move routers churn `cache.1` (every move changes the key) and
hold one extra set throughout; each free-dimension child process holds its
own. The machine is memory bound; nothing in `MemoryLimits` names this.
Weak entries are pruned on every call (build.rs:753), so the list itself
does not grow.

## 4. The static-map key: checked, no omission found (info; sure as far as derive(Debug) goes)

`statics_key` (build.rs:694) hashes layer_count, outline, edge and hole
clearances, hole_to_hole, neck_width, classes (after the rules' split and
neck classes), the nets' classes, the grid, `searched`, `ClassRules`
(ffa49a5's net rules), `no_via_pads` (ded766d) and every obstacle's Debug
form. `StaticMaps::build_with_rules` (grid.rs:514) reads nothing else:
`copper_clearance` and `fill_clearance` are methods over classes, nets'
classes and the obstacle's own fields (board.rs:310, 326). `pour_masks_key`
adds every plane's net, layer, class, connection, outline and exclusions;
`excluded_clearance` reads classes only. Brittleness: the key relies on
`#[derive(Debug)]` printing every field of `Obstacle`, `Shape`,
`RuleClass`, `ClassRules`; a hand-written Debug or a new field read by the
build but not printed would alias silently. Two `DefaultHasher`s with
different seeds give a stable 128-bit key across processes. Config fields
do not enter the maps (the build takes no `Config`); `exclusive_planes`
and the skeleton act on the router, not the statics.

## 5. Double and multiplied lowerings (low; sure)

`probes` charges the lowering of every rung's board (ladder.rs:1075), then
breaks on small boards (`layers < 4 || connections < 120`) and the regular
rung lowers and charges again through `route_kicad_board_once`
(attempt.rs:93, 160). `rung_wave` lowers each rung for the hold and
charges it (ladder.rs:1933), and the rung's `route` lowers once more. The
work is really done twice, so the charge is honest, but the lowered boards
are thrown away. The seeds charge the kept seed's `lowering_work_seconds`
`retry` times (ladder.rs:2066, 2258) though `route_seeds` lowers once.
At 0.5 to 22 ms a lowering none of this moves a share.

## 6. `run_work` mixes units (low; sure)

`plan.left = router_work(...)` is search work (ladder.rs:1686), `run_work.0
= ladder_budget` is all work; vias.rs:73 compares `left > total / 2`, so
the via reduction's extra allowance triggers less often when the search
fraction is below 1. The continuation does the same (ladder.rs:1327).
Deterministic, just not what the comment says.

## 7. A negotiation of zero disables the work limit (low; sure)

`bounded_negotiation` returns 0 when nothing is left (budget.rs) and
`core_config` then sets `negotiation_expansions = 0`, which negotiate.rs:283
reads as "no limit" (`> 0` guard). The router still stops after its first
iteration because `deadline_work` is 0 (`past_work_deadline`) and the wall
guard fires at 0 s; the bound is kept, but only by the deadline. A floor
of one expansion, or treating 0 as 0, would make the intent explicit.

## 8. Panic isolation (low; sure)

`isolate` is sound: no profile sets `panic = "abort"`, every lock uses
`unwrap_or_else(|p| p.into_inner())` so a panic inside a build poisons
nothing, `route_seeds` re-raises a seed's panic (route.rs:507), scoped
threads join before unwinding. Two edges: main.rs:17 records the *first*
panic's message, so after an isolated attempt panics a later uncaught
panic is reported with the attempt's message, and `is_closed_stdout`
(main.rs:48) then tests the wrong text (exit 4 instead of the quiet exit).
And `judge_isolated` catches a panic in KiCad's check after
`gate.skipped`/`CHECKS_SKIPPED` moved but before `charge_checks`: the
check's wall is spent uncharged (layouts only).

## 9. The build race without a hold undercounts (low; suspect)

`shared_statics` builds outside the lock; a loser of the race is returned
as shared (build.rs:770) and counts 12 ns a node for a full build. Only
routers started without a hold race: board_layout.rs:1035's parallel
seeds. Layouts charge search only, so no effect today.

## 10. Still uncharged in route mode (info)

The preparation (`note_prepare`: escapes, skeletons, plane stubs, the
global plan; its searches count, the planner does not); `core::verify` in
`finish_routed_board` (route.rs:641, measured as `verify_seconds`); emit
and write; rungs that ran in a wave and were then skipped by the
skeleton/stubs rule (ladder.rs:1748) and probes beside a finishing leader
(by design); the manual spokes' per-pad masks (documented); the hold's
statics work is not added to `BUILD_WORK_NANOS`, so
`wall_accounting.lattice_work_seconds` undercounts against
`charged.lattice_seconds`.

## 11. Checked and found consistent

Shares cannot go negative (`rung_left`, `rung_share`,
`deadline_before_reserve`, `route_step_reserve` all clamp at 0);
`search_fraction` floors at 0.25 and returns 1 before any search; the
continuation's deadline (`probed + built_router_work(left)`) and its
charge (`expansions - probed`, `pour - probe.pour`, build 0) do not double
count the probe; seeds charge `statics / len x retry = statics`;
`deadline_seconds` stretches by `wall_stretch(false)` (route.rs:21) and
the router's `guard()` by the same value (work_rate.rs:289); pour, build
and lowering prices scale by `reference_time_scale` as search does, so a
charged second is a second at the applied rate on every kind. No HashMap
iteration reaches routing order in the reviewed code (lower.rs sorts its
outputs; the caches are Vecs). Nit: `PCB_LADDER_UNCHARGED_SPOKES` also
turns off `charges_all` and `converts_shares` (ladder.rs:506-515), more
than its name says.
