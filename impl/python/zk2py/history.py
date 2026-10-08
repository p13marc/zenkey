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
    """Every problem of a history root, as ``(at, tag)``, in §9.7's order
    (0.5):
    - the root's entries, then each interface directory's, in bytewise
      order of name;
    - a root entry that is not a directory named by an interface id is
      ``directory``, and is not looked into;
    - in an interface directory, an entry not named
      ``<64 lowercase hex>.bundle.json`` is ``file_name``, a subdirectory
      included;
    - per file: reading it (``io``); verifying it, expecting the
      fingerprint its name gives (a bundle tag); its interface against its
      directory (``interface``); its bytes against the JCS of the bundle,
      all three members written (``jcs``). The first two stop that file's
      checks; ``interface`` and ``jcs`` are both reported.
    """
    problems: list[tuple[str, str]] = []
    for d in sorted(root.iterdir(), key=lambda p: p.name.encode()):
        if not d.is_dir() or not is_interface_id(d.name):
            problems.append((d.name, "directory"))
            continue
        for f in sorted(d.iterdir(), key=lambda p: p.name.encode()):
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
            full = {"contract": v.contract, "schemas": v.schemas, "extras": v.extras}
            if jcs.dumps(full) != data:
                problems.append((at, "jcs"))
    return problems
