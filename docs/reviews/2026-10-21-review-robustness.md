# Review 2026-10-21: robustness of the readers against hostile or odd input

Read-only review of main at 20956b3. Scope: the S-expression reader, the net,
footprint, netlist, project-rule and config readers, the lowering's geometry,
the free-dimension outline edit and the agent CLI's paths and subprocesses.
Nothing was built or run; "sure" means the failure follows from the code as
read, "suspect" that the exact failure mode or its cost was not confirmed.

Context that bounds severity: `main.rs:38` wraps every command in
`catch_unwind`, so a panic ends as exit 4 with a verdict. A stack overflow or
an OOM kill does not: no verdict, the harness sees a signal. Panics are
therefore "medium" at most; aborts and file writes outside the output
directory are the serious class.

## 1. Writes and reads outside the output directory

**1.1 Footprint ids from a `--footprints-from` board become output paths.**
`netlist_import.rs:1421-1447` (`write_local_libraries`): a footprint the
libraries lack is written to `output.join("{nickname}.pretty")` and
`folder.join("{name}.kicad_mod")`, nickname and name split from the board's
footprint id (`theirs.id`, `board_footprints.rs:82-87`). Neither is checked
for path separators. Trigger: a downloaded board with
`(footprint "../../x:../../y" (path "/1111") ...)` and a schematic symbol
linked to it; the import writes `<out>/../../x.pretty/../../y.kicad_mod`
(attacker-chosen name, S-expression content) and names the library in the
output's `fp-lib-table`. Medium, sure. Fix: refuse ids whose parts contain
`/`, `\` or `..`, or sanitise them before `join`.

**1.2 Footprint names from the netlist read arbitrary `.kicad_mod` files.**
`netlist_import.rs:302` (`find_footprint`): `library.join("{name}.kicad_mod")`
with `name` from the netlist's `(footprint "lib:../../../tmp/evil")`. Any
readable file with that suffix is parsed and embedded in the board. Low (read
only, must parse as S-expression), sure.

**1.3 `copy_tree` follows symlinks and can copy a project into itself.**
`netlist_import.rs:458-476` recurses on `from.is_dir()` (follows symlinks)
with no depth limit; `:650-668` allows `${KIPRJMOD}/.` (`CurDir` passes the
component check). With the output directory inside the project (the usual
`import ... <project>/out`), an fp-lib-table `(uri "${KIPRJMOD}/.")` or a
symlink `lib -> .` copies the project into `out/`, which is then copied again
inside itself, until ENAMETOOLONG ends it with an error after hundreds of
nested copies. Medium-low, suspect on the exact depth reached; the missing
checks are sure. `fs_util::copy_directory_tree` (:57) handles this case
correctly and refuses non-files; `copy_tree` should do the same.

**1.4 `board_id` is a path fragment.** `agent_cli.rs:225`, `:400`, `:710`,
`netlist_import.rs:562`, `:1177`, `footprint_links.rs:46`: every command joins
`format!("{board_id}.kicad_pcb")` unchecked, so `pcb-maker route-kicad-board
proj ../../x out` reads and writes beside the directories given. The agent
controls the argument, so low, sure; a one-line check (no separators) would
close it.

Positive: no subprocess goes through a shell. kicad-cli, `python3 -c <script>
argv...` (`silkscreen.rs:168`, `fab_export.rs:107`) and the free-dimension
probe (`src/agent_cli/free_dimension.rs:293`) all pass paths as separate
arguments; the silkscreen worker reads the board through pcbnew's own parser.
`footprint_links.rs:79-141` canonicalises every sheet path and checks
containment. kicad-cli's `-o <path> <board>` would misread a path starting with
`-`, but every path is joined onto a directory first.

## 2. Panics reachable from a file (exit 4 with a verdict)

**2.1 A footprint with fewer than two items.** `board_footprints.rs:291`
(`repose(&mut items[2..])`), `:292` (`items[1] =`), `posed` `:342`, and
`netlist_import.rs:1011-1012`, `:1672-1673`. `(footprint)` or `(module)` in a
`--footprints-from` board, or a `.kicad_mod` of `(footprint)` in a project's
`.pretty`, reaches `library_form`, `posed` (via `body_size`, `:886`) or the
`items[1]` writes: index out of bounds. Sure, low-medium (one bad library file
in a cloned repository fails the whole import).

**2.2 A footprint text box without corners on a mask layer.**
`board_router/lower.rs:583` takes `text_box_shapes` (`geometry.rs:593` returns
an empty Vec for fewer than three corners) and `:595-603` pushes it into
`mask_openings` without the emptiness check the board-level `gr_text_box` has
at `:1099`; `:1414` then indexes `shapes[0]`. Trigger:
`(fp_text_box "x" (pts (xy 0 0)) (layer "F.Mask"))` inside any footprint.
Sure, medium (every route and layout of that board panics).

**2.3 NaN coordinates in a rule area with a hole.** `copper_geometry.rs:337`
`da.partial_cmp(&dc).unwrap()`. Rust's `f64::from_str` accepts `NaN`, `inf`
and `infinity`, and no reader rejects them (only `local_clearance.rs:21`
checks finiteness). `(zone (keepout (tracks not_allowed)) (polygon (pts (xy
NaN 0) (xy 1 0) (xy 0 1))) (polygon (pts ...)))` panics. Sure, low.

**2.4 A `Footprint` property without a value, before a full one.**
`footprint_links.rs:164-174`: `schematic_property` skips a `(property
"Footprint")` of two items and finds the next, the `find` at `:166` takes the
first, `property_items[2] =` panics. Earlier schematic files of the hierarchy
have already been rewritten on disk. Sure, low.

**2.5 A net form with more than three fields.** `kicad_nets.rs:40` stores
`children().last()` while `:36` read index 2: `(pad ... (net 1 "GND" x))`
canonicalises the copper's nets to `x` (silent misread), and with a second pad
`(net 1 "GND")` the `previous.atom().unwrap()` at `:45` panics when the stored
item is a list. Sure, low.

**2.6 Exactly 32 copper layers.** `board_router/lower.rs:67` allows 32 and
`:85` computes `(1u32 << self.len()) - 1`: with overflow checks a panic, in
release the shift wraps and `all()` is 0, so every through-hole pad, via and
cutout lands on no layer. `project_rules/dru.rs:1614` guards `all_layers` but
`:1656` and `:1782` shift by the raw index. `check_capacity` permits 32 as
well. Sure (`1u32 << 32` is defined to wrap in release), low: KiCad 9 writes
up to 32 copper layers, so a real board can reach it.

**2.7 Non-finite outline coordinates size the lattice.** `grid.rs:389-390`
`((max - origin) / pitch).ceil() as usize + 1`: `inf` saturates to
`usize::MAX`, the `+ 1` panics with overflow checks and wraps to 0 in release,
after which `grid.nx - 1` (`:554`) underflows. One `(gr_line (start inf 0)
...)` on Edge.Cuts suffices; `f64::min/max` in `Grid::choose` drop NaN but keep
inf. Sure that nothing rejects it; suspect on the release-mode outcome. A
finiteness check on every coordinate the lowering reads would close 2.3 and
2.7 together.

## 3. Resource exhaustion and aborts

**3.1 Unbounded recursion in the S-expression reader.** `kicad_sexpr.rs:173-209`
(`expression` and `list` call each other), `encode_into` `:296`,
`collect_named_nets` `kicad_nets.rs:8`, `expression_count` `lower.rs:251`,
`renew_uuids` `netlist_import.rs:481`, `map_zone_points`, `mirror`, and the
implicit `Drop` of nested `Vec<Expr>`. A board file of a few hundred thousand
`(` overflows the stack: SIGSEGV, no verdict, not caught by `catch_unwind`.
Same for the `.kicad_dru` condition parser `dru.rs:371-487` (`((((` or
`!!!!` in one condition string, and `Ex`'s `Drop`); the file-level `sexps`
(`dru.rs:71`) is iterative and fine. Sure, medium: a depth limit (KiCad's own
files never exceed a few dozen levels) or parsing on a thread with a large
stack is cheap.

**3.2 The lattice is allocated before anything bounds it.**
`router/build.rs:105-125` chooses the grid from the outline's bounding box and
`grid.rs:534` allocates `vec![false; cells]` and the per-layer, per-class maps
(`:446`, `:482`). `check_capacity` (35456a1) bounds nets and layers only; the
ladder's memory gate (`ladder.rs:1133-1162`) reads `maps_mb` from a router it
has already built. One stray `(gr_line (start 0 0) (end 10000 0) (layer
"Edge.Cuts"))` makes a 10 m board: at a 0.1 mm pitch 10^10 cells per map,
OOM kill, no verdict. The chaining in `outline.rs` accepts it as long as the
loop closes. Sure, medium-high: this is the cheapest way for an odd board to
take the machine down. Fix: estimate `nx * ny * layers * classes * bytes`
from `Grid::choose` and refuse as an input error before `build_statics`;
bound the outline's extent (KiCad's own limit is about 4 m).

**3.3 Quadratic outline chaining.** `outline.rs:437-481` runs a linear
`find` over all open pieces per chained piece, and `:405-428` scans every
anchor per end: O(n^2) in Edge.Cuts pieces. Outlines traced from tens of
thousands of segments exist (the comment at `:589` names panels of
thousands); at 100k pieces that is 10^10 comparisons. `:662-668` (joining
pieces by closest vertices) and `copper_geometry.rs:327-345` (joining a rule
area's holes) are O(n*m) in vertices. Suspect: minutes, not a hang, but
charged to no budget.

**3.4 Custom-rule lowering is polynomial in the groups.** `dru.rs:1565-1581`
evaluates every pair of groups for every layer, kind pair and later rule, and
`:1761` searches `pair_clearances` linearly per push. Groups equal the nets
when a condition compares `A.NetClass == B.NetClass` and the project assigns
a class per net (KiCad 9's `netclass_assignments`): a 2000-net board with such
a rule evaluates ~10^8 conditions, each cloning strings
(`Value::NetClass(net.classes.clone())`). Suspect, low.

Not a finding: regex-lite compiles net-class patterns into a PikeVM with a
10 MB NFA limit (`regex-lite-0.1.9/src/nfa.rs:20`) and matches in linear
time, so no pattern is exponential; `wildcard_match` (`project_rules.rs:58`)
is O(n*m). The 65 533-nets check (`lower.rs:322`) runs after the lowering's
own allocations, but those are linear in the file; the maps that need the
bound are built after `Router::try_new`'s check (`build.rs:80`).

## 4. Silent misreads

**4.1 Escapes are never undone.** `kicad_sexpr.rs:273` `unquote` strips the
quotes and keeps `\"` and `\\`. Net names then meet unescaped text from the
project JSON (`netclass_assignments`, `netclass_patterns`,
`project_rules.rs:345-361`) and from `.kicad_dru` strings, which `sexps`
*does* unescape (`dru.rs:108-112`). A net named `A"B` or `A\B` gets the
Default class and matches no rule, without a warning. Sure, low.

**4.2 A pattern is also a regex, unanchored by a stray parenthesis.**
`project_rules.rs:95` wraps the text in `^(?:...)$`; the pattern `x)|(?:.*`
compiles to match every net, so one class takes the board. KiCad would report
the invalid expression. Sure, low.

**4.3 Items on layers the stack lacks vanish.** `lower.rs:1024-1027` skips a
`segment` whose layer is not in the table; `mask_of` `:101-105` drops unknown
`.Cu` names (intended for footprints made for more layers, but it also covers
typos and `In3.Cu` on a two-layer board), so a via or zone there gets mask 0
and is ignored. KiCad warns when loading such a board; pcb-maker routes
through the copper. Sure, low; a one-line warning in the report would do.

**4.4 `(at x y unlocked)` and locked flags.** Handled (`kicad_sexpr.rs:94`,
`board_footprints.rs:226`, `free_dimension.rs:323-327`); `hide` as atom and
as form (`geometry.rs:312-320`). No finding.

## Suggested order

1. 3.2 (lattice size gate) and 3.1 (parser depth limit): the two ways a board
   ends the process without a verdict.
2. 1.1 and 1.3 (file writes outside `<out>`), then 1.4 and 1.2.
3. 2.2 and 2.7/2.3 (a finiteness check on read coordinates), then 2.1, 2.6.
4. The low misreads (4.1-4.3) as warnings in `board-router.json`.
