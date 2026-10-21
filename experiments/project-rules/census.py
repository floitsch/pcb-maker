#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Census of the project rules of the 2026-10-18 intake: which rule
constructs appear in the boards' .kicad_dru and .kicad_pro files, so that
project_rules.rs covers the common ones.

    python3 -I experiments/project-rules/census.py [--data benchmarks/expansion/data] [--json OUT]

Reads the harvest's manifest (every accepted board) and, per board, its
.kicad_pro (when the harvest found one) and .kicad_dru (custom_rules_path).
Counts, by rules and by boards: constraint types (with min/opt/max),
condition atoms (a property with its operator and the kind of operand,
or a function call), layer clauses, and in the projects: the design-rule
keys present or missing, net classes, pattern kinds (wildcard or regex)
and assignments. Only reads; nothing from the boards is executed.
"""
import argparse
import collections
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def tokens(text):
    """S-expression tokens of a .kicad_dru: '(', ')', atoms and strings."""
    index, n = 0, len(text)
    while index < n:
        c = text[index]
        if c == '#' and (index == 0 or text[index - 1] == '\n' or text[:index].rsplit('\n', 1)[-1].strip() == ''):
            end = text.find('\n', index)
            index = n if end < 0 else end + 1
        elif c.isspace():
            index += 1
        elif c in '()':
            yield c
            index += 1
        elif c == '"':
            out, index = [], index + 1
            while index < n and text[index] != '"':
                if text[index] == '\\' and index + 1 < n:
                    index += 1
                out.append(text[index])
                index += 1
            index += 1
            yield ('str', ''.join(out))
        else:
            start = index
            while index < n and not text[index].isspace() and text[index] not in '()"':
                index += 1
            yield ('atom', text[start:index])


def parse(text):
    stack, top = [], []
    current = top
    for token in tokens(text):
        if token == '(':
            stack.append(current)
            current = []
        elif token == ')':
            if not stack:
                raise ValueError('unbalanced )')
            done = current
            current = stack.pop()
            current.append(done)
        else:
            current.append(token[1])
    if stack:
        raise ValueError('unbalanced (')
    return top


ATOM = re.compile(
    r"(?P<lhs>\b[AB]{1,2}\.\w+(?:\((?P<args>[^()]*)\))?)\s*(?P<op>==|!=|<=|>=|<|>)\s*"
    r"(?P<rhs>'[^']*'|\"[^\"]*\"|\b[AB]\.\w+(?:\([^()]*\))?|[-\w.]+)")
CALL = re.compile(r"\b(?P<lhs>[AB]{1,2})\.(?P<name>\w+)\((?P<args>[^()]*)\)")


def operand(rhs):
    if rhs[0] in "'\"":
        inner = rhs[1:-1]
        return "'*?'" if any(c in inner for c in '*?') else "'text'"
    if re.match(r'^[AB]\.', rhs):
        return 'other.' + re.sub(r'^[AB]\.', '', rhs)
    return 'number' if re.match(r'^-?[\d.]+\w*$', rhs) else 'word'


def atoms(expression):
    found = []
    consumed = []
    for match in ATOM.finditer(expression):
        prop = re.sub(r'^[AB]{1,2}\.', '', match.group('lhs'))
        prop = re.sub(r'\(.*\)', '()', prop)
        found.append(f"{prop.lower()} {match.group('op')} {operand(match.group('rhs'))}")
        consumed.append(match.span())
    for match in CALL.finditer(expression):
        if any(a <= match.start() < b for a, b in consumed):
            continue
        found.append(f"{match.group('lhs') if match.group('lhs') == 'AB' else ''}{match.group('name').lower()}()")
    return found


def strip_comment_lines(text):
    return '\n'.join(line for line in text.splitlines() if not line.lstrip().startswith('#'))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--data', type=Path, default=ROOT / 'benchmarks/expansion/data')
    parser.add_argument('--json', type=Path)
    arguments = parser.parse_args()
    manifest = json.loads((arguments.data / 'manifest.json').read_text())['boards']
    repo_root = arguments.data.parents[2]

    count = collections.Counter()
    boards = collections.defaultdict(set)

    def note(key, board):
        count[key] += 1
        boards[key].add(board)

    dru_boards = 0
    unparsed = []
    for record in manifest:
        name = record['name']
        directory = repo_root / record['directory']
        # The project.
        project_path = repo_root / record['project_path'] if record.get('project_path') else None
        project = None
        if project_path and project_path.is_file() and record.get('project_sha256'):
            try:
                project = json.loads(project_path.read_text())
            except (ValueError, UnicodeDecodeError):
                note('project: unreadable', name)
        if project is None:
            note('project: none', name)
        else:
            design = (project.get('board') or {}).get('design_settings') or {}
            rules = design.get('rules') or {}
            if not rules:
                note('project: no design_settings.rules', name)
            for key in ('min_track_width', 'min_clearance', 'min_copper_edge_clearance', 'min_hole_clearance',
                        'min_hole_to_hole', 'min_via_diameter', 'min_through_hole_diameter',
                        'min_via_annular_width', 'min_connection'):
                note(f"project rules: {key} {'present' if key in rules else 'missing'}", name)
            if 'min_track_width' in rules and rules['min_track_width'] is not None:
                note(f"project rules: min_track_width = {rules['min_track_width']}", name)
            settings = project.get('net_settings') or {}
            classes = settings.get('classes') or []
            note(f"net classes: {min(len(classes), 5)}{'+' if len(classes) >= 5 else ''}", name)
            for cls in classes:
                if cls.get('name') != 'Default' and cls.get('clearance') is not None:
                    note('net classes: a non-default class with a clearance', name)
                    break
            if any(cls.get('nets') for cls in classes):
                note('net classes: nets listed in a class (KiCad 6/7)', name)
            if any(cls.get('name') != 'Default' and any(cls.get(k) is None for k in ('clearance', 'track_width'))
                   for cls in classes):
                note('net classes: a class without clearance or track_width (KiCad 8+)', name)
            for pattern in settings.get('netclass_patterns') or []:
                text = pattern.get('pattern') or ''
                kind = 'regex' if re.search(r'[.\\\[\]()|+^$]', text.replace('/', '')) and re.search(r'\.\*|\\|\[|\(|\||\+', text) else 'wildcard'
                note(f'netclass pattern: {kind}', name)
            assignments = settings.get('netclass_assignments') or {}
            if assignments:
                note('netclass assignments', name)
                if any(isinstance(v, list) and len(v) > 1 for v in assignments.values()):
                    note('netclass assignments: several classes for one net', name)
            severities = design.get('rule_severities') or {}
            for key in ('clearance', 'track_width', 'copper_edge_clearance', 'hole_to_hole', 'hole_clearance'):
                value = severities.get(key)
                if value and value != 'error':
                    note(f'severity: {key} {value}', name)
        # The custom rules.
        if not record.get('custom_rules_path'):
            continue
        path = repo_root / record['custom_rules_path']
        if not path.is_file():
            note('dru: listed but missing', name)
            continue
        dru_boards += 1
        text = path.read_text(errors='replace')
        try:
            tree = parse(text)
        except ValueError as error:
            unparsed.append((name, str(error)))
            note('dru: unparsable', name)
            continue
        rules = [form for form in tree if isinstance(form, list) and form and form[0] == 'rule']
        if not rules:
            note('dru: no rules (version line only)', name)
        for rule in rules:
            note('dru: rule', name)
            body = [form for form in rule[2:] if isinstance(form, list) and form]
            conditions = [form[1] for form in body if form[0] == 'condition' and len(form) > 1]
            layers = [form[1] for form in body if form[0] == 'layer' and len(form) > 1]
            severity = [form[1] for form in body if form[0] == 'severity' and len(form) > 1]
            for layer in layers:
                note(f'layer: {layer if layer in ("outer", "inner") else "named"}', name)
            for value in severity:
                note(f'rule severity: {value}', name)
            if not conditions:
                note('condition: none', name)
            for condition in conditions:
                found = atoms(condition)
                if not found and condition.strip():
                    note('condition atom: (unrecognised) ' + condition[:60], name)
                for atom in found:
                    note('condition atom: ' + atom, name)
                if '||' in condition:
                    note('condition: uses ||', name)
                if '!' in condition.replace('!=', ''):
                    note('condition: uses !', name)
            for form in body:
                if form[0] != 'constraint' or len(form) < 2:
                    continue
                kind = form[1]
                limits = sorted({part[0] for part in form[2:] if isinstance(part, list) and part})
                words = [part for part in form[2:] if isinstance(part, str)]
                note(f"constraint: {kind} ({', '.join(limits + words) or '-'})", name)
    result = {
        'boards': len(manifest),
        'boards_with_dru_files': dru_boards,
        'unparsed': unparsed,
        'counts': {key: {'occurrences': count[key], 'boards': len(boards[key])}
                   for key in sorted(count, key=lambda k: (k.split(':')[0], -len(boards[k]), k))},
    }
    if arguments.json:
        arguments.json.write_text(json.dumps(result, indent=1) + '\n')
    print(f"{result['boards']} boards, {dru_boards} with a .kicad_dru, {len(unparsed)} unparsable")
    section = None
    for key, value in result['counts'].items():
        head = key.split(':')[0]
        if head != section:
            section = head
            print(f'\n## {head}')
        print(f"{value['boards']:6} boards {value['occurrences']:6} x  {key}")


if __name__ == '__main__':
    main()
