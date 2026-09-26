# Search, randomization, inference and learning for a KiCad placer and router: 23 ideas and a top 5

**Reference status.** Entries marked ✓ were checked by web search in this session. Unmarked classics (PathFinder, Luby, SMAC, CMA-ES and so on) are well established. Entries marked [?] have details (venue or year) I did not verify.

**New finding.** PCBWorld (LG AI Research, arXiv 2607.05915, July 2026 ✓, code at github.com/LGAI-Research/PCBWorld) is a Gymnasium environment built on KiCad's push-and-shove router. It ships 679 real open-source boards in `.kicad_pcb` format ("D3"), plus synthetic board generators and baselines: FreeRouting, OrthoRoute, KRT, and PPO/GRPO agents. This largely removes "we have only 18 boards" as a blocker, for both test data and training data. I could not fetch the paper itself (arxiv.org is blocked here), so the board counts come from the search summary.

**Build this first; most ideas depend on it.** A parallel benchmark harness:
- It runs N boards × M seeds × K configurations on all cores.
- It scores results lexicographically: DRC-clean completion, then unrouted count, then vias, then length, then time.
- It records anytime traces: overflow or unrouted count per PathFinder iteration.
- It supports leave-one-board-out and held-out splits (18 own boards plus PCBWorld D3).

Without this, every stochastic or learned method below is tuned blind to noise.

---

## A. Portfolios, restarts, parallelism

### 1. Randomized PathFinder with restarts and a parallel racing portfolio
- **References:**
  - Gomes, Selman, Kautz, "Boosting combinatorial search through randomization", AAAI-98.
  - Gomes et al., "Heavy-tailed phenomena in SAT and CSP", JAR 2000.
  - Luby, Sinclair, Zuckerman, "Optimal speedup of Las Vegas algorithms", IPL 1993.
  - ManySAT (Hamadi, Jabbour, Sais, JSAT 2009).
  - Papandreou & Yuille, "Perturb-and-MAP", ICCV 2011: noise-injected optimizers as samplers.
  - Domhan et al., IJCAI 2015: learning-curve extrapolation for early stopping.
  - Li et al., Hyperband, JMLR 2018.
- **Mapping:**
  - A run is (ladder rung, seed).
  - Randomization sources: tie-breaking in A*; net-order jitter (sort key plus Gumbel noise); multiplicative log-normal noise on history costs (perturb-and-MAP style); randomized Steiner tree growth order; randomized corridor choice among near-equal tile paths.
  - Restarts are partial, like CDCL keeping learned clauses: keep the history costs (the learned congestion), reshuffle the order and noise, and clear present occupancy.
  - Luby-scheduled budgets, counted in PathFinder iterations.
- **Racing.** Run all rungs × seeds concurrently. Every few iterations, apply successive halving on a predictor of final completion. Features: current overflow, its slope over the last k iterations, number of unrouted nets.
- **Replaces:** the sequential "one attempt per rung, keep best" loop.
- **Why it should beat the current pipeline:**
  - Stalls at 85–90% look like the classic heavy-tailed signature: a deterministic run lands in a bad basin and rip-up cycles there.
  - Best-of-k over diverse runs, plus early kills, typically cuts both time-to-quality and failure rate.
  - It is embarrassingly parallel, which fixes pain point 5 at no algorithmic cost.
  - Determinism is kept by seeding: report the seed.
- **Risks:**
  - Memory: each run holds a full grid.
  - Diversity may be low if the noise is too small; if it is too big, quality drops.
  - The racing predictor can kill late bloomers. PathFinder often converges abruptly, so use a conservative cutoff (keep the top 1/η).
- **Cheap first experiment:** 32 seeds per board with jitter on order and history only. Plot the distribution of final unrouted count and of time-to-best. If the coefficient of variation is large or the tail is long, the portfolio pays off immediately. Also compute the expected best-of-k curve.
- **Value H / Effort L.**

### 2. Algorithm configuration and per-board selection (irace / SMAC / SATzilla)
- **References:**
  - Hutter, Hoos, Leyton-Brown, SMAC (LION 2011); ParamILS (JAIR 2009).
  - López-Ibáñez et al., irace (ORP 2016).
  - Xu et al., SATzilla (JAIR 2008).
  - Falkner et al., BOHB (ICML 2018).
- **Mapping:**
  - Configuration space: via cost and its schedule, present/history multipliers (p_fac growth, h_fac), lattice pitch, corridor width, rip-up thresholds, net-order key weights, pour strategy, and placer knobs (target density, annealing schedule).
  - Instances: boards. Cost: the lexicographic score converted to a scalar, or rank-based.
  - Per-board selection: instance features (layer count, pad density, net count, BGA presence, pour fraction, RUDY peak, LP λ* from idea 10) → a classifier over a portfolio of 4–8 tuned configurations.
- **Complements:** idea 1, since the portfolio members come from here.
- **Why:** hand-tuned constants in negotiated-congestion routers are notoriously brittle, and tuning gains on the order of 10–30% are common in SAT/MIP.
- **Main risk:** overfitting to 18 boards. Mitigations:
  - Tune on PCBWorld D3 plus synthetic boards.
  - Validate on your 18 with leave-one-out.
  - Use irace's statistical racing (Friedman test) to avoid chasing noise.
  - Cost: thousands of router runs. At 10–60 s each on 32 cores, that is overnight.
- **Cheap first experiment:** irace over 6–8 router parameters on the 2-layer subset with a 60 s cap per run, then compare on held-out boards.
- **Value H / Effort L–M.**

