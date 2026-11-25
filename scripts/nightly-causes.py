#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The census of open causes (scripts/nightly.sh, after the summary;
benchmarks/agent-tasks/README.md, "Nightly"): per stage of a night, how
many of KiCad's unconnected items, and on how many boards, fall under each
cause the tool's own outputs establish.

    scripts/nightly-causes.py <night>                   # nightly.sh: causes.json, a section of summary.md
    scripts/nightly-causes.py <night> --output DIR      # read-only: DIR/causes.json and DIR/causes.md
    scripts/nightly-causes.py --run NAME=DIR --output OUT   # a run.py output directory (results.json)

Only facts a run wrote are counted: KiCad's items from the board's
drc.json (the verdict's `outputs.drc`, else drc.json beside the verdict or
in `result/`), the router's facts from verdict.json and board-router.json.
Nothing is derived from geometry: an item's ends are matched to the facts
by name only (a pad's `REF.NUMBER` and net, an item end's uuid, the net's
name). Each item goes to the first cause of PRECEDENCE that names it:

- an end on a part the layout left unplaced; both ends' uuids among the
  verdict's `board_outlines.unconnected_across_items`; a closed, dead or
  walled pad; islands of one zone;
- two pads of a net the router split (`nets[].copy`) in two different
  copies (`nets[].pads`);
- an end pad among the router's open terminals of the net
  (`nets[].open[].pads`): the router's reason (`open:<reason>`);
