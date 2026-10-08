"""Set checks over several contracts (core.md §9.2: E035, E036).

"E035 and E036 are set checks: each file is loaded on its own first."
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from .contract import Contract, Diagnostic, load_contract


class SetLoadError(ValueError):
    """A member of the set does not load on its own (an E… code)."""


@dataclass
class SetResult:
    contracts: list[Contract] = field(default_factory=list)
    diagnostics: list[Diagnostic] = field(default_factory=list)
    #: False when a member did not load: its own codes stand, and the set
    #: checks did not run (§9.2 "Set checks", 0.5)
    ran: bool = True

    @property
    def codes(self) -> list[str]:
        return sorted(d.code for d in self.diagnostics)


def check_set(paths: list[Path], spec_dir: Path | None = None) -> SetResult:
    """Load each contract alone, in file-name order, then run E036 and E035
    across them (§9.2 "Set checks")."""
    result = SetResult()
    for p in sorted(paths, key=lambda q: q.name.encode()):
        c = load_contract(p, spec_dir=spec_dir)
        result.contracts.append(c)
    failed = [c for c in result.contracts if not c.valid]
    if failed:
        # "When one does not load, its own codes stand, and the set checks
        # do not run: they need every member."
        result.ran = False
        result.diagnostics = [d for c in failed for d in c.diagnostics]
        return result
    # E036: "two contracts of a set declare one interface id", per duplicate.
    by_iface: dict[str, Contract] = {}
    for c in result.contracts:
        assert c.interface is not None
        if c.interface in by_iface:
            result.diagnostics.append(Diagnostic(
                "E036", f"{c.path.name}: {c.interface} is also declared by {by_iface[c.interface].path.name}"))
        else:
            by_iface[c.interface] = c
    # E035: "a requirement names a resource its interface does not declare,
    # when that interface is in the set", per resource name. A requirement
    # names resources by template (§3.1), and a template is unique within a
    # contract (it is the resources table's key).
    for c in result.contracts:
        for role, req in (c.raw or {}).get("requires", {}).items():
            target = by_iface.get(req["interface"])
            if target is None or req.get("resources") is None:
                continue
            declared = set(target.templates)
            for r in req["resources"]:
                if r not in declared:
                    result.diagnostics.append(Diagnostic(
                        "E035", f"{c.path.name}: requires.{role} names {r!r}, which {target.interface} does not declare"))
    return result