### 3. Parallelizing PathFinder itself (deterministic batch, Jacobi snapshot, speculative)
- **References:**
  - Moctar & Brisk, "Parallel FPGA routing based on the operator formulation", DAC 2014 ✓ (Galois, speculative).
  - Gort & Anderson, "Deterministic multi-core parallel routing for FPGAs", FPT 2010 [?].
  - Shen & Luo, "Coarse-grained parallel routing with recursive partitioning for FPGAs", TPDS 2020 ✓.
  - GPU global routers such as FastGR / GAMER [?] route nets concurrently against snapshot costs.
- **Mapping, three levels:**
  - (a) *Batch-disjoint:* per iteration, colour the nets so that nets in the same colour have non-overlapping expanded bounding boxes or corridors, and route each colour in parallel. This is deterministic.
  - (b) *Jacobi PathFinder:* all ripped nets route against a frozen snapshot of prices, then occupancy is committed. Convergence is slightly slower per iteration but far faster in wall time. This is standard on GPU.
  - (c) *Speculative:* optimistic concurrency. Commit a route if the cells it read have not changed since; otherwise re-route. Nondeterministic unless ordered commits are used.
- **Replaces:** the single-threaded inner loop. This is key for 4-layer runs of 10–20 minutes.
- **Risk:** Jacobi can oscillate, because all nets flee the same hotspot at once. Damp it by routing only a random fraction of congested nets per iteration; that fraction is itself a randomized-rounding knob.
- **Cheap first experiment:** implement (a) with rayon on a 4-layer board and measure speedup and quality versus serial. Then try (b) with a 50% rip fraction.
- **Value H / Effort M.**

---

## B. Rip-up and repair as stochastic local search

### 4. Adaptive Large Neighbourhood Search (ALNS) with bandit-selected destroy/repair operators
- **References:**
  - Shaw, LNS (CP 1998).
  - Ropke & Pisinger, ALNS (Transportation Science 2006).
  - Fialho et al., bandit-based adaptive operator selection (2010) [?].
  - Auer et al., UCB1 (2002); Thompson sampling.
- **Mapping:**
  - State: a complete (possibly illegal) routing.
  - Destroy operators:
    - rip a spatial window around the worst overflow;
    - rip a conflict-graph neighbourhood of a failed net;
    - rip nets crossing a saturated tile cut;
    - rip via-heavy nets;
    - rip the nets that fragment a pour, found via articulation points of the pour-connectivity graph;
    - rip a whole net class;
    - rip a random subset (the noise operator).
  - Repair operators: PathFinder with different orders, a finer lattice in the window, alternate layer bias, or pin-swap-enabled repair (see idea 18).
  - Operator weights are learned online with sliding-window UCB or Thompson sampling. Reward is the improvement in the lexicographic score per CPU-second.
  - Acceptance: record-to-record travel or simulated annealing, so that moves which worsen the score can be accepted.
- **Replaces or complements:** the rip-up/reinsert "when stalled" step.
- **Why:**
  - A stall at 85–90% is a local optimum for a single neighbourhood; ALNS routinely escapes such optima in vehicle routing and scheduling.
  - The bandit learns per board which neighbourhood matters, for example the pour-fragmenting operator on dense 2-layer boards.
  - Windows are independent, so different windows can be repaired in parallel on different cores (a parallel variant of LNS).
- **Risk:** operator engineering effort. Bandit rewards are non-stationary; use discounting.
- **Cheap first experiment:** three operators (window, conflict-neighbourhood, random), a UCB selector and simulated-annealing acceptance. Run it after PathFinder stalls on the 2-layer pour boards.
- **Value H / Effort M.**

### 5. WalkSAT-style noise and tabu in rip-up selection
- **References:**
  - Selman, Kautz, Cohen, "Noise strategies for improving local search" (AAAI 1994).
  - McAllester, Selman, Kautz, "Evidence for invariants in local search", AAAI 1997: Novelty heuristics.
  - Glover, tabu search (1989/1990).
- **Mapping:**
  - The "unsatisfied clauses" are overflowing resources or unrouted connections.
  - Pick one at random. With probability p, rip a random net occupying it; otherwise rip the net with the lowest "break count", meaning the net whose reroute least increases other overflow (estimated from PathFinder prices).
  - Tabu: forbid re-entry of (net, tile region) for a tenure T. Use an aspiration criterion when the move would give a new best.
- **Why:** PathFinder cycling, where the same 3–5 nets swap a channel forever, is exactly the pathology noise and tabu address. It is cheap and orthogonal to everything else.
- **Risk:** p and T need tuning (hand them to idea 2). Too much noise degrades length and via count, so polish with plain PathFinder afterwards.
- **Cheap first experiment:** log the rip-up sequence on a stalled board and measure cycle length. Add tabu on (net, corridor) and compare completion.
- **Value M–H / Effort L.**

### 6. Conflict-driven learning: nogoods, explanations, and exact local solving
- **References:**
  - Marques-Silva & Sakallah, GRASP/CDCL (1999).
  - Nam, Sakallah, Rutenbar, SAT-based FPGA detailed routing (TCAD 2002).
  - Bayless, Hoos, Hu, "Scalable, high-quality, SAT-based multi-layer escape routing", ICCAD 2016 ✓ (MonoSAT: SAT with graph and max-flow predicates).
