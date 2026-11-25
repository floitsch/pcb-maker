# Copyright (C) 2026 Florian Loitsch.
"""The SAT model of one census window (census-kicad-window's JSON).

Every net of the window is rerouted jointly on the router's lattice:

- per connection (two terminals a net must join, the census's spanning tree
  of each requirement) a path in the degree encoding: each terminal's super
  node takes exactly one way in, a used node has exactly two used incident
  steps (lattice steps, vias, ways in), an unused node none; stray cycles
  are harmless and dropped when the copper is written;
- a connection may use only the nodes of its detour budget (the census's);
- the router's clearance as the census gives it: an element (node, diagonal
  step, via, escape stub) of one net stamps keys, another net's element
  reads keys, and the two conflict when they share one. An auxiliary
  variable per (net, key) carries the stamp: element -> stamp, and
  stamp and reader -> false;
- a net's own vias keep the class's via spacing;
- each net's requirement holds only under its selector literal (a unit
  clause unless the net is dropped), so cores are computed by deletion
  over the nets.
"""
import json


class Cnf:
    def __init__(self):
        self.count = 0
        self.clauses = []
        # Variables whose phase should start true (the cut copper): written
        # with flipped polarity, so that CaDiCaL's default phase false
        # (--phase=0) decides them true first.
        self.hint = set()

    def var(self):
        self.count += 1
        return self.count

    def add(self, clause):
        self.clauses.append(clause)

    def at_most(self, lits, k):
        """Sinz's sequential counter."""
        n = len(lits)
        if n <= k:
            return
        if k == 1 and n <= 6:
            for i in range(n):
                for j in range(i + 1, n):
                    self.add([-lits[i], -lits[j]])
            return
        s = [[self.var() for _ in range(k)] for _ in range(n - 1)]
        self.add([-lits[0], s[0][0]])
        for j in range(1, k):
            self.add([-s[0][j]])
        for i in range(1, n - 1):
            self.add([-lits[i], s[i][0]])
            self.add([-s[i - 1][0], s[i][0]])
            for j in range(1, k):
                self.add([-lits[i], -s[i - 1][j - 1], s[i][j]])
                self.add([-s[i - 1][j], s[i][j]])
            self.add([-lits[i], -s[i - 1][k - 1]])
        self.add([-lits[n - 1], -s[n - 2][k - 1]])

    def lit(self, l):
        return -l if abs(l) in self.hint else l

    def write(self, path, units=()):
        h = self.hint
        with open(path, "w") as f:
            f.write(f"p cnf {self.count} {len(self.clauses) + len(units)}\n")
            f.write("".join(" ".join(str(-l if (l if l > 0 else -l) in h else l) for l in c) + " 0\n"
                            for c in self.clauses))
            for u in units:
                f.write(f"{self.lit(u)} 0\n")

    def true_vars(self, positives):
        """The original variables true in a model given as the solver's
        positive literals (of the flipped CNF)."""
        out = set(v for v in positives if v not in self.hint)
        out.update(v for v in self.hint if v not in positives)
        return out