- else the router's net of the item's net name (where the router split the
  name, the copy holding the item's pads): routed, or open for one reason
  only (`nets[].open`, its counts exact) when the pads were not listed;
- the rest is `unexplained`, split by what the router says of its net.

A verdict without `unconnected_across_items` (an older binary) names no
item across outlines, only a count: it takes that many of the board's items
not named by another fact, the unexplained ones first.

Against the previous night with a causes.json: the change per cause. On
any error the script prints one line (into summary.md too) and exits 0:
the census never fails the night."""

import argparse
import json
import re
import sys
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
STAGES = ("sweep", "designer", "expansion", "short", "fingerprints")
NIGHT_NAME = re.compile(r"^\d{4}-\d{2}-\d{2}")
HEADING = "## Open causes"
KICAD_CAP = 499  # KiCad's DRC report stops at this many items of a type.

# The causes, first match wins. (key, label, where the fact is.)
CAUSES = (
    ("unplaced_part", "pad of a part the layout left unplaced", "verdict `unplaced`"),
    ("separate_outlines", "across two board outlines (a count)", "verdict `board_outlines.unconnected_across`"),
    ("closed_pad", "closed pad: no track leaves, no via allowed",
     "verdict `closed_pads`, board-router.json `vias_in_pads.closed`"),
    ("dead_pad:no_legal_access", "dead pad, no_legal_access (a wall by the rules)",
     "board-router.json `diagnostics.dead_pads[].access`"),
    ("dead_pad:off_lattice", "dead pad, off_lattice (a wall by the rules)", "the same"),
    ("dead_pad:no_lattice_node", "dead pad, no_lattice_node (the lattice's limit)", "the same"),
    ("dead_pad:stubs_blocked", "dead pad, stubs_blocked", "the same"),
    ("dead_pad:start_blocked", "dead pad, start_blocked (the router's stub start)", "the same"),
    ("dead_pad:escape_not_kept", "dead pad, escape_not_kept", "the same"),
    ("walled_pad", "pad wired through its own footprint's copper", "verdict `walled_pads`"),
    ("zone_island", "islands of one zone", "verdict `zone_island_recovery.zone_uuid`"),
    ("split_across_copies", "pads of two copies of a net the router split (a panel's sub-boards or cells)",
     "board-router.json `nets[].copy`, `nets[].pads`"),
    # The router's reasons (board-router.json `nets[].open`, pcb_router::OpenReason).
    ("open:net_not_routable", "open: the net was never searched (fewer than two of its pads have access)",
     "`nets[].open[].reason`"),
    ("open:no_path_static", "open: no path on the whole board even through other nets' copper "
                            "(pads, keepouts, the outline in the way)", "the same"),
    ("open:no_path_hard:board", "open: the hard repair found no path around other nets' copper on the whole board",
     "the same"),
    ("open:no_path_hard:window", "open: the hard repair found no path around other nets' copper in its window",
     "the same"),
    ("open:no_path_hard:corridor", "open: the hard repair found no path around other nets' copper in its corridor",
     "the same"),
    ("open:repair_skipped:work_deadline", "open: open when the negotiation stopped, the hard repair skipped it "
                                          "(the attempt's work deadline)", "the same"),
    ("open:repair_skipped:budget", "open: open when the negotiation stopped, the hard repair skipped it "
                                   "(the repair's budget)", "the same"),
    ("open:repair_skipped:hopeless", "open: open when the negotiation stopped, the hard repair skipped it "
                                     "(the attempt judged hopeless)", "the same"),
    ("open:lost:hard_repair", "open: joined, then lost in the hard repair (a forced connection, a link retried)",
     "the same"),
    ("open:lost:trouble_repair", "open: joined, then lost in the trouble repair (displaced)", "the same"),
    ("open:lost:clean_up", "open: joined, then lost in the clean-up", "the same"),
    ("open:lost:via_reduction", "open: joined, then lost in the via reduction", "the same"),
    ("open:lost:bends", "open: joined, then lost in the bend round", "the same"),
    ("open:lost:thermal_spokes", "open: joined, then lost freeing thermal spokes", "the same"),
    ("open:lost:straighten", "open: joined, then lost in the straightening", "the same"),
    ("open:lost:compaction", "open: joined, then lost in the compaction", "the same"),
    ("open:lost:pour_stitching", "open: joined, then lost in the pour stitching (off the pour's main piece)",
     "the same"),
    ("open:lost:manual_spokes", "open: joined, then lost adding manual spokes", "the same"),
    ("open:copper_check", "open: the router counted it joined, its emitted copper leaves it apart", "the same"),
    ("open:no_access", "open: the router's terminal has no lattice node", "the same"),
    ("open:other", "open: another reason the router names", "the same"),
    ("router_counts_routed", "the router counts the net routed, KiCad does not",
     "board-router.json `nets[].status` routed"),
)
UNEXPLAINED = (  # in the order they are tried
    ("unexplained:dead_pad_without_access", "unexplained: a dead pad without `access` (an older binary)"),
    ("unexplained:dead_pad_access_unknown", "unexplained: a dead pad with access unknown (not measured)"),
    ("unexplained:open_unknown", "unexplained: the router left it open and recorded no reason (`unknown`)"),
    ("unexplained:net_open_mixed",
     "unexplained: the router left the net open for several reasons, none listed for the item's pads"),
    ("unexplained:net_open_conflicted",
     "unexplained: the router left the net open, in conflict when it stopped (`conflicted_nets`)"),
    ("unexplained:net_open", "unexplained: the router left the net open, not in conflict"),
    ("unexplained:net_split", "unexplained: the router's nets of the name disagree (a split net)"),
    ("unexplained:net_other", "unexplained: the router's net status is another (trivial, ...)"),
    ("unexplained:no_net_status", "unexplained: no router status for the item's net"),
    ("unexplained:no_outputs", "unexplained: the board's outputs are missing or unreadable"),
)
PRECEDENCE = [key for key, _, _ in CAUSES] + [key for key, _ in UNEXPLAINED]
LABELS = {key: label for key, label, *_ in CAUSES + UNEXPLAINED}
# Where separate_outlines takes its count from: the unexplained first, then
# the router's routed nets; never an item a named fact claims.
COUNT_TAKES = [key for key, _ in UNEXPLAINED] + ["router_counts_routed"]

PAD = re.compile(r"^(?:PTH |NPTH )?[Pp]ad (?P<number>\S*) (?:\[.*?\] )?of (?P<ref>\S+)")
ESCAPES = {"{slash}": "/", "{backslash}": "\\", "{quote}": '"', "{lt}": "<", "{gt}": ">", "{colon}": ":",
           "{space}": " ", "{tab}": "\t", "{return}": "\n", "{dblquote}": '"'}


def load_compare(harness=REPO):
    sys.path.insert(0, str(Path(harness) / "benchmarks/agent-tasks"))
    import compare
    return compare


# --- one board ------------------------------------------------------------------------


def net_key(name):
    """A net's name as both KiCad's report and the router's write it: the
    file's escapes undone, without the leading sheet slash."""
    if name is None:
        return None
    for escaped, plain in ESCAPES.items():
        name = name.replace(escaped, plain)
    return name.lstrip("/")


def end_of(item):
    """One end of a KiCad unconnected item: its kind (the description's
    first word), net, pad (`REF.NUMBER`, for a pad) and uuid."""
    description = item.get("description") or ""
    start = description.find("[")
    end = description.find("]", start + 1) if start >= 0 else -1
    match = PAD.match(description)
    return {"kind": description.split(" ")[0] if description else "",
            "net": net_key(description[start + 1:end]) if end > start >= 0 else None,
            "pad": f"{match['ref']}.{match['number']}" if match else None,
            "ref": match["ref"] if match else None,
            "uuid": item.get("uuid")}


def read_json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def board_files(board):
    """(verdict, drc report, board-router.json) of a board's output
    directory (`layout/` or `route/`); None for what is missing."""
    for sub in ("layout", "route"):
        verdict_path = Path(board) / sub / "verdict.json"
        verdict = read_json(verdict_path)
        if verdict is None:
            continue
        base = verdict_path.parent
        drc = None
        candidates = [(verdict.get("outputs") or {}).get("drc"), base / "drc.json", base / "result/drc.json"]
        for candidate in candidates:
            if candidate and Path(candidate).is_file() and (drc := read_json(candidate)) is not None:
                break
        router = read_json(base / "board-router.json") or read_json(base / "result/board-router.json")
        return verdict, drc, router
    return None, None, None


def facts_of(verdict, router):
    """The facts of one board by what they name: pads (cause, net key),
    zone uuids, the router's net statuses, the outlines' count."""
    router = router or {}
    pads = {}  # "REF.NUMBER" -> [(cause, the fact's net key or None)]

    def name(pad, cause, net=None):
        pads.setdefault(pad, []).append((cause, net_key(net)))

    unplaced = set(verdict.get("unplaced") or [])
    closed = verdict.get("closed_pads")
    if closed is None:
        closed = (router.get("vias_in_pads") or {}).get("closed") or []
    for entry in closed:
        for pad in entry.get("pads") or []:
            name(f"{entry.get('part')}.{pad.split(' (')[0]}", "closed_pad")
    lacking_access = 0
    for dead in (router.get("diagnostics") or {}).get("dead_pads") or []:
        access = dead.get("access")
        if access is None:
            cause, lacking_access = "unexplained:dead_pad_without_access", lacking_access + 1
        elif f"dead_pad:{access}" in LABELS:
            cause = f"dead_pad:{access}"
        else:
            cause = "unexplained:dead_pad_access_unknown"
        if dead.get("pad"):
            name(dead["pad"], cause, dead.get("net"))
    for walled in verdict.get("walled_pads") or []:
        name(f"{walled.get('part')}.{walled.get('pad')}", "walled_pad", walled.get("net"))
    island = (verdict.get("zone_island_recovery") or {}).get("zone_uuid")
    statuses = {}  # net key -> the statuses of the router's nets of that name (a split net has several)
    copies = {}  # net key -> the router's nets of that name: status, open reasons with counts, pads
    open_pads = {}  # "REF.NUMBER" -> [(the router's reason, net key)]
    for net in router.get("nets") or []:
        if isinstance(net, dict) and net.get("connection") is not None:
            key = net_key(net["connection"])
            statuses.setdefault(key, set()).add(net.get("status"))
            reasons = {}
            for group in net.get("open") or []:
                reasons[group.get("reason")] = reasons.get(group.get("reason"), 0) + (group.get("terminals") or 0)
                for label in group.get("pads") or []:
                    open_pads.setdefault(label, []).append((group.get("reason"), key))
            copies.setdefault(key, []).append({"status": net.get("status"), "reasons": reasons,
                                               "pads": set(net.get("pads") or [])})
    conflicted = {net_key(net) for net in (router.get("diagnostics") or {}).get("conflicted_nets") or []}
    outlines = verdict.get("board_outlines") or {}
    across_items = outlines.get("unconnected_across_items")
    return {"unplaced": unplaced, "pads": pads, "island": island, "statuses": statuses, "conflicted": conflicted,
            "copies": copies, "open_pads": open_pads,
            "across_items": None if across_items is None else {frozenset(pair) for pair in across_items},
            "across": int(outlines.get("unconnected_across") or 0) if across_items is None else 0,
            "lacking_access": lacking_access}


def open_cause(reason):
    """The census key of a router's reason (`nets[].open[].reason`)."""
    if reason == "unknown":
        return "unexplained:open_unknown"
    key = f"open:{reason}"
    return key if key in LABELS else "open:other"


def copy_of(ends, copies):
    """The indices of the router's nets of a split name (`copies`) holding
    each pad end, a set per pad end."""
    return [{index for index, copy in enumerate(copies) if end["pad"] in copy["pads"]}
            for end in ends if end["pad"]]


def cause_of(item, facts):
    """The cause of one KiCad unconnected item: the first of PRECEDENCE a
    fact names; `named` says whether a fact named the item itself."""
    ends = [end_of(end) for end in item.get("items") or []]
    found = []
    for end in ends:
        if end["ref"] and end["ref"] in facts["unplaced"]:
            found.append("unplaced_part")
        for cause, net in facts["pads"].get(end["pad"], []) if end["pad"] else []:
            if net is None or end["net"] is None or net == end["net"]:
                found.append(cause)
    if facts["island"] and len(ends) == 2 and all(e["kind"] == "Zone" and e["uuid"] == facts["island"]
                                                  for e in ends):
        found.append("zone_island")
    if facts["across_items"] and len(ends) == 2 and frozenset(e["uuid"] for e in ends) in facts["across_items"]:
        found.append("separate_outlines")
    net = next((e["net"] for e in ends if e["net"]), None)
    copies = facts["copies"].get(net) or []
    holding = copy_of(ends, copies) if len(copies) > 1 else []
    if len(holding) == 2 and all(len(h) == 1 for h in holding) and holding[0] != holding[1]:
        found.append("split_across_copies")
    for end in ends:
        for reason, open_net in facts["open_pads"].get(end["pad"], []) if end["pad"] else []:
            if end["net"] is None or open_net == end["net"]:
                found.append(open_cause(reason))
    named = [cause for cause in found if not cause.startswith("unexplained:")]
    if named:
        return min(named, key=PRECEDENCE.index), True
    if found:  # a dead pad the router did not classify, an open terminal without a reason
        return min(found, key=PRECEDENCE.index), False
    statuses = facts["statuses"].get(net) if net is not None else None
    if not statuses:
        return "unexplained:no_net_status", False
    if len(copies) > 1:  # several router nets of the name (sub-boards, copies): the one holding the pads
        held = set.intersection(*holding) if holding else set()
        if len(held) == 1:
            copies = [copies[next(iter(held))]]
            statuses = {copies[0]["status"]}
    if len(statuses) > 1:  # copies that disagree, the item's not known
        return "unexplained:net_split", False
    status = next(iter(statuses))
    if status == "routed":
        return "router_counts_routed", False
    if status in ("partial", "unreachable"):
        reasons = {}
        for copy in copies:
            for reason, count in copy["reasons"].items():
                reasons[reason] = reasons.get(reason, 0) + count
        if len(reasons) == 1:  # every open terminal of the net open for one reason
            return open_cause(next(iter(reasons))), False
        if len(reasons) > 1:
            return "unexplained:net_open_mixed", False
        return ("unexplained:net_open_conflicted" if net in facts["conflicted"] else "unexplained:net_open"), False
    return "unexplained:net_other", False


def census_board(board, count):
    """{cause: items} of one board with `count` KiCad items (the row's),
    and notes: `lacking_access` (dead pads without `access`), `outputs`
    (False: no verdict or no drc.json), the router's own counts."""
    verdict, drc, router = board_files(board)
    notes = {"outputs": verdict is not None and drc is not None, "lacking_access": 0}
    if not notes["outputs"]:
        return ({"unexplained:no_outputs": count} if count else {}), notes
    facts = facts_of(verdict, router)
    notes.update(lacking_access=facts["lacking_access"], open_connections=verdict.get("open_connections"),
                 unconnected_terminals=verdict.get("unconnected_terminals"))
    items = drc.get("unconnected_items") or []
    notes["capped"] = bool(drc.get("unconnected_items_capped")) or len(items) >= KICAD_CAP
    if len(items) != count:  # not the report the row judged: none of its items is explained
        notes["report_items"] = len(items)
        return ({"unexplained:no_outputs": count} if count else {}), notes
    causes = [cause_of(item, facts) for item in items]
    notes["open_by_net"] = sum(1 for cause, named in causes if cause.startswith("open:") and not named)
    # An older verdict's outlines count takes items no fact names, the
    # unexplained first.
    take = facts["across"]
    for target in COUNT_TAKES:
        for index, (cause, named) in enumerate(causes):
            if take and not named and cause == target:
                causes[index], take = ("separate_outlines", True), take - 1
    tally = {}
    for cause, _ in causes:
        tally[cause] = tally.get(cause, 0) + 1
    return tally, notes


# --- one stage ------------------------------------------------------------------------


def census_stage(compare, rows_source, boards_directory):
    """The census of one stage: its rows (a log or a run.py directory) and
    its boards' directories."""
    rows = compare.by_name(compare.rows_of(rows_source))
    stage = {"boards": len(rows), "items": 0, "boards_with_items": 0, "no_result": [], "without_outputs": [],
             "lacking_access": [], "capped": [], "router_open_connections": 0, "router_unconnected_terminals": 0,
             "open_by_net": 0,
             "router_complete_kicad_open": [], "causes": {}, "per_board": {}}
    for name, row in sorted(rows.items()):
        count = compare.unconnected(row)
        if count is None:
            stage["no_result"].append(name)
            continue
        try:
            tally, notes = census_board(Path(boards_directory) / name, count)
        except Exception as error:  # one unreadable board does not end the census
            tally, notes = ({"unexplained:no_outputs": count} if count else {}), {"outputs": False,
                                                                                  "error": str(error)}
        if count and not notes.get("outputs"):
            stage["without_outputs"].append(name)
        if notes.get("lacking_access"):
            stage["lacking_access"].append(name)
        if notes.get("capped"):
            stage["capped"].append(name)
        stage["router_open_connections"] += notes.get("open_connections") or 0
        stage["router_unconnected_terminals"] += notes.get("unconnected_terminals") or 0
        stage["open_by_net"] += notes.get("open_by_net") or 0
        if count and notes.get("outputs") and not notes.get("open_connections") \
                and not notes.get("unconnected_terminals"):
            stage["router_complete_kicad_open"].append(name)
        if not count:
            continue
        stage["items"] += count
        stage["boards_with_items"] += 1
        stage["per_board"][name] = {"items": count, "causes": tally}
        for cause, items in tally.items():
            entry = stage["causes"].setdefault(cause, {"items": 0, "boards": 0})
            entry["items"] += items
            entry["boards"] += 1
    stage["causes"] = {cause: stage["causes"][cause] for cause in PRECEDENCE if cause in stage["causes"]}
    stage["unexplained"] = sum(entry["items"] for cause, entry in stage["causes"].items()
                               if cause.startswith("unexplained:"))
    return stage


def stages_of(night):
    """The stages a night ran: nightly.json's, else those with a log."""
    record = read_json(Path(night) / "nightly.json") or {}
    ran = [stage for stage, entry in (record.get("stages") or {}).items() if entry.get("status") == "ran"]
    return [stage for stage in STAGES if (stage in ran if ran else (Path(night) / f"{stage}.log").is_file())]


def previous_census(nights, night):
    """(name, causes.json) of the newest night before `night` with one."""
    nights = Path(nights)
    if not nights.is_dir():
        return None, None
    for name in sorted((p.name for p in nights.iterdir() if p.is_dir() and NIGHT_NAME.match(p.name)),
                       reverse=True):
        if name < night and (record := read_json(nights / name / "causes.json")) is not None:
            return name, record
    return None, None


def deltas(stage, before):
    """Per cause the change in items and boards against the same stage of
    the previous census (absent there: None)."""
    if before is None:
        return None
    causes = set(stage["causes"]) | set(before.get("causes") or {})
    change = {}
    for cause in causes:
        now, then = stage["causes"].get(cause, {}), (before.get("causes") or {}).get(cause, {})
        change[cause] = {"items": now.get("items", 0) - then.get("items", 0),
                         "boards": now.get("boards", 0) - then.get("boards", 0)}
    change["total"] = {"items": stage["items"] - before.get("items", 0),
                       "boards": stage["boards_with_items"] - before.get("boards_with_items", 0)}
    return change


def census(sources, night_name, nights=None, harness=REPO):
    """The census of `sources` ({stage: (rows source, boards directory)})."""
    compare = load_compare(harness)
    previous, before = previous_census(nights, night_name) if nights else (None, None)
    record = {"night": night_name, "previous": previous, "precedence": PRECEDENCE, "stages": {}}
    for stage, (rows_source, boards) in sources.items():
        entry = census_stage(compare, rows_source, boards)
        entry["delta"] = deltas(entry, ((before or {}).get("stages") or {}).get(stage))
        record["stages"][stage] = entry
    return record


# --- the tables -----------------------------------------------------------------------


def short_board(name):
    parts = name.split("__")[1:] or [name]
    return "__".join(parts[-1:] if len(set(parts)) == 1 else parts)


def signed(value):
    return "0" if value == 0 else f"{value:+d}"


def names(boards, limit=6):
    shown = ", ".join(short_board(n) for n in boards[:limit])
    return shown + (f" and {len(boards) - limit} more" if len(boards) > limit else "")


def markdown(record):
    previous = record.get("previous")
    lines = [HEADING, "",
             "KiCad's unconnected items by the cause the tool's own outputs establish, each item under the "
             "first cause that names it (`scripts/nightly-causes.py`, `causes.json`; "
             "benchmarks/agent-tasks/README.md, \"Nightly\"); "
             + (f"against {previous}." if previous else "no earlier census.")]
    if not record["stages"]:
        lines += ["", "No stage ran."]
    for stage_name, stage in record["stages"].items():
        delta = stage.get("delta")
        head = (f"**{stage_name}**: {stage['items']} items on {stage['boards_with_items']} of "
                f"{stage['boards']} boards, {stage['unexplained']} unexplained")
        if delta:
            head += f" (items {signed(delta['total']['items'])} against {previous})"
        lines += ["", head + "."]
        rows = [cause for cause in PRECEDENCE
                if cause in stage["causes"] or (delta and delta.get(cause, {}).get("items"))]
        if rows:
            lines += ["", "| Cause | Items | Boards |" + (" Change (items, boards) |" if delta else ""),
                      "| --- | ---: | ---: |" + (" ---: |" if delta else "")]
            for cause in rows:
                entry = stage["causes"].get(cause, {"items": 0, "boards": 0})
                cell = f" {signed(delta[cause]['items'])}, {signed(delta[cause]['boards'])} |" \
                    if delta and cause in delta else (" new |" if delta else "")
                lines.append(f"| {LABELS[cause]} | {entry['items']} | {entry['boards']} |" + cell)
        notes = []
        for key, text in (("no_result", "boards without a result"),
                          ("without_outputs", "boards without outputs (no verdict or drc.json, or another count)"),
                          ("lacking_access", "boards whose dead pads lack `access` (an older binary)"),
                          ("capped", f"boards where KiCad's report stopped at {KICAD_CAP} items (more are open)"),
                          ("router_complete_kicad_open", "boards with KiCad items where the router reports "
                                                         "nothing open")):
            if stage[key]:
                notes.append(f"{text}: {len(stage[key])} ({names(stage[key])})")
        if stage.get("open_by_net"):
            notes.append(f"items under an `open:` cause by their net's only reason (their pads not listed): "
                         f"{stage['open_by_net']}")
        notes.append(f"the router's own count: {stage['router_open_connections']} open connections, "
                     f"{stage['router_unconnected_terminals']} unconnected terminals")
        lines += [""] + [f"- {note}" for note in notes]
    return "\n".join(lines) + "\n"


def replace_section(text, section):
    """`text` with its census section replaced by `section` (appended
    when it has none)."""
    start = text.find(HEADING + "\n")
    if start < 0:
        return text.rstrip("\n") + "\n\n" + section
    end = text.find("\n## ", start + len(HEADING))
    return text[:start] + section + (text[end + 1:] if end >= 0 else "")


# --- the command ----------------------------------------------------------------------


def run(arguments):
    if arguments.run:
        sources = {}
        for spec in arguments.run:
            stage, _, directory = spec.partition("=")
            sources[stage] = (Path(directory), Path(directory))
        record = census(sources, arguments.name or "run", harness=arguments.harness)
    else:
        night = arguments.night
        nights = arguments.nights or night.parent
        sources = {stage: (night / f"{stage}.log", night / stage) for stage in stages_of(night)}
        record = census(sources, night.name, nights, arguments.harness)
    section = markdown(record)
    output = arguments.output
    if output is not None:
        output.mkdir(parents=True, exist_ok=True)
        (output / "causes.json").write_text(json.dumps(record, indent=1) + "\n")
        (output / "causes.md").write_text(section)
    else:
        (arguments.night / "causes.json").write_text(json.dumps(record, indent=1) + "\n")
        summary = arguments.night / "summary.md"
        summary.write_text(replace_section(summary.read_text() if summary.is_file() else "", section))
    totals = ", ".join(f"{stage} {entry['items']} items, {entry['unexplained']} unexplained"
                       for stage, entry in record["stages"].items())
    print(f"open causes: {totals or 'no stage'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("night", type=Path, nargs="?", help="a night's directory (build/nightly/<date>)")
    parser.add_argument("--nights", type=Path, help="where the earlier nights are (default: the night's parent)")
    parser.add_argument("--run", action="append", metavar="NAME=DIR",
                        help="a run.py output directory to census as stage NAME (repeatable; needs --output)")
    parser.add_argument("--name", help="with --run: the census's name")
    parser.add_argument("--output", type=Path, help="write causes.json and causes.md here, not into the night")
    parser.add_argument("--harness", type=Path, default=REPO, help="the checkout whose compare.py reads the rows")
    arguments = parser.parse_args()
    try:
        if arguments.night is None and not (arguments.run and arguments.output):
            raise ValueError("give a night, or --run with --output")
        run(arguments)
    except Exception as error:  # never fail the night: one line, here and in summary.md
        line = f"open causes: the census failed ({type(error).__name__}: {error})"
        print(line)
        traceback.print_exc(file=sys.stderr)
        if arguments.night is not None and arguments.output is None:
            try:
                summary = arguments.night / "summary.md"
                text = summary.read_text() if summary.is_file() else ""
                summary.write_text(replace_section(text, f"{HEADING}\n\n{line}\n"))
            except Exception:
                pass
    sys.exit(0)


if __name__ == "__main__":
    main()
