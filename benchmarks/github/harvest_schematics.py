#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Fetches the schematic side of the harvested GitHub boards.

    benchmarks/github/harvest_schematics.py [--only NAME...] [--refresh] [--jobs N]

For every board of manifest.json (harvest.py), at the commit the board was
taken from (origin.json): the root schematic `<board_id>.kicad_sch` and
every sheet it names (recursively), the project file and its custom rules
(`.kicad_dru`), the project's library tables (`fp-lib-table`,
`sym-lib-table`), the symbol libraries those tables keep in the repository
(`.kicad_sym`, `.lib`) and, of the footprint libraries the tables (or
`<library>.pretty` folders of the project) keep in the repository, the
footprints the schematic's symbols name. Not fetched: footprints no symbol
names, KiCad's own libraries (on this machine), 3D models.

Files go to benchmarks/real/external/github-sch/<board>/repo/ under their
paths in the repository (gitignored like the boards: downloaded for
benchmarking only, never redistributed, and never run: they are data),
with `schematic.json` next to `repo/`: origin, the project's folder in
`repo/`, sheets, libraries, what could not be resolved, size and licence.
benchmarks/github/schematics.json (committed) has the same per board
without the files, and the boards skipped with why.

A board is skipped when its schematic is missing at the commit (or only a
KiCad 5 `.sch` is there, or a sheet it names is missing, or a file is a Git
LFS pointer), or when its licence does not let it be copied: no licence
file at all (all rights reserved), or a licence text that reserves the
rights. GitHub's own detection (`license` in manifest.json) decides where it
names a licence; for `NOASSERTION` and none, the licence files at the
repository's root and along the board's folder are read.