- **Mapping:**
  - When net n fails, compute the minimal set of foreign nets blocking it: run a min-cut in the net's corridor with foreign occupancy as capacity consumers.
  - Record a nogood: "n and {m1..mk} cannot all use cut C in this configuration". This becomes a persistent, targeted history price on (C, set) and a rip-up candidate list.
  - For small windows that stay stuck (BGA escape, a connector fan-out region), extract a local instance and solve it exactly with a SAT/SMT or flow encoding. The result is either a solution or an UNSAT proof, which is a genuine infeasibility certificate for that window with fixed boundary conditions.
- **Addresses:**
  - pain 2, via exact multi-layer escape, which is exactly the Bayless et al. setting;
  - pain 7, via explanations and certificates;
  - "which net yields": the net whose removal breaks the most nogoods, or which has the cheapest alternative.
- **Risk:**
  - The encoding size for fine lattices; use coarse escape grids.
  - UNSAT is only relative to the fixed window boundary.
  - There is no Rust MonoSAT; options are an FFI binding, a CaDiCaL/Kissat plus flow encoding, or CP-SAT.
- **Cheap first experiment:** implement the min-cut blocking-set extraction only, and use it to choose rip-up sets in place of the heuristic (A/B test). Separately, prototype BGA escape as a pure max-flow problem (single layer per ring) to test feasibility quickly.
- **Value H / Effort M (blocking sets) to H (SAT escape).**

### 7. MCTS, Nested Monte Carlo Search and NRPA over net orders and rip-up decisions
- **References:**
  - Kocsis & Szepesvári, UCT (ECML 2006).
  - Cazenave, NMCS (IJCAI 2009).
  - Rosin, NRPA (IJCAI 2011).
  - He & Bao, "Circuit routing using MCTS and DNN", arXiv 2006.13607 ✓.
  - Danihelka et al., Gumbel MuZero (ICLR 2022): policy improvement with very few simulations.
- **Mapping:**
  - State: the set of nets routed so far plus occupancy (coarse tile graph). Action: the next net to route, or at a bottleneck, which net to rip.
  - Rollout: the default ordering heuristic plus noise, routed on the coarse tile graph with capacities (milliseconds), not the fine lattice.
  - Value: the fraction routed and overflow after rollout.
  - NRPA suits permutations especially well. It learns a softmax policy over "net i follows context c" with nested adaptation, and parallelizes at the top level.
- **Complements:** it replaces the heuristic order only at the coarse level. The output order and corridor assignment feed the fine PathFinder.
- **Why:** PathFinder is nominally order-insensitive, but in practice the initial order and multi-pin tree growth matter on dense boards. MCTS makes this search explicit and anytime.
- **Risk:**
  - Evaluation cost if the rollouts are fine-grained. Must be coarse.
  - The correlation between coarse and fine results can be weak. Measure it (see idea 14).
  - The branching factor is 400 nets; restrict actions to the top-k by a prior.
- **Cheap first experiment:** NRPA level 2–3 over the order of the ~30 most-congested nets only, with coarse rollouts, on one stalled 2-layer board. Feed the best order to the full router.
- **Value M–H / Effort M.**

### 8. Cross-entropy method / Plackett-Luce distribution over orders and net weights
- **References:**
  - Rubinstein, "The cross-entropy method for combinatorial and continuous optimization" (MCAP 1999).
  - de Boer et al., CE tutorial (Annals of OR 2005).
  - Plackett-Luce models for permutations.
- **Mapping:**
  - Sample orders from a Plackett-Luce model with per-net scores s_i, or sample per-net cost multipliers from Gaussians.
  - Route each sample (coarse, or full with early kill), keep the elite ρ-quantile, and refit the scores and Gaussians.
- **Why:** it is trivially parallel (one batch per generation equals one core each), robust, has few hyperparameters, and yields a distribution rather than a single order, which is useful for the portfolio.
- **Risk:** sample-hungry, with 50–200 evaluations per generation. Only viable with coarse or cheap evaluations or on small subsets of nets.
- **Cheap first experiment:** a CE-tuned per-net history-weight multiplier on the 20 nets with the highest overflow, 32 samples per generation, 10 generations.
- **Value M / Effort L.**

### 9. Sequential Monte Carlo and beam search over partial routings
- **References:**
  - Del Moral, Doucet, Jasra, "SMC samplers" (JRSSB 2006).
  - Beam search in planning. Twisted SMC with learned value functions (for example Lawson et al. 2018) [?].
- **Mapping:**
  - A particle is a partial routing (first k nets of an order, or first k multi-pin connections).
  - Extend each particle by routing the next net with stochastic A* (perturb-and-MAP noise).
  - Weight: exp(−β·(overflow + ĥ(remaining))), where ĥ is a cheap remaining-demand estimate: RUDY over the unrouted nets' bounding boxes within the remaining capacity.
  - Resample and keep K particles.
- **Why:** it hedges against early commitments that doom later nets, which is common with BGA escape plus bus routing. K particles map naturally onto K cores.
- **Risk:** memory (use copy-on-write occupancy deltas); weight degeneracy. It is less natural in a negotiated framework, where nets are re-routed anyway. Probably best for initial routing and escape, not for the whole negotiation.
- **Cheap first experiment:** beam search with K = 8 over the escape order of a BGA's nets on a fine grid, compared against the deterministic order.
- **Value M / Effort M.**

---

## C. Relaxation, inference and message passing

