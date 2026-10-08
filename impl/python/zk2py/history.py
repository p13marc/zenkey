"""The history check (core.md §9.7).

A contract's CI keeps every published bundle at
``contracts/.history/<iface>/<hex>.bundle.json``. The check verifies every
bundle (§9.6), its fingerprint against its file name, its interface against
its directory, and that it is in JCS form. It reports ``[at, tag]`` pairs,
``at`` relative to the history root, "in directory then file order".
"""

from __future__ import annotations

import re
from pathlib import Path

from . import bundle, jcs
from .lexical import is_interface_id

_FILE = re.compile(r"([0-9a-f]{64})\.bundle\.json")


def check_history(root: Path) -> list[tuple[str, str]]:
    """Every problem of a history root, as ``(at, tag)``.

    One problem per file at most: the first of verification (with the
    fingerprint the file name implies, so a mismatch is the bundle tag
    ``fingerprint``), the interface against the directory, then the JCS
    form (SPEC-FINDINGS F-27).
    """
    problems: list[tuple[str, str]] = []
    for d in sorted(root.iterdir(), key=lambda p: p.name):
        if not d.is_dir() or not is_interface_id(d.name):
            problems.append((d.name, "directory"))
            continue
        for f in sorted(d.iterdir(), key=lambda p: p.name):
            at = f"{d.name}/{f.name}"
            m = _FILE.fullmatch(f.name)
            if not f.is_file() or m is None:
                problems.append((at, "file_name"))
                continue
            try:
                data = f.read_bytes()
            except OSError:
                problems.append((at, "io"))
                continue
            try:
                v = bundle.verify(data, expect_fingerprint=f"sha256:{m.group(1)}")
            except bundle.BundleError as e:
                problems.append((at, e.tag))
                continue
            if v.contract.get("interface") != d.name:
                problems.append((at, "interface"))
                continue
            if jcs.dumps(jcs.loads(data)) != data:
                problems.append((at, "jcs"))
    return problems