def build(census):
    """The CNF of a census and what decoding needs."""
    cnf = Cnf()
    nets = census["nets"]
    info = {"nets": []}
    stamp_vars = {}  # (net index, key) -> var
    readers = []  # (element var, net index, read keys)
    for ni, net in enumerate(nets):
        sel = cnf.var()
        nodes = net["nodes"]
        edges = net["edges"]
        vias = net["vias"]
        node_var = {}
        diag_var = {}
        via_var = {}
        esc_var = {}
        terminals = {t[0]: t for t in net["terminals"]}
        hint = set(net.get("hint_nodes", []))
        hint_vias = set(net.get("hint_vias", []))
        conns = []
        for (a, b, dist, allowed) in net["connections"]:
            allowed_set = set(allowed)
            y = {u: cnf.var() for u in allowed}
            cnf.hint.update(y[u] for u in allowed if u in hint)
            incident = {u: [] for u in allowed}
            fvars = []  # (var, kind, data)
            for ei, (u, v, diag) in enumerate(edges):
                if u in allowed_set and v in allowed_set:
                    f = cnf.var()
                    incident[u].append(f)
                    incident[v].append(f)
                    cnf.add([-f, y[u]])
                    cnf.add([-f, y[v]])
                    fvars.append((f, "edge", ei))
                    if u in hint and v in hint:
                        cnf.hint.add(f)
                    if diag:
                        if ei not in diag_var:
                            diag_var[ei] = cnf.var()
                            if u in hint and v in hint:
                                cnf.hint.add(diag_var[ei])
                        cnf.add([-f, diag_var[ei]])
            for vi, (_, _, here) in enumerate(vias):
                inside = [u for u in here if u in allowed_set]
                for i in range(len(inside)):
                    for j in range(i + 1, len(inside)):
                        f = cnf.var()
                        u, v = inside[i], inside[j]
                        incident[u].append(f)
                        incident[v].append(f)
                        cnf.add([-f, y[u]])
                        cnf.add([-f, y[v]])
                        if vi not in via_var:
                            via_var[vi] = cnf.var()
                            if vi in hint_vias:
                                cnf.hint.add(via_var[vi])
                        cnf.add([-f, via_var[vi]])
                        if vi in hint_vias:
                            cnf.hint.add(f)
                        fvars.append((f, "via", (vi, u, v)))
            ways = {}
            for side in (a, b):
                lits = []
                for (node, esc) in terminals[side][2]:
                    if node not in allowed_set:
                        continue
                    w = cnf.var()
                    if node in hint:
                        cnf.hint.add(w)
                    incident[node].append(w)
                    cnf.add([-w, y[node]])
                    if esc >= 0:
                        if esc not in esc_var:
                            esc_var[esc] = cnf.var()
                        cnf.add([-w, esc_var[esc]])
                    lits.append(w)
                    fvars.append((w, "way", (side, node, esc)))
                ways[side] = lits
                cnf.add([-sel] + lits)
                cnf.at_most(lits, 1)
            for u in allowed:
                inc = incident[u]
                if not inc:
                    cnf.add([-y[u]])
                    continue
                cnf.add([-y[u]] + inc)
                for i, e in enumerate(inc):
                    cnf.add([-y[u], -e] + inc[:i] + inc[i + 1:])
                cnf.at_most(inc, 2)
                if u not in node_var:
                    node_var[u] = cnf.var()
                    if u in hint:
                        cnf.hint.add(node_var[u])
                cnf.add([-y[u], node_var[u]])
            conns.append({"a": a, "b": b, "y": y, "f": fvars})
        for (i, j) in net["own_via_conflicts"]:
            if i in via_var and j in via_var:
                cnf.add([-via_var[i], -via_var[j]])
        # Elements.
        for (kind, index, stamps, reads) in net["elements"]:
            table = (node_var, diag_var, via_var, esc_var)[kind]
            v = table.get(index)
            if v is None:
                continue
            for k in stamps:
                s = stamp_vars.get((ni, k))
                if s is None:
                    s = stamp_vars[(ni, k)] = cnf.var()
                if v in cnf.hint:
                    cnf.hint.add(s)
                cnf.add([-v, s])
            if reads:
                readers.append((v, ni, reads))
        info["nets"].append({"sel": sel, "conns": conns, "vias": via_var, "escapes": esc_var})
    by_key = {}
    for (ni, k), s in stamp_vars.items():
        by_key.setdefault(k, []).append((ni, s))
    conflicts = 0
    for (v, ni, reads) in readers:
        for k in reads:
            for (nj, s) in by_key.get(k, ()):
                if nj != ni:
                    cnf.add([-v, -s])
                    conflicts += 1
    info["conflict_clauses"] = conflicts
    info["stamp_vars"] = len(stamp_vars)
    return cnf, info


def decode(census, info, model):
    """Per net: the steps, vias and ways in a solution uses (stray cycles
    dropped)."""
    true = model
    out = []
    for ni, net in enumerate(census["nets"]):
        ninfo = info["nets"][ni]
        adjacency = {}
        segs = set()
        vias = set()
        ways = []
        for conn in ninfo["conns"]:
            for (f, kind, data) in conn["f"]:
                if f not in true:
                    continue
                if kind == "edge":
                    u, v, _ = net["edges"][data]
                    segs.add((min(u, v), max(u, v)))
                    adjacency.setdefault(u, set()).add(v)
                    adjacency.setdefault(v, set()).add(u)
                elif kind == "via":
                    vi, u, v = data
                    vias.add(vi)
                    adjacency.setdefault(u, set()).add(v)
                    adjacency.setdefault(v, set()).add(u)
                else:
                    ways.append(data)
        # Keep what the ways in reach.
        seen = set()
        stack = [node for (_, node, _) in ways]
        while stack:
            u = stack.pop()
            if u in seen:
                continue
            seen.add(u)
            stack.extend(adjacency.get(u, ()))
        segs = {(u, v) for (u, v) in segs if u in seen}
        vias = {vi for vi in vias if any(u in seen for u in net["vias"][vi][2])}
        out.append({"segments": sorted(segs), "vias": sorted(vias), "ways": ways})
    return out