### 10. Fractional multicommodity flow (multiplicative-weights resource sharing) with randomized rounding for corridors, bounds and dual prices
- **References:**
  - Raghavan & Thompson, "Randomized rounding", Combinatorica 1987. This was originally a global routing paper.
  - Albrecht, "Global routing by new approximation algorithms for multicommodity flow", TCAD 2001.
  - Garg & Könemann (FOCS 1998 / SICOMP 2007).
  - Müller, Radke, Vygen, "Faster min-max resource sharing in theory and practice", Math. Prog. Comp. 2011 ✓ (BonnRoute; includes randomized rounding).
  - Arora, Hazan, Kale, MWU survey, ToC 2012.
- **Mapping:**
  - Tile graph with per-edge, per-layer capacity derived from net-class width and clearance.
  - Commodities are nets, with Steiner-tree oracles per net.
  - The MWU iteration is in effect a principled PathFinder: its prices play the role of history costs, and it converges to a near-optimal fractional solution with max-congestion λ*.
  - Outputs:
    - (a) λ* > 1 (with some margin) means the board is likely infeasible at this placement, which is a lower-bound certificate (pain 7);
    - (b) the saturated cuts are the named bottleneck;
    - (c) duals give each net's marginal cost; "which net should yield" is the net with the cheapest detour at dual prices;
    - (d) multiple independent roundings give diverse corridor seeds for the portfolio (idea 1).
- **Replaces:** the heuristic coarse corridor planner.
- **Why:** principled global view, anytime bounds, diversity through rounding. MWU oracle calls are independent per net and parallelize.
- **Risk:**
  - The tile-capacity model is approximate: pads, keepouts, via blockage and 45° geometry. The bound is heuristic unless capacities are conservative.
  - Pours complicate capacity; model a pour as a commodity or as reserved capacity.
- **Cheap first experiment:** implement MWU over the existing tile graph (reusing the A* oracle). Compute λ* on all 18 boards and check whether it separates the boards that complete from those that stall. Use its duals to order nets.
- **Value H / Effort M.**

### 11. Min-sum belief propagation for competing paths on the coarse graph
- **References:**
  - Yeung & Saad, "Competition for shortest paths on sparse graphs", PRL 108, 208701 (2012) ✓.
  - Altarelli, Braunstein, Dall'Asta, De Bacco, Franz, "The edge-disjoint path problem on random graphs by message-passing", PLoS ONE 2015 ✓ (code: github.com/cdebacco/MP_EDP).
  - Bayati et al., "Statistical mechanics of Steiner trees", PRL 2008.
  - Braunstein, Mézard, Zecchina, survey propagation (RSA 2005).
- **Mapping:**
  - Variables: the flow of each net on each tile edge. Factors: node flow conservation and nonlinear congestion cost.
  - BP marginals give per-net "flexibility" (entropy over corridors) and per-edge contention.
  - Route low-entropy nets first, and yield high-entropy nets at bottlenecks. Survey-propagation-style decimation fixes the most biased nets first.
- **Why:** unlike PathFinder, which sees one path per net, BP reasons over path distributions and couplings. It yields calibrated contention maps, useful for placement feedback too.
- **Risk:**
  - Loopy convergence on grid-like graphs (use damping).
  - The published results are for random sparse graphs, not planar grids.
  - Per-net message cost is significant. Probably dominated by idea 10, which gives similar information more robustly.
- **Cheap first experiment:** run the MP_EDP code on an exported coarse tile graph of one board and compare its "number of accommodated nets" with PathFinder's coarse result.
- **Value M / Effort M–H (research-grade).**

### 12. Global layer assignment and via minimization as an MRF or max-cut, for 2-layer boards
- **References:**
  - Hadlock, "Finding a maximum cut of a planar graph in polynomial time", SIAM J. Comput. 1975.
  - Chen, Kajitani, Chan, graph-theoretic via minimization for two-layer PCBs, IEEE TCAS 1983 [?].
  - Pinter, "Optimal layer assignment for interconnect" (1984) [?].
  - Kolmogorov & Rother, QPBO, PAMI 2007.
  - Gibbs sampling / parallel tempering for Ising-like models.
- **Mapping:**
  - First route topology in a "2.5-D" space: A* ignores layer but pays a crossing cost and a pour-coverage cost.
  - Then assign each wire segment a binary label (top or bottom). Crossing segments must differ (antiferromagnetic coupling). A label change along a net costs a via.
  - Unary costs keep each pour's layer clear in its critical necks.
  - Frustrated cycles force vias; the optimal via placement on planar crossing graphs reduces to max-cut, which is polynomial for planar graphs.
  - For non-planar residues, use QPBO plus ICM, or annealed Gibbs sampling with replica exchange.
  - Pour connectivity is non-local. Handle it with a Lagrangian loop: flood-fill the pour, find islands, raise the unary costs on segments cutting the isthmus, and repeat.
- **Replaces:** the per-net greedy layer choice inside A* and the "rising via cost renegotiation" step, but only on 2-layer boards.
- **Why:** on dense 2-layer boards with pours on both layers, the failure mode is usually bad *layer* decisions made sequentially. This makes them globally and optimally for a given topology. It directly targets pain point 1 and via count.
- **Risk:**
  - Geometric realizability after assignment: via sites need room.
  - The topology stage must be DRC-aware enough.
  - This is a pipeline change, not a drop-in.
- **Cheap first experiment:** take the current router's final (stalled) 2-layer result, re-solve layer labels for the routed segments with QPBO or simulated annealing holding topology fixed, and measure the drop in vias and the pour-island count. If vias fall by 20% or more, the idea is validated.
- **Value H (for 2-layer) / Effort M.**

