#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Rewrites the class rules of an exported DSN to KiCad's effective values.

KiCad's DSN exporter writes each net class's own width and clearance, but
the effective rule on the board is at least the board minimum. Freerouting
would otherwise route below the minimum and fail DRC. Via padstacks are left
alone; they must already match (the audit checks)."""

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from audit_dsn_classes import parse_dsn, child, children  # noqa: E402


def main():
    dsn_path, rules_path, report_path = map(Path, sys.argv[1:4])
    text = dsn_path.read_text()
    rules = json.loads(rules_path.read_text())['rules']
    tree = parse_dsn(text)
    network = child(tree, 'network')
    changes = []
    for cls in children(network, 'class'):
        nets = [item.strip('"') for item in cls[2:] if not isinstance(item, list)]
        effective = {json.dumps(rules[n.removeprefix('/')]) for n in nets if n.removeprefix('/') in rules}
        if len(effective) != 1:
            continue
        rule = json.loads(effective.pop())
        want_width = round(rule['trace_width_mm'] * 1000)
        want_clearance = round(rule['clearance_mm'] * 1000)
        pattern = re.compile(r'(\(class\s+' + re.escape(cls[1]) + r'\b(?:.|\n)*?\(rule\s*\(width\s+)(\d+)(\)\s*\(clearance\s+)(\d+)(\))')
        match = pattern.search(text)
        if not match:
            continue
        have = (int(match.group(2)), int(match.group(4)))
        if have == (want_width, want_clearance):
            continue
        text = text[:match.start()] + match.group(1) + str(want_width) + match.group(3) + str(want_clearance) + match.group(5) + text[match.end():]
        changes.append({'class': cls[1], 'from': have, 'to': [want_width, want_clearance]})
    dsn_path.write_text(text)
    report_path.write_text(json.dumps({'scope': __doc__, 'changes': changes}, indent=2) + '\n')
    print(json.dumps(changes))


if __name__ == '__main__':
    main()