The schematic tier (benchmarks/agent-tasks/run.py --tier schematic) runs
`import-kicad-netlist` on `repo/<project_dir>/<board_id>.kicad_sch`."""

import argparse
import json
import posixpath
import re
import sys
import time
import urllib.parse
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from harvest import gh  # noqa: E402

ROOT = HERE.parents[1]
STORE = ROOT / "benchmarks/real/external/github-sch"
MANIFEST = HERE / "manifest.json"
SUMMARY = HERE / "schematics.json"
SYSTEM_FOOTPRINTS = Path("/usr/share/kicad/footprints")
# Largest file fetched (symbol libraries of whole companies run to tens of
# MB; the schematic does not need them, its symbols are embedded).
MAX_FILE_BYTES = 30_000_000

# Licences GitHub names that let the files be copied.
OPEN_SPDX = {"MIT", "Apache-2.0", "GPL-2.0", "GPL-3.0", "LGPL-2.1", "LGPL-3.0", "AGPL-3.0", "BSD-2-Clause",
             "BSD-3-Clause", "0BSD", "Unlicense", "CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0", "CC-BY-3.0",
             "CC-BY-SA-3.0", "MPL-2.0", "ISC", "CERN-OHL-P-2.0", "CERN-OHL-W-2.0", "CERN-OHL-S-2.0",
             "CERN-OHL-1.2", "WTFPL", "BSL-1.0", "Zlib", "EUPL-1.2", "SHL-0.51", "SHL-2.1", "TAPR-OHL-1.0"}
# Words of licence texts GitHub does not recognise: grants, and the
# reservations that withhold one.
GRANTS = [("CERN Open Hardware", "CERN OHL"), ("CERN OHL", "CERN OHL"), ("CERN-OHL", "CERN OHL"),
          ("GNU Affero General Public", "AGPL"), ("you can redistribute it", "free software licence"), ("Creative Commons", "Creative Commons"),
          ("creativecommons.org", "Creative Commons"), ("Permission is hereby granted", "MIT-style"),
          ("GNU GENERAL PUBLIC LICENSE", "GPL"), ("GNU LESSER GENERAL PUBLIC", "LGPL"),
          ("Apache License", "Apache"), ("Solderpad", "Solderpad"), ("TAPR Open Hardware", "TAPR OHL"),
          ("Redistribution and use in source and binary forms", "BSD-style"), ("Mozilla Public License", "MPL"),
          ("unlicense.org", "Unlicense"), ("dedicated to the public domain", "public domain"),
          ("released into the public domain", "public domain"), ("CC BY", "Creative Commons"),
          ("CC-BY", "Creative Commons"), ("Open Hardware License", "open hardware licence"),
          ("HIPPOCRATIC LICENSE", "Hippocratic License")]
RESERVATIONS = ["all rights reserved", "may not be redistributed", "may not be copied", "proprietary",
                "not for redistribution", "without the prior written permission"]
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying)([._-].*)?$", re.IGNORECASE)


def raw(repository, commit, path):
    """A file's bytes at the commit: raw.githubusercontent.com, else the
    git blob through the API."""
    url = f"https://raw.githubusercontent.com/{repository}/{commit}/{urllib.parse.quote(path)}"
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=120) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None
            time.sleep(5 * (attempt + 1))
        except Exception:  # noqa: BLE001
            time.sleep(5 * (attempt + 1))
    return None


def block_end(text, start):
    depth, quoted, escaped = 0, False, False
    for index in range(start, len(text)):
        character = text[index]
        if escaped:
            escaped = False
        elif quoted and character == "\\":
            escaped = True
        elif character == '"':
            quoted = not quoted
        elif not quoted:
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    return index + 1
    return len(text)


def without_library_symbols(text):
    """The schematic without its embedded `(lib_symbols ...)`: their
    Footprint fields are the libraries' defaults, not the parts'."""
    start = text.find("(lib_symbols")
    if start < 0:
        return text
    return text[:start] + text[block_end(text, start):]


def sheet_files(text):
    """The files of the sheets a schematic names (KiCad 7+ `Sheetfile`,
    KiCad 6 `Sheet file`)."""
    return re.findall(r'\(property\s+"Sheet ?file"\s+"([^"]+)"', text)


def footprint_ids(text):
    return set(re.findall(r'\(property\s+"Footprint"\s+"([^"]+:[^"]+)"', without_library_symbols(text)))


def library_entries(text):
    """(name, type, uri) of a library table."""
    entries = []
    for lib in re.finditer(r"\(lib\s", text):
        block = text[lib.start():block_end(text, lib.start())]
        fields = {}
        for key in ("name", "type", "uri"):
            match = re.search(r'\(' + key + r'\s+(?:"((?:[^"\\]|\\.)*)"|([^\s)]+))\)', block)
            if match:
                fields[key] = match.group(1) if match.group(1) is not None else match.group(2)
        if "name" in fields and "uri" in fields:
            entries.append((fields["name"], fields.get("type", "KiCad"), fields["uri"]))
    return entries


def resolve(uri, project_dir):
    """A table's URI as a path in the repository (posix, no leading ./), or
    None when it is outside the project's own files (KiCad's libraries,
    another variable, an absolute path)."""
    match = re.match(r"^\$[{(]KIPRJMOD[})](.*)$", uri)
    if not match:
        return None
    path = posixpath.normpath(posixpath.join(project_dir or ".", match.group(1).lstrip("/\\").replace("\\", "/")))
    if path.startswith(".."):
        return None
    return "" if path == "." else path


def classify_licence(spdx, texts):
    """(may it be copied, why). `texts`: (path, text) of licence files."""
    if spdx and spdx != "NOASSERTION":
        if spdx in OPEN_SPDX:
            return True, spdx
        return False, f"{spdx} is not a licence known to allow copies"
    if not texts:
        return False, "no licence file (all rights reserved)"
    for path, text in texts:
        lower = " ".join(text.lower().split())
        grant = next((name for key, name in GRANTS if key.lower() in lower), None)
        if grant:
            # A grant wins over "all rights reserved" (copyright lines of
            # open licences say it too).
            # Creative Commons' NC variants (the GPL says "noncommercially"
            # too, of something else).
            noncommercial = grant == "Creative Commons" and "noncommercial" in lower.replace("-", "")
            return True, f"{path}: {grant}" + (" (non-commercial)" if noncommercial else "")
    path, text = texts[0]
    reserved = next((word for word in RESERVATIONS if word in " ".join(text.lower().split())), None)
    return False, f"{path}: " + (f'"{reserved}"' if reserved else "no licence grant recognised")


def harvest(board, refresh):
    name, repository, commit = board["name"], board["repository"], board["commit"]
    board_path = board["path"]
    project_dir = posixpath.dirname(board_path)
    stem = board["board_id"]
    target = STORE / name
    record = {"name": name, "repository": repository, "commit": commit, "board_id": stem,
              "project_dir": project_dir, "spdx": board.get("license")}
    if (target / "schematic.json").exists() and not refresh:
        return json.loads((target / "schematic.json").read_text())
    tree = gh(f"repos/{repository}/git/trees/{commit}?recursive=1")
    if not tree or "tree" not in tree:
        record.update(status="skipped", reason="repository tree unreadable at the commit")
        return record
    if tree.get("truncated"):
        record["tree_truncated"] = True
    sizes = {entry["path"]: entry.get("size", 0) for entry in tree["tree"] if entry["type"] == "blob"}
    folders = {entry["path"] for entry in tree["tree"] if entry["type"] == "tree"}
    # Libraries are often Git submodules (another repository at a pinned
    # commit): their trees join this one under the submodule's path.
    origins = {}
    submodules = [entry for entry in tree["tree"] if entry["type"] == "commit"]
    if submodules and ".gitmodules" in sizes:
        modules = (raw(repository, commit, ".gitmodules") or b"").decode("utf-8", errors="replace")
        urls = {}
        for section in re.split(r"\[submodule ", modules):
            path = re.search(r"^\s*path\s*=\s*(.+?)\s*$", section, re.MULTILINE)
            url = re.search(r"^\s*url\s*=\s*(.+?)\s*$", section, re.MULTILINE)
            if path and url:
                urls[path.group(1)] = url.group(1)
        record["submodules"] = {}
        for entry in submodules:
            url = urls.get(entry["path"], "")
            if url.startswith("../"):
                url = f"https://github.com/{posixpath.normpath(posixpath.join(repository, url))}"
            match = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?/?$", url)
            if not match:
                record["submodules"][entry["path"]] = f"{url or 'no url'}: not on GitHub, not fetched"
                continue
            other = match.group(1)
            subtree = gh(f"repos/{other}/git/trees/{entry['sha']}?recursive=1")
            if not subtree or "tree" not in subtree:
                record["submodules"][entry["path"]] = f"{other}@{entry['sha'][:8]}: tree unreadable"
                continue
            record["submodules"][entry["path"]] = f"{other}@{entry['sha']}"
            origins[entry["path"]] = (other, entry["sha"])
            folders.add(entry["path"])
            for item in subtree["tree"]:
                full = f"{entry['path']}/{item['path']}"
                if item["type"] == "blob":
                    sizes[full] = item.get("size", 0)
                elif item["type"] == "tree":
                    folders.add(full)

    def download(path):
        """A file of the repository or of one of its submodules."""
        for prefix, (other, sha) in origins.items():
            if path.startswith(prefix + "/"):
                return raw(other, sha, path[len(prefix) + 1:])
        return raw(repository, commit, path)

    # The licence.
    ancestors = [""]
    for part in project_dir.split("/") if project_dir else []:
        ancestors.append(posixpath.join(ancestors[-1], part) if ancestors[-1] else part)
    texts = []
    if not board.get("license") or board.get("license") == "NOASSERTION":
        for folder in ancestors:
            for path in sorted(sizes):
                if posixpath.dirname(path) == folder and LICENCE_FILE.match(posixpath.basename(path)) \
                        and sizes[path] < 300_000:
                    data = raw(repository, commit, path)
                    if data:
                        texts.append((path, data.decode("utf-8", errors="replace")))
    allowed, why = classify_licence(board.get("license"), texts)
    record["licence"] = {"allowed": allowed, "why": why, "files": [path for path, _ in texts]}

    root = posixpath.join(project_dir, f"{stem}.kicad_sch") if project_dir else f"{stem}.kicad_sch"
    if root not in sizes:
        legacy = root[:-len(".kicad_sch")] + ".sch"
        others = sorted(p for p in sizes if posixpath.dirname(p) == project_dir and p.endswith(".kicad_sch"))
        if legacy in sizes:
            reason = "only a KiCad 5 schematic (.sch)"
        elif others:
            reason = f"no {stem}.kicad_sch at the commit (the folder has {', '.join(posixpath.basename(p) for p in others[:3])})"
        else:
            reason = f"no {stem}.kicad_sch at the commit"
        record.update(status="skipped", reason=reason)
        return record
    if not allowed:
        record.update(status="skipped", reason=f"licence: {why}")
        return record

    files = {}
    problems = []

    def fetch(path):
        if path in files:
            return files[path]
        if sizes.get(path, 0) > MAX_FILE_BYTES:
            problems.append(f"{path}: {sizes[path]} bytes, not fetched")
            files[path] = None
            return None
        data = download(path)
        if data is not None and data.startswith(b"version https://git-lfs"):
            problems.append(f"{path}: a Git LFS pointer")
            record["lfs"] = True
            data = None
        files[path] = data
        return data

    # Sheets, recursively. KiCad resolves a sheet's file relative to the
    # project folder; older files relative to the parent sheet.
    sheets, queue, missing_sheets = [], [root], []
    while queue:
        sheet = queue.pop()
        if sheet in sheets:
            continue
        data = fetch(sheet)
        if data is None:
            missing_sheets.append(sheet)
            continue
        sheets.append(sheet)
        for name_ in sheet_files(data.decode("utf-8", errors="replace")):
            name_ = name_.replace("\\", "/")
            candidates = [posixpath.normpath(posixpath.join(project_dir or ".", name_)),
                          posixpath.normpath(posixpath.join(posixpath.dirname(sheet) or ".", name_))]
            found = next((c for c in candidates if c in sizes), None)
            if found is None:
                missing_sheets.append(candidates[0])
            elif found not in sheets:
                queue.append(found)
    record["sheets"] = sheets
    if missing_sheets:
        record["missing_sheets"] = sorted(set(missing_sheets))
    for suffix in (".kicad_pro", ".kicad_dru"):
        path = root[:-len(".kicad_sch")] + suffix
        if path in sizes:
            fetch(path)
    footprints = set()
    for sheet in sheets:
        footprints |= footprint_ids(files[sheet].decode("utf-8", errors="replace"))
    record["footprint_ids"] = len(footprints)

    # Library tables and the libraries they keep in the repository.
    local_footprint_libraries = {}
    table_uris = {}
    unresolved = []
    record["symbol_libraries"] = []
    for table in ("fp-lib-table", "sym-lib-table"):
        path = posixpath.join(project_dir, table) if project_dir else table
        if path not in sizes:
            continue
        data = fetch(path)
        if data is None:
            continue
        for library, kind, uri in library_entries(data.decode("utf-8", errors="replace")):
            location = resolve(uri, project_dir)
            if table == "fp-lib-table":
                table_uris[library] = uri
                if location is not None and location in folders:
                    local_footprint_libraries[library] = location
                elif location is not None:
                    unresolved.append(f"footprint library {library}: {uri} not in the repository")
                elif not (SYSTEM_FOOTPRINTS / f"{library}.pretty").is_dir() and kind != "Table":
                    unresolved.append(f"footprint library {library}: {uri}")
                if kind not in ("KiCad", "Table"):
                    problems.append(f"footprint library {library} is of type {kind}")
            else:
                if location is not None and location in sizes:
                    fetch(location)
                    record["symbol_libraries"].append(location)
                elif location is not None:
                    unresolved.append(f"symbol library {library}: {uri} not in the repository")
    # Footprint folders of the project without a table entry (the import
    # finds them up to three folders down).
    for folder in sorted(folders):
        relative = folder[len(project_dir) + 1:] if project_dir else folder
        if (not project_dir or folder.startswith(project_dir + "/")) and folder.endswith(".pretty") \
                and relative.count("/") < 3:
            local_footprint_libraries.setdefault(posixpath.basename(folder)[:-len(".pretty")], folder)
    record["footprint_libraries"] = local_footprint_libraries

    # Where else in the repository (submodules included) a footprint is:
    # what a designer's absolute path or a library outside the project
    # folder points to. The schematic tier adds these to the project's
    # fp-lib-table after the import reports them missing, as an agent
    # would (docs/agents.md, section 0), and imports again.
    files_by_name = {}
    for path in sizes:
        if path.endswith(".kicad_mod"):
            files_by_name.setdefault(posixpath.basename(path)[:-len(".kicad_mod")], []).append(path)
    pretty = {}
    for folder in folders:
        if folder.endswith(".pretty"):
            pretty.setdefault(posixpath.basename(folder)[:-len(".pretty")], []).append(folder)
    wanted = {}
    for footprint in footprints:
        library, _, name_ = footprint.partition(":")
        wanted.setdefault(library, set()).add(name_)
    fixes = {}

    def relocate(library, names):
        """The repository folder that holds library `library`, or None."""
        candidates = []
        uri = table_uris.get(library, "").replace("\\", "/")
        parts = [part for part in re.sub(r"^\$[{(][^})]*[})]", "", uri).split("/") if part and part != "."]
        for start in range(len(parts)):
            suffix = "/".join(parts[start:])
            candidates += [folder for folder in folders if folder == suffix or folder.endswith("/" + suffix)]
            if candidates:
                break
        candidates += pretty.get(library, [])
        holders = {}
        for name_ in names:
            for path in files_by_name.get(name_, []):
                holders[posixpath.dirname(path)] = holders.get(posixpath.dirname(path), 0) + 1
        candidates += sorted(holders, key=lambda folder: -holders[folder])
        # The folder with most of the names; among equals the one nearest
        # the project (a repository of several boards keeps a copy of a
        # library per board), then by name.
        def near(folder):
            common = posixpath.commonpath([folder, project_dir]) if project_dir else ""
            return len(common.split("/")) if common else 0

        best = max(sorted(set(candidates)),
                   key=lambda folder: (sum(f"{folder}/{n}.kicad_mod" in sizes for n in names), near(folder)),
                   default=None)
        if best is None or not any(f"{best}/{n}.kicad_mod" in sizes for n in names):
            return None
        return best

    fetched_footprints, absent, system, nowhere, relocated = 0, [], 0, [], 0
    for library, names in sorted(wanted.items()):
        folder = local_footprint_libraries.get(library)
        if folder is None and (SYSTEM_FOOTPRINTS / f"{library}.pretty").is_dir():
            system += len(names)
            continue
        moved = False
        if folder is None:
            folder = relocate(library, names)
            if folder is not None:
                fixes[library] = folder
                moved = True
        for name_ in sorted(names):
            path = f"{folder}/{name_}.kicad_mod" if folder else None
            if path in sizes and fetch(path) is not None:
                fetched_footprints += 1
                relocated += moved
                continue
            # Not where the library is: the same name elsewhere (the import
            # takes a footprint of that name from any library it knows).
            elsewhere = files_by_name.get(name_, [])
            if elsewhere and fetch(elsewhere[0]) is not None:
                holder = posixpath.dirname(elsewhere[0])
                nickname = posixpath.basename(holder).removesuffix(".pretty") or "repository"
                if holder not in local_footprint_libraries.values() and holder not in fixes.values():
                    while nickname in fixes or nickname in local_footprint_libraries:
                        nickname += "_"
                    fixes[nickname] = holder
                fetched_footprints += 1
                relocated += 1
            elif folder is not None and folder in local_footprint_libraries.values():
                absent.append(f"{library}:{name_}")
            else:
                nowhere.append(f"{library}:{name_}")
    if fixes:
        record["library_fixes"] = dict(sorted(fixes.items()))
    record["footprints"] = {"named": len(footprints), "fetched": fetched_footprints, "kicad": system,
                            "relocated": relocated, "not_in_their_library": absent, "no_library": nowhere}
    if unresolved:
        record["unresolved_libraries"] = sorted(set(unresolved))
    if problems:
        record["problems"] = problems
    if missing_sheets:
        record.update(status="skipped", reason=f"sheets missing: {', '.join(sorted(set(missing_sheets))[:3])}")
    elif record.get("lfs"):
        record.update(status="skipped", reason="Git LFS pointers instead of files")
    else:
        record["status"] = "ok"
    record["files"] = len([data for data in files.values() if data is not None])
    record["size"] = sum(len(data) for data in files.values() if data is not None)
    if record["status"] == "ok":
        if target.exists():
            import shutil
            shutil.rmtree(target)
        for path, data in files.items():
            if data is None:
                continue
            out = target / "repo" / path
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(data)
        record["schematic"] = f"repo/{root}"
        (target / "schematic.json").write_text(json.dumps(record, indent=1))
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--refresh", action="store_true", help="fetch again boards already fetched")
    parser.add_argument("--jobs", type=int, default=6)
    arguments = parser.parse_args()
    boards = json.loads(MANIFEST.read_text())["boards"]
    if arguments.only:
        boards = [board for board in boards if board["name"] in arguments.only]
    STORE.mkdir(parents=True, exist_ok=True)
    summary = json.loads(SUMMARY.read_text()) if SUMMARY.exists() else {"boards": {}}

    def one(board):
        try:
            return harvest(board, arguments.refresh)
        except Exception as error:  # noqa: BLE001
            return {"name": board["name"], "status": "skipped", "reason": f"harvester: {type(error).__name__}: {error}"}

    with ThreadPoolExecutor(arguments.jobs) as pool:
        for record in pool.map(one, boards):
            summary["boards"][record["name"]] = record
            line = record["status"] if record["status"] == "ok" else f"skipped: {record['reason']}"
            extra = ""
            if record["status"] == "ok":
                f = record["footprints"]
                extra = (f" ({len(record['sheets'])} sheets, {record['size'] // 1024} KB, footprints {f['named']}: "
                         f"{f['fetched']} fetched ({f['relocated']} found elsewhere), {f['kicad']} KiCad's, {len(f['not_in_their_library'])} absent, "
                         f"{len(f['no_library'])} without a library)")
            print(f"{record['name']}: {line}{extra}", file=sys.stderr)
    usable = sorted(name for name, record in summary["boards"].items() if record["status"] == "ok")
    summary["comment"] = ("Schematics of the harvested boards (harvest_schematics.py): per board what was fetched "
                          "into benchmarks/real/external/github-sch (gitignored), or why it was skipped.")
    summary["usable"] = len(usable)
    summary["skipped"] = len(summary["boards"]) - len(usable)
    summary["boards"] = dict(sorted(summary["boards"].items()))
    SUMMARY.write_text(json.dumps(summary, indent=1) + "\n")
    print(f"{len(usable)} of {len(summary['boards'])} boards have a usable schematic", file=sys.stderr)


if __name__ == "__main__":
    main()