### 13. Infeasibility triage by combining certificates with runtime statistics
- **References:**
  - Hoos & Stützle, runtime-distribution analysis (SLS book, 2004).
  - Survival analysis (Kaplan-Meier, Cox models).
  - Au & Beck, subset simulation (2001) and multilevel splitting for rare-event estimation.
- **Mapping, per board and placement, combine:**
  - (i) cut-density lower bounds: for each tile cut, the nets whose pins lie on both sides versus the track capacity (cheap and exact in the 2-layer channel sense);
  - (ii) LP λ* from idea 10;
  - (iii) local UNSAT from idea 6;
  - (iv) the empirical runtime distribution across randomized runs. If the hazard of completion collapses after t, P(complete | more time) is about 0.
- **Output:** "proved infeasible (window W)", "likely infeasible (λ* = 1.12, cut C, nets S)", or "gave up, P(success within 2× budget) ≈ 0.3". This decides whether to spend more router time, go back to placement, or add a layer.
- **Why:** it answers pain point 7 directly and gates the placer–router coupling loop, saving the expensive evaluations that cannot succeed.
- **Risk:** certificates are relative to the abstraction; calibrate on known outcomes.
- **Cheap first experiment:** compute (i) on all boards and runs, and correlate it with the final unrouted count.
- **Value M–H / Effort L–M.**

---

## D. Placement search under an expensive judge

### 14. Multi-fidelity surrogate-assisted placement (a routability predictor plus Bayesian optimization / Hyperband)
- **References:**
  - Spindler & Johannes, RUDY (DATE 2007).
  - Xie et al., RouteNet (ICCAD 2018).
  - Liu et al., "Global placement with deep learning-enabled explicit routability optimization", DATE 2021 ✓.
  - Chai et al., CircuitNet (arXiv 2208.01040 ✓).
  - Kandasamy et al., multi-fidelity BO (BOCA, 2017).
  - Falkner et al., BOHB (ICML 2018).
  - Oh et al., "Bayesian Optimization for Macro Placement", arXiv 2207.08398 ✓.
- **Fidelity ladder (cost per evaluation):**
  - F0: HPWL and overlap (µs).
  - F1: RUDY and pin-density maps, rats-nest crossing count, and cut-density (idea 13) (ms).
  - F2: MWU λ* on the tile graph (idea 10) (100 ms).
  - F3: coarse PathFinder on tiles (1 s).
  - F4: full route (10–600 s).
- **Model:** gradient-boosted trees (LightGBM-style) on the F0–F3 features predicting F4 outcomes (completion, vias), with a quantile or ensemble head for uncertainty. Later, a CNN or GNN on congestion images.
- **Acceptance:** candidate placements (from ideas 15–17) are screened at F1–F2. Only the top few, chosen by expected improvement or upper confidence bound, go to F3–F4.
- **Replaces:** blind "nudge and reroute" trials costing 1–20 s each.
- **Why:** most placement candidates are clearly bad at F1 or F2. The measured rank correlation between fidelities tells you how much F4 budget you can save.
- **Risk:**
  - Distribution shift between boards and between placers.
  - Only 18 boards. Mitigate with self-generated data (idea 19): thousands of (placement, route-outcome) pairs from perturbed PCBWorld and GitHub boards.
- **Cheap first experiment:** for 18 boards × 50 random perturbations of the placer output, run F1–F4. Report Spearman ρ between λ* and RUDY-peak versus F4 unrouted count. If ρ is above about 0.7, gate the coupling loop on λ* immediately.
- **Value H / Effort M.**

### 15. CMA-ES / evolution strategies over low-dimensional placement parameterizations
- **References:**
  - Hansen & Ostermeier, CMA-ES (Evolutionary Computation 2001); Hansen, IPOP/BIPOP restarts.
  - Shi et al., WireMask-BBO (NeurIPS 2023 ✓): black-box optimizers over macro positions with greedy wire-mask decoding.
  - Cell inflation in routability-driven placement (for example NTUplace4, RePlAce routability mode).
- **Mapping:** do not run CMA-ES on 750-dimensional raw coordinates. Use a genome of about 10–60 dimensions:
  - ePlace knobs: target density, density weight schedule, net-weight exponents;
  - per-cluster (functional group) offset, rotation and side;
  - per-region inflation factors;
  - net weights for the top-k critical nets.
  - Decoder: ePlace, then legalization. Fitness: F2 or F3 surrogate, then F4 for the elite.
- **Why:** it captures the non-local moves the gradient placer cannot make (swap sides for a whole group, rotate a BGA by 90°). It is parallel by design: λ offspring on λ cores.
- **Risk:** the decoder is noisy or nondeterministic; fix seeds. Cost is 100s of decodes, acceptable only with surrogate fitness.
- **Cheap first experiment:** CMA-ES over 8 ePlace and legalizer knobs with λ = 16, fitness F2 λ*, on the dense 2-layer boards.
- **Value M–H / Effort L–M.**

### 16. Quality-diversity archive of placements (MAP-Elites / SAIL)
- **References:**
  - Mouret & Clune, "Illuminating search spaces by mapping elites" (arXiv 2015).
  - Vassiliades et al., CVT-MAP-Elites (2018).
  - Gaier, Asteroth, Mouret, SAIL: surrogate-assisted illumination (Evolutionary Computation 26(3) 2018 ✓).
