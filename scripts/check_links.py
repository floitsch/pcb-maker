#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Checks the relative links of the repository's Markdown files.

A link resolves when its file (or directory) exists and, for a link into a
Markdown file with a fragment, the fragment is one of that file's heading
anchors as GitHub makes them (or an explicit `id`/`name` anchor). Links to
other sites are not fetched.

    python3 scripts/check_links.py                    # README.md, docs/, benchmarks/
    python3 scripts/check_links.py --into handover.md changelog.md
    python3 scripts/check_links.py docs/todo.md       # only these files

`--into NAME...` judges only links whose target file has one of these
names (the gate of the hand-over split); the other broken links are still
counted on the last line. Exit 1 when a judged link is broken.
"""

import argparse
import re
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

LINK = re.compile(r"!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
REFERENCE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s+.*)?$")
HEADING = re.compile(r"^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
HTML_ANCHOR = re.compile(r"""<a\s[^>]*(?:id|name)\s*=\s*["']([^"']+)["']""", re.IGNORECASE)
HTML_ID = re.compile(r"""\sid\s*=\s*["']([^"']+)["']""", re.IGNORECASE)


def prose_lines(text):
    """The lines outside fenced code blocks, with their numbers."""
    fence = None
    for number, line in enumerate(text.split("\n"), 1):
        stripped = line.lstrip()
        marker = re.match(r"(`{3,}|~{3,})", stripped)
        if marker:
            if fence is None:
                fence = marker.group(1)[0] * len(marker.group(1))
                continue
            if stripped.startswith(fence):
                fence = None
                continue
        if fence is None:
            yield number, line


def without_code_spans(line):
    return re.sub(r"(`+)(.+?)\1", lambda m: " " * len(m.group(0)), line)


def heading_text(raw):
    """A heading as GitHub renders it to text: no code ticks, link text only."""
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", raw)
    # HTML tags go, but not inside code spans (`<out-dir>` is text there).
    parts = re.split(r"(`+[^`]*`+)", text)
    return "".join(part.replace("`", "") if part.startswith("`") else re.sub(r"<[^>]+>", "", part)
                   for part in parts)


def slug(text):
    """GitHub's anchor for a heading: lower case, punctuation dropped
    (letters, digits, spaces, hyphens and underscores stay), spaces to
    hyphens."""
    out = []
    for char in text.strip().lower():
        if char in " -_" or char.isalnum() or unicodedata.category(char) in ("Mn", "Mc"):
            out.append("-" if char == " " else char)
    return "".join(out)


_anchors = {}


def anchors(path):
    if path not in _anchors:
        found = set()
        counts = {}
        text = path.read_text(encoding="utf-8")
        for _, line in prose_lines(text):
            match = HEADING.match(line)
            if match:
                base = slug(heading_text(match.group(2)))
                seen = counts.get(base, 0)
                counts[base] = seen + 1
                found.add(base if seen == 0 else f"{base}-{seen}")
            for pattern in (HTML_ANCHOR, HTML_ID):
                found.update(pattern.findall(line))
        _anchors[path] = found
    return _anchors[path]


def links(path):
    text = path.read_text(encoding="utf-8")
    for number, line in prose_lines(text):
        reference = REFERENCE.match(line)
        if reference:
            yield number, reference.group(1)
            continue
        for match in LINK.finditer(without_code_spans(line)):
            yield number, match.group(1)


def check(path):
    """(line, link, problem, target file or None) for every broken link."""
    for number, link in links(path):
        if re.match(r"^[a-z][a-z0-9+.-]*:", link, re.IGNORECASE):
            continue  # http:, https:, mailto:, ...
        target, _, fragment = link.partition("#")
        target_path = path if target == "" else (path.parent / target).resolve()
        if not target_path.exists():
            yield number, link, "no such file", target_path
            continue
        if fragment and target_path.is_file() and target_path.suffix.lower() == ".md":
            if fragment not in anchors(target_path):
                yield number, link, "no such anchor", target_path


def default_files():
    files = [ROOT / "README.md"]
    for folder in ("docs", "benchmarks"):
        files.extend(sorted((ROOT / folder).rglob("*.md")))
    return [f for f in files if "external" not in f.relative_to(ROOT).parts]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("files", nargs="*", type=Path)
    parser.add_argument("--into", nargs="+", metavar="NAME",
                        help="judge only links into Markdown files of these names")
    arguments = parser.parse_args()
    files = [f.resolve() for f in arguments.files] or default_files()

    judged = broken = other = 0
    for path in files:
        for _, link in links(path):
            target = link.partition("#")[0]
            if not arguments.into or Path(target).name in arguments.into or (
                    target == "" and path.name in arguments.into):
                judged += 1
        for number, link, problem, target in check(path):
            name = target.name if target else ""
            if arguments.into and name not in arguments.into:
                other += 1
                continue
            broken += 1
            print(f"{path.relative_to(ROOT)}:{number}: {link}: {problem}")
    scope = f"into {', '.join(arguments.into)}" if arguments.into else "in all"
    note = f"; {other} other broken links not judged" if arguments.into else ""
    print(f"{judged} links {scope} checked in {len(files)} files, {broken} broken{note}")
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
