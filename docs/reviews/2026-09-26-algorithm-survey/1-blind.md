# Cross-field techniques for automatic KiCad PCB placement and routing: 35 ideas and a top 5

**How the references were checked.** I confirmed these by web search: sparrow/jagua-rs, Yan & Wong DAC'09, Kong/Yan/Wong ASP-DAC'10, GAMER, OrthoRoute, MAPF-LNS, PBS, RUDY, Ozdal & Wong ICCAD'03, Gort & Anderson TCAD'12, SURF/Dayan/Toporouter, Chen–Kajitani–Chan, Maximum Margin Planning, and DeepPCB.
- The other classic references I know well; details I am less sure of are marked **[unverified detail]**.
- EV = expected value, Eff = effort, each rated L/M/H.
- Problem numbers (#1–#7) refer to the "where things are hard" list in the brief.

---

## A. Routing search, negotiation, speed

**1. PathFinder negotiated congestion (FPGA CAD).** McMurchie & Ebeling, FPGA'95. Textbook baseline, listed for completeness.
- **Mapping:** grid or graph nodes carry a *present* cost (current overuse) and a *history* cost (accumulated overuse). Every net reroutes each iteration and overlap is allowed until the end.
- **Why:** it answers "which net yields" implicitly, through prices. If the tool currently does strict sequential routing with rip-up, this is the single biggest structural upgrade.
- **Risk:** oscillation on 2-layer boards with pours; it needs a pressure schedule.
- **Experiment:** allow overlap, and multiply the present-cost factor by 1.3–1.5 per iteration. Compare completion on the 85–90% boards.
- **EV H if not already used, else L. Eff M.**

**2. Conflict-Based Search (CBS) and Priority-Based Search (PBS) from multi-agent path finding (MAPF).** Sharon et al., AIJ 2015 (CBS); Ma, Harabor, Stuckey, Li, Koenig, AAAI 2019 (PBS).
- **Mapping:** agents are nets (or pad-pair connections). A conflict is two nets using the same cell or passage.
  - CBS branches on "net A may not use region R" vs "net B may not use region R".
  - PBS branches on "A has priority over B" (A is routed first and B routes around it).
- **Why:** it is a *systematic* answer to #7. The search tree records exactly which yield decision was tried. PBS's partial priority ordering is a direct replacement for fixed net ordering. Use it only in the endgame, on the 5–30 nets left in a hotspot.
- **Risk:** exponential blowup. It must be bounded to a local window and a small net set.
- **Experiment:** when the router stalls, collect the unrouted nets plus the nets whose copper blocks them (typically 10–20). Run PBS with the existing A* as the low-level planner, with a node budget of about 200.
- **EV H. Eff M.**

**3. Large Neighborhood Search (LNS) / ruin-and-recreate (MAPF and vehicle routing).** Li, Chen, Harabor, Stuckey, Koenig, *Anytime MAPF via LNS*, IJCAI 2021 (code: github.com/Jiaoyang-Li/MAPF-LNS; MAPF-LNS2 also exists). Schrimpf et al., *Ruin and Recreate*, J. Comp. Phys. 2000.
- **Mapping:** repeatedly (a) pick a neighborhood (a spatial window, a random net set, or the nets adjacent to failures), (b) rip up all of it, (c) re-insert it in a new random or greedy order, (d) keep the result if it is better.
- **Neighborhood choice:** use adaptive-LNS bandit weights to pick among these strategies.
- **Why:** an anytime improver for completion, via count and length. It runs well in parallel because disjoint windows can be worked on at once (#5).
- **Risk:** the choice of neighborhood matters a lot.
- **Experiment:** add an after-routing loop of "rip up a random 5×5 mm window and reroute it" on a finished board. Measure how via count and length change over 60 s.
- **EV H. Eff L–M.**

**4. Randomized restarts plus a parallel portfolio (SAT solving).** Gomes, Selman, Crato, Kautz, *Heavy-tailed phenomena in SAT/CSP*, JAR 2000. Luby, Sinclair, Zuckerman, *Optimal speedup of Las Vegas algorithms*, IPL 1993.
- **Mapping:** a run is (seed, net order, cost weights). Runs that stall at 85–90% are likely a heavy-tailed runtime effect.
- **Why:** it uses N cores with *zero* shared state, even though the core search parallelizes poorly (#5).
- **Risk:** it helps only if outcomes vary a lot between seeds.
- **Experiment:** run 16 seeds per benchmark board and plot the distribution of completion and time. If the variance is high, ship a portfolio with a Luby-schedule restart policy.
- **EV M–H. Eff L.**

**5. Coarse-to-fine corridors (VLSI global routing / multigrid).** FastRoute (Pan & Chu), NTHU-Route 2.0, BoxRouter.
- **Mapping:**
  1. Solve global routing on a coarse grid (e.g., 2 mm tiles, where each edge's capacity is the number of tracks that fit).
  2. Run detailed A* only inside each net's corridor (its tiles plus a 1-tile margin).
- **Why:** 5–50× smaller search spaces. Global congestion is visible before detailed routing starts. Corridors from different nets overlap less, which enables parallelism.
- **Risk:** capacity estimates are wrong around fine-pitch parts, so corridors need a fallback that widens them.
- **Experiment:** compute the tile route with plain Dijkstra over tiles and restrict A* to it. Measure the time on the 4-layer boards.
- **EV H. Eff M.**

**6. Incremental and better-informed search (robotics / game AI).** LPA* and D* Lite (Koenig & Likhachev, AAAI 2002); ALT landmarks (Goldberg & Harrelson, SODA 2005).
- **Mapping:**
  - *Rip-up and reroute:* only a few cells change between attempts, so LPA* reuses the previous search tree.
  - *ALT:* precompute exact distance fields from about 8 landmarks per layer, including via cost. This gives an admissible heuristic that "knows" about walls, unlike Manhattan distance.
  - Jump Point Search does not fit, because it needs uniform costs.
- **Why:** A* on the long nets of large boards wastes most of its expansions behind obstacles.
- **Risk:** the heuristic goes stale as congestion costs change. ALT stays admissible if the landmark distances are computed with base (minimum) costs.
- **Experiment:** log node expansions per net, add ALT, and compare.
- **EV M. Eff L–M.**

**7. Optimistic (speculative) parallelism plus GPU sweeps (parallel computing).** Kulkarni et al., *Galois*, PLDI 2007; Gort & Anderson, TCAD 2012 (geographic partitioning of nets, deterministic, 2.3× on 4 cores); GAMER (Lin, Liu, Wong, ICCAD'21 / TCAD'23: shortest paths from alternating horizontal and vertical sweeps, 16×); OrthoRoute (Benchoff, GPU PathFinder via CuPy for KiCad backplanes, open source).
- **Mapping:** route K nets concurrently against a snapshot. Commit a net if the cells it touched were not changed by an earlier commit; otherwise retry it. Order nets by bounding box so that conflicts are rare.
- **Why:** it attacks #5 directly without a full redesign.
- **Risk:** long nets overlap everything, so process those serially first and parallelize the short ones.
- **Experiment:** measure the conflict rate among nets whose bounding boxes do not overlap on a 4-layer board. If it is below about 20%, implement a thread pool with commit and validate.
- **EV M–H. Eff M.**

## B. Geometry, representation, human-likeness

**8. Topological (rubber-band) routing (MCM/PCB research, 1990s).** Dai, Dayan, Staepelaere, *Topological routing in SURF*, DAC 1991; Dayan, *Rubber-band based topological router*, PhD, UCSC 1997; gEDA **Toporouter** (A. Blake, GSoC 2008, based on Dayan).
- **Mapping:** use a constrained Delaunay triangulation over pad and obstacle vertices.
  - Each net's route is a sequence of triangle edges it crosses (its homotopy class).
  - Geometry (the actual copper shape) is produced last, by tightening the rubber bands.
- **Why:** capacity is checked on triangle edges *exactly* (see idea 12). The detailed geometry never shreds space early. Results look hand-drawn, which serves "looks human".
- **Risk:** big implementation effort, and converting the result to exact geometry with vias is hard.
- **Experiment:** triangulate one dense region and route nets as paths in the dual graph with edge capacities. Check the predicted feasibility against the real router.
- **EV M–H (long term). Eff H.**

**9. Voronoi / medial-axis adaptive grid (robot motion planning).** Ó'Dúnlaing & Yap, *Retraction method*, J. Algorithms 1985; the Generalized Voronoi Diagram as a roadmap.
- **Mapping:** place routing-grid lines through the midpoints between neighboring pad edges instead of on a uniform pitch. In a BGA these midlines are exactly the Voronoi edges between balls. Add Hanan-style lines (horizontal and vertical lines through every pad edge ± clearance).
- **Why:** #2 ("a track barely fits") usually fails because the uniform grid misses the one legal centerline. A non-uniform grid plus gridless validation, using polygon offsets with Clipper2 (Minkowski inflation by clearance + half-width), recovers it.
- **Risk:** the non-uniform grid complicates cost bookkeeping and neighbor enumeration.
- **Experiment:** for one 0.75 mm BGA, generate candidate centerlines as midlines between ball rows, add them to the grid, and count how many escapes become feasible.
- **EV H for #2. Eff M.**

**10. Topology-preserving compaction and smoothing (VLSI compaction + robotics trajectory optimization).** Constraint-graph compaction (Lengauer, *Combinatorial Algorithms for IC Layout*, 1990); elastic bands (Quinlan & Khatib, ICRA 1993); TrajOpt (Schulman et al., RSS 2013).
- **Mapping:** fix each track's topology (its order relative to obstacles). Variables are segment vertex positions; clearance constraints are linear given the fixed topology. Solve an LP or QP that minimizes length plus bends, or pushes copper away from a congested passage to *make room* for a blocked net.
- **Why:** gives human-looking results (fewer jogs, 45° cleanups, uniform spacing). The "make room" mode is a principled push-and-shove.
- **Risk:** linearizing 45° constraints; possible degenerate cases.
- **Experiment:** after routing, run a per-net pull-tight step (Douglas–Peucker, then a clearance check, then a vertex-sliding QP with scipy or OSQP). Measure length, bends and DRC.
- **EV M. Eff M.**

## C. Feasibility, capacity, "who yields"

**11. Cut-condition capacity certificates (combinatorial optimization).** Okamura & Seymour, *Multicommodity flows in planar graphs*, JCTB 1981. Capacity checking on triangle edges is also used in topological routers.
- **Mapping:**
  - For every segment between two obstacles (triangle edges of a constrained Delaunay triangulation, per layer), capacity = ⌊(gap − clearance) / (width + clearance)⌋.
  - Demand = the number of nets whose terminals lie on opposite sides of a *closed* cut. Chains of segments from board edge to board edge, or around a component, form closed cuts; compute them via shortest paths in the dual graph.
  - If demand exceeds capacity summed over layers minus the ones consumed by vias, that is a *proof of infeasibility* for that region.
- **Why:** it separates "infeasible" from "gave up" (#7). It names the saturated passage and the nets competing for it. It is also a cheap routability metric for placement (#4).
- **Risk:** the cut condition is necessary but not sufficient, and vias complicate cut accounting across layers.
- **Experiment:** compute the edge-to-edge dual shortest cut between each pair of pads for a stalled net and report capacity vs. demand. Validate on known-failed and known-routable boards.
- **EV H. Eff M.**

**12. LP multicommodity-flow relaxation with dual prices (operations research).** Raghavan & Thompson, *Randomized rounding*, Combinatorica 1987; Garg & Könemann, FOCS 1998 (fast approximate multicommodity flow).
- **Mapping:** commodities are nets (a 2-pin decomposition or Steiner candidates); edges are global-routing tile boundaries with capacity.
  - LP infeasibility gives a Farkas certificate (a weighted set of saturated edges), i.e., a proof.
  - The LP duals are congestion prices to seed PathFinder history costs.
- **Why:** a global view instead of greedy net-by-net routing. Gives a lower bound on overflow and a starting guess for "who yields".
- **Risk:** Steiner nets make the LP large. Candidate-path LPs (column generation over a few paths per net) keep it small.
- **Experiment:** generate 5 candidate tile paths per net, solve the path LP with HiGHS or OR-Tools, and compare predicted overflow hotspots with the router's failures.
- **EV M. Eff M.**

**13. SAT/MaxSAT on local windows with unsat cores (FPGA SAT routing, formal methods).** Nam, Sakallah, Rutenbar, SAT-based FPGA detailed routing, TCAD 2002 **[unverified exact title]**; PySAT (RC2 MaxSAT, minimal-unsatisfiable-subset extraction).
- **Mapping:** in a window (e.g., around the BGA, 10×10 cells × 2–4 layers), Boolean variables mean "net n uses cell c on layer l" or "via at cell c". Constraints are clearance exclusion, per-net connectivity (flow encoding), and fixed boundary crossing points.
- **Outputs:**
  - UNSAT gives a proof for that window with fixed boundary.
  - A minimal unsatisfiable subset gives the minimal set of mutually incompatible nets (#7).
  - Weighted MaxSAT picks which nets yield.
- **Why:** exact where heuristics fail: BGA escape and last-10-net hotspots.
- **Risk:** encoding blowup, and boundary conditions make it only "locally infeasible".
- **Experiment:** encode a 6×6-ball BGA corner escape on 2 layers and solve with CaDiCaL via PySAT. Check timing and whether it finds escapes the router misses.
- **EV M–H. Eff M.**

**14. Auction / market mechanisms and the blocking graph (economics, distributed optimization).** Bertsekas, *Auction algorithm*, 1988.
- **Mapping:** narrow passages are goods and nets are bidders. A net's value for a passage is the detour cost if denied (computed by A* with the passage blocked). Price goes up with contention, and the net with the highest marginal detour wins.
- **Blocking-graph variant:** a directed edge A→B means "A's copper blocks B". Rip up a minimum-weight feedback vertex set or vertex cover.
- **Why:** decides "who yields" by *opportunity cost* instead of net order or length heuristics.
- **Risk:** repeated A* calls for detour costs; approximate them with the coarse grid.
- **Experiment:** on stall, compute detour costs for each contested passage and give it to the lowest-regret net. Compare with the current rip-up policy.
- **EV M. Eff L–M.**

**15. Rent's rule / wiring supply vs. demand triage (VLSI wirelength estimation).** Landman & Russo, 1971; Donath, 1979.
- **Mapping:** estimated total wirelength (Steiner lengths via FLUTE, Chu & Wong, TCAD 2008) × average pitch vs. usable area × layers × utilization factor, computed per region.
- **Why:** a 1 ms sanity check: "this board needs 4 layers" or "this region is over 100% demand". It gives early infeasibility warnings to the user.
- **Risk:** crude.
- **Experiment:** compute it for the whole corpus and correlate with routing success.
- **EV L–M. Eff L.**

## D. Copper pours (problem #1)

**16. Digital-topology "simple point" test plus bridge detection as a cost term (image processing, thinning).** Kong & Rosenfeld, *Digital topology: introduction and survey*, CVGIP 1989; Tarjan's bridge-finding; Holm, de Lichtenberg, Thorup, dynamic connectivity, JACM 2001.
- **Mapping:**
  - Rasterize each pour (per layer) into cells about (track width + 2·clearance) in size.
  - A cell is *simple* if removing it does not change local connectivity (8-neighborhood crossing number).
  - The router's step cost on a pour-covered cell = base + α·[cell not simple] + β·[cell lies on a bridge separating pad-bearing pour components].
  - Pour components and bridges are maintained incrementally as tracks are committed.
- **Why:** the router becomes aware that it is *cutting* the pour, not just using space. Humans do this intuitively.
- **Risk:** the local test is conservative (it ignores global loops), so it should be combined with the global bridge check; recomputation cost.
- **Experiment:** add the local simple-point penalty only, on a 2-layer board with a 2-sided 3V3/GND pour. Measure islands after pour fill and completion.
- **EV H. Eff M.**

**17. Backbone reservation: route the pour net's skeleton first (utility-corridor planning; power-grid-first in VLSI).**
- **Mapping:**
  1. Before signal routing, route the pour net as a real "virtual" net: a Steiner tree over its pads, width = minimum pour neck (e.g., 0.8–1.5 mm), preferring the layer intended for the pour.
  2. Signals must respect it; they may cross it only on the other layer, which costs a via pair.
  3. After signal routing, turn the backbone into pour (delete it and let the zone fill around it).
- **Why:** connectivity of the pour is *guaranteed by construction* instead of hoped for. It is exactly what experienced designers do ("GND trunk") and addresses #1 directly.
- **Risk:** the backbone consumes space the signals need, so the width and layer preference need tuning.
- **Experiment:** on the 85–90% boards, reserve a 1 mm MST trunk for GND on the bottom layer and re-run.
- **EV H. Eff L.**

**18. Island healing as a group Steiner tree (VLSI Steiner generalizations, network design).** Reich & Widmayer, *Beyond Steiner's problem: a VLSI oriented generalization*, 1989 **[unverified detail]**; Garg, Konjevod, Ravi, group Steiner approximation, J. Algorithms 2000.
- **Mapping:**
  - After fill: each pour island on each layer is a node, and each orphan pad is a terminal.
  - Edges: stitching vias where top and bottom islands overlap (cost = via), short tracks between islands (cost = length), and tracks from pads to islands.
  - Pad-bearing islands are required terminals; empty islands are optional Steiner nodes.
  - Solve with the MST 2-approximation, or exact Dreyfus–Wagner for 12 terminals or fewer. Then remove unconnected dead islands.
- **Why:** turns the vague "pour broke" into a small, solvable post-pass. Cheaper than rerouting signals.
- **Risk:** edges must be DRC-feasible, which needs one A* per candidate edge (a bounded number).
- **Experiment:** fill with kicad-cli or pcbnew and compute islands. Build the island graph with via candidates on a 1 mm lattice inside overlaps, solve the MST, insert vias, run DRC.
- **EV H. Eff L–M.**

**19. Layer assignment as max-cut with pour-damage weights (classic PCB/VLSI via minimization).** Chen, Kajitani, Chan, IEEE TCAS 1983; Pinter, *Optimal layer assignment for interconnect*, ~1982–84 **[unverified venue]**; Hadlock, planar max-cut, SIAM J. Comput. 1975.
- **Mapping:**
  - Route topologically on a merged single layer first. Wire pieces between crossings are nodes, and a crossing forces the two pieces onto different layers.
  - The objective adds a penalty for a piece's length under the pour on the "pour layer".
  - This is a max-cut / bipartization problem, exactly solvable when planar.
- **Why:** optimizes via count and pour preservation *jointly*, instead of both falling out of A* cost weights.
- **Risk:** needs an intermediate "2.5D" routing representation.
- **Experiment:** re-assign layers of an existing routed 2-layer board (fixed x-y geometry) with an integer program in CP-SAT that minimizes vias plus pour-layer length. Measure pour islands before and after.
- **EV M. Eff M.**

## E. Fine-pitch escape (problem #2)

**20. Network-flow escape routing with simultaneous pin assignment (PCB CAD).** Yan & Wong, *A correct network flow model for escape routing*, DAC 2009 (models the diagonal capacity between balls correctly; optimal); Kong, Yan, Wong, *Optimal simultaneous pin assignment and escape routing for dense PCBs*, ASP-DAC 2010; Ordered Escape Routing with Differential Pair and Blockage, TODAES 2018.
- **Mapping:**
  - Nodes are ball sites and the gaps between them, with capacity = how many tracks fit between balls, including 45° diagonals.
  - The source is the signal balls; the sink is the BGA perimeter plus via sites on inner layers.
  - Max-flow gives the maximum number of escapable nets per layer; min-cost flow minimizes vias.
- **Why:** exact and polynomial for this sub-problem, and it tells you when escape is *impossible* on N layers.
- **Risk:** the flow ignores ordering at the perimeter, which the downstream router needs. The ordered-escape variants address that.
- **Experiment:** build the Yan–Wong graph for one BGA from pad geometry and design rules, solve with networkx max-flow, and compare with the router's escape success.
- **EV H. Eff M.**

**21. Fanout pattern library / macro-operators (game AI, planning).** Culberson & Schaeffer, pattern databases, 1998; Korf, macro-operators, 1985. Industry practice: dogbone and quadrant fanout.
- **Mapping:**
  - Per package class (BGA, QFN, QFP), use pre-validated fanout templates: via offset direction by quadrant, dogbone length, which rows escape on top vs. through vias.
  - Apply the templates as a deterministic first stage; the router then connects via to via.
  - A via-in-pad option can be enabled when the design rules allow it.
- **Why:** humans never search BGA escape from scratch. This removes the hardest search and looks right.
- **Risk:** templates break near fixed obstacles, so fall back to search.
- **Experiment:** hard-code quadrant dogbones for BGA and 0.5 mm QFN, check each via's DRC, and route the rest.
- **EV H. Eff L.**

## F. Missing features (problem #6)

**22. Differential pairs as one coupled "dumbbell" agent (robot formation planning).**
- **Mapping:** plan one centerline in configuration space inflated by (2w + gap)/2 + clearance. The pair is then two offset curves. At pads, add short uncoupled breakout stubs, obtained by solving pad to coupling point for each net. Via transitions use via pairs placed symmetrically.
- **Why:** reuses the existing single-net A* unchanged, with a different inflation radius and a turn penalty.
- **Risk:** corners change the lengths of the inner and outer tracks (needs skew compensation, see idea 23). The pad-breakout geometry is fiddly.
- **Experiment:** route one USB D+/D− pair on an otherwise empty board as a fat centerline, offset it with Clipper, and check DRC plus KiCad's pair-gap check.
- **EV M. Eff M.**

**23. Length matching: Lagrangian space allocation plus post-route meanders (PCB CAD).** Ozdal & Wong, *Length-matching routing for high-speed PCBs*, ICCAD 2003 (Lagrangian relaxation that reserves area near nets for length extension).
- **Mapping:** during routing, a net needing extra length L reserves free area ≈ L·pitch/2 next to its path. Afterwards, fill that area with trombone or accordion meanders (a space-filling curve inside a polygon).
- **Why:** meanders can be added only if space was reserved. Doing it post hoc alone fails on dense boards.
- **Risk:** complexity; low demand on hobby boards.
- **Experiment:** a post-route meander inserter with no reservation. Measure the achievable length increase vs. free space.
- **EV L–M. Eff M.**

**24. Pin/gate swapping as assignment plus crossing minimization (graph drawing, operations research).** Hungarian algorithm (Kuhn 1955); Sugiyama, Tagawa, Toda, 1981 (barycenter layer crossing minimization); Eades & Wormald, Algorithmica 1994; Kong/Yan/Wong 2010 (idea 20).
- **Mapping:**
  - For each swap group (equivalent pins of an MCU or FPGA GPIO, resistor-array elements, gate units), cost(net n → pin p) = distance + λ·(ratsnest crossings created).
  - Solve the min-cost assignment.
  - For a connector-to-IC bus: order pins by the angle of their targets. Inversions between the two orderings equal forced crossings, which is the classic river-routing condition.
- **Why:** crossings on 2 layers roughly equal vias. Swapping after placement and before routing is almost free.
- **Risk:** needs swap-group metadata, which KiCad symbols do not fully provide, so user annotation is required.
- **Experiment:** on a board with a connector feeding MCU GPIOs, let the user mark the swappable pins, run the Hungarian algorithm on crossing counts, and compare via counts.
- **EV M. Eff L.**

## G. Placement (problems #3 and #4)

**25. Irregular nesting engines (cutting and packing).** Gardeyn, Vanden Berghe, Wauters, *An open-source heuristic to reboot 2D nesting research*, arXiv 2509.13329 (**sparrow**) and *Decoupling geometry from optimization…*, arXiv 2508.08341 (**jagua-rs**, a Rust collision engine); Imamichi, Yagiura, Nagamochi, overlap minimization for irregular strip packing, Discrete Optimization 2009; Bennell & Oliveira, *Geometry of nesting problems*, EJOR 2008.
- **Mapping:**
  - Items are courtyard polygons, with allowed rotations.
  - The container is the board outline minus keepouts, with holes. Fixed parts are pre-placed holes.
  - Two sides are two containers: through-hole items occupy both, SMD items one.
  - Sparrow solves a sequence of *feasibility* problems in which weighted overlap penalties (guided local search) are driven to 0. That is exactly #3.
- **Why:** this community solves "put arbitrary polygons into an arbitrary polygon without overlap" far better than EDA force-directed legalizers, which assume rows and rectangles.
- **Risk:** it optimizes packing, not wirelength. Replace the strip-length objective with the wirelength-plus-routability objective while keeping the overlap-resolution engine. Rust integration is needed.
- **Experiment:** export one "no legal solution" board as a jagua-rs instance (container = outline, items = courtyards, fixed items as holes) and run sparrow's feasibility phase. Does it find a zero-overlap solution?
- **EV H for #3. Eff M.**

**26. CP-SAT NoOverlap2D / geost for exact placement feasibility (constraint programming).** Google OR-Tools CP-SAT (`AddNoOverlap2D`, optional intervals); Beldiceanu et al., *geost*, CP 2007 **[unverified detail]**.
- **Mapping:**
  - Discretize positions (e.g., 0.25 mm) and rotations (0/90/180/270). Each rotation is an optional interval pair; courtyards are approximated by rectangles or unions of rectangles.
  - Separate no-overlap constraints per side, with through-hole parts in both.
  - Board edge and cutouts become fixed boxes; soft objectives are HPWL (linearized) and decap proximity.
  - An INFEASIBLE result is a *proof* (at that discretization), and assumptions can identify the offending subset.
- **Why:** answers "is there any legal placement?" (#3, #7). Excellent for dense boards with 15–60 movable parts.
- **Risk:** scaling past about 150 parts; non-rectangular outlines.
- **Experiment:** encode one failing dense board and give CP-SAT 60 s. Either get a legal placement to seed the SA/force placer, or get a proof.
- **EV H. Eff L–M.**

**27. Electrostatic analytic placement with two-sided density (VLSI).** Lu et al., *ePlace*, TODAES 2015; Lin et al., *DREAMPlace*, DAC 2019 (PyTorch, GPU).
- **Mapping:**
  - Components are charges, and density is solved by Poisson via FFT.
  - Use two density maps (top, bottom); through-hole parts deposit charge in both.
  - Fixed parts and keepouts are fixed charges; the outline is a potential wall.
  - Wirelength uses a weighted-average smooth model. Rotation is handled by discrete flips afterwards.
  - Then legalize with idea 25 or 26.
- **Why:** globally smooth and fast, and it spreads parts into the available area much better than pairwise repulsion.
- **Risk:** it is overkill for 15 parts, and legalizing irregular courtyards afterwards is the real work.
- **Experiment:** a 200-line numpy prototype (FFT density plus WA wirelength, Nesterov steps) on a 100-part board, legalized by the existing code.
- **EV M. Eff M.**

**28. Parallel tempering / replica exchange (statistical physics).** Swendsen & Wang, PRL 1986; Earl & Deem, review, PCCP 2005.
- **Mapping:** run N simulated-annealing placement replicas at different temperatures and swap states between neighbors by the Metropolis criterion.
- **Why:** escapes the jammed configurations that stop dense 2-sided placement (#3). Uses all cores trivially.
- **Risk:** needs temperature-ladder tuning.
- **Experiment:** wrap the existing SA in 8 replicas with geometric temperatures and compare the fraction of legal final placements.
- **EV M. Eff L.**

**29. Routability proxies beyond HPWL: RUDY, ratsnest crossing count, cell inflation (VLSI routability-driven placement, graph drawing).** Spindler & Johannes, *RUDY*, DATE 2007; ISPD 2011 routability-driven placement contest; Xie et al., *RouteNet*, ICCAD 2018 (learned).
- **Mapping:**
  - *RUDY map:* each net spreads wire density uniformly over its bounding box.
  - *Crossings:* count crossings between ratsnest or Steiner segments, since on 2 layers crossings roughly equal forced vias.
  - *Inflation:* after a failed route, enlarge the effective courtyard of parts near congestion and re-place.
  - The cut-capacity check (idea 11) is a stronger proxy.
- **Why:** #4, the shortest-HPWL placement often not routing.
- **Risk:** the proxies may not correlate with routing success. Measure that first.
- **Experiment:** over the corpus, correlate HPWL, peak RUDY, crossing count and cut overflow with routed completion. Pick the best-correlated combination as the placement objective.
- **EV H. Eff L.**

**30. Logic-based Benders decomposition: router failures become placement cuts (operations research).** Hooker & Ottosson, *Logic-based Benders decomposition*, Math. Programming 2003.
- **Mapping:**
  - The master problem is placement; the subproblem is routing.
  - When routing fails, extract a "no-good" from the failure (e.g., the saturated cut from idea 11 between parts A and B) and add a master constraint: gap(A,B) ≥ g + δ, or "not both on the same side".
  - Iterate.
- **Why:** turns an expensive router run into reusable knowledge instead of one bad score.
- **Risk:** cuts that are too weak mean many iterations.
- **Experiment:** after a failed route, find the unroutable region, add a 0.5 mm spacing constraint to the parts bordering it, re-place, re-route, and count iterations to success.
- **EV M–H. Eff M.**

**31. Multi-fidelity racing: the router as expensive judge (AutoML).** Li et al., *Hyperband*, JMLR 2018; successive halving.
- **Mapping:** generate 32 placements from diverse seeds.
  1. Screen all with proxies (idea 29) in ms.
  2. Route the top 16 with the coarse global router (idea 5).
  3. Route the top 4 with the detailed router, time-limited.
  4. Route the best 1 to completion.
- **Why:** #4, spending router time where it discriminates. Parallel across cores.
- **Risk:** early fidelities may be misleading.
- **Experiment:** on 5 boards, fully route 16 placements each (ground truth) and check whether proxy or 20%-time routing rank-correlates with the final result.
- **EV H. Eff L.**

## H. Meta, quality, verification

**32. Automated algorithm configuration and per-instance selection (AutoML / metaheuristics).** Hutter, Hoos, Leyton-Brown, *SMAC*, LION 2011; López-Ibáñez et al., *irace*, ORP 2016.
- **Mapping:** parameters are via cost, history increment, bend penalty, pour penalties α/β, net-order policy and grid pitch. The objective is corpus completion plus a runtime penalty. Optionally, select a configuration per board from features (layers, pad density, pour coverage).
- **Why:** routers are extremely sensitive to hand-set weights, and the corpus already exists.
- **Risk:** overfitting to the corpus, so hold out boards.
- **Experiment:** run irace for an overnight budget on 20 boards and validate on 10.
- **EV M–H. Eff L.**

**33. Inverse optimization / imitation of human routes (robotics imitation learning).** Ratliff, Bagnell, Zinkevich, *Maximum Margin Planning*, ICML 2006; Ratliff, *Learning to Search*, PhD 2009 / Autonomous Robots 2009.
- **Mapping:**
  - Take features per routing step: layer, direction, near-pad, under-pour, bend, via, distance to board edge, parallel-to-neighbor, and so on.
  - Learn cost weights w such that the human's route on real open-source KiCad boards (GitHub has thousands) is cheaper than the router's best route under w, by a margin.
  - This is a subgradient loop that calls the existing A*.
- **Why:** gives a measurable definition of "looks human". Also learns the via and layer habits that preserve pours.
- **Risk:** human boards were routed around different placements; use each board's own placement and ratsnest.
- **Experiment:** 20 boards, route each net alone with the human's other copper as obstacles, learn about 10 weights by MMP, and measure the overlap of the learned routes with the human routes.
- **EV M. Eff M.**

**34. Metamorphic and differential testing (software testing).** Chen, Cheung, Yiu, 1998; Segura et al., survey, IEEE TSE 2016.
- **Mapping:**
  - *Metamorphic:* mirror, rotate 90°, translate, renumber nets or shuffle input order. Completion and DRC-cleanliness should be invariant, with outcome distributions matching.
  - *Differential:* same board through Freerouting (open source, Specctra DSN/SES).
  - *Oracle:* `kicad-cli pcb drc` (KiCad 8+) on every output.
- **Why:** catches off-by-one grid bugs, rotation bugs and asymmetric clearance handling. These often masquerade as "hard boards".
- **Risk:** none significant.
- **Experiment:** run each corpus board in 4 rotations × 2 mirrors and flag any board whose completion varies by more than 5%.
- **EV M. Eff L.**

**35. Learned heuristics / reinforcement learning (for completeness, low priority).** Mirhoseini et al., Nature 2021 (graph placement, contested); InstaDeep **DeepPCB** (commercial reinforcement-learning router); Quilter (commercial).
- **Mapping:** a policy chooses the next net and region, or a CNN predicts per-cell congestion to bias A*.
- **Why:** they could eventually learn good yield orders.
- **Risk:** the data and compute cost are enormous and results are hard to reproduce. Ideas 32 and 33 capture most of the value cheaply.
- **Experiment:** train a CNN congestion predictor from self-generated router logs only if ideas 29 and 31 show that the proxies are weak.
- **EV L. Eff H.**

---

## Top 5

1. **Pour trio: backbone reservation (17), island healing as group Steiner (18), simple-point/bridge cost (16).** Problem #1 is the largest observed failure mode. Idea 17 is a one-day change that makes pour connectivity *by construction*. Idea 18 repairs what remains with a small exact graph problem. Idea 16 makes the router stop shredding the pour. None needs a router rewrite.

2. **Cut-capacity certificates on a constrained Delaunay triangulation (11).** One piece of geometry serves three problems:
   - (a) proving a region infeasible vs. "gave up" (#7);
   - (b) naming the saturated passage and competing nets, as input to "who yields";
   - (c) a routability score and Benders cuts for placement (#4, idea 30).
   It is also the first step toward topological routing (8).

3. **MAPF endgame with parallel restarts: PBS/CBS (2) on stalled hotspots, ruin-and-recreate LNS (3), a Luby-restart portfolio (4).** These directly target the 85–90% stall and the "which net yields" question with systematic, bounded search. They use many cores without the fine-grained parallelism that currently fails (#5).

4. **Nesting engine plus CP-SAT for placement legality (25 + 26).** The cutting-and-packing community has open-source, state-of-the-art overlap resolution for arbitrary polygons in containers with holes (sparrow, jagua-rs). CP-SAT gives a legal placement or an infeasibility proof for dense boards in under a minute. Together they address #3 ("a human found one; we found nothing").

5. **Multi-fidelity racing with measured proxies (31 + 29).** First measure which cheap proxy actually predicts routing success on the corpus (HPWL, RUDY, crossings, cut overflow). Then spend router time with successive halving over diverse placements. This is cheap, embarrassingly parallel, and makes the router the judge (#4) without paying full price for every candidate.

**Honorable mentions:**
- Voronoi/Hanan adaptive grid (9) and fanout templates (21): probably the fastest wins for BGA escape (#2).
- irace tuning (32): the best effort-to-value ratio overall once the pour and endgame changes are in.

---

## Sources
- sparrow: https://arxiv.org/abs/2509.13329 and https://github.com/JeroenGar/sparrow
- jagua-rs: https://arxiv.org/pdf/2508.08341
- Yan & Wong, DAC 2009: https://dl.acm.org/doi/10.1145/1629911.1630001
- Kong, Yan, Wong, simultaneous escape routing and layer assignment (related work): https://dl.acm.org/doi/10.1109/TCAD.2005.857376
- Ordered escape routing with differential pairs, TODAES 2018: https://dl.acm.org/doi/10.1145/3185783
- GAMER: https://dl.acm.org/doi/10.1109/ICCAD51958.2021.9643563
- OrthoRoute: https://github.com/bbenchoff/OrthoRoute
- MAPF-LNS: https://www.ijcai.org/proceedings/2021/0568.pdf and https://github.com/Jiaoyang-Li/MAPF-LNS
- PBS: https://ojs.aaai.org/index.php/AAAI/article/view/4758
- RUDY: https://ieeexplore.ieee.org/document/4211973/
- Ozdal & Wong, ICCAD 2003: https://ieeexplore.ieee.org/document/1257808/
- Gort & Anderson, TCAD 2012: https://ieeexplore.ieee.org/abstract/document/6106729/
- SURF: https://ieeexplore.ieee.org/document/979685
- Toporouter: https://github.com/bert/pcb/wiki/Autorouters:-gEDA-pcb-Toporouter
- Chen, Kajitani, Chan: https://ieeexplore.ieee.org/document/1085357/
- Maximum Margin Planning: https://www.ri.cmu.edu/pub_files/pub4/ratliff_nathan_2006_1/ratliff_nathan_2006_1.pdf
- DeepPCB: https://instadeep.com/2024/09/instadeep-introduces-deeppcb-pro-an-ai-powered-pcb-design-tool/