- **Mapping:**
  - Behaviour descriptors, for example:
    - fraction of component area on the bottom side;
    - congestion centroid or spread (RUDY second moment);
    - BGA or connector orientation class;
    - maximum cut density;
    - cluster compactness.
  - Fitness: surrogate routability.
  - Mutations: group moves, side flips, swaps, rotations, re-seeded ePlace.
  - SAIL-style: illuminate cheaply on the surrogate and evaluate only the elites of each niche with the real router.
- **Replaces:** the single-trajectory coupling loop. When the router fails on the current placement, fall back to a structurally *different* elite rather than nudging.
- **Why:** pain point 4 (wirelength-optimal is not routable) means the objective is deceptive. Keeping diverse elites is the standard remedy, and the archive gives the portfolio natural parallel diversity.
- **Risk:** choosing descriptors; archive size times router cost. Keep 20–50 niches.
- **Cheap first experiment:** a 2-D archive (bottom-side fraction × RUDY spread) filled by mutation plus ePlace re-seeds with F2 fitness. Route the top 8 niches in parallel and compare with the current single placement.
- **Value M–H / Effort M.**

### 17. Parallel tempering / population annealing for legalization and two-sided side assignment
- **References:**
  - Swendsen & Wang, replica Monte Carlo (PRL 1986); Earl & Deem, parallel tempering review (PCCP 2005).
  - Hukushima & Iba, population annealing (AIP Conf. Proc. 2003); Machta (PRE 2010).
  - Witte, Chamberlain, Franklin, "Parallel simulated annealing using speculative computation", IEEE TPDS 1991 ✓.
- **Mapping:**
  - State: positions, rotations and sides.
  - Energy: overlap + courtyard or keepout violation + λ·HPWL + μ·surrogate congestion.
  - Replicas at different temperatures and different overlap-penalty weights μ, exchanged by Metropolis swaps.
  - Population annealing resamples replicas by Boltzmann weight at each temperature step.
  - Moves include side flips of a component and its satellites (decaps), and swaps of same-footprint parts.
- **Addresses:** pain 3 (no legal placement found). Single-chain simulated annealing gets trapped in near-legal packings. Replica exchange and population diversity are the standard fix for glassy feasibility landscapes.
- **Fallback:** for pure feasibility, a constraint-programming no-overlap packing model (CP-SAT) proves infeasibility or finds a packing, with SA seeds as hints.
- **Risk:** tuning the temperature ladder (adaptive schemes exist).
- **Cheap first experiment:** 16 replicas on the failing two-sided board. Measure P(legal) versus wall time against 16 independent SA runs.
- **Value M–H / Effort L–M.**

### 18. Joint discrete moves: pin swap, gate swap and side choice as a search layer
- **References:**
  - Classical pin/gate swapping in PCB CAD; Hungarian assignment.
  - Ozdal & Wong, PCB length-matching and bus routing (ICCAD 2003–2004) [?].
- **Mapping:**
  - Within swap groups (FPGA I/O banks, resistor-array pins, connector pins marked swappable, identical gates), choose a permutation minimizing rats-nest crossings plus the LP dual cost.
  - Solve by min-cost assignment on crossing-aware costs, then 2-opt or simulated annealing. Expose it as an ALNS repair operator (idea 4) and a placer move (idea 17).
  - Differential pairs: model a pair as a single "fat" commodity in A*, with coupled state, and in MWU.
  - Length matching: post-route meander insertion (deterministic).
- **Why:** swapping can remove crossings that no router can resolve on 2 layers. It is cheap and high-leverage where swap groups exist.
- **Risk:** it needs netlist metadata (swap groups) and back-annotation to the schematic.
- **Cheap first experiment:** on boards with FPGA or connector buses, compute the rats-nest crossing reduction from Hungarian pin assignment.
- **Value M / Effort M.**

---

## E. Learning

### 19. A self-generated training corpus from open KiCad boards
- **Sources:**
  - PCBWorld D3: 679 real `.kicad_pcb` boards ✓.
  - GitHub scraping: `.kicad_pcb` for KiCad 5–9, with a licence filter.
  - Synthetic generators (PCBWorld D1/D2).
- **Labels you get for free from human routes:**
  - per-net layer usage, via count, detour ratio (routed length / Steiner length), net-class statistics;
  - "trunk" versus "leaf" nets; BGA and QFN fanout patterns per footprint (dogbone direction, via-in-pad, ring-by-ring layer usage);
  - placement priors: decap-to-pin distances, connectors at edges, side usage.
- **Labels you generate yourself:** strip the routing, run your router on the human placement and on K perturbations, and record completion, vias, time and runtime traces. This is self-play-style data for the surrogates (ideas 14 and 20) and for configuration (idea 2).
- **What you cannot get:** routing order and design intent. Human routes also include manual compromises.
- **Risks:**
  - Parsing across KiCad versions.
  - Licence heterogeneity: training on GPL boards is likely fine, but redistribution is another matter.
  - Hobbyist quality variance: filter on DRC-clean, fully routed boards.
  - Bias toward simple 2-layer boards.
- **Cheap first experiment:** parse D3 and compute per-net-class detour and via statistics as priors (for example an expected via budget per net) inside A* costs. Measure the effect on the 18 boards.
- **Value H (enabler) / Effort M.**

### 20. Expert iteration: learn net ordering and "which net yields" from search
- **References:**
  - Anthony, Tian, Barber, "Thinking fast and slow with deep learning and tree search", NeurIPS 2017 (expert iteration).
  - Ross, Gordon, Bagnell, DAgger (AISTATS 2011).
  - He, Daumé, Eisner, "Learning to search in branch and bound", NeurIPS 2014.
  - Qu et al., "Asynchronous RL framework for net order exploration in detailed routing", DATE 2021 ✓ (TCAD extension with transfer).
  - Zhou et al., "Transformer-based RL for net ordering in detailed routing", IJCAI 2025 ✓.
