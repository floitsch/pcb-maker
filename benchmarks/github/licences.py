# Copyright (C) 2026 Toit contributors.
"""Licence classes of harvested boards: which may be copied for benchmarking
(classify_licence, harvest.py's and harvest_schematics.py's gate) and which
may train a model (training_eligible: permissive only).

The position (benchmarks/github/README.md, "Licences"): harvested files are
downloaded for benchmarking only and never redistributed; a board whose
repository names no licence that allows copies (no licence file: all rights
reserved, or a text that reserves the rights) is not fetched unless asked
for private benchmarking (`--allow-unlicensed`), and is marked. Training
data is permissive only: a model trained on a copyleft design (GPL,
CERN-OHL-S, CC-BY-SA) or a weakly reciprocal one (CERN-OHL-W, LGPL, MPL)
may carry its obligations, which nobody here has settled, so such boards
are benchmarks, never training data."""

import re

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

# The classes, by SPDX identifier (an "-only" or "-or-later" suffix counts
# as the bare identifier).
PERMISSIVE_SPDX = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "ISC", "Unlicense", "CC0-1.0",
                   "CC-BY-4.0", "CC-BY-3.0", "CERN-OHL-P-2.0", "WTFPL", "BSL-1.0", "Zlib", "SHL-0.51", "SHL-2.1"}
WEAK_COPYLEFT_SPDX = {"LGPL-2.1", "LGPL-3.0", "MPL-2.0", "CERN-OHL-W-2.0"}
COPYLEFT_SPDX = {"GPL-2.0", "GPL-3.0", "AGPL-3.0", "CC-BY-SA-4.0", "CC-BY-SA-3.0", "CERN-OHL-S-2.0",
                 "CERN-OHL-1.2", "EUPL-1.2", "TAPR-OHL-1.0"}
# Grants found in licence texts (GRANTS' names) by class; the rest (CERN
# OHL and Creative Commons of an unread variant, Solderpad, "open hardware
# licence") are unclassified: allowed for benchmarking, not for training.
PERMISSIVE_GRANTS = {"MIT-style", "BSD-style", "Apache", "Unlicense", "public domain"}
WEAK_COPYLEFT_GRANTS = {"LGPL", "MPL"}
COPYLEFT_GRANTS = {"GPL", "AGPL", "free software licence"}
RESTRICTED_GRANTS = {"Hippocratic License"}

KINDS = ("permissive", "weak-copyleft", "copyleft", "unclassified", "restricted", "unlicensed")


def classify_licence(spdx, texts):
    """(may it be copied, why). `texts`: (path, text) of licence files."""
    if spdx and spdx != "NOASSERTION":
        if bare(spdx) in OPEN_SPDX:
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


def bare(spdx):
    return re.sub(r"-(only|or-later)$", "", spdx or "")


def kind(spdx, allowed, why):
    """The licence's class (KINDS) from classify_licence's verdict: by the
    SPDX identifier where GitHub names one, else by the grant the licence
    file showed."""
    if not allowed:
        return "unlicensed"
    identifier = bare(spdx) if spdx and spdx != "NOASSERTION" else None
    if identifier:
        for kinds, name in ((PERMISSIVE_SPDX, "permissive"), (WEAK_COPYLEFT_SPDX, "weak-copyleft"),
                            (COPYLEFT_SPDX, "copyleft")):
            if identifier in kinds:
                return name
        return "unclassified"
    if "(non-commercial)" in why:
        return "restricted"
    grant = why.split(": ", 1)[-1]
    for grants, name in ((PERMISSIVE_GRANTS, "permissive"), (WEAK_COPYLEFT_GRANTS, "weak-copyleft"),
                         (COPYLEFT_GRANTS, "copyleft"), (RESTRICTED_GRANTS, "restricted")):
        if grant in grants:
            return name
    return "unclassified"


def record(spdx, texts=None, verdict=None):
    """The licence record of a board: `allowed` (may be copied for
    benchmarking), `why`, `kind` and `training` (permissive only). `verdict`
    is an earlier classify_licence result (allowed, why, files) to reuse."""
    if verdict is None:
        allowed, why = classify_licence(spdx, texts or [])
        files = [path for path, _ in texts or []]
    else:
        allowed, why, files = verdict
    licence_kind = kind(spdx, allowed, why)
    return {"spdx": spdx, "allowed": allowed, "why": why, "files": files, "kind": licence_kind,
            "training": training_eligible(licence_kind)}


def training_eligible(licence_kind):
    """Whether a board of this class may be training data: permissive only."""
    return licence_kind == "permissive"


def spdx_training_eligible(spdx):
    """training_eligible from a GitHub SPDX identifier alone (no licence
    file read: NOASSERTION and none are not eligible)."""
    return bare(spdx) in PERMISSIVE_SPDX