- **Mapping:**
  - The expert is the expensive search: ideas 7 and 9, and ALNS runs with counterfactual rip-ups. At a bottleneck, try ripping each of the top-k candidate nets in parallel with rollouts, and label the one that led to completion with the fewest vias.
  - The apprentice is a per-net scorer (gradient-boosted trees first, then a small GNN over the net–tile bipartite graph).
  - Features: bounding box, pin count, class, local congestion, LP dual price, BP entropy, number of alternatives, whether the net fragments a pour.
  - Iterate: the apprentice becomes the prior or rollout policy of the expert (DAgger-style aggregation).
- **Why:** it turns the currently heuristic order and yield decisions into policies learned across boards. It also amortizes search cost; the apprentice adds almost no runtime.
- **Risk:**
  - The labels cost many router calls. Coarse rollouts and parallel counterfactuals help.
  - Overfitting: train on the D3 corpus and test on your 18.
  - The gains reported in IC routing are about 14–26% fewer violations, which is meaningful but not transformative.
- **Cheap first experiment:** log counterfactual rip-up outcomes at each stall on 50 boards (8 candidates per stall, in parallel). Fit gradient-boosted trees and replace the heuristic yield rule. Validate held out.
- **Value M–H / Effort M–H.**

### 21. Learned or precomputed A* heuristics
- **References:**
  - Kirilenko et al., TransPath (AAAI 2023 ✓): learned correction factors, up to 4× fewer A* expansions.
  - Yonetani et al., Neural A* (ICML 2021).
  - Arfaee, Zilles, Holte, bootstrap learning of heuristics (AIJ 2011).
  - Goldberg & Harrelson, ALT landmarks (SODA 2005).
- **Mapping:** the heuristic h(v) = octile distance × a learned correction factor from the local obstacle/price map. A cheaper non-learned variant is per-net backward Dijkstra on the coarse tile graph under current prices, which gives a near-perfect heuristic. Admissibility is not needed, since PathFinder is heuristic anyway.
- **Why:** 4-layer run time (pain 5); A* expansions dominate.
- **Risk:** inadmissible heuristics lengthen paths slightly; a neural net in the inner loop is too slow on CPU. Prefer the coarse-Dijkstra variant.
- **Cheap first experiment:** coarse-graph Dijkstra-to-target as h. Measure expansions and wall time on a 4-layer board.
- **Value M / Effort L.**

### 22. LLM-guided program search over router heuristics (FunSearch / EoH / ReEvo / AlphaEvolve)
- **References:**
  - Romera-Paredes et al., FunSearch (Nature 2024).
  - Liu et al., Evolution of Heuristics (ICML 2024 ✓).
  - Ye et al., ReEvo (NeurIPS 2024 ✓).
  - Novikov et al., AlphaEvolve (arXiv 2506.13131 ✓). OpenEvolve and CodeEvolve are open-source re-implementations.
- **Mapping:**
  - Expose small pluggable functions: net-order key; history-cost update; present-cost growth schedule; rip-up victim selection; via-cost schedule; corridor-width rule; placer congestion-to-force mapping.
  - Put them behind a scripting boundary (Rhai or WASM) or compile Rust candidates.
  - Fitness: the harness score on a training board set, with held-out validation. Islands of programs evolved by LLM mutation.
- **Why:** it explores structural heuristic changes beyond parameter tuning (idea 2), and it is cheap once the parallel harness exists. Program search has beaten hand-tuned heuristics for scheduling and bin packing.
- **Risk:**
  - Evaluation cost: each program needs a full benchmark pass. Use fast boards and fidelity F3.
  - Overfitting to 18 boards is severe here, so the held-out split is mandatory.
  - LLM API cost and nondeterminism.
- **Cheap first experiment:** evolve only the net-order key function (pure, ~20 lines) with EoH/OpenEvolve on 10 boards with a 60 s cap; validate on 8.
- **Value M / Effort M.**

### 23. Long shots: AlphaZero-style RL and diffusion placement
- **References:**
  - Mirhoseini et al., "A graph placement methodology for fast chip design", Nature 2021.
  - Cheng, Kahng et al., "Assessment of Reinforcement Learning for Macro Placement", ISPD 2023 ✓ (updated in TCAD 2025): simulated annealing and analytical placers match or beat the RL approach.
  - Vassallo & Bajada, RL for PCB placement (DATE 2024 ✓; RL_PCB code).
  - Lee et al., "Chip placement with diffusion models", ICML 2025 ✓: zero-shot placement with guided sampling and synthetic pretraining.
  - DeepPCB (InstaDeep, commercial) and PCBWorld's PPO/GRPO baselines.
- **Assessment:**
  - Do not make RL the main thrust. It is sample-inefficient and the track record against tuned classical search is poor.
  - The most plausible ML long shot is a diffusion placement prior: pretrain on D3 plus synthetic boards, then sample with guidance from your differentiable ePlace terms and surrogate. It can generate diverse initial placements for ideas 15–16.
- **Risk:** large effort, little data, and uncertain transfer to 2-sided PCB constraints.
- **Cheap first experiment:** none cheap. Revisit once idea 19 exists.
- **Value L–M / Effort H.**

---

## Pain point to idea map
| Pain point | Primary ideas | Secondary ideas |
|---|---|---|
| 1. Dense 2-layer boards with pours stall | 12, 4, 5, 1 | 10, 18 |
| 2. BGA escape | 6 (SAT/flow escape), 19 (fanout templates) | 9 |
| 3. No legal two-sided placement | 17 (plus a CP no-overlap fallback) | 15 |
| 4. Router is the expensive judge | 14, 16, 13 | 15 |
| 5. 4-layer run time and parallelism | 3, 1, 21 | — |
| 6. Diff pairs, length matching, pin swap | 18 | — |
| 7. Infeasible versus gave up; which net yields | 13, 10 (duals), 6 (blocking sets) | 20 |

## Top 5
1. **Randomized PathFinder with partial restarts and a parallel racing portfolio (idea 1), plus batch-parallel PathFinder (idea 3a).** Lowest effort and highest certainty. It turns idle cores into completion rate and wall-time gains, attacks the 85–90% stall if it is heavy-tailed (measurable in a day with the 32-seed experiment), and creates the infrastructure every other idea needs. Determinism is kept by logging the seed.
2. **ALNS with bandit operator selection, WalkSAT noise, tabu and min-cut blocking sets (ideas 4, 5, 6-lite).** This directly targets the stall mechanism: cycling in one neighbourhood. The pour-fragmenting and conflict-cut operators are PCB-specific levers that a greedy rip-up lacks. It is incremental on the existing code, and different windows repair in parallel.
3. **Fractional multicommodity flow with randomized rounding (idea 10), with infeasibility triage (idea 13).** One component gives a better corridor planner, diverse starts for the portfolio, a λ* routability number (the best cheap fidelity for placement), dual prices that answer "which net yields", and bottleneck cuts that answer "infeasible or gave up". The theory is mature (BonnRoute), and it reuses the A* oracle.
4. **Multi-fidelity surrogate gating with a quality-diversity placement archive (ideas 14 and 16), trained on self-generated data (idea 19).** This attacks the core coupling problem (pains 3 and 4): stop spending 1–20 s router calls on candidates that λ* or RUDY already rule out, and keep structurally different placements instead of local nudges. PCBWorld's 679 boards make training and validation credible beyond 18 boards.
5. **Global MRF / max-cut layer assignment for 2-layer boards (idea 12).** This is the specific structural fix for the worst pain point. Sequential per-net layer decisions are the likely root cause on double-pour 2-layer boards, and the "re-label a stalled result" experiment gives a go/no-go signal within about a week.

**Runners-up:**
- Expert-iteration yield/order policy (idea 20): high value once ideas 1 and 19 exist.
- irace configuration (idea 2): nearly free once the harness exists; do it early.
- Parallel tempering for two-sided legalization (idea 17): the cheap fix for pain 3.
- LLM program search (idea 22): worth it only after the harness and held-out split are solid.

**Overfitting rule for all learned or tuned components:** tune and train on PCBWorld D3 plus synthetic boards, report on the 18 in-house boards, and use leave-one-board-out for anything fitted on them.

## Sources (verified in this session)
- [He & Bao, arXiv 2006.13607](https://arxiv.org/pdf/2006.13607)
- [Qu et al., DATE 2021](https://yibolin.com/publications/papers/ROUTE_DATE2021_Qu.pdf)
- [Zhou et al., IJCAI 2025](https://www.ijcai.org/proceedings/2025/1055)
- [Bayless, Hoos, Hu, ICCAD 2016](https://ada.liacs.nl/papers/BayEtAl16.pdf)
- [Yeung & Saad, PRL 2012](https://journals.aps.org/prl/abstract/10.1103/PhysRevLett.108.208701)
- [Altarelli et al., PLoS ONE 2015](https://journals.plos.org/plosone/article?id=10.1371%2Fjournal.pone.0145222)
- [Lee et al., ICML 2025](https://proceedings.mlr.press/v267/lee25y.html)
- [Vassallo & Bajada, RL_PCB](https://github.com/LukeVassallo/RL_PCB)
- [Cheng, Kahng et al., ISPD 2023](https://dl.acm.org/doi/10.1145/3569052.3578926)
- [Gaier et al., SAIL](https://arxiv.org/abs/1806.05865)
- [Moctar & Brisk, DAC 2014](https://dl.acm.org/doi/10.1145/2593069.2593177)
- [WireMask-BBO](https://arxiv.org/abs/2306.16844)
- [Liu et al., DATE 2021](https://www.cse.cuhk.edu.hk/~byu/papers/C112-DATE2021-DREAMPlace-Cong.pdf)
- [EoH](https://proceedings.mlr.press/v235/liu24bs.html)
- [ReEvo](https://github.com/ai4co/reevo)
- [AlphaEvolve](https://arxiv.org/abs/2506.13131)
- [Witte et al., TPDS 1991](https://ieeexplore.ieee.org/document/97904/)
- [Müller, Radke, Vygen](http://www.or.uni-bonn.de/home/vygen/files/rs.pdf)
- [Oh et al., BO macro placement](https://arxiv.org/abs/2207.08398)
- [TransPath](https://arxiv.org/abs/2212.11730)
- [PCBWorld](https://arxiv.org/abs/2607.05915) and [PCBWorld code](https://github.com/LGAI-Research/PCBWorld)
- [CircuitNet](https://arxiv.org/pdf/2208.01040)
- [Shen & Luo, TPDS 2020](https://dl.acm.org/doi/abs/10.1109/TPDS.2020.3035787)
